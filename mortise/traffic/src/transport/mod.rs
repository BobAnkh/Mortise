pub mod io;

use crate::{AppOpt, ModeOpt};
pub use mortise_common::CongestionOpt;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum RateCtrlOp {
    Ready,
    Send(SendChunkInfo),
    Done,
}

#[derive(Debug, Clone)]
pub struct SendChunkInfo {
    pub id: u64,
    pub data_bytes: u64,
    // pub bitrate: Bandwidth,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TransportInfo {
    pub total_write_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct TransportOpt {
    pub frame: u64,
    pub mode: ModeOpt,
    pub sk_fd: i32,
    pub congestion: CongestionOpt,
    pub app: AppOpt,
}

#[async_trait::async_trait]
pub trait RateController {
    async fn next_chunk(&mut self, id: u64) -> Option<SendChunkInfo>;
    fn get_chunk_interval(&self) -> &Duration;
    fn get_sample_interval(&self) -> &Duration;
    fn update_rate_sample(&mut self) {}
}
