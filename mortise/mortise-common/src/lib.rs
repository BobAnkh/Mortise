use nix::sys::resource::{setrlimit, Resource};
use socket2::SockAddr;
use std::net::{Ipv4Addr, SocketAddrV4};
use tcp_info_sys::get_tcp_info;

pub mod congestion;
pub mod error;
pub mod op;
pub mod pidfd;
pub mod qoe;
pub mod report;
pub mod sync;

pub use congestion::CongestionOpt;
pub use error::{MortiseError, Result};
pub use op::{
    ConnectOption, FlowOperation, ManagerIpcOperation, ManagerOperation, Operation, SkArrayMap,
};

pub const NANOS_PER_SEC: i64 = 1_000_000_000;

pub enum MemorySize {
    B(u64),
    KB(u64),
    MB(u64),
}

impl From<MemorySize> for u64 {
    fn from(size: MemorySize) -> Self {
        match size {
            MemorySize::B(b) => b,
            MemorySize::KB(kb) => kb << 10,
            MemorySize::MB(mb) => mb << 20,
        }
    }
}

impl From<&MemorySize> for u64 {
    fn from(size: &MemorySize) -> Self {
        match size {
            MemorySize::B(b) => *b,
            MemorySize::KB(kb) => *kb << 10,
            MemorySize::MB(mb) => *mb << 20,
        }
    }
}

pub fn read_be_u32(input: &mut &[u8]) -> u32 {
    let (int_bytes, rest) = input.split_at(std::mem::size_of::<u32>());
    *input = rest;
    u32::from_be_bytes(int_bytes.try_into().unwrap())
}

pub fn get_clock_ns() -> i64 {
    let ts = nix::time::clock_gettime(nix::time::ClockId::CLOCK_MONOTONIC).unwrap();
    ts.tv_sec() * NANOS_PER_SEC + ts.tv_nsec()
}

pub fn any_ipv4() -> SockAddr {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0).into()
}

pub fn bump_nofile_rlimit(soft_limit: u64, hard_limit: u64) -> Result<()> {
    setrlimit(Resource::RLIMIT_NOFILE, soft_limit, hard_limit)?;
    Ok(())
}

pub fn bump_memlock_rlimit(soft_limit: MemorySize, hard_limit: MemorySize) -> Result<()> {
    setrlimit(
        Resource::RLIMIT_MEMLOCK,
        soft_limit.into(),
        hard_limit.into(),
    )?;
    Ok(())
}

pub fn get_tcp_info_not_sent(sk_fd: i32) -> Result<u32> {
    let tcp_info = get_tcp_info(sk_fd)?;
    Ok(tcp_info.tcpi_notsent_bytes)
}

pub fn get_tcp_info_total_retrans(sk_fd: i32) -> Result<u32> {
    let tcp_info = get_tcp_info(sk_fd)?;
    Ok(tcp_info.tcpi_total_retrans)
}
