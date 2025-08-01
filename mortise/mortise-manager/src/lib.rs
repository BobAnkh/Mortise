pub mod core;
pub mod ipc;
pub mod object;
mod private;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use futures::SinkExt;
use libbpf_rs::{MapFlags as BpfMapFlags, RingBufferBuilder as BpfRingBufferBuilder};
use mortise_common::op::PyOperation;
use mortise_common::{
    FlowOperation, ManagerIpcOperation, ManagerOperation, MortiseError, Operation, Result,
};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_util::codec::LengthDelimitedCodec;

pub use crate::core::*;
pub use crate::ipc::handle_uds;
pub use crate::object::*;

pub const MORTISE_SOCK_PATH: &str = "/tmp/mortise.sock";
pub const MORTISE_PY_PATH: &str = "/tmp/mortise-py.sock";

pub async fn connect_py() -> Option<mpsc::UnboundedSender<Vec<u8>>> {
    match UnixStream::connect(MORTISE_PY_PATH).await.ok() {
        Some(stream) => {
            let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
            tokio::spawn(async move {
                let mut writer = LengthDelimitedCodec::builder()
                    .length_field_type::<u32>()
                    .new_write(stream);
                loop {
                    match rx.recv().await {
                        Some(data) => {
                            writer.send(data.into()).await.unwrap();
                        }
                        None => {
                            tracing::warn!(target: "manager", "Disconnect from python process server");
                            break;
                        }
                    }
                }
            });
            Some(tx)
        }
        None => {
            tracing::warn!(target: "manager", "Fail to connect to python process server");
            None
        }
    }
}

fn handle_op(
    m: &mut MortiseManager,
    op: Operation,
    tx: &mpsc::Sender<ManagerIpcOperation>,
    py_con: &Option<mpsc::UnboundedSender<Vec<u8>>>,
) -> Result<Vec<u8>> {
    match op {
        Operation::Manager(op) => match op {
            ManagerOperation::Load { path, option } => {
                let obj_id = m.open_and_load_object(path, option);
                match &obj_id {
                    Ok(ref inner_id) => {
                        tracing::info!(target: "manager", "Load object with id {}", inner_id)
                    }
                    Err(ref e) => tracing::error!(target: "manager", "Fail to load object: {}", e),
                }
                obj_id.map(|id| id.to_be_bytes().to_vec())
            }
            ManagerOperation::Unload { obj_id } => {
                let res = m.unload_object(obj_id);
                match &res {
                    Ok(_) => tracing::info!(target: "manager", "Unload object with id {}", obj_id),
                    Err(ref e) => {
                        tracing::error!(target: "manager", "Fail to unload object: {}", e)
                    }
                }
                res.map(|_| Vec::new())
            }
            ManagerOperation::Insert {
                obj_id,
                path,
                option,
            } => {
                let res = m.insert_and_load_object(obj_id, path, option);
                match &res {
                    Ok(_) => tracing::info!(target: "manager", "Insert object with id {}", obj_id),
                    Err(ref e) => {
                        tracing::error!(target: "manager", "Fail to insert object: {}", e)
                    }
                }
                res.map(|_| Vec::new())
            }
            ManagerOperation::Shutdown => {
                // Here we do nothing, since all Shutdown operations are hijacked before entering this function.
                // m.showdown().unwrap();
                // tracing::info!("All struct_ops destroyed!");
                // break;
                Ok(Vec::new())
            }
            ManagerOperation::PingPong => {
                tracing::info!(target: "manager", "Ping-Pong");
                Ok(Vec::new())
            }
            ManagerOperation::RegisterRingBuf { obj_ids } => match m.get_rb_map_handles(&obj_ids) {
                Ok(map_hds) => {
                    tracing::info!(target: "manager", "Register RingBuf for obj_id: {:?}", obj_ids);
                    m.unregister_rb()?;
                    // TODO: can merge the logic below into register_rb()
                    let mut rb = BpfRingBufferBuilder::new();
                    for handle in map_hds.iter() {
                        let mut conn = py_con.clone();
                        let mut tx = tx.clone();
                        let handle_event =
                            move |data: &[u8]| handle_report(data, &mut tx, &mut conn);
                        rb.add(handle, handle_event)?;
                    }
                    let rb = rb.build().unwrap();
                    let notify_inner = Arc::new(AtomicBool::new(true));
                    let notify_manager = notify_inner.clone();
                    let t = thread::Builder::new()
                        .name("rb-manager".to_string())
                        .spawn(move || loop {
                            rb.poll(Duration::from_millis(200)).unwrap();
                            if !notify_inner.load(std::sync::atomic::Ordering::Relaxed) {
                                tracing::info!(target: "manager", "Stop RingBuf manager thread");
                                break;
                            }
                        })?;
                    m.register_rb(notify_manager, t)?;
                    Ok(Vec::new())
                }
                Err(e) => {
                    tracing::error!(target: "manager", "Fail to register RingBuf: {}", e);
                    Err(e)
                }
            },
            ManagerOperation::UnregisterRingBuf => {
                tracing::info!(target: "manager", "Unregister RingBuf");
                m.unregister_rb()?;
                Ok(Vec::new())
            }
        },
        Operation::Flow { flow_id, op } => match op {
            FlowOperation::SkStgMapUpdate {
                map_name,
                val,
                flag,
            } => {
                let metadata = m
                    .get_flow_metadata(flow_id)
                    .ok_or(MortiseError::FlowNotFound(flow_id))?;
                let obj_id = metadata.obj_id;
                let key = metadata.local_sk_fd;
                let bpf_flag = BpfMapFlags::from_bits(flag).ok_or(MortiseError::InvalidBpfFlags);
                match bpf_flag {
                    Ok(flag) => {
                        let res = m.update_map(obj_id, map_name, &key.to_ne_bytes(), &val, flag);
                        res.map(|_| Vec::new())
                    }
                    Err(e) => {
                        tracing::error!(target: "manager:flow", "{}", e);
                        Err(e)
                    }
                }
            }
            FlowOperation::SkStgMapLookup { map_name } => {
                let metadata = m
                    .get_flow_metadata(flow_id)
                    .ok_or(MortiseError::FlowNotFound(flow_id))?;
                let obj_id = metadata.obj_id;
                let key = metadata.local_sk_fd;
                m.lookup_map(obj_id, map_name, &key.to_ne_bytes())
            }
            FlowOperation::Connect {
                obj_id,
                sk_fd,
                pid,
                default_app_info,
            } => {
                let res = m.connect(pid, obj_id, sk_fd, default_app_info);
                let flow_id = res?;
                let r = serde_json::to_vec(&PyOperation::Connect { flow_id }).unwrap();
                if let Some(ref con) = py_con {
                    tracing::info!(target: "manager:flow", "Connect flow {} to py", flow_id);
                    con.send(r).unwrap();
                }
                // res.map(|flow_id| flow_id.to_be_bytes().to_vec())
                Ok(flow_id.to_be_bytes().to_vec())
            }
            FlowOperation::Disconnect => {
                let res = m.disconnect(flow_id);
                let r = serde_json::to_vec(&PyOperation::Disconnect { flow_id }).unwrap();
                if let Some(ref con) = py_con {
                    con.send(r).unwrap();
                }
                res.map(|_| Vec::new())
            }
            FlowOperation::QoEUpdate { .. } => Ok(Vec::new()),
        },
    }
}

pub fn manager(
    tx: mpsc::Sender<ManagerIpcOperation>,
    mut rx: mpsc::Receiver<ManagerIpcOperation>,
    py_con: Option<mpsc::UnboundedSender<Vec<u8>>>,
) {
    let mut m = MortiseManager::new();
    loop {
        match rx.blocking_recv() {
            None
            | Some(ManagerIpcOperation {
                req: Operation::Manager(ManagerOperation::Shutdown),
                resp: _,
            }) => {
                m.shutdown().unwrap();
                tracing::info!(target: "manager:shutdown", "All struct_ops destroyed!");
                break;
            }
            Some(ManagerIpcOperation { req, resp }) => {
                let res = handle_op(&mut m, req, &tx, &py_con);
                if let Err(ref e) = res {
                    tracing::error!(target: "manager", "{}", e);
                }
                resp.send(res).unwrap();
            }
        }
    }
}

fn handle_report(
    data: &[u8],
    _tx: &mut mpsc::Sender<ManagerIpcOperation>,
    py_con: &mut Option<mpsc::UnboundedSender<Vec<u8>>>,
) -> i32 {
    // data is stored as little-endian
    // let len = data.len();
    // tracing::info!("data bytes: len {}, content {:?}", len, data);

    // We can directly pass the data to python process server
    if let Some(conn) = py_con {
        tracing::trace!("data bytes: len {}, content {:?}", data.len(), data);
        conn.send(data.into()).unwrap();
    }

    // We can also parse the data
    // let data = ReportEntry::from_bytes(data);
    // tracing::info!(target: "manager:flow", "Receive report data: {:?}", data);
    // let (tmp_tx, tmp_rx) = oneshot::channel::<Result<Vec<u8>>>();
    // let op = ManagerIpcOperation {
    //     req: Operation::Manager(ManagerOperation::PingPong),
    //     resp: tmp_tx,
    // };
    // tx.blocking_send(op).unwrap();
    // let r = tmp_rx.blocking_recv().unwrap().unwrap();
    // tracing::info!(target: "manager:flow", "Ping {:?}", r);
    0
}
