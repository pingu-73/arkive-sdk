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
    pub aggregate_pubkey: Option<String>,
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
    pub is_preconfirmed: bool,
    pub is_recoverable: bool,
    pub commitment_txids: Vec<String>,
    pub spent_by: Option<String>,
    pub settled_by: Option<String>,
    pub ark_txid: Option<String>,
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
             (wallet_id, batch_id, tree_data, expiry, created_at, commitment_txid, presigned_transactions, server_pubkey, aggregate_pubkey)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                wallet_id,
                tree_data.batch_id,
                tree_json,
                tree_data.expiry.timestamp(),
                Utc::now().timestamp(),
                tree_data.commitment_txid,
                presigned_txs_json,
                tree_data.server_pubkey,
                tree_data.aggregate_pubkey,
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
        let commitment_txids_json = serde_json::to_string(&vtxo_state.commitment_txids)?;

        conn.execute(
            "INSERT OR REPLACE INTO vtxos 
             (wallet_id, outpoint, amount, status, expiry, batch_id, address, created_at, tree_path, exit_transactions, is_preconfirmed, is_recoverable, commitment_txids, spent_by, settled_by, ark_txid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
                vtxo_state.is_preconfirmed,
                vtxo_state.is_recoverable,
                commitment_txids_json,
                vtxo_state.spent_by,
                vtxo_state.settled_by,
                vtxo_state.ark_txid,
            ],
        )?;

        Ok(())
    }

    /// Load all VTXOs for a wallet with complete state
    pub async fn load_vtxo_states(&self, wallet_id: &str) -> Result<Vec<VtxoState>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT outpoint, amount, status, expiry, address, batch_id, tree_path, exit_transactions,
             COALESCE(is_preconfirmed, FALSE) as is_preconfirmed,
             COALESCE(is_recoverable, FALSE) as is_recoverable,
             COALESCE(commitment_txids, '[]') as commitment_txids,
             spent_by, settled_by, ark_txid
             FROM vtxos WHERE wallet_id = ?1 ORDER BY created_at DESC"
        )?;

        let vtxo_iter = stmt.query_map(params![wallet_id], |row| {
            let amount_sats: i64 = row.get(1)?;
            let status_str: String = row.get(2)?;
            let expiry_timestamp: i64 = row.get(3)?;
            let tree_path_str: String = row.get(6)?;
            let exit_txs_str: String = row.get(7)?;
            let commitment_txids_str: String = row.get(10)?;

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

            let commitment_txids: Vec<String> = serde_json::from_str(&commitment_txids_str)
                .map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        10,
                        "commitment_txids".to_string(),
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
                is_preconfirmed: row.get(8)?,
                is_recoverable: row.get(9)?,
                commitment_txids,
                spent_by: row.get(11)?,
                settled_by: row.get(12)?,
                ark_txid: row.get(13)?,
            })
        })?;

        let mut vtxos = Vec::new();
        for vtxo in vtxo_iter {
            vtxos.push(vtxo?);
        }

        Ok(vtxos)
    }

    /// Get spendable VTXOs
    pub async fn get_spendable_vtxos(&self, wallet_id: &str) -> Result<Vec<VtxoState>> {
        let all_vtxos = self.load_vtxo_states(wallet_id).await?;
        let now = Utc::now();

        let spendable: Vec<VtxoState> = all_vtxos
            .into_iter()
            .filter(|vtxo| {
                // Include confirmed, preconfirmed, and recoverable VTXOs that haven't expired
                (matches!(
                    vtxo.status,
                    VtxoStatus::Confirmed | VtxoStatus::Preconfirmed
                ) || vtxo.is_recoverable)
                    && vtxo.expiry > now
                    && !matches!(vtxo.status, VtxoStatus::Spent | VtxoStatus::Replaced)
            })
            .collect();

        Ok(spendable)
    }

    /// Get preconfirmed VTXOs that need batch swap
    pub async fn get_preconfirmed_vtxos(&self, wallet_id: &str) -> Result<Vec<VtxoState>> {
        let all_vtxos = self.load_vtxo_states(wallet_id).await?;
        let now = Utc::now();

        let preconfirmed: Vec<VtxoState> = all_vtxos
            .into_iter()
            .filter(|vtxo| matches!(vtxo.status, VtxoStatus::Preconfirmed) && vtxo.expiry > now)
            .collect();

        Ok(preconfirmed)
    }

    /// Get recoverable VTXOs
    pub async fn get_recoverable_vtxos(&self, wallet_id: &str) -> Result<Vec<VtxoState>> {
        let all_vtxos = self.load_vtxo_states(wallet_id).await?;
        let now = Utc::now();

        let recoverable: Vec<VtxoState> = all_vtxos
            .into_iter()
            .filter(|vtxo| vtxo.is_recoverable && vtxo.expiry > now)
            .collect();

        Ok(recoverable)
    }

    /// Update VTXO status
    pub async fn update_vtxo_status(
        &self,
        wallet_id: &str,
        outpoint: &str,
        new_status: VtxoStatus,
    ) -> Result<()> {
        let conn = self.storage.get_connection().await;

        let status_json = serde_json::to_string(&new_status)?;

        conn.execute(
            "UPDATE vtxos SET status = ?1, last_updated = ?2 WHERE wallet_id = ?3 AND outpoint = ?4",
            params![
                status_json,
                Utc::now().timestamp(),
                wallet_id,
                outpoint
            ],
        )?;

        Ok(())
    }

    /// Mark VTXO as spent
    pub async fn mark_vtxo_spent(
        &self,
        wallet_id: &str,
        outpoint: &str,
        spent_by: Option<&str>,
        ark_txid: Option<&str>,
    ) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "UPDATE vtxos SET status = ?1, spent_by = ?2, ark_txid = ?3, last_updated = ?4 
             WHERE wallet_id = ?5 AND outpoint = ?6",
            params![
                serde_json::to_string(&VtxoStatus::Spent)?,
                spent_by,
                ark_txid,
                Utc::now().timestamp(),
                wallet_id,
                outpoint
            ],
        )?;

        Ok(())
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
            "SELECT outpoint, amount, status, expiry, address, batch_id, tree_path, exit_transactions,
             COALESCE(is_preconfirmed, FALSE) as is_preconfirmed,
             COALESCE(is_recoverable, FALSE) as is_recoverable,
             COALESCE(commitment_txids, '[]') as commitment_txids,
             spent_by, settled_by, ark_txid
             FROM vtxos WHERE wallet_id = ?1 AND expiry <= ?2 AND status NOT IN ('Expired', 'Spent', 'Replaced') 
             ORDER BY expiry ASC"
        )?;

        let vtxo_iter = stmt.query_map(params![wallet_id, threshold_timestamp], |row| {
            let amount_sats: i64 = row.get(1)?;
            let status_str: String = row.get(2)?;
            let expiry_timestamp: i64 = row.get(3)?;
            let tree_path_str: String = row.get(6)?;
            let exit_txs_str: String = row.get(7)?;
            let commitment_txids_str: String = row.get(10)?;

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

            let commitment_txids: Vec<String> = serde_json::from_str(&commitment_txids_str)
                .map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        10,
                        "commitment_txids".to_string(),
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
                is_preconfirmed: row.get(8)?,
                is_recoverable: row.get(9)?,
                commitment_txids,
                spent_by: row.get(11)?,
                settled_by: row.get(12)?,
                ark_txid: row.get(13)?,
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
            "UPDATE vtxos SET status = ? WHERE wallet_id = ? AND expiry <= ? AND status NOT IN ('Expired', 'Spent', 'Replaced')",
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

    /// Save batch swap info
    pub async fn save_batch_swap(
        &self,
        wallet_id: &str,
        swap_id: &str,
        input_vtxos: &[String],
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let conn = self.storage.get_connection().await;

        let input_vtxos_json = serde_json::to_string(input_vtxos)?;

        conn.execute(
            "INSERT OR REPLACE INTO batch_swaps 
             (wallet_id, swap_id, status, input_vtxos, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                wallet_id,
                swap_id,
                serde_json::to_string(&crate::types::BatchSwapStatus::Pending)?,
                input_vtxos_json,
                Utc::now().timestamp(),
                expires_at.timestamp(),
            ],
        )?;

        Ok(())
    }

    /// Update batch swap status
    pub async fn update_batch_swap_status(
        &self,
        wallet_id: &str,
        swap_id: &str,
        status: crate::types::BatchSwapStatus,
        commitment_txid: Option<&str>,
    ) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "UPDATE batch_swaps SET status = ?1, commitment_txid = ?2, last_updated = ?3 
             WHERE wallet_id = ?4 AND swap_id = ?5",
            params![
                serde_json::to_string(&status)?,
                commitment_txid,
                Utc::now().timestamp(),
                wallet_id,
                swap_id
            ],
        )?;

        Ok(())
    }

    /// Save connector output
    pub async fn save_connector_output(
        &self,
        wallet_id: &str,
        outpoint: &str,
        amount: Amount,
        associated_vtxo: &str,
        commitment_txid: &str,
    ) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "INSERT OR REPLACE INTO connector_outputs 
             (wallet_id, outpoint, amount, associated_vtxo, status, commitment_txid, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                wallet_id,
                outpoint,
                amount.to_sat() as i64,
                associated_vtxo,
                serde_json::to_string(&crate::types::ConnectorStatus::Active)?,
                commitment_txid,
                Utc::now().timestamp(),
            ],
        )?;

        Ok(())
    }

    /// Save forfeit tx
    pub async fn save_forfeit_transaction(
        &self,
        wallet_id: &str,
        txid: &str,
        vtxo_outpoint: &str,
        connector_outpoint: &str,
        signed_psbt: &str,
        batch_swap_id: Option<&str>,
    ) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "INSERT OR REPLACE INTO forfeit_transactions 
             (wallet_id, txid, vtxo_outpoint, connector_outpoint, signed_psbt, batch_swap_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                wallet_id,
                txid,
                vtxo_outpoint,
                connector_outpoint,
                signed_psbt,
                batch_swap_id,
                Utc::now().timestamp(),
            ],
        )?;

        Ok(())
    }
}
