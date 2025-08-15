#![allow(unused_imports)]
use arkive_core::games::service::GameService;
use arkive_core::games::{
    Commitment, EscrowState, GameEscrow, GameOutcome, LotteryState, Participant, Reveal,
};
use arkive_core::{ArkAddress, ArkiveError, Result, WalletManager};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Keypair, Message, Secp256k1};
use bitcoin::{Amount, XOnlyPublicKey};
use clap::Subcommand;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Subcommand)]
pub enum GameCommands {
    /// Lottery commands
    #[command(subcommand)]
    Lottery(LotteryCommands),
}

#[derive(Subcommand)]
pub enum LotteryCommands {
    /// Create a new lottery escrow
    CreateEscrow {
        /// Creator's wallet name
        wallet: String,
        /// Entry fee in satoshis
        #[arg(short, long)]
        entry_fee: u64,
        /// Comma-separated list of participant wallet names
        #[arg(short, long, value_delimiter = ',')]
        participants: Vec<String>,
    },

    /// Fund escrow lottery
    FundEscrow {
        /// Wallet name
        wallet: String,
        /// Lottery ID
        lottery_id: String,
    },

    /// Show escrow details
    EscrowInfo {
        /// Lottery ID
        lottery_id: String,
    },

    /// Submit commitment (after all players joined)
    Commit {
        /// Wallet name
        wallet: String,
        /// Lottery ID
        lottery_id: String,
    },

    /// Submit reveal
    Reveal {
        /// Wallet name
        wallet: String,
        /// Lottery ID
        lottery_id: String,
    },

    /// Check lottery status
    Status {
        /// Lottery ID
        lottery_id: String,
    },

    /// List active lotteries
    List,
}

pub async fn handle_game_command(cmd: GameCommands, manager: &WalletManager) -> Result<()> {
    match cmd {
        GameCommands::Lottery(lottery_cmd) => handle_lottery_command(lottery_cmd, manager).await,
    }
}

async fn handle_lottery_command(cmd: LotteryCommands, manager: &WalletManager) -> Result<()> {
    match cmd {
        LotteryCommands::CreateEscrow {
            wallet,
            entry_fee,
            participants,
        } => {
            let creator_wallet = manager.load_wallet(&wallet).await?;

            println!("Creating escrow-based lottery...");
            println!("Entry fee: {} sats", entry_fee);
            println!("Creator: {}", wallet);
            println!("Additional participants: {:?}", participants);

            // Get creator's public key
            let (creator_pubkey, _) = creator_wallet.keypair.x_only_public_key();

            // Collect participant public keys
            let mut participant_pubkeys = vec![creator_pubkey];

            for participant_wallet_name in participants {
                if participant_wallet_name == wallet {
                    continue; // Skip if creator is listed
                }

                match manager.load_wallet(&participant_wallet_name).await {
                    Ok(participant_wallet) => {
                        let (pubkey, _) = participant_wallet.keypair.x_only_public_key();
                        participant_pubkeys.push(pubkey);
                        println!(
                            "Added participant: {} ({})",
                            participant_wallet_name, pubkey
                        );
                    }
                    Err(e) => {
                        println!(
                            "Warning: Could not load participant wallet '{}': {}",
                            participant_wallet_name, e
                        );
                        // Continue with available participants
                    }
                }
            }

            println!("Total participants: {}", participant_pubkeys.len());

            let game_service = creator_wallet.get_game_service();
            let lottery_id = game_service
                .create_escrow_lottery_with_keys(participant_pubkeys, Amount::from_sat(entry_fee))
                .await?;

            let lottery_escrow = game_service.load_lottery_escrow(&lottery_id).await?;
            let escrow_address = lottery_escrow.escrow_address.clone();

            println!("✅ Escrow lottery created!");
            println!("Lottery ID: {}", lottery_id);
            println!("Escrow address: {}", escrow_address);

            Ok(())
        }

        LotteryCommands::FundEscrow { wallet, lottery_id } => {
            let participant_wallet = manager.load_wallet(&wallet).await?;

            // Get available VTXOs
            let vtxos = participant_wallet.list_vtxos().await?;
            if vtxos.is_empty() {
                println!("❌ No VTXOs available");
                return Err(ArkiveError::internal("No VTXOs to fund escrow"));
            }

            // Use first available VTXO for demo
            let vtxo = &vtxos[0];
            let outpoint = bitcoin::OutPoint::from_str(&vtxo.outpoint)
                .map_err(|e| ArkiveError::internal(format!("Invalid VTXO: {}", e)))?;

            let game_service = participant_wallet.get_game_service();

            // PASS THE ACTUAL WALLET INSTANCE for real fund movement
            game_service
                .fund_lottery_escrow_with_wallet(
                    &lottery_id,
                    participant_wallet.clone(),
                    outpoint,
                    vtxo.amount,
                )
                .await?;

            println!("✅ Funded escrow lottery Ark Tx");

            Ok(())
        }

        LotteryCommands::EscrowInfo { lottery_id } => {
            // Load any wallet to get game service
            let wallets = manager.list_wallets().await?;
            if wallets.is_empty() {
                return Err(ArkiveError::internal("No wallets available"));
            }

            let wallet = manager.load_wallet(&wallets[0]).await?;
            let game_service = wallet.get_game_service();

            match game_service.load_lottery_escrow(&lottery_id).await {
                Ok(lottery_escrow) => {
                    println!("Escrow Lottery: {}", lottery_id);
                    println!("════════════════════════════════════");
                    println!("State: {:?}", lottery_escrow.state);
                    println!("Escrow Address: {}", lottery_escrow.escrow_address);
                    println!("Entry Fee: {} sats", lottery_escrow.entry_fee.to_sat());
                    println!("Total Pot: {} sats", lottery_escrow.total_pot.to_sat());
                    println!("Participants: {}", lottery_escrow.participants.len());

                    let mut participant_names = HashMap::new();
                    for wallet_name in &wallets {
                        if let Ok(wallet_instance) = manager.load_wallet(wallet_name).await {
                            let (pubkey, _) = wallet_instance.keypair.x_only_public_key();
                            participant_names.insert(pubkey, wallet_name.clone());
                        }
                    }

                    for (i, participant) in lottery_escrow.participants.iter().enumerate() {
                        if let Some(name) = participant_names.get(participant) {
                            println!("  {}. {} ({})", i + 1, name, participant);
                        } else {
                            println!("  {}. {}", i + 1, participant);
                        }
                    }

                    println!("Created: {}", lottery_escrow.created_at);
                    println!("Reveal Deadline: {}", lottery_escrow.reveal_deadline);
                    println!("Claim Deadline: {}", lottery_escrow.claim_deadline);

                    if !lottery_escrow.funding_vtxos.is_empty() {
                        println!("\nFunding Status:");
                        for (i, funding) in lottery_escrow.funding_vtxos.iter().enumerate() {
                            let participant_display =
                                if let Some(name) = participant_names.get(&funding.participant) {
                                    format!("{} ({})", name, funding.participant)
                                } else {
                                    funding.participant.to_string()
                                };
                            println!(
                                "  {}. {} sats from {}",
                                i + 1,
                                funding.amount.to_sat(),
                                participant_display
                            );
                        }
                    }

                    // Show commitment/reveal status
                    let committed = lottery_escrow.commitments.len();
                    let revealed = lottery_escrow.reveals.len();
                    if committed > 0 || revealed > 0 {
                        println!("\nProgress:");
                        println!(
                            "  Committed: {}/{}",
                            committed,
                            lottery_escrow.participants.len()
                        );
                        println!(
                            "  Revealed: {}/{}",
                            revealed,
                            lottery_escrow.participants.len()
                        );
                    }

                    if lottery_escrow.state == crate::commands::game::LotteryState::WinnerDetermined
                    {
                        match game_service
                            .lottery_coordinator()
                            .load_lottery_outcome(&lottery_id)
                            .await
                        {
                            Ok(Some(outcome)) => {
                                println!("\n🏆 Winner Determined!");
                                let winner_display =
                                    if let Some(name) = participant_names.get(&outcome.winner) {
                                        format!("{} ({})", name, outcome.winner)
                                    } else {
                                        outcome.winner.to_string()
                                    };
                                println!("  Winner: {}", winner_display);
                                println!("  Prize: {} sats", outcome.total_stake);
                            }
                            Ok(None) => {
                                println!("\n⚠️  Winner determined but outcome data not found");
                            }
                            Err(e) => {
                                println!("\n⚠️  Could not load winner info: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("❌ Failed to load lottery: {}", e);
                }
            }

            Ok(())
        }

        LotteryCommands::Commit { wallet, lottery_id } => {
            let wallet_instance = manager.load_wallet(&wallet).await?;
            let (participant_pubkey, _) = wallet_instance.keypair.x_only_public_key();

            println!("Generating commitment for escrow lottery {}...", lottery_id);

            // Load the lottery to get participant info
            let game_service = wallet_instance.get_game_service();
            let lottery_escrow = game_service.load_lottery_escrow(&lottery_id).await?;

            // Verify participant is in lottery
            if !lottery_escrow.participants.contains(&participant_pubkey) {
                return Err(ArkiveError::internal(
                    "You are not a participant in this lottery",
                ));
            }

            // Generate real commitment with proper randomness
            let commitment =
                GameService::generate_lottery_commitment(&wallet_instance.keypair, &lottery_id)?;

            // Store commitment in lottery coordinator (not in Ark VTXO storage)
            game_service
                .lottery_coordinator()
                .submit_commitment_with_escrow(&lottery_id, participant_pubkey, commitment.clone())
                .await?;

            println!("✅ Commitment submitted for lottery {}", lottery_id);
            println!("Commitment hash: {}", hex::encode(commitment.hash));

            Ok(())
        }

        LotteryCommands::Reveal { wallet, lottery_id } => {
            let wallet_instance = manager.load_wallet(&wallet).await?;
            let (participant_pubkey, _) = wallet_instance.keypair.x_only_public_key();

            println!("Revealing secret for escrow lottery {}...", lottery_id);

            // Load secret
            // let (secret, nonce) = load_lottery_secret(&lottery_id)?;
            let (secret, nonce) = load_lottery_secret(&lottery_id, &participant_pubkey)?;

            // Create reveal with proper signatures
            let reveal = create_reveal(&secret, &nonce, &wallet_instance.keypair)?;

            let game_service = wallet_instance.get_game_service();

            // Submit reveal to lottery coordinator
            let outcome_option = game_service
                .submit_escrow_reveal(&lottery_id, participant_pubkey, reveal)
                .await?;

            println!("✅ Secret revealed for lottery {}", lottery_id);
            println!("Preimage: {}", hex::encode(secret));
            println!("Nonce: {}", hex::encode(nonce));

            // Check if all revealed and determine winner
            if let Some(outcome) = outcome_option {
                if outcome.winner == participant_pubkey {
                    println!("\n🎉 CONGRATULATIONS! You won the lottery!");
                    let winner_payout = outcome
                        .payouts
                        .get(&outcome.winner)
                        .map(|amt| amt.to_sat())
                        .unwrap_or(0);
                    println!("💰 Prize: {} sats", winner_payout);
                    println!("The winnings will be available in your Ark wallet shortly.");
                } else {
                    println!("\n😔 You lost this round.");
                    println!("Winner: {}", outcome.winner);
                    println!("Better luck next time!");
                }

                // Trigger payout processing
                println!("\n📦 Processing lottery payout...");
                let ark_service = wallet_instance.get_ark_service().await?;
                match game_service
                    .lottery_coordinator()
                    .execute_winner_payout(&lottery_id, outcome.winner, &ark_service)
                    .await
                {
                    Ok(swap_id) => {
                        println!("✅ Payout initiated via batch swap: {}", swap_id);
                        println!("Run 'arkive ark sync {}' to claim your winnings", wallet);
                    }
                    Err(e) => {
                        println!("⚠️  Payout processing failed: {}", e);
                        println!("You may need to contact support or manually claim.");
                    }
                }
            } else {
                println!("Reveal submitted. Waiting for other participants...");
                println!("Run this command again if you think all reveals are complete.");
            }

            Ok(())
        }

        LotteryCommands::Status { lottery_id } => {
            // Load any wallet to get game service
            let wallets = manager.list_wallets().await?;
            if wallets.is_empty() {
                return Err(ArkiveError::internal("No wallets available"));
            }

            let wallet = manager.load_wallet(&wallets[0]).await?;
            let game_service = wallet.get_game_service();

            match game_service.load_lottery_escrow(&lottery_id).await {
                Ok(lottery_escrow) => {
                    println!("Escrow Lottery Status: {}", lottery_id);
                    println!("════════════════════════════════════");
                    println!("State: {:?}", lottery_escrow.state);
                    println!("Escrow Address: {}", lottery_escrow.escrow_address);
                    println!("Entry Fee: {} sats", lottery_escrow.entry_fee.to_sat());
                    println!("Total Pot: {} sats", lottery_escrow.total_pot.to_sat());
                    println!("Participants: {}", lottery_escrow.participants.len());

                    // Map pubkeys to wallet names
                    let mut participant_names = HashMap::new();
                    for wallet_name in &wallets {
                        if let Ok(wallet_instance) = manager.load_wallet(wallet_name).await {
                            let (pubkey, _) = wallet_instance.keypair.x_only_public_key();
                            participant_names.insert(pubkey, wallet_name.clone());
                        }
                    }

                    for (i, participant) in lottery_escrow.participants.iter().enumerate() {
                        if let Some(name) = participant_names.get(participant) {
                            println!("  {}. {} ({})", i + 1, name, participant);
                        } else {
                            println!("  {}. {}", i + 1, participant);
                        }
                    }

                    println!("Created: {}", lottery_escrow.created_at);
                    println!("Reveal Deadline: {}", lottery_escrow.reveal_deadline);
                    println!("Claim Deadline: {}", lottery_escrow.claim_deadline);

                    if !lottery_escrow.funding_vtxos.is_empty() {
                        println!("\nFunding Status:");
                        for (i, funding) in lottery_escrow.funding_vtxos.iter().enumerate() {
                            let participant_display =
                                if let Some(name) = participant_names.get(&funding.participant) {
                                    format!("{} ({})", name, funding.participant)
                                } else {
                                    funding.participant.to_string()
                                };
                            println!(
                                "  {}. {} sats from {}",
                                i + 1,
                                funding.amount.to_sat(),
                                participant_display
                            );
                        }
                    }

                    // Show commitment/reveal status
                    let committed = lottery_escrow.commitments.len();
                    let revealed = lottery_escrow.reveals.len();
                    if committed > 0 || revealed > 0 {
                        println!("\nProgress:");
                        println!(
                            "  Committed: {}/{}",
                            committed,
                            lottery_escrow.participants.len()
                        );
                        println!(
                            "  Revealed: {}/{}",
                            revealed,
                            lottery_escrow.participants.len()
                        );
                    }

                    if lottery_escrow.state == crate::commands::game::LotteryState::WinnerDetermined
                    {
                        match game_service
                            .lottery_coordinator()
                            .load_lottery_outcome(&lottery_id)
                            .await
                        {
                            Ok(Some(outcome)) => {
                                println!("\n🏆 Winner Determined!");
                                let winner_display =
                                    if let Some(name) = participant_names.get(&outcome.winner) {
                                        format!("{} ({})", name, outcome.winner)
                                    } else {
                                        outcome.winner.to_string()
                                    };
                                println!("  Winner: {}", winner_display);
                                println!("  Prize: {} sats", outcome.total_stake);
                            }
                            Ok(None) => {
                                println!("\n⚠️  Winner determined but outcome data not found");
                            }
                            Err(e) => {
                                println!("\n⚠️  Could not load winner info: {}", e);
                            }
                        }
                    }

                    // Show next action based on state
                    match lottery_escrow.state {
                        LotteryState::AwaitingFunding => {
                            println!("\n📋 Next: Wait for all participants to fund escrow");
                        }
                        LotteryState::CommitmentPhase => {
                            println!(
                                "\n📋 Next: All participants should commit their random values"
                            );
                        }
                        LotteryState::RevealPhase => {
                            println!("\n📋 Next: All participants should reveal their secrets");
                        }
                        LotteryState::WinnerDetermined => {
                            println!("\n🏆 Winner determined! Processing payouts...");
                        }
                        LotteryState::Completed => {
                            println!("\n✅ Lottery complete!");
                        }
                        LotteryState::TimedOut => {
                            println!("\n⏰ Lottery timed out");
                        }
                        LotteryState::Disputed => {
                            println!("\n⚠️  Lottery disputed");
                        }
                    }
                }
                Err(e) => {
                    println!("❌ Failed to load lottery: {}", e);
                }
            }

            Ok(())
        }

        LotteryCommands::List => {
            // Load any wallet to get game service
            let wallets = manager.list_wallets().await?;
            if wallets.is_empty() {
                return Err(ArkiveError::internal("No wallets available"));
            }

            let wallet = manager.load_wallet(&wallets[0]).await?;
            let game_service = wallet.get_game_service();

            // Load escrow lotteries from the coordinator, not from Ark VTXO storage
            let lotteries = game_service.lottery_coordinator().list_lotteries().await?;

            println!("Active Escrow Lotteries:");
            println!("═════════════════════════");

            if lotteries.is_empty() {
                println!("No active escrow lotteries found.");
                println!("Create one with: arkive game lottery create-escrow <wallet> --entry-fee <sats> --participants <wallets>");
                return Ok(());
            }

            for lottery in lotteries {
                println!("Lottery ID: {}", lottery.lottery_id);
                println!("  State: {:?}", lottery.state);
                println!(
                    "  Funds Deposited: {}/{}",
                    lottery.funding_vtxos.len(),
                    lottery.participants.len()
                );
                println!("  Total Pot: {} sats", lottery.total_pot.to_sat());
                println!("  Escrow Address: {}", lottery.escrow_address);
                println!();
            }

            Ok(())
        }
    }
}

fn create_reveal(secret: &[u8; 32], nonce: &[u8; 32], keypair: &Keypair) -> Result<Reveal> {
    let secp = Secp256k1::new();
    let msg = Message::from_digest(*secret);
    let signature = secp.sign_schnorr(&msg, keypair);

    Ok(Reveal {
        preimage: *secret,
        nonce: *nonce,
        signature,
    })
}

pub fn load_lottery_secret(
    lottery_id: &str,
    participant_pubkey: &XOnlyPublicKey,
) -> Result<([u8; 32], [u8; 32])> {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("arkive")
        .join("lottery_secrets")
        .join(format!("{}.json", lottery_id));

    let data = std::fs::read_to_string(&data_dir)
        .map_err(|e| ArkiveError::internal(format!("Failed to read secret file: {}", e)))?;

    let json: serde_json::Value = serde_json::from_str(&data)
        .map_err(|e| ArkiveError::internal(format!("Invalid secret file format: {}", e)))?;

    // Get participant's secrets from the participants map
    let participant_secrets = json["participants"][participant_pubkey.to_string()]
        .as_object()
        .ok_or_else(|| ArkiveError::internal("No secrets found for participant"))?;

    let secret_hex = participant_secrets["secret"]
        .as_str()
        .ok_or_else(|| ArkiveError::internal("Missing secret for participant"))?;
    let nonce_hex = participant_secrets["nonce"]
        .as_str()
        .ok_or_else(|| ArkiveError::internal("Missing nonce for participant"))?;

    let secret = hex::decode(secret_hex)
        .map_err(|e| ArkiveError::internal(format!("Invalid secret hex: {}", e)))?;
    let nonce = hex::decode(nonce_hex)
        .map_err(|e| ArkiveError::internal(format!("Invalid nonce hex: {}", e)))?;

    Ok((
        secret
            .try_into()
            .map_err(|_| ArkiveError::internal("Invalid secret length"))?,
        nonce
            .try_into()
            .map_err(|_| ArkiveError::internal("Invalid nonce length"))?,
    ))
}
