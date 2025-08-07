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

/// Initialize the gaming module with real tapscript support
// pub async fn initialize_gaming(
//     wallet_manager: std::sync::Arc<arkive_core::WalletManager>,
// ) -> Result<TwoPlayerLottery> {
//     // Create lottery instance with real VTXO support
//     let lottery = TwoPlayerLottery::new(wallet_manager);

//     // Load existing games
//     lottery.load_existing_games().await?;

//     let scripts_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");

//     // Verify tapscripts can be compiled
//     let output = std::process::Command::new("make")
//         .current_dir(&scripts_path)
//         .arg("validate")
//         .output();

//     match output {
//         Ok(output) => {
//             if !output.status.success() {
//                 tracing::warn!("Failed to validate tapscripts: {}",
//                     String::from_utf8_lossy(&output.stderr));
//             } else {
//                 tracing::info!("Successfully validated lottery tapscripts");
//             }
//         }
//         Err(e) => {
//             tracing::warn!("Could not run tapscript validation: {}", e);
//             tracing::warn!("Make sure miniscript-compiler is installed and 'make' is available");
//         }
//     }

//     tracing::info!("Initialized arkive-gaming with real tapscript support");

//     Ok(lottery)
// }

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
