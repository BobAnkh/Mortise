use crate::private;
use libbpf_rs::{
    Link as BpfLink, Map as BpfMap, MapHandle as BpfMapHandle, MapType as BpfMapType,
    Object as BpfObject, OpenMap as BpfOpenMap, OpenObject as BpfOpenObject,
    OpenProgram as BpfOpenProgram, Program as BprProgram,
};
use mortise_common::{ConnectOption, Result};
use rustc_hash::FxHashMap as HashMap;
use std::os::fd::{AsFd, AsRawFd};

pub trait MortiseObjectState: private::MortiseSealed {}

pub struct MortiseObject {
    pub object: BpfObject,
    pub links: HashMap<String, BpfLink>,
    pub option: Option<ConnectOption>,
    pub maps: HashMap<u32, Vec<BpfMapHandle>>,
}

pub struct MortiseOpenObject {
    pub object: BpfOpenObject,
}

pub struct MortiseManagedObject<T: MortiseObjectState> {
    pub path: String,
    pub object: T,
}

impl MortiseObjectState for MortiseObject {}
impl MortiseObjectState for MortiseOpenObject {}
impl Drop for MortiseObject {
    fn drop(&mut self) {
        // Clear all the maps temporarily created by manager
        for (_, maps) in self.maps.drain() {
            for map in maps {
                let fd = map.as_fd().as_raw_fd();
                unsafe {
                    let _ = libc::close(fd);
                }
            }
        }
        // Clear links to remove struct_ops maps
        self.links.clear();
    }
}

impl MortiseManagedObject<MortiseOpenObject> {
    pub fn load(
        self,
        option: Option<ConnectOption>,
    ) -> Result<MortiseManagedObject<MortiseObject>> {
        let obj = self.object.object.load()?;
        let obj = MortiseObject {
            object: obj,
            links: HashMap::default(),
            option,
            maps: HashMap::default(),
        };
        let obj = MortiseManagedObject {
            path: self.path,
            object: obj,
        };
        Ok(obj)
    }

    pub fn map<T: AsRef<str>>(&self, name: T) -> Option<&BpfOpenMap> {
        self.object.object.map(name)
    }

    pub fn map_mut<T: AsRef<str>>(&mut self, name: T) -> Option<&mut BpfOpenMap> {
        self.object.object.map_mut(name)
    }

    pub fn maps_iter(&self) -> impl Iterator<Item = &BpfOpenMap> {
        self.object.object.maps_iter()
    }

    pub fn maps_iter_mut(&mut self) -> impl Iterator<Item = &mut BpfOpenMap> {
        self.object.object.maps_iter_mut()
    }

    pub fn prog<T: AsRef<str>>(&self, name: T) -> Option<&BpfOpenProgram> {
        self.object.object.prog(name)
    }

    pub fn prog_mut<T: AsRef<str>>(&mut self, name: T) -> Option<&mut BpfOpenProgram> {
        self.object.object.prog_mut(name)
    }

    pub fn progs_iter(&self) -> impl Iterator<Item = &BpfOpenProgram> {
        self.object.object.progs_iter()
    }

    pub fn progs_iter_mut(&mut self) -> impl Iterator<Item = &mut BpfOpenProgram> {
        self.object.object.progs_iter_mut()
    }
}

impl MortiseManagedObject<MortiseObject> {
    pub fn map<T: AsRef<str>>(&self, name: T) -> Option<&BpfMap> {
        self.object.object.map(name)
    }

    pub fn map_mut<T: AsRef<str>>(&mut self, name: T) -> Option<&mut BpfMap> {
        self.object.object.map_mut(name)
    }

    pub fn maps_iter(&self) -> impl Iterator<Item = &BpfMap> {
        self.object.object.maps_iter()
    }

    pub fn maps_iter_mut(&mut self) -> impl Iterator<Item = &mut BpfMap> {
        self.object.object.maps_iter_mut()
    }

    pub fn prog<T: AsRef<str>>(&self, name: T) -> Option<&BprProgram> {
        self.object.object.prog(name)
    }

    pub fn prog_mut<T: AsRef<str>>(&mut self, name: T) -> Option<&mut BprProgram> {
        self.object.object.prog_mut(name)
    }

    pub fn progs_iter(&self) -> impl Iterator<Item = &BprProgram> {
        self.object.object.progs_iter()
    }

    pub fn progs_iter_mut(&mut self) -> impl Iterator<Item = &mut BprProgram> {
        self.object.object.progs_iter_mut()
    }

    pub fn links(&self) -> &HashMap<String, BpfLink> {
        &self.object.links
    }

    pub fn links_mut(&mut self) -> &mut HashMap<String, BpfLink> {
        &mut self.object.links
    }

    pub fn attach_struct_ops(&mut self) -> Result<()> {
        let mut links = HashMap::default();
        self.links_mut().clear();
        for map in self.maps_iter_mut() {
            if map.map_type() == BpfMapType::StructOps {
                let link = map.attach_struct_ops()?;
                let name = map.name().to_string();
                links.insert(name, link);
            }
        }
        self.links_mut().extend(links);
        Ok(())
    }

    pub fn connect_option(&self) -> Option<ConnectOption> {
        self.object.option.clone()
    }

    pub fn set_sk_array_maps(&mut self, flow_id: u32, sk_array_maps: Vec<BpfMapHandle>) {
        self.object.maps.insert(flow_id, sk_array_maps);
    }

    pub fn remove_sk_array_maps(&mut self, flow_id: u32) {
        let maps = self.object.maps.remove(&flow_id);
        if let Some(maps) = maps {
            for map in maps {
                let fd = map.as_fd().as_raw_fd();
                unsafe {
                    let _ = libc::close(fd);
                }
            }
        }
    }
}
