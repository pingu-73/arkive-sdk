use crate::error::{ArkiveError, Result};
use bitcoin::Network;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
    pub network: Network,
    pub ark_server_url: String,
    pub esplora_url: String,
    pub auto_renew_vtxos: bool,
    pub renewal_threshold: Duration,
    pub fee_policy: FeePolicy,
    pub is_mutinynet: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeePolicy {
    pub default_priority: FeePriority,
    pub max_fee_rate: u64, // sat/vB
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FeePriority {
    Slow,
    Normal,
    Fast,
    Fastest,
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            network: Network::Regtest,
            ark_server_url: "http://localhost:7070".to_string(),
            esplora_url: "http://localhost:3000".to_string(),
            auto_renew_vtxos: true,
            renewal_threshold: Duration::from_secs(3600), // 1 hour
            fee_policy: FeePolicy {
                default_priority: FeePriority::Normal,
                max_fee_rate: 100, // 100 sat/vB
            },
            is_mutinynet: false,
        }
    }
}

impl WalletConfig {
    pub fn new(network: Network) -> Self {
        Self::new_with_mutinynet(network, false)
    }

    pub fn new_mutinynet() -> Self {
        Self::new_with_mutinynet(Network::Signet, true)
    }

    pub fn new_with_mutinynet(network: Network, is_mutinynet: bool) -> Self {
        let mut config = Self::default();
        config.network = network;
        config.is_mutinynet = is_mutinynet;

        match (network, is_mutinynet) {
            (Network::Signet, true) => {
                config.esplora_url = "https://mutinynet.com/api".to_string();
                config.ark_server_url = "https://mutinynet.arkade.sh".to_string();
            }
            (Network::Signet, false) => {
                config.esplora_url = "https://mempool.space/signet/api".to_string();
                config.ark_server_url = "https://signet.arkade.sh".to_string();
            }
            (Network::Regtest, _) => {
                // keep defaults for regtest
            }
            _ => {
                config.esplora_url = "http://localhost:3000".to_string();
                config.ark_server_url = "http://localhost:7070".to_string();
            }
        }

        config
    }

    pub fn validate(&self) -> Result<()> {
        if self.ark_server_url.is_empty() {
            return Err(ArkiveError::config("Ark server URL cannot be empty"));
        }

        if self.esplora_url.is_empty() {
            return Err(ArkiveError::config("Esplora URL cannot be empty"));
        }

        if self.fee_policy.max_fee_rate == 0 {
            return Err(ArkiveError::config("Max fee rate must be greater than 0"));
        }

        Ok(())
    }
}
