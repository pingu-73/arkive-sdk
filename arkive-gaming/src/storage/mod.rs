use crate::core::{TwoPlayerLotteryState, SerializableTwoPlayerLotteryState};
use crate::error::{GamingError, Result};
use crate::escrow::{SerializableLotteryEscrow, LotteryEscrow};
use crate::ContractManager;
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
        fs::write(&self.storage_path, json).await
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

        let content = fs::read_to_string(&self.storage_path).await
            .map_err(|e| GamingError::internal(format!("Failed to read games: {}", e)))?;

        let serializable_games: HashMap<String, SerializableTwoPlayerLotteryState> = serde_json::from_str(&content)
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

    pub async fn save_escrow(&self, escrow: &LotteryEscrow) -> Result<()> {
        let escrows = self.load_all_escrows_serializable().await.unwrap_or_default();
        let mut updated_escrows = escrows;
        updated_escrows.insert(escrow.game_id.clone(), escrow.to_serializable());
        
        let json = serde_json::to_string_pretty(&updated_escrows)?;
        fs::write("escrows.json", json).await
            .map_err(|e| GamingError::internal(format!("Failed to save escrows: {}", e)))?;
        Ok(())
    }

    async fn load_all_escrows_serializable(&self) -> Result<HashMap<String, SerializableLotteryEscrow>> {
        let escrow_path = "escrows.json";
        
        if !Path::new(escrow_path).exists() {
            return Ok(HashMap::new());
        }

        let content = fs::read_to_string(escrow_path).await
            .map_err(|e| GamingError::internal(format!("Failed to read escrows: {}", e)))?;

        let escrows: HashMap<String, SerializableLotteryEscrow> = serde_json::from_str(&content)
            .map_err(|e| GamingError::internal(format!("Failed to parse escrows: {}", e)))?;

        Ok(escrows)
    }

    pub async fn load_all_escrows(&self, contract_manager: &ContractManager) -> Result<HashMap<String, LotteryEscrow>> {
        let serializable_escrows = self.load_all_escrows_serializable().await?;
        
        let mut escrows = HashMap::new();
        for (game_id, serializable_escrow) in serializable_escrows {
            match LotteryEscrow::from_serializable(serializable_escrow, contract_manager) {
                Ok(escrow) => {
                    escrows.insert(game_id, escrow);
                }
                Err(e) => {
                    tracing::warn!("Failed to deserialize escrow for game {}: {}", game_id, e);
                }
            }
        }

        Ok(escrows)
    }
}
