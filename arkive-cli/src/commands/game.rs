use arkive_core::{Amount, ArkiveError, Result, WalletManager};
use arkive_gaming::{GameManager, StoredGameState};
use clap::Subcommand;
use comfy_table::{presets::UTF8_FULL, Table};
use std::path::Path;
use uuid::Uuid;

fn map_gaming_error(err: arkive_gaming::GamingError) -> ArkiveError {
    ArkiveError::internal(format!("Gaming error: {}", err))
}

#[derive(Subcommand)]
pub enum GameCommands {
    /// Create a new lottery game
    CreateLottery {
        /// Wallet name to use for escrow
        wallet: String,
        /// Bet amount in satoshis
        #[arg(short, long)]
        amount: u64,
    },
    /// List all games
    List,
    /// Show game details
    Info {
        /// Game ID
        game_id: String,
    },
    /// Join an existing game
    Join {
        /// Game ID
        game_id: String,
        /// Wallet name
        wallet: String,
    },
    /// Place bet in a game
    Bet {
        /// Game ID
        game_id: String,
        /// Wallet name
        wallet: String,
    },
    /// Submit commitment (automatic secret generation)
    Commit {
        /// Game ID
        game_id: String,
        /// Wallet name
        wallet: String,
    },
    /// Reveal commitment (uses stored secret)
    Reveal {
        /// Game ID
        game_id: String,
        /// Wallet name
        wallet: String,
    },
    /// Forfeit a game
    Forfeit {
        /// Game ID
        game_id: String,
        /// Wallet name
        wallet: String,
    },
    /// Delete a completed game
    Delete {
        /// Game ID
        game_id: String,
    },
}

pub async fn handle_game_command(
    cmd: GameCommands,
    manager: &WalletManager,
    data_dir: &Path,
) -> Result<()> {
    let game_manager = GameManager::new(data_dir).await.map_err(map_gaming_error)?;

    match cmd {
        GameCommands::CreateLottery { wallet, amount } => {
            let wallet_instance = manager.load_wallet(&wallet).await?;
            let bet_amount = Amount::from_sat(amount);

            println!("Creating lottery game with {} sats bet...", amount);

            let game_id = game_manager
                .create_lottery_game(bet_amount, wallet_instance)
                .await
                .map_err(map_gaming_error)?;

            println!("Lottery game created successfully!");
            println!("Game ID: {}", game_id);
            println!(
                "Players can join with: arkive game join {} <wallet>",
                game_id
            );
        }

        GameCommands::List => {
            let games = game_manager.list_games().await.map_err(map_gaming_error)?;

            if games.is_empty() {
                println!("No games found.");
                println!(
                    "Create a new game with: arkive game create-lottery <wallet> --amount <sats>"
                );
                return Ok(());
            }

            let mut table = Table::new();
            table.load_preset(UTF8_FULL);
            table.set_header(vec!["Game ID", "Type", "Created", "Status"]);

            for game_id in games {
                match game_manager.get_game_info(game_id).await {
                    Ok(stored_state) => {
                        let status = extract_game_status(&stored_state);
                        table.add_row(vec![
                            &game_id.to_string()[..8],
                            &stored_state.game_type,
                            &stored_state.created_at.format("%Y-%m-%d %H:%M").to_string(),
                            &status,
                        ]);
                    }
                    Err(_) => {
                        table.add_row(vec![
                            &game_id.to_string()[..8],
                            "Unknown",
                            "Unknown",
                            "Error",
                        ]);
                    }
                }
            }

            println!("{}", table);
        }

        GameCommands::Info { game_id } => {
            let game_uuid = Uuid::parse_str(&game_id)
                .map_err(|_| arkive_core::ArkiveError::config("Invalid game ID format"))?;

            let stored_state = game_manager
                .get_game_info(game_uuid)
                .await
                .map_err(map_gaming_error)?;

            println!("Game Information:");
            println!("  ID: {}", stored_state.game_id);
            println!("  Type: {}", stored_state.game_type);
            println!(
                "  Created: {}",
                stored_state.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
            println!(
                "  Updated: {}",
                stored_state.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
            );

            // Extract and display game-specific info
            if let Ok(game_info) =
                serde_json::from_value::<serde_json::Value>(stored_state.state.clone())
            {
                println!("  Status: {}", extract_game_status(&stored_state));

                if let Some(bet_amount) = game_info.get("bet_amount") {
                    println!("  Bet Amount: {} sats", bet_amount);
                }

                if let Some(player_count) = game_info.get("player_count") {
                    println!("  Players: {}/2", player_count);
                }

                if let Some(escrow_address) = game_info.get("escrow_address") {
                    if !escrow_address.is_null() {
                        println!(
                            "  Escrow Address: {}",
                            escrow_address.as_str().unwrap_or("N/A")
                        );
                    }
                }
            }
        }

        GameCommands::Join { game_id, wallet } => {
            let _game_uuid = Uuid::parse_str(&game_id)
                .map_err(|_| arkive_core::ArkiveError::config("Invalid game ID format"))?;
            let _wallet_instance = manager.load_wallet(&wallet).await?;

            // TODO: Implement actual game joining logic
            println!("Game joining functionality coming soon!");
            println!("Game ID: {}", game_id);
            println!("Wallet: {}", wallet);
        }

        GameCommands::Bet { game_id, wallet } => {
            let _game_uuid = Uuid::parse_str(&game_id)
                .map_err(|_| arkive_core::ArkiveError::config("Invalid game ID format"))?;
            let _wallet_instance = manager.load_wallet(&wallet).await?;

            // TODO: Implement actual betting logic
            println!("Betting functionality coming soon!");
            println!("Game ID: {}", game_id);
            println!("Wallet: {}", wallet);
        }

        GameCommands::Commit { game_id, wallet } => {
            let _game_uuid = Uuid::parse_str(&game_id)
                .map_err(|_| arkive_core::ArkiveError::config("Invalid game ID format"))?;
            let _wallet_instance = manager.load_wallet(&wallet).await?;

            // TODO: Implement commitment logic
            println!("Commitment functionality coming soon!");
            println!("Game ID: {}", game_id);
            println!("Wallet: {}", wallet);
        }

        GameCommands::Reveal { game_id, wallet } => {
            let _game_uuid = Uuid::parse_str(&game_id)
                .map_err(|_| arkive_core::ArkiveError::config("Invalid game ID format"))?;
            let _wallet_instance = manager.load_wallet(&wallet).await?;

            // TODO: Implement reveal logic
            println!("Reveal functionality coming soon!");
            println!("Game ID: {}", game_id);
            println!("Wallet: {}", wallet);
        }

        GameCommands::Forfeit { game_id, wallet } => {
            let _game_uuid = Uuid::parse_str(&game_id)
                .map_err(|_| arkive_core::ArkiveError::config("Invalid game ID format"))?;
            let _wallet_instance = manager.load_wallet(&wallet).await?;

            // TODO: Implement forfeit logic
            println!("Forfeit functionality coming soon!");
            println!("Game ID: {}", game_id);
            println!("Wallet: {}", wallet);
        }

        GameCommands::Delete { game_id } => {
            let game_uuid = Uuid::parse_str(&game_id)
                .map_err(|_| arkive_core::ArkiveError::config("Invalid game ID format"))?;

            if !game_manager.game_exists(game_uuid).await {
                println!("Game {} not found.", game_id);
                return Ok(());
            }

            game_manager
                .delete_game(game_uuid)
                .await
                .map_err(map_gaming_error)?;
            println!("Game {} deleted successfully.", game_id);
        }
    }

    Ok(())
}

/// Extract game status from stored state
fn extract_game_status(stored_state: &StoredGameState) -> String {
    if let Ok(game_info) = serde_json::from_value::<serde_json::Value>(stored_state.state.clone()) {
        if let Some(state) = game_info.get("state") {
            match state.as_str() {
                Some("WaitingForPlayers") => "Waiting for Players".to_string(),
                Some("WaitingForBets") => "Waiting for Bets".to_string(),
                Some("BetsCollected") => "Bets Collected".to_string(),
                Some("InProgress") => "In Progress".to_string(),
                Some(s) if s.starts_with("Completed") => "Completed".to_string(),
                Some(s) if s.starts_with("Aborted") => "Aborted".to_string(),
                _ => "Unknown".to_string(),
            }
        } else {
            "Unknown".to_string()
        }
    } else {
        "Unknown".to_string()
    }
}
