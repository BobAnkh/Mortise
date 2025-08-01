use crate::{MortiseManagedObject, MortiseObject, MortiseOpenObject};
use libbpf_rs::{MapFlags as BpfMapFlags, MapHandle as BpfMapHandle, MapType as BpfMapType};
use mortise_common::{
    bump_memlock_rlimit, bump_nofile_rlimit,
    pidfd::{pid_open, pidfd_getfd},
    qoe::AppInfo,
    ConnectOption, MemorySize, MortiseError, Result,
};
use rustc_hash::FxHashMap as HashMap;
use std::{
    os::fd::{AsFd, AsRawFd},
    sync::{atomic::AtomicBool, Arc},
    thread::{self, JoinHandle},
};

pub struct RingBufManager {
    pub notify: Arc<AtomicBool>,
    pub handle: thread::JoinHandle<()>,
}

pub struct FlowMetadata {
    pub pid: i32,
    pub sk_fd: i32,
    pub local_sk_fd: i32,
    pub obj_id: u32,
}

/// Local socket file descriptor cell
///
/// Should be global unique so that will not double close the local socket file descriptor.
///
/// Store flow_id in order to loop up the FlowMetadata.
pub struct SkFdCell {
    pub local_sk_fd: i32,
    pub flow_id: u32,
}

/// Hold the pid_fd and all SkFdCell related to this pid.
pub struct PidManager {
    pub pid_fd: i32,
    // record <raw_sk_fd from remote process, FlowMetadata>
    pub sk_fd_map: HashMap<i32, SkFdCell>,
}

#[derive(Default)]
pub struct FlowManager {
    pub pid_map: HashMap<i32, PidManager>,
    // record <flow_id,  FlowMetadata> held by this process
    pub flow_map: HashMap<u32, FlowMetadata>,
    // self-incr flow_id
    pub flow_id: u32,
}

pub struct MortiseManager {
    pub obj_id: u32,
    pub objs: HashMap<u32, MortiseManagedObject<MortiseObject>>,
    pub open_objs: HashMap<u32, MortiseManagedObject<MortiseOpenObject>>,
    pub rb_manager: Option<RingBufManager>,
    pub flow_manager: FlowManager,
}

impl Drop for SkFdCell {
    fn drop(&mut self) {
        tracing::trace!(target: "manager:flow", "Dropping SkFdCell for {}", self.local_sk_fd);
        unsafe {
            let _ = libc::close(self.local_sk_fd);
        }
    }
}

impl Drop for PidManager {
    fn drop(&mut self) {
        tracing::trace!(target: "manager:flow", "Dropping SkFdManager for {}", self.pid_fd);
        for (_, sk_fd_cell) in self.sk_fd_map.drain() {
            drop(sk_fd_cell);
        }
        unsafe {
            let _ = libc::close(self.pid_fd);
        }
    }
}

impl PidManager {
    pub fn new(pid: i32) -> Result<Self> {
        let pid_fd = match pid_open(pid, false) {
            Ok(fd) => fd,
            Err(e) => {
                tracing::error!(target: "manager:flow", "Failed to open pid_fd of {}: {}", pid, e);
                return Err(e);
            }
        };
        Ok(PidManager {
            pid_fd,
            sk_fd_map: HashMap::default(),
        })
    }
}

impl Drop for FlowManager {
    fn drop(&mut self) {
        tracing::trace!(target: "manager:flow", "Dropping PidFdManager");
        for (_, sk_fd_manager) in self.pid_map.drain() {
            drop(sk_fd_manager);
        }
    }
}

impl FlowManager {
    pub fn new() -> Self {
        Self {
            pid_map: HashMap::default(),
            flow_map: HashMap::default(),
            flow_id: 0,
        }
    }

    pub fn contains(&self, pid: i32, sk_fd: i32) -> bool {
        if let Some(sk_fd_manager) = self.pid_map.get(&pid) {
            if sk_fd_manager.sk_fd_map.contains_key(&sk_fd) {
                return true;
            }
        }
        false
    }

    pub fn insert(&mut self, pid: i32, sk_fd: i32, obj_id: u32) -> Result<u32> {
        // If the pid as never been connected, create a new SkFdManager
        let sk_fd_manager = if let Some(m) = self.pid_map.get_mut(&pid) {
            m
        } else {
            let sk_fd_manager = PidManager::new(pid)?;
            self.pid_map.insert(pid, sk_fd_manager);
            self.pid_map.get_mut(&pid).unwrap()
        };

        // If the sk_fd has already been connected, return the flow_id with a specific error type
        if let Some(sk_fd_cell) = sk_fd_manager.sk_fd_map.get(&sk_fd) {
            tracing::warn!(target: "manager:flow", "sk_fd {} of pid {} already connected", sk_fd, pid);
            return Err(MortiseError::FlowConnected(sk_fd_cell.flow_id));
        }
        let local_sk_fd = match pidfd_getfd(sk_fd_manager.pid_fd, sk_fd) {
            Ok(fd) => fd,
            Err(e) => {
                tracing::error!(target: "manager:flow", "get local sk fd error: {}", e);
                return Err(e);
            }
        };
        self.flow_id += 1;
        let flow_metadata = FlowMetadata {
            pid,
            sk_fd,
            local_sk_fd,
            obj_id,
        };
        sk_fd_manager.sk_fd_map.insert(
            sk_fd,
            SkFdCell {
                local_sk_fd,
                flow_id: self.flow_id,
            },
        );
        self.flow_map.insert(self.flow_id, flow_metadata);
        Ok(self.flow_id)
    }

    pub fn remove(&mut self, flow_id: u32) -> Option<FlowMetadata> {
        if let Some(flow_metadata) = self.flow_map.remove(&flow_id) {
            if let Some(pid_manager) = self.pid_map.get_mut(&flow_metadata.pid) {
                if let Some(sk_fd_cell) = pid_manager.sk_fd_map.remove(&flow_metadata.sk_fd) {
                    drop(sk_fd_cell);
                }
                if pid_manager.sk_fd_map.is_empty() {
                    self.pid_map.remove(&flow_metadata.pid);
                }
            }
            return Some(flow_metadata);
        }
        None
    }
}

impl Default for MortiseManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MortiseManager {
    pub fn new() -> Self {
        bump_memlock_rlimit(MemorySize::MB(1024), MemorySize::MB(1024)).unwrap();
        bump_nofile_rlimit(8192, 8192).unwrap();
        Self {
            obj_id: 0,
            objs: HashMap::default(),
            open_objs: HashMap::default(),
            rb_manager: None,
            flow_manager: FlowManager::new(),
        }
    }

    /// Returns a handle to an open object. The handle can be used to refer to the
    /// object in future calls to other functions in this module.
    ///
    /// If the open operation fails, the error is printed to stderr and the
    /// program exits.
    ///
    /// # Arguments
    ///
    /// * `path` - A string containing the path to the object file.
    ///
    /// # Returns
    ///
    /// A handle to the open object.
    pub fn open_object(&mut self, path: String) -> Result<u32> {
        let mut obj_builder = libbpf_rs::ObjectBuilder::default();
        obj_builder.name(&path).relaxed_maps(true);
        let obj = obj_builder.open_file(path.clone())?;
        let obj = MortiseOpenObject { object: obj };
        let obj = MortiseManagedObject { path, object: obj };
        self.obj_id += 1;
        self.open_objs.insert(self.obj_id, obj);
        Ok(self.obj_id)
    }

    pub fn insert_object(&mut self, obj_id: u32, path: String) -> Result<u32> {
        let mut obj_builder = libbpf_rs::ObjectBuilder::default();
        obj_builder.name(&path).relaxed_maps(true);
        let obj = obj_builder.open_file(path.clone())?;
        let obj = MortiseOpenObject { object: obj };
        let obj = MortiseManagedObject { path, object: obj };
        self.obj_id = std::cmp::max(self.obj_id, obj_id);
        self.open_objs.insert(obj_id, obj);
        Ok(obj_id)
    }

    pub fn close_object(&mut self, obj_id: u32) -> Result<()> {
        self.open_objs
            .remove(&obj_id)
            .ok_or_else(|| MortiseError::ObjectNotFound(obj_id))?;
        Ok(())
    }

    pub fn load_object(&mut self, obj_id: u32, option: Option<ConnectOption>) -> Result<()> {
        let obj = self
            .open_objs
            .remove(&obj_id)
            .ok_or_else(|| MortiseError::ObjectNotFound(obj_id))?;
        let mut obj = obj.load(option)?;
        obj.attach_struct_ops()?;
        self.objs.insert(obj_id, obj);
        Ok(())
    }

    pub fn open_and_load_object(
        &mut self,
        path: String,
        option: Option<ConnectOption>,
    ) -> Result<u32> {
        let obj_id = self.open_object(path)?;
        self.load_object(obj_id, option)?;
        Ok(obj_id)
    }

    pub fn insert_and_load_object(
        &mut self,
        obj_id: u32,
        path: String,
        option: Option<ConnectOption>,
    ) -> Result<u32> {
        self.insert_object(obj_id, path)?;
        self.load_object(obj_id, option)?;
        Ok(obj_id)
    }

    pub fn unload_object(&mut self, obj_id: u32) -> Result<()> {
        self.objs
            .remove(&obj_id)
            .ok_or_else(|| MortiseError::ObjectNotFound(obj_id))?;
        Ok(())
    }

    pub fn get_object(&self, obj_id: u32) -> Result<&MortiseManagedObject<MortiseObject>> {
        self.objs
            .get(&obj_id)
            .ok_or_else(|| MortiseError::ObjectNotFound(obj_id))
    }

    pub fn get_object_mut(
        &mut self,
        obj_id: u32,
    ) -> Result<&mut MortiseManagedObject<MortiseObject>> {
        self.objs
            .get_mut(&obj_id)
            .ok_or_else(|| MortiseError::ObjectNotFound(obj_id))
    }

    pub fn get_open_object(&self, obj_id: u32) -> Result<&MortiseManagedObject<MortiseOpenObject>> {
        self.open_objs
            .get(&obj_id)
            .ok_or_else(|| MortiseError::ObjectNotFound(obj_id))
    }

    pub fn get_open_object_mut(
        &mut self,
        obj_id: u32,
    ) -> Result<&mut MortiseManagedObject<MortiseOpenObject>> {
        self.open_objs
            .get_mut(&obj_id)
            .ok_or_else(|| MortiseError::ObjectNotFound(obj_id))
    }

    pub fn update_map<T>(
        &mut self,
        obj_id: u32,
        map_name: T,
        key: &[u8],
        val: &[u8],
        flags: BpfMapFlags,
    ) -> Result<()>
    where
        T: AsRef<str>,
    {
        let map = self
            .get_object_mut(obj_id)?
            .map_mut(&map_name)
            .ok_or_else(|| MortiseError::MapNotFound(map_name.as_ref().to_string()))?;
        map.update(key, val, flags)?;
        Ok(())
    }

    pub fn lookup_map<T>(&self, obj_id: u32, map_name: T, key: &[u8]) -> Result<Vec<u8>>
    where
        T: AsRef<str>,
    {
        let map = self
            .get_object(obj_id)?
            .map(&map_name)
            .ok_or_else(|| MortiseError::MapNotFound(map_name.as_ref().to_string()))?;
        let res = map
            .lookup(key, libbpf_rs::MapFlags::empty())?
            .ok_or_else(|| MortiseError::ElemNotFound(map_name.as_ref().to_string()))?;
        Ok(res)
    }

    // TODO: change interface, may have other names for ring buffer
    pub fn get_rb_map_handles(&mut self, obj_ids: &[u32]) -> Result<Vec<BpfMapHandle>> {
        let mut handles = Vec::new();
        for obj_id in obj_ids {
            let obj = self.get_object_mut(*obj_id)?;
            let map = obj
                .map_mut("rb")
                .ok_or_else(|| MortiseError::MapNotFound("rb".to_string()))?;
            let handle = BpfMapHandle::try_clone(map)?;
            handles.push(handle);
        }
        Ok(handles)
    }

    pub fn register_rb(&mut self, notify: Arc<AtomicBool>, handle: JoinHandle<()>) -> Result<()> {
        self.rb_manager = Some(RingBufManager { notify, handle });
        Ok(())
    }

    pub fn unregister_rb(&mut self) -> Result<()> {
        if let Some(rb_manager) = self.rb_manager.take() {
            rb_manager
                .notify
                .store(false, std::sync::atomic::Ordering::Relaxed);
            rb_manager
                .handle
                .join()
                .map_err(|_| MortiseError::JoinError)?;
        }
        Ok(())
    }

    pub fn get_flow_metadata(&self, flow_id: u32) -> Option<&FlowMetadata> {
        self.flow_manager.flow_map.get(&flow_id)
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.unregister_rb()?;
        self.objs.clear();
        self.open_objs.clear();
        Ok(())
    }

    pub fn connect(
        &mut self,
        pid: i32,
        obj_id: u32,
        sk_fd: i32,
        default_app_info: Option<u64>,
    ) -> Result<u32> {
        // TODO: handle double connect, insert should return a error indicating the flow_id is already in use
        // We can make an enum to hold the flow_id
        let flow_id = match self.flow_manager.insert(pid, sk_fd, obj_id) {
            Ok(id) => id,
            Err(e) => {
                if let MortiseError::FlowConnected(flow_id) = e {
                    return Ok(flow_id);
                } else {
                    tracing::error!(target: "manager:flow", "Failed to insert flow metadata: {}", e);
                    return Err(e);
                }
            }
        };
        let local_sk_fd = self.get_flow_metadata(flow_id).unwrap().local_sk_fd;
        let obj = self.get_object_mut(obj_id)?;
        if let Some(option) = obj.connect_option() {
            if !option.sk_array_maps.is_empty() {
                let mut new_maps = Vec::new();
                for sk_array_map in option.sk_array_maps.iter() {
                    let map = obj
                        .map_mut(&sk_array_map.mim)
                        .ok_or_else(|| MortiseError::MapNotFound(sk_array_map.mim.clone()))?;
                    tracing::debug!(target: "manager:flow", "map name: {}", map.name());
                    let opts = libbpf_rs::libbpf_sys::bpf_map_create_opts {
                        sz: std::mem::size_of::<libbpf_rs::libbpf_sys::bpf_map_create_opts>()
                            as libbpf_rs::libbpf_sys::size_t,
                        map_flags: libbpf_rs::libbpf_sys::BPF_F_NO_PREALLOC,
                        btf_fd: 0,
                        btf_key_type_id: 0,
                        btf_value_type_id: 0,
                        btf_vmlinux_value_type_id: 0,
                        inner_map_fd: 0,
                        map_extra: 0,
                        numa_node: 0,
                        map_ifindex: 0,
                    };
                    let sub_map = match BpfMapHandle::create::<String>(
                        BpfMapType::Hash,
                        None,
                        4,
                        sk_array_map.value_size,
                        sk_array_map.max_entries,
                        &opts,
                    ) {
                        Ok(map) => map,
                        Err(e) => {
                            tracing::error!(target: "manager:flow", "Failed to create map: {}", e);
                            return Err(e.into());
                        }
                    };
                    tracing::debug!(target: "manager:flow", "new map created: {}", sub_map.name());
                    let map_fd = sub_map.as_fd().as_raw_fd();
                    new_maps.push(sub_map);
                    // should pin the map to /sys/fs/bpf/xxx_map
                    let key = flow_id.to_ne_bytes();
                    let val = map_fd.to_ne_bytes();
                    if let Err(e) = map.update(&key, &val, BpfMapFlags::ANY) {
                        if let libbpf_rs::Error::System(7) = e {
                            tracing::error!(target: "manager:flow", "Exceed max_entries of map {}", map.name());
                        } else {
                            tracing::error!(target: "manager:flow", "Failed to update map {}: {}", map.name(), e);
                        }
                        return Err(e.into());
                    }
                    tracing::debug!(target: "manager:flow", "Successfully connect {flow_id} -> {map_fd}");
                }
                obj.set_sk_array_maps(flow_id, new_maps);

                // update flow_id
                let flow_id_map = obj
                    .map_mut("flow_id_stg")
                    .ok_or_else(|| MortiseError::MapNotFound("flow_id_stg".to_string()))?;
                let key = local_sk_fd.to_ne_bytes();
                let val = flow_id.to_ne_bytes();
                flow_id_map.update(&key, &val, BpfMapFlags::ANY)?;
                tracing::debug!(target: "manager:flow", "Updated map {}", flow_id_map.name());
            }
        }
        if let Some(default_app_info) = default_app_info {
            let app_info_map = obj
                .map_mut("sk_stg_map")
                .ok_or_else(|| MortiseError::MapNotFound("sk_stg_map".to_string()))?;
            let key = local_sk_fd.to_ne_bytes();
            let app_info = AppInfo {
                req: default_app_info,
                resp: 0,
            };
            let val = Vec::from(app_info.as_bytes());
            app_info_map.update(&key, &val, BpfMapFlags::ANY)?;
            tracing::debug!(target: "manager:flow", "Updated map {}", app_info_map.name());
        }
        Ok(flow_id)
    }

    pub fn disconnect(&mut self, flow_id: u32) -> Result<()> {
        if let Some(metadata) = self.flow_manager.remove(flow_id) {
            let obj_id = metadata.obj_id;
            let obj = self.get_object_mut(obj_id)?;
            if let Some(option) = obj.connect_option() {
                if !option.sk_array_maps.is_empty() {
                    for sk_array_map in option.sk_array_maps.iter() {
                        let map = obj
                            .map_mut(&sk_array_map.mim)
                            .ok_or_else(|| MortiseError::MapNotFound(sk_array_map.mim.clone()))?;
                        let key = flow_id.to_ne_bytes();
                        map.delete(&key)?;
                    }
                    obj.remove_sk_array_maps(flow_id);
                }
            }
        }
        Ok(())
    }
}
