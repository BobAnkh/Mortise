use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Database {
    pub url: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CommonConfig {
    #[serde(default = "default_result_directory")]
    pub result_directory: String,
    #[serde(default = "default_tasks")]
    pub tasks: usize,
}

fn default_tasks() -> usize {
    40
}

fn default_result_directory() -> String {
    "result".to_string()
}

#[derive(Debug, Deserialize)]
pub struct SenderConfig {
    #[serde(default)]
    pub pcap: bool,
    #[serde(default = "default_trace")]
    pub trace: String,
    #[serde(default = "default_loss")]
    pub loss: f64,
    #[serde(default = "default_iteration")]
    pub iteration: u32,
    #[serde(default = "default_delay")]
    pub delay: u32,
    #[serde(default = "default_queue")]
    pub queue: String,
    #[serde(default = "default_buffer_size")]
    pub buffer_size: String,
    #[serde(default = "default_frame_cnt")]
    pub frame_cnt: u32,
    #[serde(default = "default_tcp_ca")]
    pub tcp_ca: String,
    #[serde(default = "default_mode_cnt")]
    pub mode_cnt: u32,
    #[serde(default = "default_app")]
    pub app: String,
    #[serde(default)]
    pub default_app_info: Option<Vec<u64>>,
}

fn default_trace() -> String {
    "traces/exp.trace".to_string()
}

fn default_iteration() -> u32 {
    1
}

fn default_loss() -> f64 {
    6e-4
}

fn default_delay() -> u32 {
    30
}

fn default_queue() -> String {
    "droptail".to_string()
}

fn default_buffer_size() -> String {
    "\"packets=200\"".to_string()
}

fn default_frame_cnt() -> u32 {
    5000
}

fn default_tcp_ca() -> String {
    "mortise-pcc".to_string()
}

fn default_mode_cnt() -> u32 {
    1
}

fn default_app() -> String {
    "video".to_string()
}

#[derive(Debug, Deserialize)]
pub struct ExpConfig {
    pub common: CommonConfig,
    pub sender: SenderConfig,
}

impl ExpConfig {
    pub fn new() -> Result<Self, ConfigError> {
        let s = Config::builder()
            .add_source(File::with_name("config.toml"))
            .add_source(Environment::with_prefix("mortise"))
            .build()?;

        // You can deserialize (and thus freeze) the entire configuration as
        s.try_deserialize()
    }

    pub fn new_with_file(file: &str) -> Result<Self, ConfigError> {
        let s = Config::builder()
            .add_source(File::with_name(file))
            .add_source(Environment::with_prefix("mortise"))
            .build()?;

        // You can deserialize (and thus freeze) the entire configuration as
        s.try_deserialize()
    }
}

#[derive(Debug, Deserialize)]
pub struct DbConfig {
    pub database: Database,
}

impl DbConfig {
    pub fn new() -> Result<Self, ConfigError> {
        let s = Config::builder()
            .add_source(File::with_name("db.toml"))
            .add_source(Environment::with_prefix("mortise"))
            .build()?;

        // You can deserialize (and thus freeze) the entire configuration as
        s.try_deserialize()
    }
}
