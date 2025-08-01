use anyhow::Result;
use bytes::Bytes;
use clap::Parser;
use csv::Writer;
use futures::SinkExt;
use mortise_common::get_clock_ns;
use speedy::{Readable as _, Writable};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use tokio::io::AsyncBufReadExt;
use tokio::net::{TcpSocket, TcpStream};
use tokio::time::{Duration, Instant};
use tokio_stream::StreamExt;
use tokio_util::codec::LengthDelimitedCodec;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use traffic::*;

const MAHIMAHI_IP: &str = "MAHIMAHI_BASE";

/// Trace high run queue latency
#[derive(Debug, Parser, Clone)]
#[clap(rename_all = "kebab-case")]
struct CommandArgs {
    /// Verbose debug output
    #[clap(short, long)]
    verbose: bool,
    #[clap(short, long)]
    egress_port: Option<u16>,
    #[clap(long, short)]
    bind: Option<String>,
    #[clap(long, short)]
    connect: Option<String>,
    #[clap(long, short)]
    port: Option<u16>,
    #[clap(short = 'C', long, value_enum, default_value_t = transport::CongestionOpt::Cubic)]
    congestion: transport::CongestionOpt,
    #[clap(short, long, default_value = "result.csv")]
    output: String,
    #[clap(short, long)]
    workload: Option<PathBuf>,
}

fn parse_sk_addr(opts: &CommandArgs) -> Result<(SocketAddr, Option<SocketAddr>)> {
    let server_port = opts.port.unwrap_or(5000);
    let server_ip = {
        match opts.connect {
            Some(ref host) => host.parse()?,
            None => {
                if let Ok(ip) = std::env::var(MAHIMAHI_IP) {
                    ip.parse().unwrap_or(Ipv4Addr::UNSPECIFIED)
                } else {
                    Ipv4Addr::UNSPECIFIED
                }
            }
        }
    };
    let server_addr: SocketAddr = SocketAddrV4::new(server_ip, server_port).into();
    let client_addr = match (opts.bind.as_ref(), opts.egress_port) {
        (Some(host), Some(port)) => {
            let client_ip = host.parse()?;
            Some(SocketAddrV4::new(client_ip, port).into())
        }
        (Some(host), None) => {
            let client_port = 5001;
            let client_ip = host.parse()?;
            Some(SocketAddrV4::new(client_ip, client_port).into())
        }
        (None, Some(port)) => {
            let client_ip = Ipv4Addr::UNSPECIFIED;
            Some(SocketAddrV4::new(client_ip, port).into())
        }
        (None, None) => None,
    };
    Ok((server_addr, client_addr))
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
    let ctrl_cancel_token = cancel_token.clone();

    ctrlc::set_handler(move || {
        ctrl_cancel_token.cancel();
    })
    .expect("Error setting Ctrl-C handler");

    let (server_addr, client_addr) = parse_sk_addr(&opts)?;

    let stream = match client_addr {
        Some(addr) => {
            let socket = TcpSocket::new_v4()?;
            socket.bind(addr)?;
            socket.connect(server_addr).await?
        }
        None => TcpStream::connect(server_addr).await?,
    };
    stream.set_nodelay(true)?;

    tracing::info!(target: "sender", "Wait for Ctrl-C or transmission finished...");
    transmit(opts, stream, cancel_token).await?;

    Ok(())
}

async fn transmit(
    opts: CommandArgs,
    stream: TcpStream,
    cancel_token: CancellationToken,
) -> Result<()> {
    let (rh, wh) = stream.into_split();
    let mut writer = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .new_write(wh);
    let mut reader = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .new_read(rh);
    let connect_opt = ClientRequestOpt::Connect(ClientConnectOpt {
        congestion: opts.congestion,
    });
    let b: Bytes = { connect_opt.write_to_vec().map(Into::into).unwrap() };
    writer.send(b).await.unwrap();
    match reader.next().await {
        Some(Ok(b)) => {
            let _ = ServerResponse::read_from_buffer(&b)?;
        }
        Some(Err(e)) => {
            return Err(anyhow::anyhow!("Error reading response: {:?}", e));
        }
        None => {
            return Err(anyhow::anyhow!("No response received"));
        }
    }
    let traces = match opts.workload {
        Some(ref path) => read_trace_file(path).await?,
        None => Vec::new(),
    };
    let mut cnt = traces.len();
    let writer_cancel_token = cancel_token.clone();
    let w = tokio::spawn(async move {
        let mut now = Instant::now();
        let mut id = 1_u32;
        for (gap, size) in traces {
            tokio::select! {
                biased;
                _ = writer_cancel_token.cancelled() => {
                    tracing::info!(target: "sender:send", "Cancel signal received!");
                    break;
                }
                _ = tokio::time::sleep_until(now + gap) => {}
            }
            let client_send = get_clock_ns() as u64;
            now += gap;
            let req = ClientRequestOpt::Request(ClientRequest {
                size,
                client_send,
                id,
            });
            id += 1;
            let b: Bytes = req.write_to_vec().map(Into::into).unwrap();
            writer.send(b).await.unwrap();
        }
        writer
    });

    let mut stats = Vec::new();
    while cnt > 0 {
        let resp = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                tracing::info!(target: "sender:recv", "Cancel signal received!");
                break;
            }
            resp = reader.next() => resp
        };
        let client_recv = get_clock_ns() as u64;
        match resp {
            Some(Ok(resp)) => {
                let resp = match ServerResponse::read_from_buffer(&resp) {
                    Ok(resp) => resp,
                    Err(e) => {
                        tracing::error!(target: "sender:recv", "Error parsing response: {:?}", e);
                        cancel_token.cancel();
                        break;
                    }
                };
                stats.push(ClientRequestStats {
                    id: resp.id,
                    size: resp.data.len() as u32,
                    client_send: resp.client_send,
                    server_recv: resp.server_recv,
                    client_recv,
                });
                cnt -= 1;
            }
            Some(Err(err)) => {
                tracing::error!(target: "sender:recv", "Error reading response: {:?}", err);
                cancel_token.cancel();
                break;
            }
            None => {
                tracing::warn!(target: "sender:recv", "No more response!");
                cancel_token.cancel();
                break;
            }
        }
    }

    // Post process: disconnect with server
    // Collect summarized stats from server
    let mut writer = w.await?;
    let req = ClientRequestOpt::Finish;
    let b: Bytes = req.write_to_vec().map(Into::into)?;
    writer.send(b).await?;
    let resp = reader.next().await.unwrap()?;
    let resp = ServerResponse::read_from_buffer(&resp)?;
    stats.push(ClientRequestStats {
        id: resp.id,
        size: 0,
        client_send: resp.client_send,
        server_recv: resp.server_recv,
        client_recv: 0,
    });

    // output stats to csv
    tracing::info!(
        "All requests finished. Statistics are saved to {}",
        opts.output
    );
    write_stat_csv(opts.output, stats).await?;
    Ok(())
}

async fn read_trace_file<P: AsRef<Path>>(path: P) -> Result<Vec<(Duration, u32)>> {
    let mut traces = Vec::new();
    let file = tokio::fs::File::open(path).await?;
    let mut lines = tokio::io::BufReader::new(file).lines();
    while let Some(line) = lines.next_line().await? {
        let mut parts = line.split_whitespace();
        let time = parts
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or(anyhow::anyhow!("Invalid trace file format"))?;
        let time = Duration::from_millis(time);
        let size = parts
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or(anyhow::anyhow!("Invalid trace file format"))?;
        traces.push((time, size));
    }
    Ok(traces)
}

async fn write_stat_csv(path: String, stats: Vec<ClientRequestStats>) -> Result<()> {
    let mut wtr = Writer::from_path(path)?;
    wtr.write_record([
        "id",
        "size",
        "client_send",
        "server_recv",
        "client_recv",
        "rct",
    ])?;
    for stat in stats {
        wtr.write_record(&[
            stat.id.to_string(),
            stat.size.to_string(),
            stat.client_send.to_string(),
            stat.server_recv.to_string(),
            stat.client_recv.to_string(),
            ((stat.client_recv - stat.client_send) / 1_000_000).to_string(),
        ])?;
    }
    wtr.flush()?;
    Ok(())
}
