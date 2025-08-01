use mortise_common::{read_be_u32, CongestionOpt, ManagerIpcOperation, ManagerOperation, Result};
use mortise_manager::*;
use std::{os::unix::prelude::PermissionsExt, thread};
use tokio::{
    net::UnixListener,
    sync::{mpsc, oneshot},
};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(env_filter)
        .init();
    // Try to connect to the python process server
    let py_con = connect_py().await;
    let (manager_tx, manager_rx) = mpsc::channel::<ManagerIpcOperation>(32);
    let inner_manager_tx = manager_tx.clone();
    let manager_handle = thread::Builder::new()
        .name("mortise-manager".to_string())
        .spawn(move || manager(inner_manager_tx, manager_rx, py_con))?;

    // Load some default CCAs
    let ca_list = vec![CongestionOpt::MortiseCopa];

    for tcp_ca in ca_list {
        let (tx, rx) = oneshot::channel::<Result<Vec<u8>>>();
        let op = ManagerIpcOperation {
            req: tcp_ca.get_load_option().into(),
            resp: tx,
        };
        manager_tx.send(op).await?;
        let r = match rx.await? {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(target: "manager:load", "Fail to load {}: {:?}", tcp_ca, e);
                continue;
            }
        };
        let mut res = r.as_ref();
        let obj_id = read_be_u32(&mut res);
        tracing::info!(target: "manager:load", "Load {} with obj_id {}", tcp_ca, obj_id);
    }
    // Register RingBuffer of some CCAs to report data
    let (tx, rx) = oneshot::channel::<Result<Vec<u8>>>();
    let op = ManagerIpcOperation {
        req: ManagerOperation::RegisterRingBuf { obj_ids: vec![1] }.into(),
        resp: tx,
    };
    manager_tx.send(op).await?;
    if let Err(e) = rx.await? {
        tracing::error!(target: "manager:register", "Fail to register RingBuf: {:?}", e);
    }

    // Unix Domain Socket
    // privdrop::PrivDrop::default()
    //     .user("nobody")
    //     .apply()
    //     .unwrap_or_else(|e| panic!("Failed to drop privileges: {}", e));
    // println!("Dropped privileges to {}", privdrop::PrivDrop::default().user);
    let (ctrlc_tx, mut ctrlc_rx) = mpsc::channel(1);
    ctrlc::set_handler(move || {
        ctrlc_tx
            .blocking_send(())
            .expect("Could not send signal on channel.")
    })
    .expect("Error setting Ctrl-C handler");
    let _ = std::fs::remove_file(MORTISE_SOCK_PATH);
    let listener = UnixListener::bind(MORTISE_SOCK_PATH)?;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    std::fs::set_permissions(MORTISE_SOCK_PATH, std::fs::Permissions::from_mode(0o666))?;

    // Event loop
    loop {
        let manager_tx = manager_tx.clone();
        tokio::select! {
            biased;
            _ = ctrlc_rx.recv() => {
                tracing::warn!(target: "manager:shutdown", "Gracefully shutdown of ctrl_c. Wait for 1 seconds...");
                let (tx, _) = oneshot::channel::<Result<Vec<u8>>>();
                manager_tx.send(ManagerIpcOperation {
                    req: ManagerOperation::Shutdown.into(),
                    resp: tx,
                }).await?;
                manager_handle.join().unwrap();
                let _ = std::fs::remove_file(MORTISE_SOCK_PATH);
                tracing::info!(target: "manager:shutdown", "Shutdown finished");
                break;
            },
            res = listener.accept() => {
                if let Ok((receiver, _)) = res {
                    tracing::info!("receive one new connect");
                    tokio::spawn(async move {
                        handle_uds(receiver, manager_tx).await;
                    });
                }
            }
        }
    }
    Ok(())
}
