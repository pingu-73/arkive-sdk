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
}

pub struct WalletStore<'a> {
    storage: &'a Storage,
}

impl<'a> WalletStore<'a> {
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    pub async fn save_wallet(&self, wallet_data: &WalletData) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "INSERT OR REPLACE INTO wallets (id, name, network, created_at, encrypted_seed, config)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                wallet_data.id,
                wallet_data.name,
                wallet_data.network.to_string(),
                wallet_data.created_at.timestamp(),
                wallet_data.encrypted_seed,
                wallet_data.config,
            ],
        )?;

        Ok(())
    }

    pub async fn load_wallet(&self, wallet_id: &str) -> Result<WalletData> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT id, name, network, created_at, encrypted_seed, config 
             FROM wallets WHERE id = ?1",
        )?;

        let wallet_data = stmt.query_row(params![wallet_id], |row| {
            let network_str: String = row.get(2)?;
            let network = match network_str.as_str() {
                "bitcoin" => Network::Bitcoin,
                "testnet" => Network::Testnet,
                "signet" => Network::Signet,
                "regtest" => Network::Regtest,
                _ => Network::Regtest,
            };

            Ok(WalletData {
                id: row.get(0)?,
                name: row.get(1)?,
                network,
                created_at: chrono::DateTime::from_timestamp(row.get(3)?, 0)
                    .unwrap_or_else(|| Utc::now()),
                encrypted_seed: row.get(4)?,
                config: row.get(5)?,
            })
        })?;

        Ok(wallet_data)
    }

    pub async fn list_wallets(&self) -> Result<Vec<WalletData>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT id, name, network, created_at, encrypted_seed, config 
             FROM wallets ORDER BY created_at DESC",
        )?;

        let wallet_iter = stmt.query_map([], |row| {
            let network_str: String = row.get(2)?;
            let network = match network_str.as_str() {
                "bitcoin" => Network::Bitcoin,
                "testnet" => Network::Testnet,
                "signet" => Network::Signet,
                "regtest" => Network::Regtest,
                _ => Network::Regtest,
            };

            Ok(WalletData {
                id: row.get(0)?,
                name: row.get(1)?,
                network,
                created_at: chrono::DateTime::from_timestamp(row.get(3)?, 0)
                    .unwrap_or_else(|| Utc::now()),
                encrypted_seed: row.get(4)?,
                config: row.get(5)?,
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
}
