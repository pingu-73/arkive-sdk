use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    pub data_dir: PathBuf,
    pub default_network: String,
    pub verbose: bool,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            data_dir: dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("arkive"),
            default_network: "regtest".to_string(),
            verbose: false,
        }
    }
}
