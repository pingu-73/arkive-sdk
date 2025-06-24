#![allow(unused_imports)]
use crate::error::{ArkiveError, Result};
use crate::storage::Storage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    pub wallet_id: String,
    pub device_id: String,
    pub last_sync: DateTime<Utc>,
    pub sync_version: u32,
    pub data_hash: String,
    pub pending_changes: Vec<SyncChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncChange {
    pub id: String,
    pub change_type: ChangeType,
    pub table_name: String,
    pub record_id: String,
    pub data: serde_json::Value,
    pub timestamp: DateTime<Utc>,
    pub device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeType {
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflict {
    pub id: String,
    pub wallet_id: String,
    pub conflict_type: ConflictType,
    pub local_change: SyncChange,
    pub remote_change: SyncChange,
    pub timestamp: DateTime<Utc>,
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConflictType {
    UpdateUpdate, // Both devices updated the same record
    UpdateDelete, // One updated, one deleted
    DeleteUpdate, // One deleted, one updated
}

pub struct SyncManager {
    storage: Arc<Storage>,
    pub device_id: String,
}

impl SyncManager {
    pub fn new(storage: Arc<Storage>) -> Self {
        let device_id = Self::get_or_create_device_id();
        Self { storage, device_id }
    }

    /// Get current device ID or create new one
    fn get_or_create_device_id() -> String {
        // Try to load from system or create new
        if let Ok(id) = std::env::var("ARKIVE_DEVICE_ID") {
            id
        } else {
            Uuid::new_v4().to_string()
        }
    }

    /// Initialize sync for a wallet
    pub async fn init_sync(&self, wallet_id: &str) -> Result<()> {
        let conn = self.storage.get_connection().await;

        let data_hash = self.calculate_wallet_hash(wallet_id).await?;

        conn.execute(
            "INSERT OR REPLACE INTO sync_metadata (wallet_id, device_id, last_sync, sync_version, data_hash)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                wallet_id,
                self.device_id,
                Utc::now().timestamp(),
                1u32,
                data_hash,
            ],
        )?;

        tracing::info!(
            "Initialized sync for wallet: {} on device: {}",
            wallet_id,
            self.device_id
        );
        Ok(())
    }

    /// Get sync state for wallet
    pub async fn get_sync_state(&self, wallet_id: &str) -> Result<Option<SyncState>> {
        let conn = self.storage.get_connection().await;

        let result = conn.query_row(
            "SELECT last_sync, sync_version, data_hash FROM sync_metadata WHERE wallet_id = ?1 AND device_id = ?2",
            rusqlite::params![wallet_id, self.device_id],
            |row| {
                Ok(SyncState {
                    wallet_id: wallet_id.to_string(),
                    device_id: self.device_id.clone(),
                    last_sync: DateTime::from_timestamp(row.get::<_, i64>(0)?, 0).unwrap_or_else(Utc::now),
                    sync_version: row.get::<_, u32>(1)?,
                    data_hash: row.get::<_, String>(2)?,
                    pending_changes: Vec::new(), // TODO: Load pending changes
                })
            },
        );

        match result {
            Ok(state) => Ok(Some(state)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(ArkiveError::Storage(e)),
        }
    }

    /// Create sync package for export
    pub async fn create_sync_package(&self, wallet_id: &str) -> Result<SyncPackage> {
        let backup_manager = crate::backup::BackupManager::new(self.storage.clone());
        let backup_data = backup_manager.collect_wallet_data(wallet_id).await?;

        let sync_state = self
            .get_sync_state(wallet_id)
            .await?
            .ok_or_else(|| ArkiveError::internal("Sync not initialized for wallet"))?;

        Ok(SyncPackage {
            version: 1,
            wallet_id: wallet_id.to_string(),
            device_id: self.device_id.clone(),
            sync_version: sync_state.sync_version,
            data_hash: sync_state.data_hash,
            backup_data,
            changes: Vec::new(), // TODO: Include incremental changes
            timestamp: Utc::now(),
        })
    }

    /// Apply sync package from another device
    pub async fn apply_sync_package(&self, package: &SyncPackage) -> Result<Vec<SyncConflict>> {
        let mut conflicts = Vec::new();

        // Get current sync state
        let current_state = self.get_sync_state(&package.wallet_id).await?;

        if let Some(current) = current_state {
            // Check for conflicts
            if current.data_hash != package.data_hash {
                tracing::warn!("Data hash mismatch detected, checking for conflicts");
                conflicts = self.detect_conflicts(package).await?;
            }
        }

        if conflicts.is_empty() {
            // No conflicts, apply changes directly
            self.apply_backup_data(&package.backup_data).await?;
            self.update_sync_metadata(package).await?;
            tracing::info!("Applied sync package without conflicts");
        } else {
            // Store conflicts for resolution
            self.store_conflicts(&conflicts).await?;
            tracing::warn!(
                "Sync package has {} conflicts requiring resolution",
                conflicts.len()
            );
        }

        Ok(conflicts)
    }

    /// Detect conflicts between local and remote data
    async fn detect_conflicts(&self, package: &SyncPackage) -> Result<Vec<SyncConflict>> {
        let mut conflicts = Vec::new();

        // Compare VTXOs
        let local_vtxos = self.get_local_vtxos(&package.wallet_id).await?;
        let remote_vtxos = &package.backup_data.vtxos;

        for remote_vtxo in remote_vtxos {
            if let Some(local_vtxo) = local_vtxos
                .iter()
                .find(|v| v.outpoint == remote_vtxo.outpoint)
            {
                if local_vtxo.status != remote_vtxo.status
                    || local_vtxo.amount != remote_vtxo.amount
                {
                    // Conflict detected
                    conflicts.push(SyncConflict {
                        id: Uuid::new_v4().to_string(),
                        wallet_id: package.wallet_id.clone(),
                        conflict_type: ConflictType::UpdateUpdate,
                        local_change: SyncChange {
                            id: Uuid::new_v4().to_string(),
                            change_type: ChangeType::Update,
                            table_name: "vtxos".to_string(),
                            record_id: local_vtxo.outpoint.clone(),
                            data: serde_json::to_value(local_vtxo)?,
                            timestamp: Utc::now(),
                            device_id: self.device_id.clone(),
                        },
                        remote_change: SyncChange {
                            id: Uuid::new_v4().to_string(),
                            change_type: ChangeType::Update,
                            table_name: "vtxos".to_string(),
                            record_id: remote_vtxo.outpoint.clone(),
                            data: serde_json::to_value(remote_vtxo)?,
                            timestamp: Utc::now(),
                            device_id: package.device_id.clone(),
                        },
                        timestamp: Utc::now(),
                        resolved: false,
                    });
                }
            }
        }

        Ok(conflicts)
    }

    /// Calculate hash of wallet data for sync comparison
    async fn calculate_wallet_hash(&self, wallet_id: &str) -> Result<String> {
        use sha2::{Digest, Sha256};

        let conn = self.storage.get_connection().await;

        // Get all relevant data for hashing
        let mut hasher = Sha256::new();

        // Hash wallet info
        let wallet_data: String = conn.query_row(
            "SELECT name || network || created_at FROM wallets WHERE id = ?1",
            [wallet_id],
            |row| row.get(0),
        )?;
        hasher.update(wallet_data.as_bytes());

        // Hash VTXOs
        let mut vtxo_stmt = conn.prepare(
            "SELECT outpoint || amount || status || expiry FROM vtxos WHERE wallet_id = ?1 ORDER BY outpoint"
        )?;
        let vtxo_rows = vtxo_stmt.query_map([wallet_id], |row| {
            let data: String = row.get(0)?;
            Ok(data)
        })?;

        for row in vtxo_rows {
            hasher.update(row?.as_bytes());
        }

        // Hash transactions
        let mut tx_stmt = conn.prepare(
            "SELECT txid || amount || timestamp || tx_type FROM transactions WHERE wallet_id = ?1 ORDER BY txid"
        )?;
        let tx_rows = tx_stmt.query_map([wallet_id], |row| {
            let data: String = row.get(0)?;
            Ok(data)
        })?;

        for row in tx_rows {
            hasher.update(row?.as_bytes());
        }

        Ok(hex::encode(hasher.finalize()))
    }

    async fn get_local_vtxos(&self, wallet_id: &str) -> Result<Vec<crate::backup::BackupVtxo>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT outpoint, amount, status, expiry, address, batch_id FROM vtxos WHERE wallet_id = ?1"
        )?;

        let vtxos = stmt
            .query_map([wallet_id], |row| {
                Ok(crate::backup::BackupVtxo {
                    outpoint: row.get(0)?,
                    amount: row.get::<_, i64>(1)? as u64,
                    status: row.get(2)?,
                    expiry: DateTime::from_timestamp(row.get::<_, i64>(3)?, 0)
                        .unwrap_or_else(Utc::now),
                    address: row.get(4)?,
                    batch_id: row.get(5)?,
                    tree_path: Vec::new(), // Simplified for conflict detection
                    exit_transactions: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(vtxos)
    }

    async fn apply_backup_data(&self, backup: &crate::backup::WalletBackup) -> Result<()> {
        let backup_manager = crate::backup::BackupManager::new(self.storage.clone());
        backup_manager.restore_wallet_data(backup).await?;
        Ok(())
    }

    async fn update_sync_metadata(&self, package: &SyncPackage) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "UPDATE sync_metadata SET last_sync = ?1, sync_version = ?2, data_hash = ?3 
             WHERE wallet_id = ?4 AND device_id = ?5",
            rusqlite::params![
                Utc::now().timestamp(),
                package.sync_version + 1,
                package.data_hash,
                package.wallet_id,
                self.device_id,
            ],
        )?;

        Ok(())
    }

    async fn store_conflicts(&self, conflicts: &[SyncConflict]) -> Result<()> {
        let conn = self.storage.get_connection().await;

        for conflict in conflicts {
            conn.execute(
                "INSERT INTO sync_conflicts (wallet_id, conflict_type, local_data, remote_data, timestamp, resolved)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    conflict.wallet_id,
                    serde_json::to_string(&conflict.conflict_type)?,
                    serde_json::to_string(&conflict.local_change)?,
                    serde_json::to_string(&conflict.remote_change)?,
                    conflict.timestamp.timestamp(),
                    conflict.resolved,
                ],
            )?;
        }

        Ok(())
    }

    /// Get unresolved conflicts for a wallet
    pub async fn get_conflicts(&self, wallet_id: &str) -> Result<Vec<SyncConflict>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT id, conflict_type, local_data, remote_data, timestamp 
             FROM sync_conflicts WHERE wallet_id = ?1 AND resolved = FALSE",
        )?;

        let conflicts = stmt
            .query_map([wallet_id], |row| {
                let conflict_type: String = row.get(1)?;
                let local_data: String = row.get(2)?;
                let remote_data: String = row.get(3)?;
                let timestamp: i64 = row.get(4)?;

                Ok(SyncConflict {
                    id: row.get::<_, i64>(0)?.to_string(),
                    wallet_id: wallet_id.to_string(),
                    conflict_type: serde_json::from_str(&conflict_type).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            1,
                            "conflict_type".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?,
                    local_change: serde_json::from_str(&local_data).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            2,
                            "local_data".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?,
                    remote_change: serde_json::from_str(&remote_data).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            3,
                            "remote_data".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?,
                    timestamp: DateTime::from_timestamp(timestamp, 0).unwrap_or_else(Utc::now),
                    resolved: false,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(conflicts)
    }

    /// Resolve a conflict by choosing local or remote version
    pub async fn resolve_conflict(&self, conflict_id: &str, use_local: bool) -> Result<()> {
        let conn = self.storage.get_connection().await;

        // Mark conflict as resolved
        conn.execute(
            "UPDATE sync_conflicts SET resolved = TRUE WHERE id = ?1",
            [conflict_id],
        )?;

        // TODO: Apply the chosen resolution
        tracing::info!(
            "Resolved conflict {} using {} version",
            conflict_id,
            if use_local { "local" } else { "remote" }
        );

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPackage {
    pub version: u32,
    pub wallet_id: String,
    pub device_id: String,
    pub sync_version: u32,
    pub data_hash: String,
    pub backup_data: crate::backup::WalletBackup,
    pub changes: Vec<SyncChange>,
    pub timestamp: DateTime<Utc>,
}

use hex;
