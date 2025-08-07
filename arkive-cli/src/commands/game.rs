#![allow(unused_imports)]
use arkive_core::{Result as ArkiveResult, WalletManager};
use arkive_gaming::{initialize_gaming, GameState, TwoPlayerLottery};
use bitcoin::Amount;
use clap::Subcommand;
use comfy_table::{presets::UTF8_FULL, Table};

#[derive(Subcommand)]
pub enum GameCommands {
    /// Create a new two-player lottery
    CreateLottery {
        /// Game ID
        game_id: String,
        /// Bet amount in satoshis
        amount: u64,
        /// Creator wallet name
        wallet: String,
    },
    /// Join an existing lottery
    JoinLottery {
        /// Game ID to join
        game_id: String,
        /// Joiner wallet name
        wallet: String,
    },
    /// Submit commitment for a game
    Commit {
        /// Game ID
        game_id: String,
        /// Player wallet name
        wallet: String,
    },
    /// Submit reveal for a game
    Reveal {
        /// Game ID
        game_id: String,
        /// Player wallet name
        wallet: String,
        /// Commitment data (JSON format)
        commitment_data: String,
    },
    /// Show game status
    Status {
        /// Game ID
        game_id: String,
    },
    /// List all games for a wallet
    List {
        /// Wallet name
        wallet: String,
    },
    /// Handle timeout for a game
    Timeout {
        /// Game ID
        game_id: String,
    },
    /// Execute mutual abort for a game
    Abort {
        /// Game ID
        game_id: String,
    },
}

pub async fn handle_game_command(cmd: GameCommands, manager: &WalletManager) -> ArkiveResult<()> {
    // Initialize gaming module with updated signature
    let lottery = initialize_gaming(std::sync::Arc::new(manager.clone()))
        .await
        .map_err(|e| {
            arkive_core::ArkiveError::internal(format!("Failed to initialize gaming: {}", e))
        })?;

    match cmd {
        GameCommands::CreateLottery {
            game_id,
            amount,
            wallet,
        } => {
            let wallet_instance = manager.load_wallet(&wallet).await?;
            let ark_address = wallet_instance.get_ark_address().await?;

            let bet_amount = Amount::from_sat(amount);

            println!("Creating two-player lottery...");
            println!("Game ID: {}", game_id);
            println!("Bet Amount: {} sats", amount);
            println!("Creator: {}", wallet);

            let created_game_id = lottery
                .create_game(
                    game_id.clone(),
                    bet_amount,
                    &wallet,
                    arkive_core::ArkAddress::decode(&ark_address.address).map_err(|e| {
                        arkive_core::ArkiveError::internal(format!("Invalid ark address: {}", e))
                    })?,
                )
                .await
                .map_err(|e| {
                    arkive_core::ArkiveError::internal(format!("Failed to create game: {}", e))
                })?;

            println!("✅ Lottery created successfully!");
            println!("Game ID: {}", created_game_id);
            println!("Waiting for another player to join...");
        }

        GameCommands::JoinLottery { game_id, wallet } => {
            let wallet_instance = manager.load_wallet(&wallet).await?;
            let ark_address = wallet_instance.get_ark_address().await?;

            println!("Joining lottery game: {}", game_id);
            println!("Player: {}", wallet);

            lottery
                .join_game(
                    &game_id,
                    &wallet,
                    arkive_core::ArkAddress::decode(&ark_address.address).map_err(|e| {
                        arkive_core::ArkiveError::internal(format!("Invalid ark address: {}", e))
                    })?,
                )
                .await
                .map_err(|e| {
                    arkive_core::ArkiveError::internal(format!("Failed to join game: {}", e))
                })?;

            println!("✅ Successfully joined the lottery!");
            println!("Game is now ready. Escrow will be created automatically.");
            println!("You can now proceed to make commitments.");
        }

        GameCommands::Commit { game_id, wallet } => {
            println!("Submitting commitment for game: {}", game_id);

            // Get game state to determine player ID
            let game_state = lottery.get_game_state(&game_id).await.map_err(|e| {
                arkive_core::ArkiveError::internal(format!("Failed to get game state: {}", e))
            })?;

            let player_id = game_state
                .players
                .iter()
                .find(|p| p.wallet_id == wallet)
                .map(|p| p.id.clone())
                .ok_or_else(|| arkive_core::ArkiveError::internal("Player not found in game"))?;

            let (commitment, commitment_data) = lottery
                .submit_commitment(&game_id, &player_id)
                .await
                .map_err(|e| {
                    arkive_core::ArkiveError::internal(format!(
                        "Failed to submit commitment: {}",
                        e
                    ))
                })?;

            println!("✅ Commitment submitted successfully!");
            println!("Commitment Hash: {}", commitment.hash);
            println!("⚠️  IMPORTANT: Save this commitment data securely!");
            println!("You will need it for the reveal phase:");
            println!(
                "{}",
                serde_json::to_string_pretty(&commitment_data).map_err(|e| {
                    arkive_core::ArkiveError::internal(format!(
                        "Failed to serialize commitment data: {}",
                        e
                    ))
                })?
            );
        }

        GameCommands::Reveal {
            game_id,
            wallet,
            commitment_data,
        } => {
            println!("Submitting reveal for game: {}", game_id);

            // Parse commitment data
            let commitment_data: arkive_gaming::CommitmentData =
                serde_json::from_str(&commitment_data).map_err(|e| {
                    arkive_core::ArkiveError::internal(format!("Invalid commitment data: {}", e))
                })?;

            // Get game state to determine player ID
            let game_state = lottery.get_game_state(&game_id).await.map_err(|e| {
                arkive_core::ArkiveError::internal(format!("Failed to get game state: {}", e))
            })?;

            let player_id = game_state
                .players
                .iter()
                .find(|p| p.wallet_id == wallet)
                .map(|p| p.id.clone())
                .ok_or_else(|| arkive_core::ArkiveError::internal("Player not found in game"))?;

            lottery
                .submit_reveal(&game_id, &player_id, commitment_data)
                .await
                .map_err(|e| {
                    arkive_core::ArkiveError::internal(format!("Failed to submit reveal: {}", e))
                })?;

            println!("✅ Reveal submitted successfully!");
            println!("Waiting for other player to reveal...");

            // Check if game is finished
            let updated_game_state = lottery.get_game_state(&game_id).await.map_err(|e| {
                arkive_core::ArkiveError::internal(format!(
                    "Failed to get updated game state: {}",
                    e
                ))
            })?;

            if let Some(result) = &updated_game_state.result {
                if let Some(winner) = &result.winner {
                    println!("🎉 Game finished! Winner: {}", winner);
                    println!("Prize: {} sats", result.pot_amount.to_sat());
                    if let Some(txid) = &result.transaction_id {
                        println!("Payout transaction: {}", txid);
                    }
                }
            }
        }

        GameCommands::Status { game_id } => {
            let game_state = lottery.get_game_state(&game_id).await.map_err(|e| {
                arkive_core::ArkiveError::internal(format!("Failed to get game state: {}", e))
            })?;

            println!("Game Status for: {}", game_id);
            println!("================");
            println!("State: {:?}", game_state.state);
            println!("Bet Amount: {} sats", game_state.bet_amount.to_sat());
            println!("Pot Amount: {} sats", game_state.pot_amount.to_sat());
            println!(
                "Created: {}",
                game_state.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
            println!(
                "Updated: {}",
                game_state.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
            );

            if let Some(escrow_addr) = &game_state.escrow_address {
                println!("Escrow Address: {}", escrow_addr);
            }

            println!("\nPlayers:");
            let mut table = Table::new();
            table.load_preset(UTF8_FULL);
            table.set_header(vec!["ID", "Wallet", "Address", "Committed", "Revealed"]);

            for player in &game_state.players {
                let committed = if game_state.has_committed(&player.id) {
                    "✅"
                } else {
                    "❌"
                };
                let revealed = if game_state.has_revealed(&player.id) {
                    "✅"
                } else {
                    "❌"
                };

                table.add_row(vec![
                    &player.id,
                    &player.wallet_id,
                    &format!("{}...", &player.ark_address.to_string()[..20]),
                    committed,
                    revealed,
                ]);
            }
            println!("{}", table);

            println!("\nTimeouts:");
            println!(
                "Commitment: {}",
                game_state
                    .timeouts
                    .commitment_timeout
                    .format("%Y-%m-%d %H:%M:%S UTC")
            );
            println!(
                "Reveal: {}",
                game_state
                    .timeouts
                    .reveal_timeout
                    .format("%Y-%m-%d %H:%M:%S UTC")
            );
            println!(
                "Game: {}",
                game_state
                    .timeouts
                    .game_timeout
                    .format("%Y-%m-%d %H:%M:%S UTC")
            );

            if let Some(result) = &game_state.result {
                println!("\nResult:");
                if let Some(winner) = &result.winner {
                    println!("Winner: {}", winner);
                    println!("Prize: {} sats", result.pot_amount.to_sat());
                } else {
                    println!("Game aborted - no winner");
                }
                println!("Reason: {:?}", result.reason);
                if let Some(txid) = &result.transaction_id {
                    println!("Transaction: {}", txid);
                }
                println!(
                    "Finished: {}",
                    result.finished_at.format("%Y-%m-%d %H:%M:%S UTC")
                );
            }
        }

        GameCommands::List { wallet } => {
            let games = lottery.get_player_games(&wallet).await.map_err(|e| {
                arkive_core::ArkiveError::internal(format!("Failed to get player games: {}", e))
            })?;

            if games.is_empty() {
                println!("No games found for wallet: {}", wallet);
                return Ok(());
            }

            println!("Games for wallet: {}", wallet);
            println!("===================");

            let mut table = Table::new();
            table.load_preset(UTF8_FULL);
            table.set_header(vec![
                "Game ID",
                "State",
                "Bet (sats)",
                "Pot (sats)",
                "Players",
                "Created",
                "Winner",
            ]);

            for game in games {
                let winner = game
                    .result
                    .as_ref()
                    .and_then(|r| r.winner.as_ref())
                    .unwrap_or(&"-".to_string())
                    .clone();

                table.add_row(vec![
                    &game.game_id,
                    &format!("{:?}", game.state),
                    &game.bet_amount.to_sat().to_string(),
                    &game.pot_amount.to_sat().to_string(),
                    &game.players.len().to_string(),
                    &game.created_at.format("%m-%d %H:%M").to_string(),
                    &winner,
                ]);
            }

            println!("{}", table);
        }

        GameCommands::Timeout { game_id } => {
            println!("Handling timeout for game: {}", game_id);

            lottery.handle_timeout(&game_id).await.map_err(|e| {
                arkive_core::ArkiveError::internal(format!("Failed to handle timeout: {}", e))
            })?;

            println!("✅ Timeout handled successfully!");

            // Show updated game status
            let game_state = lottery.get_game_state(&game_id).await.map_err(|e| {
                arkive_core::ArkiveError::internal(format!("Failed to get game state: {}", e))
            })?;

            if let Some(result) = &game_state.result {
                if let Some(winner) = &result.winner {
                    println!("Winner by timeout: {}", winner);
                    println!("Prize: {} sats", result.pot_amount.to_sat());
                } else {
                    println!("Game aborted due to timeout - refunds issued");
                }
                if let Some(txid) = &result.transaction_id {
                    println!("Transaction: {}", txid);
                }
            }
        }

        GameCommands::Abort { game_id } => {
            println!("Executing mutual abort for game: {}", game_id);

            lottery.execute_mutual_abort(&game_id).await.map_err(|e| {
                arkive_core::ArkiveError::internal(format!("Failed to execute mutual abort: {}", e))
            })?;

            println!("✅ Mutual abort executed successfully!");
            println!("Refunds have been issued to both players.");

            // Show updated game status
            let game_state = lottery.get_game_state(&game_id).await.map_err(|e| {
                arkive_core::ArkiveError::internal(format!("Failed to get game state: {}", e))
            })?;

            if let Some(result) = &game_state.result {
                if let Some(txid) = &result.transaction_id {
                    println!("Refund transaction: {}", txid);
                }
            }
        }
    }

    Ok(())
}
