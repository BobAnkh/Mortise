use crate::{ConnectOption, ManagerOperation, SkArrayMap};
use clap::ValueEnum;
use speedy::{Readable, Writable};

#[derive(ValueEnum, Clone, Debug, Writable, Readable)]
#[clap(rename_all = "snake_case")]
pub enum CongestionOpt {
    Cubic,
    MortiseCopa,
    Mvfst,
    Vegas,
    CCP,
    Copa,
    Bbr,
}

impl std::fmt::Display for CongestionOpt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CongestionOpt::MortiseCopa => write!(f, "mortise_copa"),
            CongestionOpt::Mvfst => write!(f, "mvfst"),
            CongestionOpt::Cubic => write!(f, "cubic"),
            CongestionOpt::Vegas => write!(f, "vegas"),
            CongestionOpt::CCP => write!(f, "ccp"),
            CongestionOpt::Copa => write!(f, "copa"),
            CongestionOpt::Bbr => write!(f, "bbr"),
        }
    }
}

impl CongestionOpt {
    pub fn get_tcp_ca(&self) -> (u32, &[u8]) {
        match self {
            CongestionOpt::MortiseCopa => (1, b"mortise_copa"),
            CongestionOpt::Cubic => (0, b"cubic"),
            CongestionOpt::Vegas => (0, b"vegas"),
            CongestionOpt::Mvfst => (0, b"mvfst"),
            CongestionOpt::CCP => (0, b"ccp"),
            CongestionOpt::Copa => (0, b"copa"),
            CongestionOpt::Bbr => (0, b"bbr"),
        }
    }

    pub fn get_obj_id(&self) -> u32 {
        self.get_tcp_ca().0
    }

    pub fn get_tcp_ca_name(&self) -> &[u8] {
        self.get_tcp_ca().1
    }

    pub fn get_load_option(&self) -> ManagerOperation {
        match self {
            CongestionOpt::MortiseCopa => {
                let sk_array_maps = vec![
                    SkArrayMap {
                        name: None,
                        mim: "mim_rtt".to_string(),
                        value_size: 16,
                        max_entries: 100000,
                    },
                    SkArrayMap {
                        name: None,
                        mim: "mim_increase".to_string(),
                        value_size: 8,
                        max_entries: 100000,
                    },
                ];
                ManagerOperation::Load {
                    path: "/home/vagrant/algorithm/bpf-kern/build/mortise_copa.bpf.o".to_string(),
                    option: Some(ConnectOption { sk_array_maps }),
                }
            }
            CongestionOpt::Mvfst => ManagerOperation::Load {
                path: "".to_string(),
                option: None,
            },
            CongestionOpt::Copa => ManagerOperation::Load {
                path: "".to_string(),
                option: None,
            },
            CongestionOpt::Bbr => ManagerOperation::Load {
                path: "".to_string(),
                option: None,
            },
            CongestionOpt::CCP => ManagerOperation::Load {
                path: "".to_string(),
                option: None,
            },
            CongestionOpt::Cubic => ManagerOperation::Load {
                path: "".to_string(),
                option: None,
            },
            CongestionOpt::Vegas => ManagerOperation::Load {
                path: "".to_string(),
                option: None,
            },
        }
    }
}
