use anyhow::Result;
use clap::Parser;
use futures::{SinkExt, StreamExt};
use mortise_common::{get_clock_ns, get_tcp_info_total_retrans, MortiseError};
use socket2::{Domain, Socket, Type};
use speedy::{Readable, Writable};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::AtomicU64;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::codec::LengthDelimitedCodec;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use traffic::*;

#[derive(Debug, Parser, Clone)]
struct CommandArgs {
    /// Verbose debug output
    #[clap(short, long)]
    verbose: bool,
    #[clap(long, short)]
    bind: Option<String>,
    #[clap(long, short, default_value_t = 5000)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(env_filter)
        .init();
    let opts = CommandArgs::parse();
    let cancel_token = CancellationToken::new();
    let ctrlc_cancel_token = cancel_token.clone();
    let (manager_tx, manager_rx) = mpsc::channel::<(u64, ClientIpcOperation)>(32);

    ctrlc::set_handler(move || {
        ctrlc_cancel_token.cancel();
    })
    .expect("Error setting Ctrl-C handler");
    let manager_ipc_alive = CancellationToken::new();
    let manager_ipc_avlie_inner = manager_ipc_alive.clone();
    let manager_handle = tokio::spawn(async move {
        manager_ipc(manager_rx).await;
        manager_ipc_avlie_inner.cancel();
    });

    let addr = {
        let ip = {
            if let Some(ip) = opts.bind {
                ip.parse().unwrap()
            } else {
                Ipv4Addr::UNSPECIFIED
            }
        };
        SocketAddrV4::new(ip, opts.port)
    };
    let listener = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();

    // listener.set_tcp_congestion(new_tcp_ca).unwrap();
    listener.bind(&addr.into()).unwrap();
    // The backlog argument defines the maximum length to which the
    // queue of pending connections for sockfd may grow.  If a
    // connection request arrives when the queue is full, the client may
    // receive an error with an indication of ECONNREFUSED or, if the
    // underlying protocol supports retransmission, the request may be
    // ignored so that a later reattempt at connection succeeds.
    listener.listen(1024).unwrap();
    listener.set_nodelay(true).unwrap();
    listener.set_nonblocking(true).unwrap();
    listener.set_reuse_address(true).unwrap();
    let listener: TcpListener = TcpListener::from_std(listener.into()).unwrap();
    tracing::info!(target: "server", "Listens on {}. Wait for ctrl-c to shutdown...", addr);
    let client_conn_id = AtomicU64::new(1);

    let mut set = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                tracing::info!("Received cancel signal. Shutdown...");
                break;
            }
            conn = listener.accept() => {
                let (stream, _) = match conn {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("Failed to accept socket; error = {e}");
                        continue;
                    }
                };
                let id = client_conn_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let manager_tx = manager_tx.clone();
                let alive_token = manager_ipc_alive.clone();
                set.spawn(async move {
                    if let Err(e) = process(stream, manager_tx, alive_token, id).await {
                        tracing::error!("Failed to process connection; error = {e}");
                    }
                });
            }
        }
    }
    tracing::warn!("Gracefully shutdown of ctrl_c. Wait for 1 seconds...");
    if !manager_ipc_alive.is_cancelled() {
        manager_tx.send((0, ClientIpcOperation::Shutdown)).await?;
    }
    manager_handle.await?;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    set.shutdown().await;
    tracing::info!("Shutdown finished");
    Ok(())
}

async fn process(
    stream: TcpStream,
    manager_tx: mpsc::Sender<(u64, ClientIpcOperation)>,
    alive_token: CancellationToken,
    id: u64,
) -> anyhow::Result<()> {
    let mut framed_read = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .new_read(stream);
    let connect_opt = match framed_read.next().await {
        None => {
            tracing::error!("Empty request");
            return Ok(());
        }
        Some(res) => match res {
            Ok(bytes) => {
                let connect_opt = ClientRequestOpt::read_from_buffer(bytes.as_ref())?;
                if let ClientRequestOpt::Connect(opt) = connect_opt {
                    opt
                } else {
                    tracing::error!("Invalid request option");
                    return Ok(());
                }
            }
            Err(e) => {
                tracing::error!("Failed to handshake");
                return Err(e.into());
            }
        },
    };
    let (obj_id, tcp_ca) = connect_opt.congestion.get_tcp_ca();
    let stream = framed_read.into_inner();
    let std_sk = stream.into_std()?;
    let sk = socket2::Socket::from(std_sk);
    sk.set_tcp_congestion(tcp_ca)?;
    sk.set_nonblocking(true)?;
    let stream = TcpStream::from_std(sk.into())?;
    let fd = stream.as_raw_fd();

    let total_retrans = get_tcp_info_total_retrans(fd)?;

    // Inform manager with new connection
    if !alive_token.is_cancelled() {
        let (tmp_tx, tmp_rx) = tokio::sync::oneshot::channel();
        manager_tx
            .send((
                id,
                ClientIpcOperation::Connect {
                    obj_id,
                    sk_raw_fd: fd,
                    default_app_info: None,
                    resp: tmp_tx,
                },
            ))
            .await
            .unwrap();
        tmp_rx.await.unwrap().map_err(MortiseError::Custom)?;
    }

    let mut framed_client = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .new_framed(stream);
    let resp = ServerResponse {
        id: 0,
        client_send: 0,
        server_recv: get_clock_ns() as u64,
        data: Vec::new(),
    };
    let resp_bytes = resp.write_to_vec().map(Into::into).unwrap();
    framed_client.send(resp_bytes).await?;
    loop {
        match framed_client.next().await {
            None => {
                break;
            }
            Some(res) => match res {
                Ok(bytes) => {
                    let server_recv = get_clock_ns() as u64;
                    let req = ClientRequestOpt::read_from_buffer(bytes.as_ref())?;
                    match req {
                        ClientRequestOpt::Connect(_) => (),
                        ClientRequestOpt::Request(request) => {
                            let resp = ServerResponse {
                                id: request.id,
                                client_send: request.client_send,
                                server_recv,
                                data: vec![0; request.size as usize],
                            };
                            let resp_bytes = resp.write_to_vec().map(Into::into).unwrap();
                            framed_client.send(resp_bytes).await?;
                        }
                        // collect delta of total_retrans
                        ClientRequestOpt::Finish => {
                            let total_retrans = get_tcp_info_total_retrans(fd)? - total_retrans;
                            let resp = ServerResponse {
                                id: 0,
                                client_send: 0,
                                server_recv: total_retrans as u64,
                                data: Vec::new(),
                            };
                            let resp_bytes = resp.write_to_vec().map(Into::into).unwrap();
                            framed_client.send(resp_bytes).await?;
                            break;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Fail to process request from client: {:?}", e);
                    break;
                }
            },
        }
    }

    // Inform manager with disconnection
    if !alive_token.is_cancelled() {
        let (tmp_tx, tmp_rx) = tokio::sync::oneshot::channel();
        manager_tx
            .send((
                id,
                ClientIpcOperation::Disconnect {
                    obj_id,
                    resp: tmp_tx,
                },
            ))
            .await
            .unwrap();
        tmp_rx.await.unwrap().map_err(MortiseError::Custom)?;
    }
    Ok(())
}
