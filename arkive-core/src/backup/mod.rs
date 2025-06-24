#![allow(unused_imports)]
pub mod encryption;

use crate::error::{ArkiveError, Result};
use crate::storage::Storage;
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletBackup {
    pub version: u32,
    pub wallet_id: String,
    pub name: String,
    pub network: String,
    pub created_at: DateTime<Utc>,
    pub backup_timestamp: DateTime<Utc>,
    pub encrypted_seed: Vec<u8>,
    pub config: Option<String>,
    pub addresses: Vec<BackupAddress>,
    pub transactions: Vec<BackupTransaction>,
    pub vtxo_trees: Vec<BackupVtxoTree>,
    pub vtxos: Vec<BackupVtxo>,
    pub sync_metadata: Option<SyncMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupAddress {
    pub address: String,
    pub address_type: String,
    pub derivation_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupTransaction {
    pub txid: String,
    pub amount: i64,
    pub timestamp: DateTime<Utc>,
    pub tx_type: String,
    pub status: String,
    pub fee: Option<u64>,
    pub raw_data: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupVtxoTree {
    pub batch_id: String,
    pub commitment_txid: String,
    pub tree_data: String,                   // JSON serialized tree structure
    pub presigned_transactions: Vec<String>, // Base64 encoded transactions
    pub expiry: DateTime<Utc>,
    pub server_pubkey: String,
    pub user_pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupVtxo {
    pub outpoint: String,
    pub amount: u64,
    pub status: String,
    pub expiry: DateTime<Utc>,
    pub address: String,
    pub batch_id: String,
    pub tree_path: Vec<u32>,
    pub exit_transactions: Vec<String>, // Base64 encoded
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMetadata {
    pub device_id: String,
    pub last_sync: DateTime<Utc>,
    pub sync_version: u32,
    pub data_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedBackup {
    pub version: u32,
    pub encryption_method: String,
    pub salt: Vec<u8>,
    pub nonce: Vec<u8>,
    pub encrypted_data: Vec<u8>,
    pub checksum: String,
    pub created_at: DateTime<Utc>,
}

pub struct BackupManager {
    storage: std::sync::Arc<Storage>,
}

impl BackupManager {
    pub fn new(storage: std::sync::Arc<Storage>) -> Self {
        Self { storage }
    }

    /// Create encrypted backup of wallet
    pub async fn create_backup(&self, wallet_id: &str, password: &str) -> Result<EncryptedBackup> {
        // Collect all wallet data
        let backup_data = self.collect_wallet_data(wallet_id).await?;

        // Serialize to JSON
        let json_data = serde_json::to_string(&backup_data)?;

        // Encrypt with password
        let encrypted = encryption::encrypt_data(json_data.as_bytes(), password)?;

        tracing::info!("Created encrypted backup for wallet: {}", wallet_id);
        Ok(encrypted)
    }

    /// Restore wallet from encrypted backup
    pub async fn restore_backup(
        &self,
        encrypted_backup: &EncryptedBackup,
        password: &str,
    ) -> Result<String> {
        // Decrypt backup
        let decrypted_data = encryption::decrypt_data(encrypted_backup, password)?;

        // Parse backup data
        let backup_data: WalletBackup = serde_json::from_slice(&decrypted_data)?;

        // Restore wallet data
        let wallet_id = self.restore_wallet_data(&backup_data).await?;

        tracing::info!("Restored wallet from backup: {}", wallet_id);
        Ok(wallet_id)
    }

    /// Export backup to file
    pub async fn export_to_file(
        &self,
        wallet_id: &str,
        password: &str,
        file_path: &str,
    ) -> Result<()> {
        let backup = self.create_backup(wallet_id, password).await?;
        let backup_json = serde_json::to_string_pretty(&backup)?;

        tokio::fs::write(file_path, backup_json).await?;
        tracing::info!("Exported backup to file: {}", file_path);
        Ok(())
    }

    /// Import backup from file
    pub async fn import_from_file(&self, file_path: &str, password: &str) -> Result<String> {
        let backup_json = tokio::fs::read_to_string(file_path).await?;
        let encrypted_backup: EncryptedBackup = serde_json::from_str(&backup_json)?;

        let wallet_id = self.restore_backup(&encrypted_backup, password).await?;
        tracing::info!("Imported backup from file: {}", file_path);
        Ok(wallet_id)
    }

    pub async fn collect_wallet_data(&self, wallet_id: &str) -> Result<WalletBackup> {
        let conn = self.storage.get_connection().await;

        // Get wallet info
        let (name, network, created_at, encrypted_seed, config): (
            String,
            String,
            i64,
            Vec<u8>,
            Option<String>,
        ) = conn.query_row(
            "SELECT name, network, created_at, encrypted_seed, config FROM wallets WHERE id = ?1",
            [wallet_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;

        // Get addresses
        let mut addr_stmt = conn.prepare(
            "SELECT address, address_type, derivation_path, created_at FROM addresses WHERE wallet_id = ?1"
        )?;
        let addresses: Vec<BackupAddress> = addr_stmt
            .query_map([wallet_id], |row| {
                Ok(BackupAddress {
                    address: row.get(0)?,
                    address_type: row.get(1)?,
                    derivation_path: row.get(2)?,
                    created_at: DateTime::from_timestamp(row.get::<_, i64>(3)?, 0)
                        .unwrap_or_else(Utc::now),
                })
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
            .map_err(ArkiveError::Storage)?;

        // Get Tx
        let mut tx_stmt = conn.prepare(
            "SELECT txid, amount, timestamp, tx_type, status, fee, raw_data FROM transactions WHERE wallet_id = ?1"
        )?;
        let transactions: Vec<BackupTransaction> = tx_stmt
            .query_map([wallet_id], |row| {
                Ok(BackupTransaction {
                    txid: row.get(0)?,
                    amount: row.get(1)?,
                    timestamp: DateTime::from_timestamp(row.get::<_, i64>(2)?, 0)
                        .unwrap_or_else(Utc::now),
                    tx_type: row.get(3)?,
                    status: row.get(4)?,
                    fee: row.get(5)?,
                    raw_data: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
            .map_err(ArkiveError::Storage)?;

        // Get VTXO trees
        let mut tree_stmt = conn.prepare(
            "SELECT batch_id, commitment_txid, tree_data, presigned_transactions, expiry FROM vtxo_trees WHERE wallet_id = ?1"
        )?;
        let vtxo_trees: Vec<BackupVtxoTree> = tree_stmt
            .query_map([wallet_id], |row| {
                let tree_data: String = row.get(2)?;
                let presigned_txs: String = row.get(3)?;
                let expiry: i64 = row.get(4)?;

                // Parse presigned transactions
                let presigned_transactions: Vec<Vec<u8>> = serde_json::from_str(&presigned_txs)
                    .map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            3,
                            "presigned_transactions".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?;

                let presigned_b64: Vec<String> = presigned_transactions
                    .into_iter()
                    .map(|tx| general_purpose::STANDARD.encode(tx))
                    .collect();

                Ok(BackupVtxoTree {
                    batch_id: row.get(0)?,
                    commitment_txid: row.get(1)?,
                    tree_data,
                    presigned_transactions: presigned_b64,
                    expiry: DateTime::from_timestamp(expiry, 0).unwrap_or_else(Utc::now),
                    server_pubkey: "".to_string(), // [TODO] Extract from tree_data
                    user_pubkey: "".to_string(),   // [TODO] Extract from tree_data
                })
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
            .map_err(ArkiveError::Storage)?;

        // Get VTXOs
        let mut vtxo_stmt = conn.prepare(
            "SELECT outpoint, amount, status, expiry, address, batch_id, tree_path, exit_transactions FROM vtxos WHERE wallet_id = ?1"
        )?;
        let vtxos: Vec<BackupVtxo> = vtxo_stmt
            .query_map([wallet_id], |row| {
                let tree_path_str: String = row.get(6)?;
                let exit_txs_str: String = row.get(7)?;

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

                let exit_txs_b64: Vec<String> = exit_transactions
                    .into_iter()
                    .map(|tx| general_purpose::STANDARD.encode(tx))
                    .collect();

                Ok(BackupVtxo {
                    outpoint: row.get(0)?,
                    amount: row.get::<_, i64>(1)? as u64,
                    status: row.get(2)?,
                    expiry: DateTime::from_timestamp(row.get::<_, i64>(3)?, 0)
                        .unwrap_or_else(Utc::now),
                    address: row.get(4)?,
                    batch_id: row.get(5)?,
                    tree_path,
                    exit_transactions: exit_txs_b64,
                })
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
            .map_err(ArkiveError::Storage)?;

        Ok(WalletBackup {
            version: 1,
            wallet_id: wallet_id.to_string(),
            name,
            network,
            created_at: DateTime::from_timestamp(created_at, 0).unwrap_or_else(Utc::now),
            backup_timestamp: Utc::now(),
            encrypted_seed,
            config,
            addresses,
            transactions,
            vtxo_trees,
            vtxos,
            sync_metadata: None, // [TODO] Implement sync metadata
        })
    }

    pub async fn restore_wallet_data(&self, backup: &WalletBackup) -> Result<String> {
        let conn = self.storage.get_connection().await;

        // Start Tx
        let tx = conn.unchecked_transaction()?;

        // Restore wallet
        tx.execute(
            "INSERT OR REPLACE INTO wallets (id, name, network, created_at, encrypted_seed, config)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                backup.wallet_id,
                backup.name,
                backup.network,
                backup.created_at.timestamp(),
                backup.encrypted_seed,
                backup.config,
            ],
        )?;

        // Restore addresses
        for addr in &backup.addresses {
            tx.execute(
                "INSERT OR REPLACE INTO addresses (wallet_id, address, address_type, derivation_path, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    backup.wallet_id,
                    addr.address,
                    addr.address_type,
                    addr.derivation_path,
                    addr.created_at.timestamp(),
                ],
            )?;
        }

        // Restore Tx
        for transaction in &backup.transactions {
            tx.execute(
                "INSERT OR REPLACE INTO transactions (wallet_id, txid, amount, timestamp, tx_type, status, fee, raw_data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    backup.wallet_id,
                    transaction.txid,
                    transaction.amount,
                    transaction.timestamp.timestamp(),
                    transaction.tx_type,
                    transaction.status,
                    transaction.fee,
                    transaction.raw_data,
                ],
            )?;
        }

        // Restore VTXO trees
        for tree in &backup.vtxo_trees {
            // Decode presigned Tx
            let presigned_txs: Vec<Vec<u8>> = tree
                .presigned_transactions
                .iter()
                .map(|b64| general_purpose::STANDARD.decode(b64))
                .collect::<std::result::Result<Vec<_>, base64::DecodeError>>()
                .map_err(|e| {
                    ArkiveError::internal(format!("Failed to decode presigned transactions: {}", e))
                })?;

            let presigned_txs_json = serde_json::to_string(&presigned_txs)?;

            tx.execute(
                "INSERT OR REPLACE INTO vtxo_trees (wallet_id, batch_id, commitment_txid, tree_data, presigned_transactions, expiry, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    backup.wallet_id,
                    tree.batch_id,
                    tree.commitment_txid,
                    tree.tree_data,
                    presigned_txs_json,
                    tree.expiry.timestamp(),
                    Utc::now().timestamp(),
                ],
            )?;
        }

        // Restore VTXOs
        for vtxo in &backup.vtxos {
            // Decode exit Tx
            let exit_txs: Vec<Vec<u8>> = vtxo
                .exit_transactions
                .iter()
                .map(|b64| general_purpose::STANDARD.decode(b64))
                .collect::<std::result::Result<Vec<_>, base64::DecodeError>>()
                .map_err(|e| {
                    ArkiveError::internal(format!("Failed to decode exit transactions: {}", e))
                })?;

            let tree_path_json = serde_json::to_string(&vtxo.tree_path)?;
            let exit_txs_json = serde_json::to_string(&exit_txs)?;

            tx.execute(
                "INSERT OR REPLACE INTO vtxos (wallet_id, outpoint, amount, status, expiry, batch_id, address, tree_path, exit_transactions, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    backup.wallet_id,
                    vtxo.outpoint,
                    vtxo.amount as i64,
                    vtxo.status,
                    vtxo.expiry.timestamp(),
                    vtxo.batch_id,
                    vtxo.address,
                    tree_path_json,
                    exit_txs_json,
                    Utc::now().timestamp(),
                ],
            )?;
        }

        tx.commit()?;
        Ok(backup.wallet_id.clone())
    }
}

use base64;
