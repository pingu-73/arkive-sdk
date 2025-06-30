#![allow(unused_imports)]
pub mod boarding_store;
pub mod vtxo_store;
pub mod wallet_store;
pub use boarding_store::{BoardingOutputState, BoardingStore};

pub use vtxo_store::VtxoStore;
pub use wallet_store::WalletStore;

use crate::error::{ArkiveError, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use tokio::sync::Mutex;

pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    pub async fn new(db_path: &Path) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ArkiveError::internal(format!("Failed to create directory: {}", e)))?;
        }

        let conn = Connection::open(db_path)?;
        let storage = Self {
            conn: Mutex::new(conn),
        };

        storage.init_schema().await?;
        Ok(storage)
    }

    async fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().await;

        // Wallets table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS wallets (
                id TEXT PRIMARY KEY,
                name TEXT UNIQUE NOT NULL,
                network TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                encrypted_seed BLOB NOT NULL,
                config TEXT,
                is_mutinynet BOOLEAN DEFAULT FALSE
            )",
            [],
        )?;

        // Addresses table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS addresses (
                wallet_id TEXT NOT NULL,
                address TEXT NOT NULL,
                address_type TEXT NOT NULL,
                derivation_path TEXT,
                created_at INTEGER NOT NULL,
                FOREIGN KEY (wallet_id) REFERENCES wallets(id),
                PRIMARY KEY (wallet_id, address, address_type)
            )",
            [],
        )?;

        // Tx table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS transactions (
                wallet_id TEXT NOT NULL,
                txid TEXT NOT NULL,
                amount INTEGER NOT NULL,
                timestamp INTEGER NOT NULL,
                tx_type TEXT NOT NULL,
                status TEXT NOT NULL,
                fee INTEGER,
                raw_data TEXT,
                FOREIGN KEY (wallet_id) REFERENCES wallets(id),
                PRIMARY KEY (wallet_id, txid)
            )",
            [],
        )?;

        // VTXO trees table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS vtxo_trees (
                wallet_id TEXT NOT NULL,
                batch_id TEXT NOT NULL,
                commitment_txid TEXT NOT NULL,
                tree_data TEXT NOT NULL,
                presigned_transactions TEXT NOT NULL,
                expiry INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY (wallet_id) REFERENCES wallets(id),
                PRIMARY KEY (wallet_id, batch_id)
            )",
            [],
        )?;

        // VTXOs table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS vtxos (
                wallet_id TEXT NOT NULL,
                outpoint TEXT NOT NULL,
                amount INTEGER NOT NULL,
                status TEXT NOT NULL,
                expiry INTEGER NOT NULL,
                batch_id TEXT NOT NULL,
                address TEXT NOT NULL,
                tree_path TEXT NOT NULL,
                exit_transactions TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_updated INTEGER DEFAULT 0,
                FOREIGN KEY (wallet_id) REFERENCES wallets(id),
                PRIMARY KEY (wallet_id, outpoint)
            )",
            [],
        )?;

        // Boarding output storage
        conn.execute(
            "CREATE TABLE IF NOT EXISTS boarding_outputs (
                wallet_id TEXT NOT NULL,
                outpoint TEXT NOT NULL,
                amount INTEGER NOT NULL,
                address TEXT NOT NULL,
                script_pubkey TEXT NOT NULL,
                exit_delay INTEGER NOT NULL,
                server_pubkey TEXT NOT NULL,
                user_pubkey TEXT NOT NULL,
                confirmation_blocktime INTEGER,
                is_spent BOOLEAN DEFAULT FALSE,
                created_at INTEGER NOT NULL,
                FOREIGN KEY (wallet_id) REFERENCES wallets(id),
                PRIMARY KEY (wallet_id, outpoint)
            )",
            [],
        )?;

        // Sync metadata table for multi-device sync
        conn.execute(
            "CREATE TABLE IF NOT EXISTS sync_metadata (
                wallet_id TEXT NOT NULL,
                device_id TEXT NOT NULL,
                last_sync INTEGER NOT NULL,
                sync_version INTEGER NOT NULL,
                data_hash TEXT NOT NULL,
                FOREIGN KEY (wallet_id) REFERENCES wallets(id),
                PRIMARY KEY (wallet_id, device_id)
            )",
            [],
        )?;

        // Sync conflicts table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS sync_conflicts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                wallet_id TEXT NOT NULL,
                conflict_type TEXT NOT NULL,
                local_data TEXT NOT NULL,
                remote_data TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                resolved BOOLEAN DEFAULT FALSE,
                FOREIGN KEY (wallet_id) REFERENCES wallets(id)
            )",
            [],
        )?;

        Ok(())
    }

    pub async fn get_connection(&self) -> tokio::sync::MutexGuard<'_, Connection> {
        self.conn.lock().await
    }
}
