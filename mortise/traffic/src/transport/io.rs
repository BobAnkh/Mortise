use super::{RateCtrlOp, SendChunkInfo, TransportInfo, TransportOpt};
use crate::{ClientIpcOperation, ModeOpt};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use mortise_common::qoe::FrameQoE;
use mortise_common::CongestionOpt;
use mortise_common::{get_clock_ns, sync::AtomicRawCell};
use rustc_hash::FxHashMap as HashMap;
use speedy::{Readable, Writable};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::LengthDelimitedCodec;

// 20 bytes from converting structs to bytes, the other 4 bytes from frame head (u32)
pub const HEADER_OVERHEAD: u64 = 24;
pub const TIMEOUT: u64 = 300;

#[derive(Debug, Clone, Default, Readable, Writable)]
pub struct DataChunk {
    pub id: u64,
    pub server_send: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default, Readable, Writable)]
pub struct ChunkAck {
    pub id: u64,
    pub server_send: u64,
    pub client_recv: u64,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct Stat {
    pub server_send: u64,
    pub client_recv: u64,
    pub server_recv: u64,
    pub size: u64,
    pub qoe: FrameQoE,
}

#[derive(Debug, Clone, Readable, Writable)]
pub struct ClientConnectOpt {
    pub congestion: CongestionOpt,
}

#[derive(Debug, Clone, Readable, Writable)]
pub struct ClientRequest {
    pub id: u32,
    pub size: u32,
    pub client_send: u64,
}

#[derive(Debug, Clone, Readable, Writable)]
pub struct ServerResponse {
    pub id: u32,
    pub client_send: u64,
    /// This is the time that the server handles the request.
    /// The request may already stay in the queue for a while.
    pub server_recv: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Readable, Writable)]
pub enum ClientRequestOpt {
    Connect(ClientConnectOpt),
    Request(ClientRequest),
    Finish,
}

pub struct ClientRequestStats {
    pub id: u32,
    pub size: u32,
    pub client_send: u64,
    pub server_recv: u64,
    pub client_recv: u64,
}

pub async fn handle_send(
    writer: OwnedWriteHalf,
    mut app_rx: mpsc::UnboundedReceiver<RateCtrlOp>,
    app_tx: mpsc::UnboundedSender<()>,
    transport_info: Arc<AtomicRawCell<TransportInfo>>,
) {
    let mut writer = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .max_frame_length(500 * 1024 * 1024) // 500MB
        .new_write(writer);
    let mut total_write_bytes: u64 = 0;
    transport_info.store(Box::new(TransportInfo { total_write_bytes }));

    let start_time = tokio::time::Instant::now();
    let timeout_duration = Duration::from_secs(TIMEOUT);

    loop {
        tokio::select! {
            // 接收 RateCtrlOp 消息
            Some(op) = app_rx.recv() => {
                match op {
                    RateCtrlOp::Ready => {
                        app_tx.send(()).unwrap();
                    }
                    RateCtrlOp::Send(SendChunkInfo { id, data_bytes }) => {
                        let frame = DataChunk {
                            id,
                            server_send: get_clock_ns() as u64,
                            data: vec![74; data_bytes as usize],
                        };
                        let b: Bytes = { frame.write_to_vec().map(Into::into).unwrap() };
                        total_write_bytes += data_bytes + HEADER_OVERHEAD;
                        transport_info.store(Box::new(TransportInfo { total_write_bytes }));
                        if let Err(e) = writer.send(b).await {
                            tracing::error!(target: "sender:send", "Failed to send data: {:?}", e);
                            break;
                        }
                    }
                    RateCtrlOp::Done => {
                        tracing::info!(target: "sender:send", "All data is sent!");
                        break;
                    }
                }
            }
            // timeout handle
            _ = tokio::time::sleep_until(start_time + timeout_duration) => {
                tracing::warn!(target: "sender:send", "Timeout reached (300 seconds), stopping send loop.");
                // app_tx.send(()).unwrap();
                break;
            }
        }
    }

    // close connection
    if let Err(e) = writer.into_inner().shutdown().await {
        tracing::error!(target: "sender:send", "Failed to close writer: {:?}", e);
    } else {
        tracing::info!(target: "sender:send", "Writer successfully closed.");
    }
}

pub async fn handle_recv(
    reader: OwnedReadHalf,
    manager_tx: mpsc::Sender<ClientIpcOperation>,
    transport_opt: TransportOpt,
) -> HashMap<u64, Stat> {
    let mut statistics = HashMap::default();
    let mut reader = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .max_frame_length(500 * 1024 * 1024) // 500MB
        .new_read(reader);
    tracing::info!(target: "sender:recv", "Begin to receive!");
    let obj_id = transport_opt.congestion.get_obj_id();
    loop {
        match reader.next().await {
            Some(res) => match res {
                Ok(bytes) => {
                    let server_recv = get_clock_ns() as u64;
                    let ack: ChunkAck = ChunkAck::read_from_buffer(bytes.as_ref()).unwrap();
                    // println!(
                    //     "recv ack: {}, s -> {} -> r -> {} -> s, size {}",
                    //     ack.id,
                    //     (ack.client_recv - ack.server_send) / 1000000,
                    //     (server_recv - ack.client_recv) / 1000000,
                    //     ack.size
                    // );
                    let qoe = FrameQoE {
                        server_send: ack.server_send,
                        client_recv: ack.client_recv,
                        server_recv,
                        size: ack.size,
                        frame_interval: Duration::from_micros(1_000_000 / 60),
                        frame_id: ack.id,
                    };
                    if transport_opt.mode == ModeOpt::Mortise {
                        let (tmp_tx, tmp_rx) = oneshot::channel();
                        manager_tx
                            .send(ClientIpcOperation::QoEUpdate {
                                obj_id,
                                qoe: qoe.clone(),
                                resp: tmp_tx,
                            })
                            .await
                            .unwrap();
                        let _ = tmp_rx.await.unwrap().unwrap();
                    }
                    statistics.insert(
                        ack.id,
                        Stat {
                            server_recv,
                            client_recv: ack.client_recv,
                            server_send: ack.server_send,
                            size: ack.size,
                            qoe,
                        },
                    );
                }
                Err(e) => {
                    tracing::error!(target: "sender:recv", "{:?}", e);
                    break;
                }
            },
            None => {
                tracing::info!(target: "sender:recv", "All data is received!");
                break;
            }
        }
    }
    statistics
}
