#![allow(unused_variables, dead_code)]
use crate::core::TwoPlayerLotteryState;
use crate::error::{GamingError, Result};
use crate::escrow::VtxoEscrowManager;
use crate::storage::GameStorage;
use crate::LotteryVtxo;
use arkive_core::{ArkAddress, WalletManager};
use bitcoin::Amount;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct TwoPlayerLottery {
    wallet_manager: Arc<WalletManager>,
    vtxo_escrow_manager: Arc<VtxoEscrowManager>,
    games: Arc<RwLock<HashMap<String, TwoPlayerLotteryState>>>,
    lottery_vtxos: Arc<RwLock<HashMap<String, LotteryVtxo>>>,
    storage: GameStorage,
}

impl TwoPlayerLottery {
    pub fn new(wallet_manager: Arc<WalletManager>) -> Self {
        let vtxo_escrow_manager = Arc::new(VtxoEscrowManager::new(wallet_manager.clone()));
        let storage = GameStorage::new("games.json");

        Self {
            wallet_manager,
            vtxo_escrow_manager,
            games: Arc::new(RwLock::new(HashMap::new())),
            lottery_vtxos: Arc::new(RwLock::new(HashMap::new())),
            storage,
        }
    }

    /// Create escrow using real VTXO with tapscripts
    async fn create_escrow_for_game(&self, game_id: &str) -> Result<()> {
        let (player1_wallet, player2_wallet, commitment_hashes, bet_amount) = {
            let games = self.games.read().await;
            let game_state = games
                .get(game_id)
                .ok_or_else(|| GamingError::GameNotFound {
                    game_id: game_id.to_string(),
                })?;

            if game_state.players.len() != 2 {
                return Err(GamingError::escrow(
                    "Need exactly 2 players to create escrow",
                ));
            }

            let player1_wallet = game_state.players[0].wallet_id.clone();
            let player2_wallet = game_state.players[1].wallet_id.clone();

            // get commitment hashes should be set when commitments are made
            let commitment1_hash = game_state
                .commitments
                .get("player1")
                .map(|c| self.hash_to_bytes32(&c.hash))
                .unwrap_or([0u8; 32]);
            let commitment2_hash = game_state
                .commitments
                .get("player2")
                .map(|c| self.hash_to_bytes32(&c.hash))
                .unwrap_or([0u8; 32]);

            (
                player1_wallet,
                player2_wallet,
                (commitment1_hash, commitment2_hash),
                game_state.bet_amount,
            )
        };

        // create real VTXO-based escrow
        let lottery_vtxo = self
            .vtxo_escrow_manager
            .create_lottery_vtxo(
                game_id,
                &player1_wallet,
                &player2_wallet,
                bet_amount,
                commitment_hashes.0,
                commitment_hashes.1,
            )
            .await?;

        let escrow_address = lottery_vtxo.get_escrow_address();

        // store the VTXO
        {
            let mut vtxos = self.lottery_vtxos.write().await;
            vtxos.insert(game_id.to_string(), lottery_vtxo);
        }

        // update game state with real escrow address
        {
            let mut games = self.games.write().await;
            if let Some(game_state) = games.get_mut(game_id) {
                game_state.escrow_address = Some(escrow_address.to_string());
                game_state.pot_amount = bet_amount * 2;
                game_state.update_timestamp();
            }
        }

        // save updated game state
        {
            let games = self.games.read().await;
            if let Some(game_state) = games.get(game_id) {
                self.storage.save_game(game_state).await?;
            }
        }

        tracing::info!(
            "Created real VTXO escrow for game {} at address {}",
            game_id,
            escrow_address
        );
        Ok(())
    }

    /// Execute real payout using VTXO and tapscripts
    async fn execute_payout(
        &self,
        game_id: &str,
        winner_id: &str,
        winner_address: &str,
        amount: Amount,
    ) -> Result<()> {
        // get lottery VTXO
        let lottery_vtxo = {
            let vtxos = self.lottery_vtxos.read().await;
            vtxos
                .get(game_id)
                .ok_or_else(|| GamingError::escrow("Lottery VTXO not found for game"))?
                .clone()
        };

        // get reveal data for both players
        let (reveal1, reveal2) = {
            let games = self.games.read().await;
            let game_state = games
                .get(game_id)
                .ok_or_else(|| GamingError::GameNotFound {
                    game_id: game_id.to_string(),
                })?;

            let reveals: Vec<&crate::commitment::Reveal> = game_state.reveals.values().collect();
            if reveals.len() != 2 {
                return Err(GamingError::game("Expected exactly 2 reveals"));
            }

            (reveals[0].clone(), reveals[1].clone())
        };

        // parse winner add
        let winner_ark_address = ArkAddress::decode(winner_address)
            .map_err(|e| GamingError::internal(format!("Invalid winner address: {}", e)))?;

        // execute real payout using tapscripts
        let txid = self
            .vtxo_escrow_manager
            .execute_winner_payout(&lottery_vtxo, &winner_ark_address, &reveal1, &reveal2)
            .await?;

        // mark VTXO as spent
        self.vtxo_escrow_manager.mark_vtxo_spent(game_id).await?;

        // update game result with tx ID
        {
            let mut games = self.games.write().await;
            if let Some(game_state) = games.get_mut(game_id) {
                if let Some(ref mut result) = game_state.result {
                    result.transaction_id = Some(txid.clone());
                }
                game_state.update_timestamp();
            }
        }

        // save updated game state
        {
            let games = self.games.read().await;
            if let Some(game_state) = games.get(game_id) {
                self.storage.save_game(game_state).await?;
            }
        }

        // remove the VTXO as it's now spent
        {
            let mut vtxos = self.lottery_vtxos.write().await;
            vtxos.remove(game_id);
        }

        tracing::info!(
            "Executed real payout of {} sats to {} in transaction {}",
            amount.to_sat(),
            winner_address,
            txid
        );

        Ok(())
    }

    /// Handle timeout with real VTXO transaction
    pub async fn handle_timeout(&self, game_id: &str) -> Result<()> {
        // determine timeout action
        let timeout_action = {
            let mut games = self.games.write().await;
            let game_state = games
                .get_mut(game_id)
                .ok_or_else(|| GamingError::GameNotFound {
                    game_id: game_id.to_string(),
                })?;

            if game_state.is_finished() {
                return Err(GamingError::game("Game is already finished"));
            }

            self.determine_timeout_action(game_state)?
        };

        // Execute timeout action with real transactions
        match timeout_action {
            TimeoutAction::WinnerPayout {
                winner_id,
                winner_address,
                pot_amount,
            } => {
                self.execute_payout(game_id, &winner_id, &winner_address, pot_amount)
                    .await?;
                tracing::info!("Game {} ended by timeout, winner: {}", game_id, winner_id);
            }
            TimeoutAction::MutualRefund {
                player1_address,
                player2_address,
                refund_amount,
            } => {
                self.execute_timeout_refund(
                    game_id,
                    &player1_address,
                    &player2_address,
                    refund_amount,
                )
                .await?;
                tracing::info!("Game {} aborted due to timeout, refunds issued", game_id);
            }
            TimeoutAction::None => {
                // No timeout action needed
            }
        }

        Ok(())
    }

    /// Execute timeout refund using real VTXO transaction
    async fn execute_timeout_refund(
        &self,
        game_id: &str,
        player1_address: &str,
        player2_address: &str,
        refund_amount: Amount,
    ) -> Result<()> {
        let lottery_vtxo = {
            let vtxos = self.lottery_vtxos.read().await;
            vtxos
                .get(game_id)
                .ok_or_else(|| GamingError::escrow("Lottery VTXO not found for game"))?
                .clone()
        };

        let player1_ark_address = ArkAddress::decode(player1_address)
            .map_err(|e| GamingError::internal(format!("Invalid player1 address: {}", e)))?;
        let player2_ark_address = ArkAddress::decode(player2_address)
            .map_err(|e| GamingError::internal(format!("Invalid player2 address: {}", e)))?;

        // execute real timeout refund using tapscripts
        let txid = self
            .vtxo_escrow_manager
            .execute_timeout_refund(&lottery_vtxo, &player1_ark_address, &player2_ark_address)
            .await?;

        // update game result with real tx ID
        {
            let mut games = self.games.write().await;
            if let Some(game_state) = games.get_mut(game_id) {
                if let Some(ref mut result) = game_state.result {
                    result.transaction_id = Some(txid.clone());
                }
                game_state.update_timestamp();
            }
        }

        // Remove the VTXO as it's now spent
        {
            let mut vtxos = self.lottery_vtxos.write().await;
            vtxos.remove(game_id);
        }

        tracing::info!(
            "Executed real timeout refund of {} sats each in transaction {}",
            refund_amount.to_sat(),
            txid
        );

        Ok(())
    }

    /// Execute mutual abort using real VTXO transaction
    pub async fn execute_mutual_abort(&self, game_id: &str) -> Result<()> {
        let (player1_address, player2_address, refund_amount) = {
            let games = self.games.read().await;
            let game_state = games
                .get(game_id)
                .ok_or_else(|| GamingError::GameNotFound {
                    game_id: game_id.to_string(),
                })?;

            let player1_address = game_state.players[0].ark_address.to_string();
            let player2_address = game_state.players[1].ark_address.to_string();
            let refund_amount = game_state.bet_amount;

            (player1_address, player2_address, refund_amount)
        };

        let lottery_vtxo = {
            let vtxos = self.lottery_vtxos.read().await;
            vtxos
                .get(game_id)
                .ok_or_else(|| GamingError::escrow("Lottery VTXO not found for game"))?
                .clone()
        };

        let player1_ark_address = ArkAddress::decode(&player1_address)
            .map_err(|e| GamingError::internal(format!("Invalid player1 address: {}", e)))?;
        let player2_ark_address = ArkAddress::decode(&player2_address)
            .map_err(|e| GamingError::internal(format!("Invalid player2 address: {}", e)))?;

        // execute real mutual abort using tapscripts
        let txid = self
            .vtxo_escrow_manager
            .execute_mutual_abort(&lottery_vtxo, &player1_ark_address, &player2_ark_address)
            .await?;

        // update game state
        {
            let mut games = self.games.write().await;
            if let Some(game_state) = games.get_mut(game_id) {
                game_state.state = crate::core::GameState::Aborted;
                game_state.result = Some(crate::core::GameResult {
                    winner: None,
                    pot_amount: Amount::ZERO,
                    transaction_id: Some(txid.clone()),
                    finished_at: chrono::Utc::now(),
                    reason: crate::core::GameEndReason::PlayerAbort,
                });
                game_state.update_timestamp();
            }
        }

        // remove the VTXO as it's now spent
        {
            let mut vtxos = self.lottery_vtxos.write().await;
            vtxos.remove(game_id);
        }

        tracing::info!(
            "Executed mutual abort with refund of {} sats each in transaction {}",
            refund_amount.to_sat(),
            txid
        );

        Ok(())
    }

    fn hash_to_bytes32(&self, hash_str: &str) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        if let Ok(decoded) = hex::decode(hash_str) {
            let len = std::cmp::min(decoded.len(), 32);
            bytes[..len].copy_from_slice(&decoded[..len]);
        }
        bytes
    }

    fn determine_timeout_action(
        &self,
        game_state: &mut crate::core::TwoPlayerLotteryState,
    ) -> Result<TimeoutAction> {
        let now = chrono::Utc::now();

        match game_state.state {
            crate::core::GameState::WaitingForCommitments => {
                if now > game_state.timeouts.commitment_timeout {
                    let committed_players: Vec<_> = game_state
                        .players
                        .iter()
                        .filter(|p| game_state.has_committed(&p.id))
                        .collect();

                    if committed_players.len() == 1 {
                        let winner_id = committed_players[0].id.clone();
                        let winner_address = committed_players[0].ark_address.to_string();
                        let pot_amount = game_state.pot_amount;

                        let result = crate::core::GameResult {
                            winner: Some(winner_id.clone()),
                            pot_amount,
                            transaction_id: None,
                            finished_at: chrono::Utc::now(),
                            reason: crate::core::GameEndReason::PlayerTimeout,
                        };

                        game_state.result = Some(result);
                        game_state.state = crate::core::GameState::Finished;
                        game_state.update_timestamp();

                        Ok(TimeoutAction::WinnerPayout {
                            winner_id,
                            winner_address,
                            pot_amount,
                        })
                    } else {
                        let result = crate::core::GameResult {
                            winner: None,
                            pot_amount: Amount::ZERO,
                            transaction_id: None,
                            finished_at: chrono::Utc::now(),
                            reason: crate::core::GameEndReason::PlayerAbort,
                        };

                        game_state.result = Some(result);
                        game_state.state = crate::core::GameState::Aborted;
                        game_state.update_timestamp();

                        let player1_address = game_state.players[0].ark_address.to_string();
                        let player2_address = game_state.players[1].ark_address.to_string();
                        let refund_amount = game_state.bet_amount;

                        Ok(TimeoutAction::MutualRefund {
                            player1_address,
                            player2_address,
                            refund_amount,
                        })
                    }
                } else {
                    Ok(TimeoutAction::None)
                }
            }
            crate::core::GameState::WaitingForReveals => {
                if now > game_state.timeouts.reveal_timeout {
                    let revealed_players: Vec<_> = game_state
                        .players
                        .iter()
                        .filter(|p| game_state.has_revealed(&p.id))
                        .collect();

                    if revealed_players.len() == 1 {
                        let winner_id = revealed_players[0].id.clone();
                        let winner_address = revealed_players[0].ark_address.to_string();
                        let pot_amount = game_state.pot_amount;

                        let result = crate::core::GameResult {
                            winner: Some(winner_id.clone()),
                            pot_amount,
                            transaction_id: None,
                            finished_at: chrono::Utc::now(),
                            reason: crate::core::GameEndReason::PlayerTimeout,
                        };

                        game_state.result = Some(result);
                        game_state.state = crate::core::GameState::Finished;
                        game_state.update_timestamp();

                        Ok(TimeoutAction::WinnerPayout {
                            winner_id,
                            winner_address,
                            pot_amount,
                        })
                    } else {
                        let result = crate::core::GameResult {
                            winner: None,
                            pot_amount: Amount::ZERO,
                            transaction_id: None,
                            finished_at: chrono::Utc::now(),
                            reason: crate::core::GameEndReason::PlayerAbort,
                        };

                        game_state.result = Some(result);
                        game_state.state = crate::core::GameState::Aborted;
                        game_state.update_timestamp();

                        let player1_address = game_state.players[0].ark_address.to_string();
                        let player2_address = game_state.players[1].ark_address.to_string();
                        let refund_amount = game_state.bet_amount;

                        Ok(TimeoutAction::MutualRefund {
                            player1_address,
                            player2_address,
                            refund_amount,
                        })
                    }
                } else {
                    Ok(TimeoutAction::None)
                }
            }
            _ => {
                if now > game_state.timeouts.game_timeout {
                    let result = crate::core::GameResult {
                        winner: None,
                        pot_amount: Amount::ZERO,
                        transaction_id: None,
                        finished_at: chrono::Utc::now(),
                        reason: crate::core::GameEndReason::PlayerAbort,
                    };

                    game_state.result = Some(result);
                    game_state.state = crate::core::GameState::Aborted;
                    game_state.update_timestamp();

                    let player1_address = game_state.players[0].ark_address.to_string();
                    let player2_address = game_state.players[1].ark_address.to_string();
                    let refund_amount = game_state.bet_amount;

                    Ok(TimeoutAction::MutualRefund {
                        player1_address,
                        player2_address,
                        refund_amount,
                    })
                } else {
                    Ok(TimeoutAction::None)
                }
            }
        }
    }

    pub async fn load_existing_games(&self) -> Result<()> {
        let stored_games = self.storage.load_all_games().await?;
        let mut games = self.games.write().await;
        *games = stored_games;

        // We don't load VTXOs from storage anymore since they're managed differently
        // VTXOs will be recreated when needed based on game state

        tracing::info!("Loaded {} existing games", games.len());
        Ok(())
    }

    /// Create a new two-player lottery game
    pub async fn create_game(
        &self,
        game_id: String,
        bet_amount: Amount,
        creator_wallet: &str,
        creator_ark_address: arkive_core::ArkAddress,
    ) -> Result<String> {
        if bet_amount.to_sat() == 0 {
            return Err(GamingError::InvalidBetAmount {
                amount: bet_amount.to_sat(),
            });
        }

        // load wallet to get keypair info
        let _wallet = self
            .wallet_manager
            .load_wallet(creator_wallet)
            .await
            .map_err(|e| GamingError::player(format!("Failed to load creator wallet: {}", e)))?;

        // create player
        let creator_player = crate::core::Player::new(
            "player1".to_string(),
            creator_wallet.to_string(),
            creator_ark_address,
            &bitcoin::key::Keypair::new(
                &bitcoin::secp256k1::Secp256k1::new(),
                &mut rand::thread_rng(),
            ), // Placeholder
        );

        // create game state
        let mut game_state = crate::core::TwoPlayerLotteryState::new(game_id.clone(), bet_amount);
        game_state.players.push(creator_player);
        game_state.update_timestamp();

        // Store game
        {
            let mut games = self.games.write().await;
            games.insert(game_id.clone(), game_state.clone());
        }

        self.storage.save_game(&game_state).await?;

        tracing::info!(
            "Created two-player lottery game {} with bet amount {} sats",
            game_id,
            bet_amount.to_sat()
        );

        Ok(game_id)
    }

    /// Join an existing game
    pub async fn join_game(
        &self,
        game_id: &str,
        joiner_wallet: &str,
        joiner_ark_address: arkive_core::ArkAddress,
    ) -> Result<()> {
        // load from storage if not in memory
        {
            let games = self.games.read().await;
            if !games.contains_key(game_id) {
                drop(games);

                if let Some(game_state) = self.storage.load_game(game_id).await? {
                    let mut games = self.games.write().await;
                    games.insert(game_id.to_string(), game_state);
                } else {
                    return Err(GamingError::GameNotFound {
                        game_id: game_id.to_string(),
                    });
                }
            }
        }

        let can_join = {
            let games = self.games.read().await;
            let game_state = games
                .get(game_id)
                .ok_or_else(|| GamingError::GameNotFound {
                    game_id: game_id.to_string(),
                })?;
            game_state.can_join()
        };

        if !can_join {
            return Err(GamingError::GameAlreadyStarted);
        }

        // load joiner wallet
        let _wallet = self
            .wallet_manager
            .load_wallet(joiner_wallet)
            .await
            .map_err(|e| GamingError::player(format!("Failed to load joiner wallet: {}", e)))?;

        // create player
        let joiner_player = crate::core::Player::new(
            "player2".to_string(),
            joiner_wallet.to_string(),
            joiner_ark_address,
            &bitcoin::key::Keypair::new(
                &bitcoin::secp256k1::Secp256k1::new(),
                &mut rand::thread_rng(),
            ), // Placeholder
        );

        // update game state
        let should_create_escrow = {
            let mut games = self.games.write().await;
            let game_state = games
                .get_mut(game_id)
                .ok_or_else(|| GamingError::GameNotFound {
                    game_id: game_id.to_string(),
                })?;

            game_state.players.push(joiner_player);
            game_state.update_timestamp();

            // check if we should create escrow
            let should_create = game_state.is_ready_for_commitments();
            if should_create {
                game_state.state = crate::core::GameState::WaitingForCommitments;
                game_state.update_timestamp();
            }
            should_create
        };

        // create escrow if needed (outside the lock)
        if should_create_escrow {
            self.create_escrow_for_game(game_id).await?;
        }

        {
            let games = self.games.read().await;
            if let Some(game_state) = games.get(game_id) {
                self.storage.save_game(game_state).await?;
            }
        }

        tracing::info!("Player joined game {}", game_id);
        Ok(())
    }

    /// Submit commitment
    pub async fn submit_commitment(
        &self,
        game_id: &str,
        player_id: &str,
    ) -> Result<(
        crate::commitment::Commitment,
        crate::commitment::CommitmentData,
    )> {
        let (commitment, commitment_data, should_save) = {
            let mut games = self.games.write().await;
            let game_state = games
                .get_mut(game_id)
                .ok_or_else(|| GamingError::GameNotFound {
                    game_id: game_id.to_string(),
                })?;

            if !matches!(
                game_state.state,
                crate::core::GameState::WaitingForCommitments
            ) {
                return Err(GamingError::InvalidState {
                    expected: "WaitingForCommitments".to_string(),
                    found: format!("{:?}", game_state.state),
                });
            }

            if !game_state.is_player(player_id) {
                return Err(GamingError::PlayerNotFound {
                    player_id: player_id.to_string(),
                });
            }

            if game_state.has_committed(player_id) {
                return Err(GamingError::game("Player has already committed"));
            }

            // generate random value for commitment
            let random_value = crate::commitment::CommitmentScheme::generate_random_value();

            // create commitment
            let (commitment, commitment_data) = crate::commitment::CommitmentScheme::commit(
                random_value,
                player_id,
                game_id,
                crate::commitment::CommitmentType::RandomValue,
            )?;

            // verify this commitment is different from any existing ones (prevent replay attacks)
            for existing_commitment in game_state.commitments.values() {
                if !crate::commitment::CommitmentScheme::verify_different_commitments(
                    &commitment,
                    existing_commitment,
                ) {
                    return Err(GamingError::CommitmentVerificationFailed {
                        reason: "Duplicate commitment detected".to_string(),
                    });
                }
            }

            // store commitment
            game_state
                .commitments
                .insert(player_id.to_string(), commitment.clone());
            game_state.update_timestamp();

            // check if all players have committed
            let should_transition = game_state.all_committed();
            if should_transition {
                game_state.state = crate::core::GameState::WaitingForReveals;
                tracing::info!(
                    "All players committed for game {}, moving to reveal phase",
                    game_id
                );
            }
            (commitment.clone(), commitment_data, true)
        };

        if should_save {
            let games = self.games.read().await;
            if let Some(game_state) = games.get(game_id) {
                self.storage.save_game(game_state).await?;
            }
        }

        tracing::info!(
            "Player {} submitted commitment for game {}",
            player_id,
            game_id
        );
        Ok((commitment, commitment_data))
    }

    /// Submit reveal
    pub async fn submit_reveal(
        &self,
        game_id: &str,
        player_id: &str,
        commitment_data: crate::commitment::CommitmentData,
    ) -> Result<()> {
        let (reveal, should_finalize) = {
            let games = self.games.read().await;
            let game_state = games.get(game_id)
                .ok_or_else(|| GamingError::GameNotFound {
                    game_id: game_id.to_string(),
                })?;

            if !matches!(game_state.state, crate::core::GameState::WaitingForReveals) {
                return Err(GamingError::InvalidState {
                    expected: "WaitingForReveals".to_string(),
                    found: format!("{:?}", game_state.state),
                });
            }

            if !game_state.is_player(player_id) {
                return Err(GamingError::PlayerNotFound {
                    player_id: player_id.to_string(),
                });
            }

            if game_state.has_revealed(player_id) {
                return Err(GamingError::game("Player has already revealed"));
            }

            let commitment = game_state.commitments.get(player_id)
                .ok_or_else(|| GamingError::game("No commitment found for player"))?
                .clone();

            let reveal = crate::commitment::Reveal {
                commitment_data,
                timestamp: chrono::Utc::now(),
                proof_of_work: None,
            };

            if !crate::commitment::CommitmentScheme::verify(&commitment, &reveal)? {
                return Err(GamingError::CommitmentVerificationFailed {
                    reason: "Reveal does not match commitment".to_string(),
                });
            }

            let would_complete = game_state.reveals.len() + 1 == game_state.players.len();
            
            (reveal, would_complete)
        };

        if should_finalize {
            let vtxos = self.lottery_vtxos.read().await;
            if !vtxos.contains_key(game_id) {
                return Err(GamingError::escrow("Lottery VTXO not found for game"));
            }
        }

        {
            let mut games = self.games.write().await;
            let game_state = games.get_mut(game_id)
                .ok_or_else(|| GamingError::GameNotFound {
                    game_id: game_id.to_string(),
                })?;

            game_state.reveals.insert(player_id.to_string(), reveal);
            game_state.update_timestamp();
        }

        {
            let games = self.games.read().await;
            if let Some(game_state) = games.get(game_id) {
                self.storage.save_game(game_state).await?;
            }
        }

        if should_finalize {
            self.finalize_game(game_id).await?;
        }

        tracing::info!("Player {} submitted reveal for game {}", player_id, game_id);
        Ok(())
    }

    /// Get game state
    pub async fn get_game_state(
        &self,
        game_id: &str,
    ) -> Result<crate::core::TwoPlayerLotteryState> {
        let games = self.games.read().await;
        let game_state = games
            .get(game_id)
            .ok_or_else(|| GamingError::GameNotFound {
                game_id: game_id.to_string(),
            })?;

        Ok(game_state.clone())
    }

    /// Get games for a specific player
    pub async fn get_player_games(
        &self,
        player_wallet: &str,
    ) -> Result<Vec<crate::core::TwoPlayerLotteryState>> {
        let games = self.games.read().await;
        let player_games: Vec<_> = games
            .values()
            .filter(|game| game.players.iter().any(|p| p.wallet_id == player_wallet))
            .cloned()
            .collect();

        Ok(player_games)
    }

    /// Finalize the game and determine winner
    async fn finalize_game(&self, game_id: &str) -> Result<()> {
        // get data we need to determine winner
        let (winner_id, winner_address, pot_amount) = {
            let mut games = self.games.write().await;
            let game_state = games
                .get_mut(game_id)
                .ok_or_else(|| GamingError::GameNotFound {
                    game_id: game_id.to_string(),
                })?;

            if !game_state.all_revealed() {
                return Err(GamingError::game("Not all players have revealed"));
            }

            // get reveals
            let reveals: Vec<&crate::commitment::Reveal> = game_state.reveals.values().collect();
            if reveals.len() != 2 {
                return Err(GamingError::game("Expected exactly 2 reveals"));
            }

            // determine winner
            let winner_id =
                crate::commitment::CommitmentScheme::determine_winner(reveals[0], reveals[1])?;

            // get winner info before we modify game_state
            let winner_address = game_state
                .get_player(&winner_id)
                .ok_or_else(|| GamingError::PlayerNotFound {
                    player_id: winner_id.clone(),
                })?
                .ark_address
                .to_string();

            let pot_amount = game_state.pot_amount;

            // create game result
            let result = crate::core::GameResult {
                winner: Some(winner_id.clone()),
                pot_amount,
                transaction_id: None, // Will be set after payout
                finished_at: chrono::Utc::now(),
                reason: crate::core::GameEndReason::NormalCompletion,
            };

            game_state.result = Some(result);
            game_state.state = crate::core::GameState::Finished;
            game_state.update_timestamp();

            (winner_id, winner_address, pot_amount)
        };

        // execute payout (outside the lock)
        self.execute_payout(game_id, &winner_id, &winner_address, pot_amount)
            .await?;

        {
            let games = self.games.read().await;
            if let Some(game_state) = games.get(game_id) {
                self.storage.save_game(game_state).await?;
            }
        }

        tracing::info!(
            "Game {} finished, winner: {}, pot: {} sats",
            game_id,
            winner_id,
            pot_amount.to_sat()
        );

        Ok(())
    }
}

enum TimeoutAction {
    None,
    WinnerPayout {
        winner_id: String,
        winner_address: String,
        pot_amount: Amount,
    },
    MutualRefund {
        player1_address: String,
        player2_address: String,
        refund_amount: Amount,
    },
}
