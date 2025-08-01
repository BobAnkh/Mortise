use crate::qoe::FrameQoE;
use crate::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SkArrayMap {
    /// The name of the outer map.
    pub mim: String,
    pub name: Option<String>,
    pub value_size: u32,
    pub max_entries: u32,
}

/// Set connect option when loading a new object.
/// This is used when a new socket is created and wants
/// to use the CCA in the object. This process is called 'connect'.
///
/// `sk_array_maps` means a list of maps that are used to store
/// CCA's data for each socket. These maps should be created from user space.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConnectOption {
    pub sk_array_maps: Vec<SkArrayMap>,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum ManagerOperation {
    Load {
        path: String,
        option: Option<ConnectOption>,
    },
    Unload {
        obj_id: u32,
    },
    Insert {
        obj_id: u32,
        path: String,
        option: Option<ConnectOption>,
    },
    Shutdown,
    PingPong,
    RegisterRingBuf {
        obj_ids: Vec<u32>,
    },
    UnregisterRingBuf,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum FlowOperation {
    SkStgMapUpdate {
        map_name: String,
        val: Vec<u8>,
        flag: u64,
    },
    SkStgMapLookup {
        map_name: String,
    },
    Connect {
        obj_id: u32,
        sk_fd: i32,
        pid: i32,
        default_app_info: Option<u64>,
    },
    Disconnect,
    QoEUpdate {
        qoe: FrameQoE,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub enum PyOperation {
    Disconnect { flow_id: u32 },
    Connect { flow_id: u32 },
}

#[derive(Debug, Deserialize, Serialize)]
pub enum Operation {
    Manager(ManagerOperation),
    Flow { flow_id: u32, op: FlowOperation },
}

impl From<ManagerOperation> for Operation {
    fn from(value: ManagerOperation) -> Self {
        Operation::Manager(value)
    }
}

impl FlowOperation {
    pub fn to_op(self, flow_id: u32) -> Operation {
        Operation::Flow { flow_id, op: self }
    }
}

#[derive(Debug)]
pub struct ManagerIpcOperation {
    pub req: Operation,
    pub resp: oneshot::Sender<Result<Vec<u8>>>,
}
