use crate::error::{ArkiveError, Result};
use crate::storage::Storage;
use crate::types::{VtxoInfo, VtxoStatus};
use bitcoin::{Amount, Transaction};
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VtxoTreeData {
    pub batch_id: String,
    pub commitment_txid: String,
    pub tree_structure: Vec<u8>,              // Serialized tree
    pub presigned_transactions: Vec<Vec<u8>>, // Serialized exit tx
    pub expiry: DateTime<Utc>,
    pub server_pubkey: String,
    pub user_pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VtxoState {
    pub outpoint: String,
    pub amount: Amount,
    pub status: VtxoStatus,
    pub expiry: DateTime<Utc>,
    pub address: String,
    pub batch_id: String,
    pub tree_path: Vec<u32>,             // Path to this VTXO in the tree
    pub exit_transactions: Vec<Vec<u8>>, // Presigned exit path
}

pub struct VtxoStore<'a> {
    storage: &'a Storage,
}

impl<'a> VtxoStore<'a> {
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    /// Save complete VTXO tree data for unilateral exit capability
    pub async fn save_vtxo_tree(&self, wallet_id: &str, tree_data: &VtxoTreeData) -> Result<()> {
        let conn = self.storage.get_connection().await;

        let tree_json = serde_json::to_string(tree_data)?;
        let presigned_txs_json = serde_json::to_string(&tree_data.presigned_transactions)?;

        conn.execute(
            "INSERT OR REPLACE INTO vtxo_trees 
             (wallet_id, batch_id, tree_data, expiry, created_at, commitment_txid, presigned_transactions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                wallet_id,
                tree_data.batch_id,
                tree_json,
                tree_data.expiry.timestamp(),
                Utc::now().timestamp(),
                tree_data.commitment_txid,
                presigned_txs_json,
            ],
        )?;

        tracing::info!("Saved VTXO tree for batch: {}", tree_data.batch_id);
        Ok(())
    }

    /// Load VTXO tree data for unilateral exit
    pub async fn load_vtxo_tree(&self, wallet_id: &str, batch_id: &str) -> Result<VtxoTreeData> {
        let conn = self.storage.get_connection().await;

        let tree_json: String = conn.query_row(
            "SELECT tree_data FROM vtxo_trees WHERE wallet_id = ?1 AND batch_id = ?2",
            params![wallet_id, batch_id],
            |row| row.get(0),
        )?;

        let tree_data: VtxoTreeData = serde_json::from_str(&tree_json)?;
        Ok(tree_data)
    }

    /// Save individual VTXO with complete state
    pub async fn save_vtxo_state(&self, wallet_id: &str, vtxo_state: &VtxoState) -> Result<()> {
        let conn = self.storage.get_connection().await;

        let status_json = serde_json::to_string(&vtxo_state.status)?;
        let tree_path_json = serde_json::to_string(&vtxo_state.tree_path)?;
        let exit_txs_json = serde_json::to_string(&vtxo_state.exit_transactions)?;

        conn.execute(
            "INSERT OR REPLACE INTO vtxos 
             (wallet_id, outpoint, amount, status, expiry, batch_id, address, created_at, tree_path, exit_transactions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                wallet_id,
                vtxo_state.outpoint,
                vtxo_state.amount.to_sat() as i64,
                status_json,
                vtxo_state.expiry.timestamp(),
                vtxo_state.batch_id,
                vtxo_state.address,
                Utc::now().timestamp(),
                tree_path_json,
                exit_txs_json,
            ],
        )?;

        Ok(())
    }

    /// Load all VTXOs for a wallet with complete state
    pub async fn load_vtxo_states(&self, wallet_id: &str) -> Result<Vec<VtxoState>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT outpoint, amount, status, expiry, address, batch_id, tree_path, exit_transactions 
             FROM vtxos WHERE wallet_id = ?1 ORDER BY created_at DESC"
        )?;

        let vtxo_iter = stmt.query_map(params![wallet_id], |row| {
            let amount_sats: i64 = row.get(1)?;
            let status_str: String = row.get(2)?;
            let expiry_timestamp: i64 = row.get(3)?;
            let tree_path_str: String = row.get(6)?;
            let exit_txs_str: String = row.get(7)?;

            let status: VtxoStatus = serde_json::from_str(&status_str).map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    2,
                    "status".to_string(),
                    rusqlite::types::Type::Text,
                )
            })?;

            let tree_path: Vec<u32> = serde_json::from_str(&tree_path_str).map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    6,
                    "tree_path".to_string(),
                    rusqlite::types::Type::Text,
                )
            })?;

            let exit_transactions: Vec<Vec<u8>> =
                serde_json::from_str(&exit_txs_str).map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        7,
                        "exit_transactions".to_string(),
                        rusqlite::types::Type::Text,
                    )
                })?;

            Ok(VtxoState {
                outpoint: row.get(0)?,
                amount: Amount::from_sat(amount_sats as u64),
                status,
                expiry: DateTime::from_timestamp(expiry_timestamp, 0).unwrap_or_else(Utc::now),
                address: row.get(4)?,
                batch_id: row.get(5)?,
                tree_path,
                exit_transactions,
            })
        })?;

        let mut vtxos = Vec::new();
        for vtxo in vtxo_iter {
            vtxos.push(vtxo?);
        }

        Ok(vtxos)
    }

    /// Get VTXOs approaching expiry
    pub async fn get_expiring_vtxos(
        &self,
        wallet_id: &str,
        threshold_hours: i64,
    ) -> Result<Vec<VtxoState>> {
        let conn = self.storage.get_connection().await;
        let threshold_timestamp =
            (Utc::now() + chrono::Duration::hours(threshold_hours)).timestamp();

        let mut stmt = conn.prepare(
            "SELECT outpoint, amount, status, expiry, address, batch_id, tree_path, exit_transactions 
             FROM vtxos WHERE wallet_id = ?1 AND expiry <= ?2 AND status != 'Expired' 
             ORDER BY expiry ASC"
        )?;

        let vtxo_iter = stmt.query_map(params![wallet_id, threshold_timestamp], |row| {
            let amount_sats: i64 = row.get(1)?;
            let status_str: String = row.get(2)?;
            let expiry_timestamp: i64 = row.get(3)?;
            let tree_path_str: String = row.get(6)?;
            let exit_txs_str: String = row.get(7)?;

            let status: VtxoStatus = serde_json::from_str(&status_str).map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    2,
                    "status".to_string(),
                    rusqlite::types::Type::Text,
                )
            })?;

            let tree_path: Vec<u32> = serde_json::from_str(&tree_path_str).map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    6,
                    "tree_path".to_string(),
                    rusqlite::types::Type::Text,
                )
            })?;

            let exit_transactions: Vec<Vec<u8>> =
                serde_json::from_str(&exit_txs_str).map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        7,
                        "exit_transactions".to_string(),
                        rusqlite::types::Type::Text,
                    )
                })?;

            Ok(VtxoState {
                outpoint: row.get(0)?,
                amount: Amount::from_sat(amount_sats as u64),
                status,
                expiry: DateTime::from_timestamp(expiry_timestamp, 0).unwrap_or_else(Utc::now),
                address: row.get(4)?,
                batch_id: row.get(5)?,
                tree_path,
                exit_transactions,
            })
        })?;

        let mut vtxos = Vec::new();
        for vtxo in vtxo_iter {
            vtxos.push(vtxo?);
        }

        Ok(vtxos)
    }

    /// Clean up expired VTXOs and trees
    pub async fn cleanup_expired(&self, wallet_id: &str) -> Result<usize> {
        let conn = self.storage.get_connection().await;
        let now = Utc::now().timestamp();

        // Mark expired VTXOs
        let expired_vtxos = conn.execute(
            "UPDATE vtxos SET status = ? WHERE wallet_id = ? AND expiry <= ? AND status != 'Expired'",
            params![serde_json::to_string(&VtxoStatus::Expired)?, wallet_id, now],
        )?;

        // Clean up old expired trees (older than 30 days)
        let cleanup_threshold = (Utc::now() - chrono::Duration::days(30)).timestamp();
        conn.execute(
            "DELETE FROM vtxo_trees WHERE wallet_id = ? AND expiry <= ?",
            params![wallet_id, cleanup_threshold],
        )?;

        tracing::info!(
            "Cleaned up {} expired VTXOs for wallet {}",
            expired_vtxos,
            wallet_id
        );
        Ok(expired_vtxos)
    }
}
