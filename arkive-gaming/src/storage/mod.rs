use crate::core::{SerializableTwoPlayerLotteryState, TwoPlayerLotteryState};
use crate::error::{GamingError, Result};
use serde_json;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

pub struct GameStorage {
    storage_path: String,
}

impl GameStorage {
    pub fn new(storage_path: &str) -> Self {
        Self {
            storage_path: storage_path.to_string(),
        }
    }

    pub async fn save_game(&self, game_state: &TwoPlayerLotteryState) -> Result<()> {
        let games = self.load_all_games().await.unwrap_or_default();
        let mut updated_games = games;
        updated_games.insert(game_state.game_id.clone(), game_state.clone());

        // Convert to serializable format
        let serializable_games: HashMap<String, SerializableTwoPlayerLotteryState> = updated_games
            .iter()
            .map(|(k, v)| (k.clone(), v.to_serializable()))
            .collect();

        let json = serde_json::to_string_pretty(&serializable_games)?;
        fs::write(&self.storage_path, json)
            .await
            .map_err(|e| GamingError::internal(format!("Failed to save games: {}", e)))?;

        Ok(())
    }

    pub async fn load_game(&self, game_id: &str) -> Result<Option<TwoPlayerLotteryState>> {
        let games = self.load_all_games().await?;
        Ok(games.get(game_id).cloned())
    }

    pub async fn load_all_games(&self) -> Result<HashMap<String, TwoPlayerLotteryState>> {
        if !Path::new(&self.storage_path).exists() {
            return Ok(HashMap::new());
        }

        let content = fs::read_to_string(&self.storage_path)
            .await
            .map_err(|e| GamingError::internal(format!("Failed to read games: {}", e)))?;

        let serializable_games: HashMap<String, SerializableTwoPlayerLotteryState> =
            serde_json::from_str(&content)
                .map_err(|e| GamingError::internal(format!("Failed to parse games: {}", e)))?;

        // Convert back to runtime format
        let mut games = HashMap::new();
        for (game_id, serializable_game) in serializable_games {
            match TwoPlayerLotteryState::from_serializable(serializable_game) {
                Ok(game_state) => {
                    games.insert(game_id, game_state);
                }
                Err(e) => {
                    tracing::warn!("Failed to deserialize game {}: {}", game_id, e);
                }
            }
        }

        Ok(games)
    }

    /// Save VTXO data (simplified for new architecture)
    pub async fn save_vtxo_data(&self, game_id: &str, vtxo_data: &VtxoStorageData) -> Result<()> {
        let vtxo_path = format!("vtxos_{}.json", game_id);
        let json = serde_json::to_string_pretty(vtxo_data)?;
        fs::write(&vtxo_path, json)
            .await
            .map_err(|e| GamingError::internal(format!("Failed to save VTXO data: {}", e)))?;
        Ok(())
    }

    /// Load VTXO data (simplified for new architecture)
    pub async fn load_vtxo_data(&self, game_id: &str) -> Result<Option<VtxoStorageData>> {
        let vtxo_path = format!("vtxos_{}.json", game_id);

        if !Path::new(&vtxo_path).exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&vtxo_path)
            .await
            .map_err(|e| GamingError::internal(format!("Failed to read VTXO data: {}", e)))?;

        let vtxo_data: VtxoStorageData = serde_json::from_str(&content)
            .map_err(|e| GamingError::internal(format!("Failed to parse VTXO data: {}", e)))?;

        Ok(Some(vtxo_data))
    }

    /// Clean up storage for a completed game
    pub async fn cleanup_game_data(&self, game_id: &str) -> Result<()> {
        // remove VTXO data file
        let vtxo_path = format!("vtxos_{}.json", game_id);
        if Path::new(&vtxo_path).exists() {
            fs::remove_file(&vtxo_path).await.map_err(|e| {
                GamingError::internal(format!("Failed to cleanup VTXO data: {}", e))
            })?;
        }

        tracing::info!("Cleaned up storage data for game: {}", game_id);
        Ok(())
    }
}

/// Simplified VTXO storage data for the new architecture
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VtxoStorageData {
    pub game_id: String,
    pub escrow_address: String,
    pub player1_pk: String,
    pub player2_pk: String,
    pub server_pk: String,
    pub bet_amount: u64,
    pub commitment1_hash: String,
    pub commitment2_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub is_spent: bool,
}

impl VtxoStorageData {
    pub fn new(
        game_id: String,
        escrow_address: String,
        player1_pk: bitcoin::XOnlyPublicKey,
        player2_pk: bitcoin::XOnlyPublicKey,
        server_pk: bitcoin::XOnlyPublicKey,
        bet_amount: bitcoin::Amount,
        commitment1_hash: [u8; 32],
        commitment2_hash: [u8; 32],
    ) -> Self {
        Self {
            game_id,
            escrow_address,
            player1_pk: hex::encode(player1_pk.serialize()),
            player2_pk: hex::encode(player2_pk.serialize()),
            server_pk: hex::encode(server_pk.serialize()),
            bet_amount: bet_amount.to_sat(),
            commitment1_hash: hex::encode(commitment1_hash),
            commitment2_hash: hex::encode(commitment2_hash),
            created_at: chrono::Utc::now(),
            is_spent: false,
        }
    }
}
