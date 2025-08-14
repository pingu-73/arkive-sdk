#![allow(unused_imports)]
use arkive_core::games::service::GameService;
use arkive_core::games::{Commitment, EscrowState, GameEscrow, GameOutcome, Participant, Reveal};
use arkive_core::{ArkAddress, ArkiveError, Result, WalletManager};
use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Keypair, Message, Secp256k1};
use bitcoin::{Amount, XOnlyPublicKey};
use clap::Subcommand;
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

    /// Claim winnings via Ark
    Claim {
        /// Wallet name
        wallet: String,
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

                    for (i, participant) in lottery_escrow.participants.iter().enumerate() {
                        println!("  {}. {}", i + 1, participant);
                    }

                    println!("Created: {}", lottery_escrow.created_at);
                    println!("Reveal Deadline: {}", lottery_escrow.reveal_deadline);
                    println!("Claim Deadline: {}", lottery_escrow.claim_deadline);

                    // Show funding status
                    if !lottery_escrow.funding_vtxos.is_empty() {
                        println!("\nFunding Status:");
                        for (i, funding) in lottery_escrow.funding_vtxos.iter().enumerate() {
                            println!(
                                "  {}. {} sats from {}",
                                i + 1,
                                funding.amount.to_sat(),
                                funding.participant
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
            let wallet = manager.load_wallet(&wallet).await?;

            println!("Revealing secret for Ark VTXO lottery {}...", lottery_id);

            // Load secret
            let (secret, nonce) = load_lottery_secret(&lottery_id)?;

            // Create and store reveal
            let reveal = create_reveal(&secret, &nonce, &wallet.keypair)?;
            let game_service = wallet.get_game_service();
            let conn = game_service.storage.get_connection().await;
            let (pubkey, _) = wallet.keypair.x_only_public_key();

            conn.execute(
                "UPDATE game_commitments 
                 SET reveal_preimage = ?1, reveal_nonce = ?2, reveal_signature = ?3
                 WHERE escrow_id = ?4 AND participant_pubkey = ?5",
                rusqlite::params![
                    hex::encode(reveal.preimage),
                    hex::encode(reveal.nonce),
                    hex::encode(reveal.signature.as_ref()),
                    &lottery_id,
                    pubkey.to_string(),
                ],
            )?;

            println!("✅ Secret revealed!");

            // Check if all revealed
            let reveal_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM game_commitments 
                 WHERE escrow_id = ?1 AND reveal_preimage IS NOT NULL",
                rusqlite::params![&lottery_id],
                |row| row.get(0),
            )?;

            let participants: String = conn.query_row(
                "SELECT participants FROM game_escrows WHERE escrow_id = ?1",
                rusqlite::params![&lottery_id],
                |row| row.get(0),
            )?;
            let participants: Vec<String> = serde_json::from_str(&participants)?;

            if reveal_count == participants.len() as i64 {
                println!("\n🎲 All secrets revealed! Calculating winner...");

                // Calculate winner
                let outcome = calculate_winner_with_ark(&conn, &lottery_id, &wallet).await?;

                if outcome.winner == pubkey {
                    println!(
                        "\n🎉 CONGRATULATIONS! You won {} sats!",
                        outcome.total_pot.to_sat()
                    );
                    println!("The pot will be transferred to you via Ark in the next batch swap.");
                    println!(
                        "Run: arkive game lottery claim --wallet {} {}",
                        wallet.name(),
                        lottery_id
                    );
                } else {
                    println!("\n😔 You lost. Winner: {}", outcome.winner);
                    println!("Your entry fee VTXO will be forfeited to the winner.");
                }

                // Update state
                conn.execute(
                    "UPDATE game_escrows SET state = ?1 WHERE escrow_id = ?2",
                    rusqlite::params![serde_json::to_string(&EscrowState::Resolved)?, &lottery_id,],
                )?;

                // Trigger batch swap for settlement
                println!("\n📦 Triggering Ark batch swap for lottery settlement...");
                match wallet.batch_swap(None).await {
                    Ok(Some(swap_id)) => {
                        println!("✅ Batch swap initiated: {}", swap_id);
                        println!("This will consolidate the lottery and transfer winnings.");
                    }
                    Ok(None) => {
                        println!("⏳ Batch swap will be triggered in the next round.");
                    }
                    Err(e) => {
                        println!("⚠️  Batch swap failed: {}", e);
                        println!("You may need to manually trigger it later.");
                    }
                }
            } else {
                println!(
                    "Reveals: {}/{} - Waiting for other player(s)...",
                    reveal_count,
                    participants.len()
                );
            }

            Ok(())
        }

        LotteryCommands::Claim { wallet, lottery_id } => {
            let wallet = manager.load_wallet(&wallet).await?;

            println!("Claiming Ark VTXO lottery winnings for {}...", lottery_id);

            // Load outcome
            let game_service = wallet.get_game_service();
            let conn = game_service.storage.get_connection().await;
            let (pubkey, _) = wallet.keypair.x_only_public_key();

            // Check if winner
            let result = conn.query_row(
                "SELECT winner_pubkey, total_stake FROM game_outcomes 
                 JOIN game_escrows ON game_outcomes.escrow_id = game_escrows.escrow_id
                 WHERE game_outcomes.escrow_id = ?1",
                rusqlite::params![&lottery_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            );

            match result {
                Ok((winner_str, pot)) => {
                    let winner = XOnlyPublicKey::from_str(&winner_str).map_err(|e| {
                        ArkiveError::internal(format!("Invalid winner pubkey: {}", e))
                    })?;

                    if winner != pubkey {
                        return Err(ArkiveError::internal("You are not the winner"));
                    }

                    println!("✅ You are the winner!");
                    println!("💰 Pot: {} sats", pot);

                    // The actual claiming happens through batch swaps
                    // The losing VTXOs are forfeited and the winner gets the combined amount
                    println!("\n📋 The winnings will be consolidated in your next batch swap.");
                    println!("The Ark operator will process forfeit transactions from losers.");
                    println!("\nYour balance should update after the next round completes.");

                    // Mark as claimed
                    conn.execute(
                        "UPDATE game_outcomes SET payout_psbt = ?1 WHERE escrow_id = ?2",
                        rusqlite::params![
                            format!("ark_claimed_{}", chrono::Utc::now().timestamp()),
                            &lottery_id,
                        ],
                    )?;
                }
                Err(_) => {
                    println!("❌ Lottery outcome not found or not yet resolved.");
                    println!("Make sure all players have revealed their secrets.");
                }
            }

            Ok(())
        }

        LotteryCommands::Status { lottery_id } => {
            println!("Ark VTXO Lottery Status: {}", lottery_id);
            println!("════════════════════════════");

            // Load from any wallet's storage
            let wallets = manager.list_wallets().await?;
            if wallets.is_empty() {
                return Err(ArkiveError::internal("No wallets found"));
            }

            let wallet = manager.load_wallet(&wallets[0]).await?;
            let game_service = wallet.get_game_service();
            let conn = game_service.storage.get_connection().await;

            // Get lottery info
            let result = conn.query_row(
                "SELECT state, participants, total_stake, timeout_block, taproot_address 
                 FROM game_escrows WHERE escrow_id = ?1",
                rusqlite::params![&lottery_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            );

            match result {
                Ok((state_str, participants_str, stake, timeout, escrow_addr)) => {
                    let state: EscrowState = serde_json::from_str(&state_str)?;
                    let participants: Vec<String> = serde_json::from_str(&participants_str)?;

                    println!("State: {:?}", state);
                    println!("Participants: {}/2", participants.len());
                    println!("Total pot: {} sats", stake);
                    println!("Timeout block: {}", timeout);
                    println!("Escrow Ark address: {}...", &escrow_addr[..20]);
                    println!("Payment method: Ark VTXOs");

                    // Show participants
                    if !participants.is_empty() {
                        println!("\nParticipants:");
                        for (i, p) in participants.iter().enumerate() {
                            println!("  {}. {}...", i + 1, &p[..16]);
                        }
                    }

                    // Check commitments and reveals
                    let commits: i64 = conn
                        .query_row(
                            "SELECT COUNT(*) FROM game_commitments 
                         WHERE escrow_id = ?1 AND commitment_signature IS NOT NULL",
                            rusqlite::params![&lottery_id],
                            |row| row.get(0),
                        )
                        .unwrap_or(0);

                    let reveals: i64 = conn
                        .query_row(
                            "SELECT COUNT(*) FROM game_commitments 
                         WHERE escrow_id = ?1 AND reveal_preimage IS NOT NULL",
                            rusqlite::params![&lottery_id],
                            |row| row.get(0),
                        )
                        .unwrap_or(0);

                    if commits > 0 || reveals > 0 {
                        println!("\nProgress:");
                        println!("  Commitments: {}/{}", commits, participants.len());
                        println!("  Reveals: {}/{}", reveals, participants.len());
                    }

                    // Check for winner
                    if let Ok(winner) = conn.query_row::<String, _, _>(
                        "SELECT winner_pubkey FROM game_outcomes WHERE escrow_id = ?1",
                        rusqlite::params![&lottery_id],
                        |row| row.get(0),
                    ) {
                        println!("\n🏆 Winner: {}...", &winner[..16]);
                    }

                    // Show next action based on state
                    match state {
                        EscrowState::Gathering => {
                            println!("\n📋 Next: Wait for all players to join");
                        }
                        EscrowState::CommitPhase => {
                            println!("\n📋 Next: All players should commit their random values");
                        }
                        EscrowState::RevealPhase => {
                            println!("\n📋 Next: All players should reveal their secrets");
                        }
                        EscrowState::Resolved => {
                            println!("\n✅ Lottery complete! Winner can claim the pot.");
                        }
                        _ => {}
                    }
                }
                Err(_) => {
                    println!("Lottery not found: {}", lottery_id);
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

async fn calculate_winner_with_ark(
    conn: &tokio::sync::MutexGuard<'_, rusqlite::Connection>,
    lottery_id: &str,
    _wallet: &arkive_core::ArkWallet,
) -> Result<LotteryOutcome> {
    // Load reveals - need to parse the commitment_hash field properly
    let mut stmt = conn.prepare(
        "SELECT participant_pubkey, reveal_preimage, reveal_nonce, commitment_hash 
         FROM game_commitments WHERE escrow_id = ?1 AND reveal_preimage IS NOT NULL",
    )?;

    let reveals: Vec<(String, Vec<u8>, Vec<u8>, String)> = stmt
        .query_map(rusqlite::params![lottery_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                hex::decode(row.get::<_, String>(1)?).unwrap_or_default(),
                hex::decode(row.get::<_, String>(2)?).unwrap_or_default(),
                row.get::<_, String>(3)?, // Get the full commitment_hash field
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // Calculate winner using provably fair algorithm
    let mut combined = Vec::new();
    for (_, preimage, nonce, _) in &reveals {
        combined.extend_from_slice(preimage);
        combined.extend_from_slice(nonce);
    }

    let seed = sha256::Hash::hash(&combined).to_byte_array();
    let seed_num = u64::from_le_bytes(seed[0..8].try_into().unwrap());
    let winner_index = (seed_num % reveals.len() as u64) as usize;

    let winner_pubkey = XOnlyPublicKey::from_str(&reveals[winner_index].0)
        .map_err(|e| ArkiveError::internal(format!("Invalid winner pubkey: {}", e)))?;

    // Get total stake
    let total_stake: i64 = conn.query_row(
        "SELECT total_stake FROM game_escrows WHERE escrow_id = ?1",
        rusqlite::params![lottery_id],
        |row| row.get(0),
    )?;

    // Store outcome
    conn.execute(
        "INSERT OR REPLACE INTO game_outcomes 
         (escrow_id, winner_pubkey, payouts, proof, finalized_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            lottery_id,
            winner_pubkey.to_string(),
            serde_json::to_string(&vec![(winner_pubkey.to_string(), total_stake)])?,
            hex::encode(seed),
            chrono::Utc::now().timestamp(),
        ],
    )?;

    // Get the entry fee transaction IDs for forfeiting
    let loser_txids: Vec<String> = reveals
        .iter()
        .filter(|(pk, _, _, _)| XOnlyPublicKey::from_str(pk).ok() != Some(winner_pubkey))
        .filter_map(|(_, _, _, commitment_hash)| {
            // Parse the entry_fee_txid from commitment_hash
            commitment_hash
                .split('|')
                .find(|s| s.starts_with("entry_fee_txid:"))
                .and_then(|s| s.strip_prefix("entry_fee_txid:"))
                .map(String::from)
        })
        .collect();

    println!("\n💸 Losing VTXOs to be forfeited: {}", loser_txids.len());
    for txid in &loser_txids {
        println!("   - {}", txid);
    }

    Ok(LotteryOutcome {
        winner: winner_pubkey,
        total_pot: Amount::from_sat(total_stake as u64),
    })
}

struct LotteryOutcome {
    winner: XOnlyPublicKey,
    total_pot: Amount,
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

fn load_lottery_secret(lottery_id: &str) -> Result<([u8; 32], [u8; 32])> {
    let path = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("arkive")
        .join("lottery_secrets")
        .join(format!("{}.json", lottery_id));

    let data = std::fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&data)?;

    let secret = hex::decode(json["secret"].as_str().unwrap())
        .map_err(|e| ArkiveError::internal(format!("Invalid secret hex: {}", e)))?;
    let nonce = hex::decode(json["nonce"].as_str().unwrap())
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
