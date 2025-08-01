use plain::Plain;

impl ReportEntry {
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

#[derive(Debug, Clone)]
#[repr(C)]
pub struct ReportEntry {
    pub flow_id: u32,
    pub chunk_id: i16,
    pub chunk_len: u16,
    pub data_array: [ReportDataElem; 50],
}

unsafe impl Plain for ReportEntry {}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReportDataElem {
    pub rtt: u32,
    pub acked_bytes: u32,
    pub timestamp: u64,
}

impl Default for ReportEntry {
    fn default() -> Self {
        Self {
            flow_id: 0,
            chunk_id: 0,
            chunk_len: 0,
            data_array: [ReportDataElem::default(); 50],
        }
    }
}
