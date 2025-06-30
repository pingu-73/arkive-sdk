use crate::error::{ArkiveError, Result};
use crate::storage::Storage;
use bitcoin::Network;
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletData {
    pub id: String,
    pub name: String,
    pub network: Network,
    pub created_at: chrono::DateTime<Utc>,
    pub encrypted_seed: Vec<u8>,
    pub config: Option<String>,
    pub is_mutinynet: bool,
}

pub struct WalletStore<'a> {
    storage: &'a Storage,
}

impl<'a> WalletStore<'a> {
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    pub async fn save_wallet(&self, wallet_data: &WalletData) -> Result<()> {
        self.validate_network(wallet_data.network, wallet_data.is_mutinynet)?;

        let conn = self.storage.get_connection().await;

        conn.execute(
             "INSERT OR REPLACE INTO wallets (id, name, network, created_at, encrypted_seed, config, is_mutinynet)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                wallet_data.id,
                wallet_data.name,
                wallet_data.network.to_string(),
                wallet_data.created_at.timestamp(),
                wallet_data.encrypted_seed,
                wallet_data.config,
                wallet_data.is_mutinynet,
            ],
        )?;

        Ok(())
    }

    pub async fn load_wallet(&self, wallet_id: &str) -> Result<WalletData> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT id, name, network, created_at, encrypted_seed, config, is_mutinynet
             FROM wallets WHERE id = ?1",
        )?;

        let wallet_data = stmt.query_row(params![wallet_id], |row| {
            let network_str: String = row.get(2)?;
            let is_mutinynet: bool = row.get(6).unwrap_or(false);
            let network = Self::parse_supported_network(&network_str)?;

            Ok(WalletData {
                id: row.get(0)?,
                name: row.get(1)?,
                network,
                created_at: chrono::DateTime::from_timestamp(row.get(3)?, 0)
                    .unwrap_or_else(Utc::now),
                encrypted_seed: row.get(4)?,
                config: row.get(5)?,
                is_mutinynet,
            })
        })?;

        Ok(wallet_data)
    }

    pub async fn list_wallets(&self) -> Result<Vec<WalletData>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT id, name, network, created_at, encrypted_seed, config, is_mutinynet
             FROM wallets ORDER BY created_at DESC",
        )?;

        let wallet_iter = stmt.query_map([], |row| {
            let network_str: String = row.get(2)?;
            let is_mutinynet: bool = row.get(6).unwrap_or(false);
            let network = Self::parse_supported_network(&network_str)?;

            Ok(WalletData {
                id: row.get(0)?,
                name: row.get(1)?,
                network,
                created_at: chrono::DateTime::from_timestamp(row.get(3)?, 0)
                    .unwrap_or_else(Utc::now),
                encrypted_seed: row.get(4)?,
                config: row.get(5)?,
                is_mutinynet,
            })
        })?;

        let mut wallets = Vec::new();
        for wallet in wallet_iter {
            wallets.push(wallet?);
        }

        Ok(wallets)
    }

    pub async fn delete_wallet(&self, wallet_id: &str) -> Result<()> {
        let conn = self.storage.get_connection().await;

        // Delete in order due to foreign key constraints
        conn.execute("DELETE FROM vtxos WHERE wallet_id = ?1", params![wallet_id])?;
        conn.execute(
            "DELETE FROM vtxo_trees WHERE wallet_id = ?1",
            params![wallet_id],
        )?;
        conn.execute(
            "DELETE FROM transactions WHERE wallet_id = ?1",
            params![wallet_id],
        )?;
        conn.execute(
            "DELETE FROM addresses WHERE wallet_id = ?1",
            params![wallet_id],
        )?;
        conn.execute("DELETE FROM wallets WHERE id = ?1", params![wallet_id])?;

        Ok(())
    }

    pub async fn wallet_exists(&self, name: &str) -> Result<bool> {
        let conn = self.storage.get_connection().await;

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM wallets WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;

        Ok(count > 0)
    }

    /// Validate that the network is supported by Ark
    fn validate_network(&self, network: Network, is_mutinynet: bool) -> Result<()> {
        match (network, is_mutinynet) {
            (Network::Signet, _) | (Network::Regtest, false) => Ok(()),
            (Network::Regtest, true) => Err(ArkiveError::config(
                "Mutinynet cannot be used with regtest network",
            )),
            (Network::Bitcoin, _) => Err(ArkiveError::config(
                "Ark is not yet available on Bitcoin mainnet. Use signet, mutinynet, or regtest.",
            )),
            (Network::Testnet, _) => Err(ArkiveError::config(
                "Ark is not available on Bitcoin testnet. Use signet, mutinynet, or regtest.",
            )),
            _ => Err(ArkiveError::config(
                "Unsupported network. Ark only supports signet, mutinynet, and regtest.",
            )),
        }
    }

    /// Parse network string, only allowing supported networks
    fn parse_supported_network(network_str: &str) -> std::result::Result<Network, rusqlite::Error> {
        match network_str {
            "signet" => Ok(Network::Signet),
            "regtest" => Ok(Network::Regtest),
            "bitcoin" | "testnet" => Err(rusqlite::Error::InvalidColumnType(
                2,
                "network".to_string(),
                rusqlite::types::Type::Text,
            )),
            _ => {
                // Default to regtest for unknown networks
                Ok(Network::Regtest)
            }
        }
    }
}
