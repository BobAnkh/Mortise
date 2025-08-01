use anyhow::Result;
use clap::Parser;
use futures::{stream::FuturesUnordered, StreamExt};
use multitask::config::DbConfig;
use multitask::utils::{save_to_db, write_df_csv};
use polars::prelude::*;
use std::str::FromStr;
use std::time::Duration;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio::process::Command;

pub const FRAME_SIZE: f64 = 50.0 / 1024.0 * 8.0;

/// Trace high run queue latency
#[derive(Debug, Parser)]
struct CommandArgs {
    #[clap(short = 'o', long = "output", default_value = "result")]
    output: String,
    #[clap(short, long)]
    log: bool,
    #[clap(short, long)]
    pcap: bool,
    #[clap(long, default_value_t = 0.2)]
    lambda: f64,
    #[clap(long, default_value_t = 5000)]
    port: usize,
    #[clap(long)]
    percentile: Option<u32>,
    #[clap(short, long, default_value_t = 40)]
    tasks: usize,
    #[clap(short, long, default_value_t = 1000)]
    frame_start: usize,
    #[clap(short, long, default_value_t = 8000)]
    frame_end: usize,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "20s")]
    start_time: Duration,
    #[clap(long, value_parser = humantime::parse_duration, default_value = "100s")]
    end_time: Duration,
    /// Whether to store data to mongodb
    #[clap(long, requires_all=["collection", "config"])]
    mongo: bool,
    /// The name of the collection to store data
    #[clap(long, requires = "mongo")]
    collection: Option<String>,
    /// The configuration of mahimahi to be stored in db
    #[clap(long, requires = "mongo")]
    config: Option<String>,
    /// The extra description to be stored in db
    #[clap(long, requires = "mongo")]
    desc: Option<String>,
    #[clap(long)]
    prefix: Vec<String>,
}

#[derive(Debug)]
pub struct Packet {
    pub seq: u32,
    pub send_time: Duration,
    pub ack_time: Option<Duration>,
}

fn is_match_file(file_name: &std::ffi::OsStr, prefix: &Vec<String>) -> bool {
    let file_name = file_name.to_str().unwrap();
    for p in prefix {
        if file_name.starts_with(p) {
            return true;
        }
    }
    false
}

pub fn process_iter<P>(input_path: P, frame_start: usize, frame_end: usize) -> Result<DataFrame>
where
    P: Into<PathBuf>,
{
    let mut sub_schema = Schema::with_capacity(1);
    sub_schema.with_column("delay_punish".into(), DataType::Float64);
    let df = CsvReader::from_path(input_path)?
        .has_header(true)
        .with_dtypes(Some(sub_schema.into()))
        .finish()?;
    let df = df
        .tail(Some(df.height() - frame_start))
        .head(Some(frame_end - frame_start));
    let df = df
        .lazy()
        .select([
            col("rtt")
                .filter(col("rtt").gt(100))
                .count()
                .alias("ddl_100"),
            col("rtt")
                .filter(col("rtt").gt(120))
                .count()
                .alias("ddl_120"),
            col("rtt")
                .filter(col("rtt").gt(150))
                .count()
                .alias("ddl_150"),
            col("rtt")
                .filter(col("rtt").gt(200))
                .count()
                .alias("ddl_200"),
            col("rtt")
                .filter(col("rtt").gt(250))
                .count()
                .alias("ddl_250"),
            col("rtt")
                .filter(col("rtt").gt(300))
                .count()
                .alias("ddl_300"),
            col("rtt").mean().alias("mean_rtt"),
            col("rtt")
                .quantile(0.5.into(), QuantileInterpolOptions::Nearest)
                .alias("p50_rtt"),
            col("rtt")
                .quantile(0.75.into(), QuantileInterpolOptions::Nearest)
                .alias("p75_rtt"),
            col("rtt")
                .quantile(0.9.into(), QuantileInterpolOptions::Nearest)
                .alias("p90_rtt"),
            col("rtt")
                .quantile(0.95.into(), QuantileInterpolOptions::Nearest)
                .alias("p95_rtt"),
            col("rtt")
                .quantile(0.99.into(), QuantileInterpolOptions::Nearest)
                .alias("p99_rtt"),
            // col("rtt")
            //     .filter((col("id") % 60.into()).eq(0))
            //     .quantile(0.5.into(), QuantileInterpolOptions::Nearest)
            //     .alias("p50_key"),
            // col("rtt")
            //     .filter((col("id") % 60.into()).eq(0))
            //     .quantile(0.75.into(), QuantileInterpolOptions::Nearest)
            //     .alias("p75_key"),
            // col("rtt")
            //     .filter((col("id") % 60.into()).eq(0))
            //     .quantile(0.9.into(), QuantileInterpolOptions::Nearest)
            //     .alias("p90_key"),
            // col("rtt")
            //     .filter((col("id") % 60.into()).eq(0))
            //     .quantile(0.95.into(), QuantileInterpolOptions::Nearest)
            //     .alias("p95_key"),
            // col("rtt")
            //     .filter((col("id") % 60.into()).eq(0))
            //     .quantile(0.99.into(), QuantileInterpolOptions::Nearest)
            //     .alias("p99_key"),
            (col("server_recv").tail(Some(1)) - col("server_send").head(Some(1))).alias("fct"),
            (col("server_recv").count()).alias("frame_cnt"),
            col("qoe").mean().alias("mean_qoe"),
            col("ssim").mean().alias("mean_ssim"),
            col("ssim")
                .quantile(0.1.into(), QuantileInterpolOptions::Nearest)
                .alias("p10_ssim"),
            col("ssim")
                .quantile(0.5.into(), QuantileInterpolOptions::Nearest)
                .alias("p50_ssim"),
            col("ssim")
                .quantile(0.9.into(), QuantileInterpolOptions::Nearest)
                .alias("p90_ssim"),
            col("ssim_reward").mean().alias("mean_ssim_reward"),
            col("ssim_reward")
                .quantile(0.1.into(), QuantileInterpolOptions::Nearest)
                .alias("p10_ssim_reward"),
            col("ssim_reward")
                .quantile(0.5.into(), QuantileInterpolOptions::Nearest)
                .alias("p50_ssim_reward"),
            col("ssim_reward")
                .quantile(0.9.into(), QuantileInterpolOptions::Nearest)
                .alias("p90_ssim_reward"),
            col("delay_punish").mean().alias("mean_delay_punish"),
            col("delay_punish")
                .quantile(0.1.into(), QuantileInterpolOptions::Nearest)
                .alias("p10_delay_punish"),
            col("delay_punish")
                .quantile(0.5.into(), QuantileInterpolOptions::Nearest)
                .alias("p50_delay_punish"),
            col("delay_punish")
                .quantile(0.9.into(), QuantileInterpolOptions::Nearest)
                .alias("p90_delay_punish"),
            col("qoe").mean().alias("mean_qoe"),
            col("qoe")
                .quantile(0.01.into(), QuantileInterpolOptions::Nearest)
                .alias("p01_qoe"),
            col("qoe")
                .quantile(0.05.into(), QuantileInterpolOptions::Nearest)
                .alias("p05_qoe"),
            col("qoe")
                .quantile(0.1.into(), QuantileInterpolOptions::Nearest)
                .alias("p10_qoe"),
            col("qoe")
                .quantile(0.25.into(), QuantileInterpolOptions::Nearest)
                .alias("p25_qoe"),
            col("qoe")
                .quantile(0.5.into(), QuantileInterpolOptions::Nearest)
                .alias("p50_qoe"),
            col("qoe")
                .quantile(0.75.into(), QuantileInterpolOptions::Nearest)
                .alias("p75_qoe"),
            col("qoe")
                .quantile(0.9.into(), QuantileInterpolOptions::Nearest)
                .alias("p90_qoe"),
            col("qoe")
                .quantile(0.95.into(), QuantileInterpolOptions::Nearest)
                .alias("p95_qoe"),
            col("qoe")
                .quantile(0.99.into(), QuantileInterpolOptions::Nearest)
                .alias("p99_qoe"),
        ])
        .collect()?;
    Ok(df)
}

async fn _analyze_frame_statistic<P>(
    result_dir: P,
    opts: &CommandArgs,
    config: &DbConfig,
) -> Result<Vec<f64>>
where
    P: AsRef<Path>,
{
    let prefix = &opts.prefix;
    let concurrent_num = opts.tasks;
    let mut tasks = FuturesUnordered::new();
    let mut origin_df = DataFrame::empty();
    for entry in fs::read_dir(&result_dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::metadata(&path)?;
        if metadata.is_file() {
            let file_name = path.file_name().ok_or("No filename").unwrap();
            let full_name = result_dir.as_ref().join(file_name);
            let frame_start = opts.frame_start;
            let frame_end = opts.frame_end;
            if is_match_file(file_name, prefix) {
                tasks.push(tokio::spawn(async move {
                    // let file_name = file_name.clone();
                    (process_iter(&full_name, frame_start, frame_end), full_name)
                }));
            }
        }
        if tasks.len() >= concurrent_num {
            if let Some(r) = tasks.next().await {
                let (df, _) = r.unwrap();
                let df = df?;
                if origin_df.is_empty() {
                    origin_df = df;
                } else {
                    origin_df = origin_df.vstack(&df)?;
                }
            }
        }
    }
    while let Some(r) = tasks.next().await {
        let (df, _) = r.unwrap();
        let df = df?;
        if origin_df.is_empty() {
            origin_df = df;
        } else {
            origin_df = origin_df.vstack(&df)?;
        }
    }
    origin_df.rechunk();
    for col in unsafe { origin_df.get_columns_mut() } {
        *col = col.sort(false);
    }
    let prefix_name = if prefix.is_empty() {
        "origin".to_string()
    } else {
        prefix[0].clone()
    };

    // Mbps
    let frame_cnts = origin_df
        .column("frame_cnt")
        .unwrap()
        .u32()
        .unwrap()
        .into_iter();

    let fcts = origin_df.column("fct").unwrap().i64().unwrap().into_iter();

    let thr: Vec<f64> = frame_cnts
        .zip(fcts)
        .map(|(cnts, fct)| {
            cnts.unwrap() as f64 * FRAME_SIZE * 1_000_000_000.0 / fct.unwrap() as f64
        })
        .collect();

    // let avg_thr = thr.iter().sum::<f64>() / thr.len() as f64;

    write_df_csv(&mut origin_df, format!("{prefix_name}-frame.csv"))?;
    if opts.mongo {
        save_to_db(
            &mut origin_df,
            &config.database,
            &opts.collection.as_ref().unwrap_or(&"".to_string()),
            format!(
                "{}-origin",
                &opts.config.as_ref().unwrap_or(&"".to_string())
            ),
            &opts.desc.as_ref().unwrap_or(&"".to_string()),
            "trace.json",
        )
        .await?;
    }
    // println!("{:}", avg_thr);
    // thr.iter().for_each(|&t| println!("{:?}", t));
    Ok(thr)
}

async fn parse_pcap(
    f: &str,
    port: usize,
    start_time: Duration,
    end_time: Duration,
    _lambda: f64,
) -> (Vec<Duration>, Vec<(Duration, u32)>) {
    let cmd = format!("tcpdump -tt -r {f} port {port} | awk '{{split($3,a,\".\");split($5,b,\":\");split(b[1],c,\".\"); print $1,a[5],c[5],$7,$8,$9}}'");
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .await
        .expect("failed to execute process");
    let raw_data = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = raw_data.split(",\n").collect();
    let mut packets: Vec<Packet> = Vec::new();
    // let mut send_seq: std::collections::HashMap<u32, Duration> = std::collections::HashMap::new();
    let mut tmp_lat_list: Vec<Duration> = Vec::new();
    let mut latency_list: Vec<Duration> = Vec::new();
    let mut throughput_list: Vec<(Duration, u32)> = Vec::new();
    let mut pkt_cnt: u32 = 0;
    // let mut total_pkt_cnt: u32 = 0;
    let mut first_pkt_time: Option<Duration> = None;
    let mut valid_pkt_cnt: u32 = 0;
    // let mut head = (FRAME_SIZE * 1024.0 * 1024.0 * 1000.0 / 8.0 / 1448.0) as u32;
    let mut head_idx: u32 = 0;
    let mut interval_start: Duration = Duration::from_secs(0);
    let mut interval_end: Duration = Duration::from_secs(0);
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let b: Vec<&str> = line.split_whitespace().collect();
        if b[3] == "[S.],"
            || b[3] == "[F.],"
            || b[3] == "[S],"
            || b[3] == "[F],"
            || b[3] == "[R],"
            || b[3] == "[FP.],"
        {
            continue;
        }
        if b[2] == format!("{port}") && b[4] == "seq" {
            tracing::trace!("Seq pkt: {:?}", b);
            let t = b[0]
                .split('.')
                .map(u64::from_str)
                .collect::<Result<Vec<u64>, _>>()
                .unwrap();
            let cur_pkt_time = Duration::from_secs(t[0]) + Duration::from_micros(t[1]);
            if first_pkt_time.is_none() {
                first_pkt_time = Some(cur_pkt_time);
            };
            let k = b[5]
                .split(':')
                .map(u32::from_str)
                .collect::<Result<Vec<u32>, _>>()
                .unwrap();
            if k.len() != 2 {
                tracing::info!("{:?}", b);
                continue;
            }
            // send_seq.insert(k[1], d);
            // it starts with 1449 from 1:1449
            let index = k[1] / 1448 - 1;
            if cur_pkt_time - first_pkt_time.unwrap() <= start_time {
                if index == head_idx + 1 {
                    head_idx = index;
                }
                continue;
            }
            if index > head_idx {
                let index = (index - head_idx - 1) as usize;
                if packets.len() != index {
                    tracing::debug!("{} {} {}", line, index, packets.len());
                    continue;
                }
                packets.push(Packet {
                    seq: k[1],
                    send_time: cur_pkt_time,
                    ack_time: None,
                });
            }
        } else if b[1] == format!("{port}") && b[4] == "ack" && first_pkt_time.is_some() {
            let t = b[0]
                .split('.')
                .map(u64::from_str)
                .collect::<Result<Vec<u64>, _>>()
                .unwrap();
            let cur_pkt_time = Duration::from_secs(t[0]) + Duration::from_micros(t[1]);
            if cur_pkt_time - first_pkt_time.unwrap() > end_time {
                break;
            }
            let k: u32 = b[5].parse().unwrap();
            // if let Some(_v) = send_seq.get(&k) {
            // if pkt_cnt == 0 {
            //     interval_send = *v;
            //     interval_recv = d;
            // } else {
            //     interval_recv = d;
            // }
            // pkt_cnt += 1;
            // if pkt_cnt >= 1000 {
            //     throughput_list.push((interval_recv - interval_send, pkt_cnt));
            //     pkt_cnt = 0;
            // }
            // latency_list.push(d - *v);
            // }
            let index = k / 1448;
            if index > head_idx {
                let index = (index - head_idx - 1) as usize;
                if index >= packets.len() {
                    continue;
                }
                if packets[index].ack_time.is_none() {
                    packets[index].ack_time = Some(cur_pkt_time);
                }
            }
        }
    }
    for packet in packets.into_iter() {
        if pkt_cnt == 0 {
            interval_start = packet.send_time;
            interval_end = packet.send_time;
        }
        pkt_cnt += 1;
        // println!("sendtime: {} {}", packet.seq, packet.send_time.as_secs_f64());
        if let Some(r) = packet.ack_time {
            // println!("{} {}", packet.send_time.as_secs_f64(), packet.ack_time.unwrap().as_secs_f64());
            if valid_pkt_cnt == 0 {
                interval_start = r;
            }
            valid_pkt_cnt = pkt_cnt;
            interval_end = r;
            if r.as_secs_f64() < packet.send_time.as_secs_f64() {
                // println!("warning! {} {}", r.as_secs_f64(), packet.send_time.as_secs_f64());
                continue;
            }
            tmp_lat_list.push(r - packet.send_time);
        }
        // if pkt_cnt == 200 {
        if interval_end - interval_start >= Duration::from_millis(500) {
            // let tmp_tput_list = vec![(interval_end - interval_start, valid_pkt_cnt); 1];
            let mut rtt_mean = tmp_lat_list
                .iter()
                .map(|&x| x.as_secs_f64())
                .collect::<Vec<f64>>()
                .iter()
                .sum::<f64>()
                / tmp_lat_list.len() as f64;
            // let rtt_mean = rtt_mean.min(0.35_f64);
            // println!(
            //     "{} {} {} {}",
            //     (interval_end - first_pkt_time.unwrap()).as_secs_f64(),
            //     tput_mean,
            //     rtt_mean,
            //     tput_mean - rtt_mean * lambda
            // );
            println!(
                "{} {} {}",
                (interval_start - first_pkt_time.unwrap()).as_secs_f64(),
                rtt_mean * 1000.0,
                valid_pkt_cnt as f64 * 1448.0 * 8.0
                    / (interval_end - interval_start).as_secs_f64()
                    / 1024.0
                    / 1024.0,
            );
            rtt_mean = rtt_mean.min(0.5_f64);
            latency_list.push(Duration::from_secs_f64(rtt_mean));
            // throughput_list.extend(tmp_tput_list.iter());
            // latency_list.push(Duration::from_secs_f64(rtt_mean / 1000.0));
            throughput_list.push((interval_end - interval_start, valid_pkt_cnt));
            // total_pkt_cnt += valid_pkt_cnt;
            pkt_cnt = 0;
            valid_pkt_cnt = 0;
            tmp_lat_list.clear();
            interval_end = interval_start;
        }
    }
    // println!("{}", tmp_lat_list.len());
    // if tmp_lat_list.len() > 0 && valid_pkt_cnt > 0 {
    // let rtt_mean = tmp_lat_list
    //     .iter()
    //     .map(|&x| x.as_secs_f64())
    //     .collect::<Vec<f64>>()
    //     .iter()
    //     .sum::<f64>()
    //     / tmp_lat_list.len() as f64;
    // let tmp_tput_list = vec![(end_time - start_time, total_pkt_cnt); latency_list.len()];
    // println!("interval end: {}, first pkt: {}, total pkts: {}", interval_end.as_secs_f64(), first_pkt_time.unwrap().as_secs_f64(), total_pkt_cnt);
    // throughput_list.extend(tmp_tput_list.iter());
    // latency_list.push(Duration::from_secs_f64(rtt_mean));
    // }
    (latency_list, throughput_list)
}

async fn get_data_from_pcap(
    f: &str,
    port: usize,
    start_time: Duration,
    end_time: Duration,
    lambda: f64,
) -> (Vec<f64>, Vec<f64>) {
    // fn average(numbers: &[u64]) -> f64 {
    //     numbers.iter().sum::<u64>() as f64 / numbers.len() as f64
    // }
    let (latency, throughput) = parse_pcap(f, port, start_time, end_time, lambda).await;
    let latency_in_ms = latency
        .iter()
        .map(|x| x.as_secs_f64() * 1000.0)
        .collect::<Vec<f64>>();
    let throughput_in_mbps = throughput
        .iter()
        .map(|t| {
            let interval = t.0.as_secs_f64();
            let cnt = t.1;
            cnt as f64 * 1448.0 * 8.0 / interval / 1024.0 / 1024.0
        })
        .collect();
    // Return value is average latency in ms
    (latency_in_ms, throughput_in_mbps)
}

async fn analyze_pcap_statistic<P>(result_dir: P, opts: &CommandArgs) -> Result<()>
where
    P: AsRef<Path>,
{
    let concurrent_num = opts.tasks;
    let prefix = &opts.prefix;
    let lambda = opts.lambda;
    let start_time = opts.start_time;
    let end_time = opts.end_time;
    let port = opts.port;
    let mut tasks = FuturesUnordered::new();
    let mut avg_lat: Vec<f64> = Vec::new();
    let mut avg_thr: Vec<f64> = Vec::new();
    let mut origin_lat: Vec<f64> = Vec::new();
    let mut origin_thr: Vec<f64> = Vec::new();
    let mut realtime_qoe: Vec<f64> = Vec::new();
    for entry in std::fs::read_dir(result_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let metadata = fs::metadata(&path)?;
        if metadata.is_file() && path.extension().unwrap() == "pcap" {
            let file_name = path.file_name().ok_or("No filename").unwrap();
            if is_match_file(file_name, prefix) {
                tasks.push(tokio::spawn(async move {
                    let p = path.to_str().unwrap().to_owned();
                    get_data_from_pcap(&p, port, start_time, end_time, lambda).await
                }));
            }
        }

        if tasks.len() >= concurrent_num {
            if let Some(r) = tasks.next().await {
                let (mut lat, mut throughput_list) = r.unwrap();
                let mut qoe: Vec<f64> = (throughput_list.iter().zip(lat.iter()))
                    .map(|(&tput, &lat)| tput - lambda * lat)
                    .collect();
                // println!("{}", lat.iter().sum::<f64>() / lat.len() as f64);
                avg_lat.push(lat.iter().sum::<f64>() / lat.len() as f64);
                avg_thr.push(throughput_list.iter().sum::<f64>() / throughput_list.len() as f64);
                origin_lat.append(&mut lat);
                origin_thr.append(&mut throughput_list);
                realtime_qoe.append(&mut qoe)
            }
        }
    }
    while let Some(r) = tasks.next().await {
        let (mut lat, mut throughput_list) = r.unwrap();
        let mut qoe: Vec<f64> = (throughput_list.iter().zip(lat.iter()))
            .map(|(&tput, &lat)| tput - lambda * lat)
            .collect();
        // println!("{}", lat.iter().sum::<f64>() / lat.len() as f64);
        avg_lat.push(lat.iter().sum::<f64>() / lat.len() as f64);
        avg_thr.push(throughput_list.iter().sum::<f64>() / throughput_list.len() as f64);
        origin_lat.append(&mut lat);
        origin_thr.append(&mut throughput_list);
        realtime_qoe.append(&mut qoe)
    }

    origin_lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    origin_thr.sort_by(|a, b| a.partial_cmp(b).unwrap());
    realtime_qoe.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let percentiles: Vec<f64>;
    let lat: Vec<f64>;
    let thr: Vec<f64>;
    if let Some(cnt) = opts.percentile {
        percentiles = (0..cnt).map(|e| e as f64 / cnt as f64).collect();
        lat = percentiles
            .iter()
            .map(|e| origin_lat[(e * origin_lat.len() as f64) as usize])
            .collect();
        thr = percentiles
            .iter()
            .map(|e| origin_thr[(e * origin_thr.len() as f64) as usize])
            .collect();
        realtime_qoe = percentiles
            .iter()
            .map(|e| realtime_qoe[(e * realtime_qoe.len() as f64) as usize])
            .collect();
    } else {
        percentiles = (0..origin_lat.len())
            .map(|e| e as f64 / origin_lat.len() as f64)
            .collect();
        lat = origin_lat;
        thr = origin_thr;
    };
    let qoe: Vec<f64> = (avg_lat.iter().zip(avg_thr.iter()))
        .map(|(&d, &t)| {
            println!("{t} {d}");
            t - lambda * d
        })
        .collect();
    qoe.iter().for_each(|&qoe| println!("{qoe}"));

    let s1 = Series::new("percentile", percentiles);
    let s2 = Series::new("rtt", lat);
    let s3 = Series::new("tput", thr);
    let s4 = Series::new("realtime_qoe", realtime_qoe);

    let mut df = DataFrame::new(vec![s1, s2, s3, s4])?;
    let prefix_name = if prefix.is_empty() {
        "origin".to_string()
    } else {
        prefix[0].clone()
    };
    write_df_csv(&mut df, format!("{prefix_name}-pcap.csv"))?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // tracing_subscriber::registry()
    //     .with(fmt::layer())
    //     .with(env_filter)
    //     .init();
    let opts = CommandArgs::parse();
    // let config = DbConfig::new()?;
    let result_dir = &opts.output;
    // analyze_frame_statistic(Path::new(result_dir).join("frame"), &opts, &config).await?;
    if opts.pcap {
        analyze_pcap_statistic(Path::new(result_dir).join("pcap"), &opts).await?;
    }
    Ok(())
}
