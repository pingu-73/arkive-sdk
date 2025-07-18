use crate::error::{ArkiveError, Result};
use crate::storage::Storage;
use bitcoin::{Amount, OutPoint};
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardingOutputState {
    pub outpoint: OutPoint,
    pub amount: Amount,
    pub address: String,
    pub script_pubkey: String,
    pub exit_delay: u32,
    pub server_pubkey: String,
    pub user_pubkey: String,
    pub confirmation_blocktime: Option<DateTime<Utc>>,
    pub is_spent: bool,
    pub is_mutinynet: bool,
}

impl BoardingOutputState {
    pub fn to_boarding_output(
        &self,
        network: bitcoin::Network,
    ) -> Result<ark_core::BoardingOutput> {
        let secp = bitcoin::secp256k1::Secp256k1::new();

        let server_pk = bitcoin::XOnlyPublicKey::from_str(&self.server_pubkey)
            .map_err(|e| ArkiveError::internal(format!("Invalid server pubkey: {}", e)))?;

        let user_pk = bitcoin::XOnlyPublicKey::from_str(&self.user_pubkey)
            .map_err(|e| ArkiveError::internal(format!("Invalid user pubkey: {}", e)))?;

        // Use the correct network for Mutinynet
        let effective_network = if self.is_mutinynet {
            bitcoin::Network::Signet
        } else {
            network
        };

        tracing::debug!(
            "Creating boarding output: server_pk={}, user_pk={}, exit_delay={}, network={:?}, is_mutinynet={}",
            self.server_pubkey, self.user_pubkey, self.exit_delay, effective_network, self.is_mutinynet
        );

        let boarding_output = ark_core::BoardingOutput::new(
            &secp,
            server_pk,
            user_pk,
            bitcoin::Sequence::from_consensus(self.exit_delay),
            effective_network,
        )?;

        tracing::debug!(
            "Created boarding output with address: {}",
            boarding_output.address()
        );
        Ok(boarding_output)
    }
}

pub struct BoardingStore<'a> {
    storage: &'a Storage,
}

impl<'a> BoardingStore<'a> {
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    pub async fn save_boarding_output(
        &self,
        wallet_id: &str,
        boarding_state: &BoardingOutputState,
    ) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "INSERT OR REPLACE INTO boarding_outputs 
             (wallet_id, outpoint, amount, address, script_pubkey, exit_delay, 
              server_pubkey, user_pubkey, confirmation_blocktime, is_spent, is_mutinynet, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                wallet_id,
                boarding_state.outpoint.to_string(),
                boarding_state.amount.to_sat() as i64,
                boarding_state.address,
                boarding_state.script_pubkey,
                boarding_state.exit_delay as i64,
                boarding_state.server_pubkey,
                boarding_state.user_pubkey,
                boarding_state.confirmation_blocktime.map(|t| t.timestamp()),
                boarding_state.is_spent,
                boarding_state.is_mutinynet,
                Utc::now().timestamp(),
            ],
        )?;

        tracing::info!(
            "Saved boarding output: {} with {} sats (mutinynet: {})",
            boarding_state.outpoint,
            boarding_state.amount.to_sat(),
            boarding_state.is_mutinynet
        );
        Ok(())
    }

    pub async fn load_boarding_outputs(&self, wallet_id: &str) -> Result<Vec<BoardingOutputState>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT outpoint, amount, address, script_pubkey, exit_delay, 
                    server_pubkey, user_pubkey, confirmation_blocktime, is_spent,
                    COALESCE(is_mutinynet, FALSE) as is_mutinynet
             FROM boarding_outputs 
             WHERE wallet_id = ?1 AND is_spent = FALSE
             ORDER BY created_at DESC",
        )?;

        let boarding_iter = stmt.query_map(params![wallet_id], |row| {
            let outpoint_str: String = row.get(0)?;
            let amount_sats: i64 = row.get(1)?;
            let exit_delay: i64 = row.get(4)?;
            let confirmation_blocktime: Option<i64> = row.get(7)?;
            let is_mutinynet: bool = row.get(9)?;

            let outpoint = OutPoint::from_str(&outpoint_str).map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    0,
                    "outpoint".to_string(),
                    rusqlite::types::Type::Text,
                )
            })?;

            Ok(BoardingOutputState {
                outpoint,
                amount: Amount::from_sat(amount_sats as u64),
                address: row.get(2)?,
                script_pubkey: row.get(3)?,
                exit_delay: exit_delay as u32,
                server_pubkey: row.get(5)?,
                user_pubkey: row.get(6)?,
                confirmation_blocktime: confirmation_blocktime
                    .and_then(|t| DateTime::from_timestamp(t, 0)),
                is_spent: row.get(8)?,
                is_mutinynet,
            })
        })?;

        let mut boarding_outputs = Vec::new();
        for boarding in boarding_iter {
            boarding_outputs.push(boarding?);
        }

        Ok(boarding_outputs)
    }

    pub async fn mark_boarding_output_spent(
        &self,
        wallet_id: &str,
        outpoint: &OutPoint,
    ) -> Result<()> {
        let conn = self.storage.get_connection().await;

        let result = conn.execute(
            "UPDATE boarding_outputs SET is_spent = TRUE WHERE wallet_id = ?1 AND outpoint = ?2",
            params![wallet_id, outpoint.to_string()],
        )?;

        if result == 0 {
            tracing::warn!(
                "No boarding output found to mark as spent: {} for wallet {}",
                outpoint,
                wallet_id
            );
        } else {
            tracing::info!(
                "Marked boarding output {} as spent for wallet {}",
                outpoint,
                wallet_id
            );
        }

        Ok(())
    }

    pub async fn load_unspent_boarding_outputs(
        &self,
        wallet_id: &str,
    ) -> Result<Vec<BoardingOutputState>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT outpoint, amount, address, script_pubkey, exit_delay, 
                    server_pubkey, user_pubkey, confirmation_blocktime, is_spent,
                    COALESCE(is_mutinynet, FALSE) as is_mutinynet
             FROM boarding_outputs 
             WHERE wallet_id = ?1 AND is_spent = FALSE AND confirmation_blocktime IS NOT NULL
             ORDER BY created_at DESC",
        )?;

        let boarding_iter = stmt.query_map(params![wallet_id], |row| {
            let outpoint_str: String = row.get(0)?;
            let amount_sats: i64 = row.get(1)?;
            let exit_delay: i64 = row.get(4)?;
            let confirmation_blocktime: Option<i64> = row.get(7)?;
            let is_mutinynet: bool = row.get(9)?;

            let outpoint = OutPoint::from_str(&outpoint_str).map_err(|_| {
                rusqlite::Error::InvalidColumnType(
                    0,
                    "outpoint".to_string(),
                    rusqlite::types::Type::Text,
                )
            })?;

            Ok(BoardingOutputState {
                outpoint,
                amount: Amount::from_sat(amount_sats as u64),
                address: row.get(2)?,
                script_pubkey: row.get(3)?,
                exit_delay: exit_delay as u32,
                server_pubkey: row.get(5)?,
                user_pubkey: row.get(6)?,
                confirmation_blocktime: confirmation_blocktime
                    .and_then(|t| DateTime::from_timestamp(t, 0)),
                is_spent: row.get(8)?,
                is_mutinynet,
            })
        })?;

        let mut boarding_outputs = Vec::new();
        for boarding in boarding_iter {
            boarding_outputs.push(boarding?);
        }

        Ok(boarding_outputs)
    }
}

use std::str::FromStr;
