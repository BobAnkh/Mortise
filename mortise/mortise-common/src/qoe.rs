use plain::Plain;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const BASE_SSIM: f64 = 14.4;
const MID_SSIM: f64 = 18.0 - BASE_SSIM;
const HIGH_SSIM: f64 = 19.7 - BASE_SSIM;
const DELAY_IGNORE_THRESHOLD: f64 = 80.0;
const DELAY_DDL: f64 = 120.0;
const DELAY_LIMIT: f64 = 150.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameQoE {
    pub server_send: u64,
    pub client_recv: u64,
    pub server_recv: u64,
    pub size: u64,
    pub frame_interval: Duration,
    pub frame_id: u64,
}

impl FrameQoE {
    pub fn bitrate_kbps(&self) -> f64 {
        self.size as f64 / self.frame_interval.as_secs_f64() * 8.0 / 1024.0
    }

    pub fn ssim(&self) -> f64 {
        let ssim = 5.0 * (self.bitrate_kbps() / 20.0).log10() + 6.0 - BASE_SSIM;
        if ssim < 0.0 {
            0.0
        } else {
            ssim
        }
    }

    pub fn delay_ms(&self) -> f64 {
        (self.server_recv - self.server_send) as f64 / 1_000_000.0
    }

    pub fn ssim_reward(&self) -> f64 {
        let ssim = self.ssim();
        if ssim <= MID_SSIM {
            3.1 * ssim
        } else if ssim <= HIGH_SSIM && ssim > MID_SSIM {
            1.55 * (ssim - MID_SSIM) + 3.1 * MID_SSIM
        } else {
            0.75 * (ssim - HIGH_SSIM) + 1.55 * (HIGH_SSIM - MID_SSIM) + 3.1 * MID_SSIM
        }
    }

    pub fn delay_punish(&self) -> f64 {
        let delay = self.delay_ms().min(DELAY_LIMIT);
        if delay <= DELAY_IGNORE_THRESHOLD {
            0.0
        } else if delay > DELAY_IGNORE_THRESHOLD && delay < DELAY_DDL {
            0.04 * (delay - DELAY_IGNORE_THRESHOLD)
        } else {
            0.002 * (delay - DELAY_DDL + 1.0) * (delay - DELAY_DDL + 1.0)
                + 0.04 * (DELAY_DDL - DELAY_IGNORE_THRESHOLD)
        }
    }

    pub fn score(&self) -> f64 {
        // TODO: change SSIM function
        // -1.92 * 0.001 * delay + 0.101 * ssim + 2.67
        let ssim_reward = self.ssim_reward();
        let delay_punish = self.delay_punish();
        -delay_punish + ssim_reward - 9.2
    }
}

impl AppInfo {
    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self as *const Self as *const u8,
                std::mem::size_of_val(self),
            )
        }
    }

    pub fn from_bytes(buf: &[u8]) -> &Self {
        plain::from_bytes(buf).expect("The buffer is either too short or not aligned!")
    }

    pub fn from_mut_bytes(buf: &mut [u8]) -> &mut Self {
        plain::from_mut_bytes(buf).expect("The buffer is either too short or not aligned!")
    }

    pub fn copy_from_bytes(buf: &[u8]) -> Self {
        let mut h = Self::default();
        h.copy_from_bytes(buf).expect("The buffer is too short!");
        h
    }
}

#[derive(Debug, Copy, Clone, Default)]
#[repr(C)]
pub struct AppInfo {
    pub req: u64,
    pub resp: u64,
}

unsafe impl Plain for AppInfo {}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::FrameQoE;

    #[test]
    fn test_qoe() {
        let frame_4mbps_80ms = FrameQoE {
            server_send: 0,
            server_recv: Duration::from_millis(80).as_nanos() as u64,
            client_recv: 0,
            size: 8738,
            frame_interval: Duration::from_micros(16667),
            frame_id: 0,
        };
        let frame_4_4mbps_96ms = FrameQoE {
            server_send: 0,
            server_recv: Duration::from_millis(96).as_nanos() as u64,
            client_recv: 0,
            size: 8738 * 110 / 100,
            frame_interval: Duration::from_micros(16667),
            frame_id: 0,
        };
        println!(
            "ssim: {},{}",
            frame_4mbps_80ms.ssim(),
            frame_4_4mbps_96ms.ssim()
        );
        println!(
            "delay_punish: {},{}",
            frame_4mbps_80ms.delay_punish(),
            frame_4_4mbps_96ms.delay_punish()
        );
        println!(
            "ssim_reward: {},{}",
            frame_4mbps_80ms.ssim_reward(),
            frame_4_4mbps_96ms.ssim_reward()
        );
        println!(
            "qoe: {},{}",
            frame_4mbps_80ms.score(),
            frame_4_4mbps_96ms.score()
        );

        let frame_8mbps_80ms = FrameQoE {
            server_send: 0,
            server_recv: Duration::from_millis(80).as_nanos() as u64,
            client_recv: 0,
            size: 17476,
            frame_interval: Duration::from_micros(16667),
            frame_id: 0,
        };
        let frame_8_8mbps_88ms = FrameQoE {
            server_send: 0,
            server_recv: Duration::from_millis(88).as_nanos() as u64,
            client_recv: 0,
            size: 17476 * 110 / 100,
            frame_interval: Duration::from_micros(16667),
            frame_id: 0,
        };
        println!(
            "ssim: {},{}",
            frame_8mbps_80ms.ssim(),
            frame_8_8mbps_88ms.ssim()
        );
        println!(
            "delay_punish: {},{}",
            frame_8mbps_80ms.delay_punish(),
            frame_8_8mbps_88ms.delay_punish()
        );
        println!(
            "ssim_reward: {},{}",
            frame_8mbps_80ms.ssim_reward(),
            frame_8_8mbps_88ms.ssim_reward()
        );
        println!(
            "qoe: {},{}",
            frame_8mbps_80ms.score(),
            frame_8_8mbps_88ms.score()
        );

        let frame_12mbps_80ms = FrameQoE {
            server_send: 0,
            server_recv: Duration::from_millis(80).as_nanos() as u64,
            client_recv: 0,
            size: 26214,
            frame_interval: Duration::from_micros(16667),
            frame_id: 0,
        };
        let frame_13_2mbps_84ms = FrameQoE {
            server_send: 0,
            server_recv: Duration::from_millis(84).as_nanos() as u64,
            client_recv: 0,
            size: 26214 * 110 / 100,
            frame_interval: Duration::from_micros(16667),
            frame_id: 0,
        };
        println!(
            "ssim: {},{}",
            frame_12mbps_80ms.ssim(),
            frame_13_2mbps_84ms.ssim()
        );
        println!(
            "delay_punish: {},{}",
            frame_12mbps_80ms.delay_punish(),
            frame_13_2mbps_84ms.delay_punish()
        );
        println!(
            "ssim_reward: {},{}",
            frame_12mbps_80ms.ssim_reward(),
            frame_13_2mbps_84ms.ssim_reward()
        );
        println!(
            "qoe: {},{}",
            frame_12mbps_80ms.score(),
            frame_13_2mbps_84ms.score()
        );
    }
}
