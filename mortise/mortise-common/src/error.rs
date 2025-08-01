use thiserror::Error;

use crate::op::ManagerIpcOperation;

/// Nix Result Type
pub type Result<T> = std::result::Result<T, MortiseError>;

#[derive(Error, Debug)]
pub enum MortiseError {
    #[error("Nix error: {0}")]
    NixError(#[from] nix::Error),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Libbpf error: {0}")]
    BpfError(#[from] libbpf_rs::Error),
    #[error("Object of id {0} not found")]
    ObjectNotFound(u32),
    #[error("Map of name {0} not found")]
    MapNotFound(String),
    #[error("Element of Map {0} not found")]
    ElemNotFound(String),
    #[error("Flow of id {0} not found")]
    FlowNotFound(u32),
    #[error("Fail to join thread")]
    JoinError,
    #[error("Invalid bpf flags")]
    InvalidBpfFlags,
    #[error("Manager IPC channel send error: {0}")]
    ManagerChannelSendError(#[from] tokio::sync::mpsc::error::SendError<ManagerIpcOperation>),
    #[error("Manager IPC channel recv error: {0}")]
    ManagerChannelRecvError(#[from] tokio::sync::oneshot::error::RecvError),
    #[error("Flow of id {0} already connected")]
    FlowConnected(u32),
    #[error("Unknown data store error: {0}")]
    Unknown(String),
    #[error("{0}")]
    Custom(String),
    // #[error(transparent)]
    // Other(#[from] anyhow::Error),
}
