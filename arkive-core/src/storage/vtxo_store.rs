use crate::error::{ArkiveError, Result};
use crate::storage::Storage;
use crate::types::{VtxoInfo, VtxoStatus};
use bitcoin::Amount;
use chrono::{DateTime, Utc};
use rusqlite::params;

pub struct VtxoStore<'a> {
    storage: &'a Storage,
}

impl<'a> VtxoStore<'a> {
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    pub async fn save_vtxo(&self, wallet_id: &str, vtxo: &VtxoInfo) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "INSERT OR REPLACE INTO vtxos 
             (wallet_id, outpoint, amount, status, expiry, batch_id, address, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                wallet_id,
                vtxo.outpoint,
                vtxo.amount.to_sat() as i64,
                serde_json::to_string(&vtxo.status)?,
                vtxo.expiry.timestamp(),
                "", // batch_id - [TODO]
                vtxo.address,
                Utc::now().timestamp(),
            ],
        )?;

        Ok(())
    }

    pub async fn load_vtxos(&self, wallet_id: &str) -> Result<Vec<VtxoInfo>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT outpoint, amount, status, expiry, address 
             FROM vtxos WHERE wallet_id = ?1 ORDER BY created_at DESC",
        )?;

        let vtxo_iter = stmt.query_map(params![wallet_id], |row| {
            let amount_sats: i64 = row.get(1)?;
            let status_str: String = row.get(2)?;
            let expiry_timestamp: i64 = row.get(3)?;

            let status: VtxoStatus = serde_json::from_str(&status_str).map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    2,
                    "status".to_string(),
                    rusqlite::types::Type::Text,
                )
            })?;

            Ok(VtxoInfo {
                outpoint: row.get(0)?,
                amount: Amount::from_sat(amount_sats as u64),
                status,
                expiry: DateTime::from_timestamp(expiry_timestamp, 0).unwrap_or_else(|| Utc::now()),
                address: row.get(4)?,
            })
        })?;

        let mut vtxos = Vec::new();
        for vtxo in vtxo_iter {
            vtxos.push(vtxo?);
        }

        Ok(vtxos)
    }

    pub async fn save_vtxo_tree(
        &self,
        wallet_id: &str,
        batch_id: &str,
        tree_data: &[u8],
        expiry: DateTime<Utc>,
    ) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "INSERT OR REPLACE INTO vtxo_trees (wallet_id, batch_id, tree_data, expiry, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                wallet_id,
                batch_id,
                tree_data,
                expiry.timestamp(),
                Utc::now().timestamp(),
            ],
        )?;

        Ok(())
    }

    pub async fn load_vtxo_tree(&self, wallet_id: &str, batch_id: &str) -> Result<Vec<u8>> {
        let conn = self.storage.get_connection().await;

        let tree_data: Vec<u8> = conn.query_row(
            "SELECT tree_data FROM vtxo_trees WHERE wallet_id = ?1 AND batch_id = ?2",
            params![wallet_id, batch_id],
            |row| row.get(0),
        )?;

        Ok(tree_data)
    }
}
