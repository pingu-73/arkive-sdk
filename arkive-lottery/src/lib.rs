//! Zero-Collateral Lottery Implementation for  2 Player
//!
//! This implements a simple 2 player commitment-based lottery.
//! Players commit to random values, reveal them, and the winner is determined by XOR.

pub mod commitment;
pub mod error;
pub mod game;
pub mod player;

pub use commitment::{Commitment, CommitmentScheme, HashCommitment};
pub use error::{LotteryError, Result};
pub use game::{GameState, TwoPlayerGame};
pub use player::{Player, PlayerState};

use arkive_core::Amount;
use std::sync::Arc;
use uuid::Uuid;

/// Create a new 2-player lottery game
pub async fn create_game(bet_amount: Amount) -> Result<TwoPlayerGame> {
    TwoPlayerGame::new(bet_amount).await
}

/// Join an existing game as the second player
pub async fn join_game(_game_id: Uuid, wallet: arkive_core::ArkWallet) -> Result<Player> {
    Player::new(Arc::new(wallet)).await
}
