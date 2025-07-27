//! ARKive Gaming - Zero-collateral lottery protocols and gaming primitives
//!
//! This library provides production-ready implementations of cryptographic
//! gaming protocols, starting with zero-collateral two-player lotteries.

pub mod commitment;
pub mod contracts;
pub mod core;
pub mod error;
pub mod escrow;
pub mod games;
pub mod storage;

pub use commitment::{Commitment, CommitmentData, CommitmentScheme, Reveal};
pub use contracts::{ArkadeCompiler, CompiledContract, ContractManager, LotteryContract};
pub use core::{
    GameState, Player, TwoPlayerLotteryState, GameResult, GameEndReason,
    player::SerializablePlayer, SerializableTwoPlayerLotteryState, SerializableGameResult
};
pub use error::{GamingError, Result};
pub use escrow::{EscrowManager, LotteryEscrow, PayoutRevealData};
pub use games::TwoPlayerLottery;
pub use storage::GameStorage;

/// Initialize the gaming module with required dependencies
pub async fn initialize_gaming(
    wallet_manager: std::sync::Arc<arkive_core::WalletManager>,
    compiler_path: Option<String>,
) -> Result<TwoPlayerLottery> {
    // Initialize contract manager
    let mut contract_manager = ContractManager::new(compiler_path);
    contract_manager.initialize().await?;
    let contract_manager = std::sync::Arc::new(contract_manager);

    // Initialize escrow manager
    let escrow_manager = std::sync::Arc::new(EscrowManager::new(
        wallet_manager.clone(),
        contract_manager.clone(),
    ));

    // Create lottery instance
    let lottery = TwoPlayerLottery::new(
        wallet_manager,
        contract_manager,
        escrow_manager,
    );

    lottery.load_existing_games().await?;

    Ok(lottery)
}
