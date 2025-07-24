//! ARKive Gaming Framework - Modular 2-Player Gaming System
//!
//! This crate provides a modular framework for implementing 2-player games
//! with proper escrow management and commitment schemes.

pub mod commitment;
pub mod core;
pub mod error;
pub mod escrow;
pub mod games;
pub mod storage;

pub use core::{Game, GameResult, Player, PlayerId};
pub use error::{GamingError, Result};
pub use escrow::{EscrowId, EscrowManager};
pub use games::lottery::TwoPlayerLottery;
pub use storage::{GameStorage, StoredGameState};

use arkive_core::{Amount, ArkWallet};
use std::sync::Arc;
use uuid::Uuid;

/// Create a new 2-player lottery game
pub async fn create_lottery(
    bet_amount: Amount,
    escrow_wallet: Arc<ArkWallet>,
) -> Result<TwoPlayerLottery> {
    TwoPlayerLottery::new(bet_amount, escrow_wallet).await
}

/// Game manager for handling multiple games with file storage
pub struct GameManager {
    storage: GameStorage,
}

impl GameManager {
    pub async fn new(data_dir: &std::path::Path) -> Result<Self> {
        let storage = GameStorage::new(data_dir).await?;
        Ok(Self { storage })
    }

    /// Create and save a new lottery game
    pub async fn create_lottery_game(
        &self,
        bet_amount: Amount,
        escrow_wallet: Arc<ArkWallet>,
    ) -> Result<Uuid> {
        let lottery = TwoPlayerLottery::new(bet_amount, escrow_wallet).await?;
        let game_id = lottery.id();

        // Convert to storable format
        let game_info = lottery.get_info().await?;
        let stored_state = StoredGameState::new(
            game_id,
            "lottery".to_string(),
            serde_json::to_value(game_info)?,
        );

        self.storage.save_game(game_id, &stored_state).await?;
        Ok(game_id)
    }

    /// List all games
    pub async fn list_games(&self) -> Result<Vec<Uuid>> {
        self.storage.list_games().await
    }

    /// Get game info
    pub async fn get_game_info(&self, game_id: Uuid) -> Result<StoredGameState> {
        self.storage.load_game(game_id).await
    }

    /// Delete a game
    pub async fn delete_game(&self, game_id: Uuid) -> Result<()> {
        self.storage.delete_game(game_id).await
    }

    /// Check if game exists
    pub async fn game_exists(&self, game_id: Uuid) -> bool {
        self.storage.game_exists(game_id).await
    }
}
