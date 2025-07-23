//! ARKive Gaming Framework - Modular 2-Player Gaming System
//!
//! This crate provides a modular framework for implementing 2-player games
//! with proper escrow management and commitment schemes.

pub mod commitment;
pub mod core;
pub mod error;
pub mod escrow;
pub mod games;

pub use core::{Game, GameResult, Player, PlayerId};
pub use error::{GamingError, Result};
pub use escrow::{EscrowId, EscrowManager};
pub use games::lottery::TwoPlayerLottery;

use arkive_core::{Amount, ArkWallet};
use std::sync::Arc;

/// Create a new 2-player lottery game
pub async fn create_lottery(
    bet_amount: Amount,
    escrow_wallet: Arc<ArkWallet>,
) -> Result<TwoPlayerLottery> {
    TwoPlayerLottery::new(bet_amount, escrow_wallet).await
}
