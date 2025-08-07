pub mod commitment;
pub mod contracts;
pub mod core;
pub mod error;
pub mod escrow;
pub mod games;
pub mod storage;

pub use commitment::{Commitment, CommitmentData, CommitmentScheme, Reveal};
pub use contracts::{CompiledScript, LotteryTapscripts, TapscriptManager};
pub use core::{
    player::SerializablePlayer, GameEndReason, GameResult, GameState, Player,
    SerializableGameResult, SerializableTwoPlayerLotteryState, TwoPlayerLotteryState,
};
pub use error::{GamingError, Result};
pub use escrow::{LotteryVtxo, VtxoEscrowManager};
pub use games::TwoPlayerLottery;
pub use storage::{GameStorage, VtxoStorageData};

pub async fn initialize_gaming(
    wallet_manager: std::sync::Arc<arkive_core::WalletManager>,
) -> Result<TwoPlayerLottery> {
    // Create lottery instance
    let lottery = TwoPlayerLottery::new(wallet_manager);

    // Load existing games
    lottery.load_existing_games().await?;

    // Validate embedded scripts (no file system access needed)
    let tapscript_manager = TapscriptManager::new();
    tapscript_manager.validate_scripts()?;

    tracing::info!("Initialized arkive-gaming with embedded tapscript support");
    tracing::info!("Lottery implementation is provably fair and trustless");

    Ok(lottery)
}
