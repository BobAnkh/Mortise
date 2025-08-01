use crate::Stat;
use csv::Writer;
use rustc_hash::FxHashMap as HashMap;
use std::path::Path;

pub fn write_stat_csv<P>(
    output_csv_file_path: P,
    statistics: &HashMap<u64, Stat>,
) -> anyhow::Result<()>
where
    P: AsRef<Path>,
{
    let mut wtr = Writer::from_path(output_csv_file_path)?;
    wtr.write_record([
        "id",
        "size",
        "server_send",
        "client_recv",
        "server_recv",
        "go_owd",
        "back_owd",
        "rtt",
        "ssim",
        "ssim_reward",
        "delay_punish",
        "qoe",
    ])?;
    for i in 0..statistics.len() as u64 {
        if let Some(stat) = statistics.get(&i) {
            wtr.write_record(&[
                i.to_string(),
                stat.size.to_string(),
                stat.server_send.to_string(),
                stat.client_recv.to_string(),
                stat.server_recv.to_string(),
                ((stat.client_recv - stat.server_send) / 1_000_000).to_string(),
                ((stat.server_recv - stat.client_recv) / 1_000_000).to_string(),
                ((stat.server_recv - stat.server_send) / 1_000_000).to_string(),
                stat.qoe.ssim().to_string(),
                stat.qoe.ssim_reward().to_string(),
                stat.qoe.delay_punish().to_string(),
                stat.qoe.score().to_string(),
            ])
            .expect("write error");
        }
    }
    wtr.flush()?;
    Ok(())
}

pub fn print_sys_tcp_ca() {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("sysctl net.ipv4.tcp_available_congestion_control")
        .output()
        .unwrap();
    tracing::debug!(
        target: "system",
        "Available ca: {}",
        String::from_utf8_lossy(&output.stdout).trim_end_matches('\n')
    );
}
