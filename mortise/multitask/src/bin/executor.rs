use backon::{ExponentialBuilder, Retryable};
use clap::Parser;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use multitask::config::ExpConfig;
use std::time::Instant;
use tokio::process::Command;
use tokio::time::{sleep, Duration};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Debug, Parser)]
struct CommandArgs {
    // #[clap(short = 'o', long = "output", default_value = "result")]
    // output: String,
    // #[clap(short, long)]
    // pcap: bool,
    // #[clap(short, long, default_value_t = 40)]
    // tasks: usize,
    // #[clap(long, default_value = "exp.trace")]
    // trace: String,
    #[clap(long, default_value_t = 5001)]
    base_port: u32,
    #[clap(short, long, default_value = "config.toml")]
    config: String,
}

fn mode_to_str(mode: u32) -> &'static str {
    match mode {
        0 => "origin",
        1 => "mortise",
        _ => "unknown",
    }
}

#[tokio::main]
async fn main() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(env_filter)
        .init();
    let opts = CommandArgs::parse();
    let config = ExpConfig::new_with_file(&opts.config).unwrap();
    let now = Instant::now();
    let mut tasks = FuturesUnordered::new();
    let concurrent_num = config.common.tasks;
    let trace_file = config.sender.trace;
    let loss = config.sender.loss;
    let default_app_info_cnt = config
        .sender
        .default_app_info
        .as_ref()
        .map(|v| v.len())
        .unwrap_or(1) as u32;
    let total_num = config.sender.iteration * config.sender.mode_cnt * default_app_info_cnt;
    let mut success_tasks = 0;
    let mut failed_tasks = 0;
    let m = MultiProgress::new();
    let pb1 = m.add(ProgressBar::new(total_num as u64));
    pb1.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold.dim}: [{elapsed_precise}<{eta_precise}] {wide_bar} [{pos}/{len}]",
        )
        .unwrap(),
    );
    pb1.set_prefix(" Spawned Tasks");
    let pb2 = m.insert_after(&pb1, ProgressBar::new(total_num as u64));
    pb2.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold.dim}: [{elapsed_precise}<{eta_precise}] {wide_bar} [{pos}/{len}]",
        )
        .unwrap(),
    );
    pb2.set_prefix("Finished Tasks");
    for i in 0..total_num {
        let port = opts.base_port + i;
        let result_dir = &config.common.result_directory;
        let (mode, _default_app_info_args, default_app_info_display) =
            match config.sender.default_app_info {
                Some(ref app_info) => {
                    let i = i % (config.sender.mode_cnt * default_app_info_cnt);
                    let mode = i / default_app_info_cnt;
                    let mode = mode_to_str(mode);
                    let default_app_info_args = format!(
                        "--default-app-info {}",
                        app_info[i as usize % default_app_info_cnt as usize]
                    );
                    let default_app_info_display = format!("-{}", app_info[i as usize]);
                    (mode, default_app_info_args, default_app_info_display)
                }
                None => (
                    mode_to_str(i % config.sender.mode_cnt),
                    "".to_string(),
                    "".to_string(),
                ),
            };
        let delay = config.sender.delay;
        let queue = &config.sender.queue;
        let buffer_size = &config.sender.buffer_size;
        let _frame_cnt = config.sender.frame_cnt;
        let _app = &config.sender.app;
        let tcp_ca = &config.sender.tcp_ca;
        let out_log = format!("{result_dir}/{tcp_ca}-{port}-{i}.log");
        let stat_csv = format!("{result_dir}/{port}.csv");
        let pcap_args = if config.sender.pcap {
            format!(
                "--pcap {result_dir}/pcap/{mode}{default_app_info_display}-{tcp_ca}-{port}.pcap"
            )
        } else {
            "".to_string()
        };
        let cmd = format!("mm-delay {delay} mm-loss downlink {loss} mm-link {trace_file} {trace_file} --uplink-queue={queue} --downlink-queue={queue} --uplink-queue-args={buffer_size} --downlink-queue-args={buffer_size} --downlink-log={out_log} -- ./scripts/run_sender.sh {pcap_args} -e {port} -o {stat_csv} -C {tcp_ca}");
        tasks.push(tokio::spawn(async move {
            let f = || async {
                let output = Command::new("sh").arg("-c").arg(&cmd).output();
                let res = output.await.unwrap();
                if !res.status.success() {
                    let err = String::from_utf8_lossy(&res.stderr).to_string();
                    Err(err)
                } else {
                    Ok(res)
                }
            };
            let back_off = ExponentialBuilder::default()
                .with_min_delay(Duration::from_millis(10))
                .with_max_delay(Duration::from_secs(1));
            f.retry(&back_off).await.unwrap()
        }));
        pb1.inc(1);
        sleep(Duration::from_millis(20)).await;
        if tasks.len() >= concurrent_num {
            if let Some(output) = tasks.next().await {
                let output = output.unwrap();
                if output.status.success() {
                    success_tasks += 1;
                    pb2.inc(1);
                    // tracing::debug!("{success_tasks} Task Success");
                } else {
                    failed_tasks += 1;
                    tracing::error!(
                        "Task failed with error: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    // tracing::debug!("stdout: {}", String::from_utf8_lossy(&output.stdout));
                }
            }
        }
    }
    pb1.finish_with_message("All tasks spawned");
    while let Some(output) = tasks.next().await {
        let output = output.unwrap();
        if output.status.success() {
            success_tasks += 1;
            pb2.inc(1);
            // tracing::debug!("{success_tasks} Task Success");
        } else {
            failed_tasks += 1;
            tracing::error!(
                "Task failed with error: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            // tracing::debug!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        }
    }
    pb2.finish_with_message("All tasks finished");
    let elapsed = now.elapsed().as_secs();
    let hours = elapsed / 3600;
    let minutes = (elapsed % 3600) / 60;
    let seconds = elapsed % 60;
    let time = format!("{hours}h{minutes}m{seconds}s");
    tracing::info!("All tasks finished within {time}");
    tracing::info!("Total: {total_num}, Success: {success_tasks}, Failed: {failed_tasks}");
}
