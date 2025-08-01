use crate::ManagerIpcOperation;
use futures::{SinkExt, StreamExt};
use mortise_common::{
    qoe::{AppInfo, FrameQoE},
    read_be_u32, FlowOperation, ManagerOperation, Operation, Result,
};
use std::collections::{HashSet, VecDeque};
use tokio::{
    net::UnixStream,
    sync::{mpsc, oneshot},
};
use tokio_util::codec::LengthDelimitedCodec;

async fn process_request(
    req: Operation,
    manager_tx: &mpsc::Sender<ManagerIpcOperation>,
    info: &mut PerUdsLocalInfo,
) -> Result<Vec<u8>> {
    let (tx, rx) = oneshot::channel();
    match req {
        Operation::Manager(op) => match op {
            ManagerOperation::Shutdown => {
                let op = ManagerIpcOperation {
                    req: ManagerOperation::Shutdown.into(),
                    resp: tx,
                };
                manager_tx.send(op).await?;
                Ok(Vec::new())
            }
            _ => {
                let m_op = ManagerIpcOperation {
                    req: op.into(),
                    resp: tx,
                };
                manager_tx.send(m_op).await?;
                rx.await?
            }
        },
        Operation::Flow { flow_id, op } => match op {
            FlowOperation::Connect {
                obj_id,
                sk_fd,
                pid,
                default_app_info,
            } => {
                let op = ManagerIpcOperation {
                    req: FlowOperation::Connect {
                        obj_id,
                        sk_fd,
                        pid,
                        default_app_info,
                    }
                    .to_op(flow_id),
                    resp: tx,
                };
                manager_tx.send(op).await?;
                let res = rx.await??;
                let flow_id = read_be_u32(&mut res.as_ref());
                info.flows.insert(flow_id);
                Ok(res)
            }
            FlowOperation::QoEUpdate { qoe } => {
                // TODO: how to leverage client QoE info
                tracing::trace!(target: "manager:qoe", "update value: {:?}", qoe);
                let transient_tradeoff = qoe_tradeoff(qoe.score());
                let stable_tradeoff = info.last_stable_tradeoff;
                info.qoe_record.push_back(qoe);
                if info.qoe_record.len() > 5 {
                    info.qoe_record.pop_front();
                }
                let mut mean_score = 0.0;
                for q in &info.qoe_record {
                    mean_score += q.score();
                }
                mean_score /= info.qoe_record.len() as f64;
                info.last_stable_tradeoff = qoe_tradeoff(mean_score);
                let tradeoff = (stable_tradeoff + transient_tradeoff) / 2;
                if tradeoff != stable_tradeoff {
                    let val = AppInfo {
                        req: tradeoff,
                        resp: 0,
                    };
                    let op = ManagerIpcOperation {
                        req: FlowOperation::SkStgMapUpdate {
                            map_name: "sk_stg_map".to_string(),
                            val: Vec::from(val.as_bytes()),
                            flag: 0,
                        }
                        .to_op(flow_id),
                        resp: tx,
                    };
                    manager_tx.send(op).await?;
                    let res = rx.await?;
                    if let Err(e) = res {
                        tracing::error!(target: "manager:qoe", "Fail to update trade off: {:?}", e);
                    }
                }
                Ok(Vec::new())
            }
            _ => {
                let m_op = ManagerIpcOperation {
                    req: op.to_op(flow_id),
                    resp: tx,
                };
                manager_tx.send(m_op).await?;
                rx.await?
            }
        },
    }
}

fn qoe_tradeoff(score: f64) -> u64 {
    if score < 5.0 {
        300
    } else if (5.0..6.0).contains(&score) {
        250
    } else if (6.0..6.5).contains(&score) {
        200
    } else if (6.5..7.5).contains(&score) {
        150
    } else if (7.5..8.0).contains(&score) {
        100
    } else {
        30
    }
}

#[derive(Default)]
pub struct PerUdsLocalInfo {
    pub flows: HashSet<u32>,
    pub qoe_record: VecDeque<FrameQoE>,
    pub last_stable_tradeoff: u64,
}

impl PerUdsLocalInfo {
    pub fn new() -> Self {
        PerUdsLocalInfo {
            flows: HashSet::new(),
            qoe_record: VecDeque::new(),
            last_stable_tradeoff: 0,
        }
    }

    pub async fn release(mut self, manager_tx: &mpsc::Sender<ManagerIpcOperation>) {
        let ops: Vec<Operation> = self
            .flows
            .drain()
            .map(|flow_id| Operation::Flow {
                flow_id,
                op: FlowOperation::Disconnect,
            })
            .collect();
        for op in ops {
            let _ = process_request(op, manager_tx, &mut self).await;
        }
    }
}

pub async fn handle_uds(mut receiver: UnixStream, manager_tx: mpsc::Sender<ManagerIpcOperation>) {
    // let pid = receiver.peer_cred().unwrap().pid().unwrap();
    // tracing::debug!("Peer pid: {}", pid);
    // let pid_fd = match pid_open(pid, false) {
    //     Ok(fd) => fd,
    //     Err(e) => {
    //         tracing::error!("Failed to open pid fd: {}", e);
    //         return;
    //     }
    // };
    let (rh, wh) = receiver.split();
    let mut reader = LengthDelimitedCodec::builder()
        .length_field_offset(0) // default value
        .length_field_type::<u32>()
        .length_adjustment(0) // default value
        .new_read(rh);
    let mut writer = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .new_write(wh);
    let mut info = PerUdsLocalInfo::new();
    loop {
        match reader.next().await {
            None => {
                tracing::info!(target: "manager:uds", "Connection closed");
                info.release(&manager_tx).await;
                break;
            }
            Some(res) => match res {
                Ok(bytes) => {
                    let req: Operation = serde_json::from_slice(bytes.as_ref()).unwrap();
                    let resp = process_request(req, &manager_tx, &mut info)
                        .await
                        .map_err(|e| {
                            tracing::error!(target: "manager:uds", "{}", e);
                            e.to_string()
                        });
                    let resp_bytes = serde_json::to_vec(&resp).map(Into::into).unwrap();
                    writer.send(resp_bytes).await.unwrap();
                }
                Err(e) => {
                    info.release(&manager_tx).await;
                    tracing::error!(target: "manager:uds", "Error: {:?}", e);
                    break;
                }
            },
        }
    }
}
