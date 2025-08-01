use std::collections::HashMap;

use clap::ValueEnum;
use futures::{SinkExt, StreamExt};
use libbpf_rs::MapFlags as BpfMapFlags;
use mortise_common::qoe::{AppInfo, FrameQoE};
use mortise_common::{read_be_u32, FlowOperation};
use tokio::net::UnixStream;
use tokio::sync::mpsc::Receiver;
use tokio::sync::oneshot;
use tokio_util::codec::LengthDelimitedCodec;

pub mod transport;
pub mod utils;
pub use transport::io::*;
pub use utils::*;

#[derive(ValueEnum, Clone, Debug)]
#[clap(rename_all = "lower")]
pub enum AppOpt {
    Video,
    Bulk,
    WebRTC,
}

#[derive(ValueEnum, Clone, Debug, PartialEq)]
pub enum ModeOpt {
    Origin,
    Mortise,
}

impl std::fmt::Display for ModeOpt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModeOpt::Origin => write!(f, "origin"),
            ModeOpt::Mortise => write!(f, "mortise"),
        }
    }
}

#[derive(Debug)]
pub enum ClientIpcOperation {
    Load {
        path: String,
        resp: oneshot::Sender<std::result::Result<u32, String>>,
    },
    MapUpdate {
        obj_id: u32,
        map_name: String,
        val: AppInfo,
        flag: BpfMapFlags,
        resp: oneshot::Sender<std::result::Result<(), String>>,
    },
    MapLookup {
        obj_id: u32,
        map_name: String,
        resp: oneshot::Sender<std::result::Result<Vec<u8>, String>>,
    },
    Shutdown,
    Connect {
        obj_id: u32,
        sk_raw_fd: i32,
        default_app_info: Option<u64>,
        resp: oneshot::Sender<std::result::Result<(), String>>,
    },
    Disconnect {
        obj_id: u32,
        resp: oneshot::Sender<std::result::Result<(), String>>,
    },
    QoEUpdate {
        obj_id: u32,
        qoe: FrameQoE,
        resp: oneshot::Sender<std::result::Result<Vec<u8>, String>>,
    },
}

pub async fn manager_ipc(mut rx: Receiver<(u64, ClientIpcOperation)>) {
    let mut stream = match UnixStream::connect("/tmp/mortise.sock").await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(target: "sender:manager", "Failed to connect to manager: {}", e);
            return;
        }
    };
    let (rh, wh) = stream.split();
    let mut reader = LengthDelimitedCodec::builder()
        .length_field_offset(0) // default value
        .length_field_type::<u32>()
        .length_adjustment(0) // default value
        .new_read(rh);
    let mut writer = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .new_write(wh);
    let pid = std::process::id() as i32;
    tracing::debug!(target: "sender:manager", "sender manager pid: {}", pid);
    let mut flow_id_map = HashMap::new();
    loop {
        let (id, op) = match rx.recv().await {
            None => break,
            Some((id, op)) => (id, op),
        };
        match op {
            ClientIpcOperation::Load { .. } => {
                continue;
            }
            ClientIpcOperation::Shutdown => {
                break;
            }
            ClientIpcOperation::MapUpdate {
                obj_id,
                map_name,
                val,
                flag,
                resp,
            } => {
                if obj_id == 0 {
                    resp.send(Ok(())).unwrap();
                    continue;
                }
                let flow_id = *flow_id_map.get(&id).unwrap_or(&0);
                let req = FlowOperation::SkStgMapUpdate {
                    map_name,
                    val: Vec::from(val.as_bytes()),
                    flag: flag.bits(),
                }
                .to_op(flow_id);
                let req_bytes = serde_json::to_vec(&req).map(Into::into).unwrap();
                writer.send(req_bytes).await.unwrap();
                let resp_bytes = reader.next().await.unwrap().unwrap();
                let response: std::result::Result<Vec<u8>, String> =
                    serde_json::from_slice(resp_bytes.as_ref()).unwrap();
                match response {
                    Ok(_) => {
                        resp.send(Ok(())).unwrap();
                    }
                    Err(e) => {
                        resp.send(Err(e)).unwrap();
                    }
                }
            }
            ClientIpcOperation::MapLookup {
                obj_id,
                map_name,
                resp,
            } => {
                if obj_id == 0 {
                    resp.send(Ok(Vec::new())).unwrap();
                    continue;
                }
                let flow_id = *flow_id_map.get(&id).unwrap_or(&0);
                let req = FlowOperation::SkStgMapLookup { map_name }.to_op(flow_id);
                let req_bytes = serde_json::to_vec(&req).map(Into::into).unwrap();
                writer.send(req_bytes).await.unwrap();
                let resp_bytes = reader.next().await.unwrap().unwrap();
                let response: std::result::Result<Vec<u8>, String> =
                    serde_json::from_slice(resp_bytes.as_ref()).unwrap();
                match response {
                    Ok(val) => {
                        resp.send(Ok(val)).unwrap();
                    }
                    Err(e) => {
                        resp.send(Err(e)).unwrap();
                    }
                }
            }
            ClientIpcOperation::Connect {
                obj_id,
                sk_raw_fd,
                default_app_info,
                resp,
            } => {
                if obj_id == 0 {
                    resp.send(Ok(())).unwrap();
                    continue;
                }
                let req = FlowOperation::Connect {
                    pid,
                    obj_id,
                    sk_fd: sk_raw_fd,
                    default_app_info,
                }
                .to_op(0);
                let req_bytes = serde_json::to_vec(&req).map(Into::into).unwrap();
                writer.send(req_bytes).await.unwrap();
                let resp_bytes = reader.next().await.unwrap().unwrap();
                let response: std::result::Result<Vec<u8>, String> =
                    serde_json::from_slice(resp_bytes.as_ref()).unwrap();
                match response {
                    Ok(r) => {
                        let remote_flow_id = read_be_u32(&mut r.as_ref());
                        flow_id_map.insert(id, remote_flow_id);
                        resp.send(Ok(())).unwrap();
                    }
                    Err(e) => {
                        resp.send(Err(e)).unwrap();
                    }
                }
            }
            ClientIpcOperation::Disconnect { obj_id, resp } => {
                if obj_id == 0 {
                    resp.send(Ok(())).unwrap();
                    continue;
                }
                let flow_id = *flow_id_map.get(&id).unwrap_or(&0);
                let req = FlowOperation::Disconnect {}.to_op(flow_id);
                let req_bytes = serde_json::to_vec(&req).map(Into::into).unwrap();
                writer.send(req_bytes).await.unwrap();
                let resp_bytes = reader.next().await.unwrap().unwrap();
                let response: std::result::Result<Vec<u8>, String> =
                    serde_json::from_slice(resp_bytes.as_ref()).unwrap();
                match response {
                    Ok(_) => {
                        resp.send(Ok(())).unwrap();
                    }
                    Err(e) => {
                        resp.send(Err(e)).unwrap();
                    }
                }
            }
            ClientIpcOperation::QoEUpdate { obj_id, qoe, resp } => {
                if obj_id == 0 {
                    resp.send(Ok(Vec::new())).unwrap();
                    continue;
                }
                let flow_id = *flow_id_map.get(&id).unwrap_or(&0);
                let req = FlowOperation::QoEUpdate { qoe }.to_op(flow_id);
                let req_bytes = serde_json::to_vec(&req).map(Into::into).unwrap();
                writer.send(req_bytes).await.unwrap();
                let resp_bytes = reader.next().await.unwrap().unwrap();
                let response: std::result::Result<Vec<u8>, String> =
                    serde_json::from_slice(resp_bytes.as_ref()).unwrap();
                match response {
                    Ok(val) => {
                        resp.send(Ok(val)).unwrap();
                    }
                    Err(e) => {
                        resp.send(Err(e)).unwrap();
                    }
                }
            }
        }
    }
}
