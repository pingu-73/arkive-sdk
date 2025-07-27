#![allow(unused_variables)]
use crate::commitment::{Commitment, CommitmentData, CommitmentScheme, Reveal};
use crate::contracts::manager::ContractManager;
use crate::core::{GameState, Player, TwoPlayerLotteryState, GameResult, GameEndReason};
use crate::error::{GamingError, Result};
use crate::escrow::manager::{EscrowManager, LotteryEscrow, PayoutRevealData};
use crate::storage::GameStorage;
use arkive_core::WalletManager;
use bitcoin::Amount;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[allow(dead_code)]
pub struct TwoPlayerLottery {
    wallet_manager: Arc<WalletManager>,
    contract_manager: Arc<ContractManager>,
    escrow_manager: Arc<EscrowManager>,
    games: Arc<RwLock<HashMap<String, TwoPlayerLotteryState>>>,
    escrows: Arc<RwLock<HashMap<String, LotteryEscrow>>>,
    storage: GameStorage,
}

impl TwoPlayerLottery {
    pub fn new(
        wallet_manager: Arc<WalletManager>,
        contract_manager: Arc<ContractManager>,
        escrow_manager: Arc<EscrowManager>,
    ) -> Self {
        let storage = GameStorage::new("games.json");

        Self {
            wallet_manager,
            contract_manager,
            escrow_manager,
            games: Arc::new(RwLock::new(HashMap::new())),
            escrows: Arc::new(RwLock::new(HashMap::new())),
            storage,
        }
    }

    pub async fn load_existing_games(&self) -> Result<()> {
        let stored_games = self.storage.load_all_games().await?;
        let mut games = self.games.write().await;
        *games = stored_games;

        let stored_escrows = self.storage.load_all_escrows(&self.contract_manager).await?;
        let mut escrows = self.escrows.write().await;
        *escrows = stored_escrows;

        Ok(())
    }

    /// Create a new two-player lottery game
    pub async fn create_game(
        &self,
        game_id: String,
        bet_amount: Amount,
        creator_wallet: &str,
        creator_ark_address: ark_core::ArkAddress,
    ) -> Result<String> {
        if bet_amount.to_sat() == 0 {
            return Err(GamingError::InvalidBetAmount {
                amount: bet_amount.to_sat(),
            });
        }

        // to get keypair info
        let _wallet = self.wallet_manager.load_wallet(creator_wallet).await
            .map_err(|e| GamingError::player(format!("Failed to load creator wallet: {}", e)))?;

        // create player
        let creator_player = Player::new(
            "player1".to_string(),
            creator_wallet.to_string(),
            creator_ark_address,
            &bitcoin::key::Keypair::new(&bitcoin::secp256k1::Secp256k1::new(), &mut rand::thread_rng()), // Placeholder
        );

        // create game state
        let mut game_state = TwoPlayerLotteryState::new(game_id.clone(), bet_amount);
        game_state.players.push(creator_player);
        game_state.update_timestamp();

        // store game
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
        joiner_ark_address: ark_core::ArkAddress,
    ) -> Result<()> {
        // Load from storage if not in memory
        {
            let games = self.games.read().await;
            if !games.contains_key(game_id) {
                drop(games);
                
                if let Some(game_state) = self.storage.load_game(game_id).await? {
                    let mut games = self.games.write().await;
                    games.insert(game_id.to_string(), game_state);
                } else {
                    return Err(GamingError::GameNotFound { game_id: game_id.to_string() });
                }
            }
        }

        let can_join = {
            let games = self.games.read().await;
            let game_state = games.get(game_id)
                .ok_or_else(|| GamingError::GameNotFound { game_id: game_id.to_string() })?;
            game_state.can_join()
        };

        if !can_join {
            return Err(GamingError::GameAlreadyStarted);
        }

        // Load joiner wallet
        let _wallet = self.wallet_manager.load_wallet(joiner_wallet).await
            .map_err(|e| GamingError::player(format!("Failed to load joiner wallet: {}", e)))?;

        // Create player
        let joiner_player = Player::new(
            "player2".to_string(),
            joiner_wallet.to_string(),
            joiner_ark_address,
            &bitcoin::key::Keypair::new(&bitcoin::secp256k1::Secp256k1::new(), &mut rand::thread_rng()), // Placeholder
        );

        // Update game state
        let should_create_escrow = {
            let mut games = self.games.write().await;
            let game_state = games.get_mut(game_id)
                .ok_or_else(|| GamingError::GameNotFound { game_id: game_id.to_string() })?;

            game_state.players.push(joiner_player);
            game_state.update_timestamp();

            // Check if we should create escrow
            let should_create = game_state.is_ready_for_commitments();
            if should_create {
                game_state.state = GameState::WaitingForCommitments;
                game_state.update_timestamp();
            }
            should_create
        };

        // Create escrow if needed (outside the lock)
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

    /// Create escrow for a game
    async fn create_escrow_for_game(&self, game_id: &str) -> Result<()> {
        let (player1_wallet, player2_wallet, mut game_state_clone) = {
            let games = self.games.read().await;
            let game_state = games.get(game_id)
                .ok_or_else(|| GamingError::GameNotFound { game_id: game_id.to_string() })?;

            if game_state.players.len() != 2 {
                return Err(GamingError::escrow("Need exactly 2 players to create escrow"));
            }

            let player1_wallet = game_state.players[0].wallet_id.clone();
            let player2_wallet = game_state.players[1].wallet_id.clone();
            let game_state_clone = game_state.clone();

            (player1_wallet, player2_wallet, game_state_clone)
        };

        let server_wallet = "server";

        let escrow = self.escrow_manager.create_lottery_escrow(
            &mut game_state_clone,
            &player1_wallet,
            &player2_wallet,
            server_wallet,
        ).await?;

        // Store escrow
        {
            let mut escrows = self.escrows.write().await;
            escrows.insert(game_id.to_string(), escrow.clone());
        }
        self.storage.save_escrow(&escrow).await?;

        // Update game state with escrow addr
        {
            let mut games = self.games.write().await;
            if let Some(game_state) = games.get_mut(game_id) {
                game_state.escrow_address = game_state_clone.escrow_address;
                game_state.pot_amount = game_state_clone.pot_amount;
                game_state.update_timestamp();
            }
        }

        {
            let games = self.games.read().await;
            if let Some(game_state) = games.get(game_id) {
                self.storage.save_game(game_state).await?;
            }
        }

        tracing::info!("Created escrow for game {}", game_id);
        Ok(())
    }

    /// Deposit funds to escrow
    pub async fn deposit_funds(
        &self,
        game_id: &str,
        player_id: &str,
    ) -> Result<String> {
        let (player_wallet, bet_amount) = {
            let games = self.games.read().await;
            let game_state = games.get(game_id)
                .ok_or_else(|| GamingError::GameNotFound { game_id: game_id.to_string() })?;

            let player = game_state.get_player(player_id)
                .ok_or_else(|| GamingError::PlayerNotFound { player_id: player_id.to_string() })?;

            (player.wallet_id.clone(), game_state.bet_amount)
        };

        let txid = {
            let mut escrows = self.escrows.write().await;
            let escrow = escrows.get_mut(game_id)
                .ok_or_else(|| GamingError::escrow("Escrow not found for game"))?;

            self.escrow_manager.deposit_to_escrow(
                escrow,
                &player_wallet,
                player_id,
                bet_amount,
            ).await?
        };

        tracing::info!(
            "Player {} deposited {} sats to game {} in transaction {}",
            player_id,
            bet_amount.to_sat(),
            game_id,
            txid
        );

        Ok(txid)
    }

    /// Submit commitment
    pub async fn submit_commitment(
        &self,
        game_id: &str,
        player_id: &str,
    ) -> Result<(Commitment, CommitmentData)> {
        let (commitment, commitment_data, should_save) = {
            let mut games = self.games.write().await;
            let game_state = games.get_mut(game_id)
                .ok_or_else(|| GamingError::GameNotFound { game_id: game_id.to_string() })?;

            if !matches!(game_state.state, GameState::WaitingForCommitments) {
                return Err(GamingError::InvalidState {
                    expected: "WaitingForCommitments".to_string(),
                    found: format!("{:?}", game_state.state),
                });
            }

            if !game_state.is_player(player_id) {
                return Err(GamingError::PlayerNotFound { player_id: player_id.to_string() });
            }

            if game_state.has_committed(player_id) {
                return Err(GamingError::game("Player has already committed"));
            }

            // Generate random value for commitment
            let random_value = CommitmentScheme::generate_random_value();

            // Create commitment
            let (commitment, commitment_data) = CommitmentScheme::commit(
                random_value,
                player_id,
                game_id,
            )?;

            // Verify this commitment is different from any existing ones (prevent replay attacks)
            for existing_commitment in game_state.commitments.values() {
                if !CommitmentScheme::verify_different_commitments(&commitment, existing_commitment) {
                    return Err(GamingError::CommitmentVerificationFailed {
                        reason: "Duplicate commitment detected".to_string(),
                    });
                }
            }

            // Store commitment
            game_state.commitments.insert(player_id.to_string(), commitment.clone());
            game_state.update_timestamp();

            // Check if all players have committed
            let should_transition = game_state.all_committed();
            if should_transition {
                game_state.state = GameState::WaitingForReveals;
                tracing::info!("All players committed for game {}, moving to reveal phase", game_id);
            }
            (commitment.clone(), commitment_data, true)
        };
        if should_save {
            let games = self.games.read().await;
            if let Some(game_state) = games.get(game_id) {
                self.storage.save_game(game_state).await?;
            }
        }

        tracing::info!("Player {} submitted commitment for game {}", player_id, game_id);
        Ok((commitment, commitment_data))
    }

    /// Submit reveal
    pub async fn submit_reveal(
        &self,
        game_id: &str,
        player_id: &str,
        commitment_data: CommitmentData,
    ) -> Result<()> {
        let should_finalize = {
            let mut games = self.games.write().await;
            let game_state = games.get_mut(game_id)
                .ok_or_else(|| GamingError::GameNotFound { game_id: game_id.to_string() })?;

            if !matches!(game_state.state, GameState::WaitingForReveals) {
                return Err(GamingError::InvalidState {
                    expected: "WaitingForReveals".to_string(),
                    found: format!("{:?}", game_state.state),
                });
            }

            if !game_state.is_player(player_id) {
                return Err(GamingError::PlayerNotFound { player_id: player_id.to_string() });
            }

            if game_state.has_revealed(player_id) {
                return Err(GamingError::game("Player has already revealed"));
            }

            // Get the original commitment
            let commitment = game_state.commitments.get(player_id)
                .ok_or_else(|| GamingError::game("No commitment found for player"))?
                .clone();

            // Create reveal
            let reveal = Reveal {
                commitment_data,
                timestamp: chrono::Utc::now(),
            };

            // Verify the reveal
            if !CommitmentScheme::verify(&commitment, &reveal)? {
                return Err(GamingError::CommitmentVerificationFailed {
                    reason: "Reveal does not match commitment".to_string(),
                });
            }

            // Store reveal
            game_state.reveals.insert(player_id.to_string(), reveal);
            game_state.update_timestamp();

            // Check if all players have revealed
            game_state.all_revealed()
        };

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

    /// Finalize the game and determine winner
    async fn finalize_game(&self, game_id: &str) -> Result<()> {
        // Get the data we need to determine winner
        let (winner_id, winner_address, pot_amount) = {
            let mut games = self.games.write().await;
            let game_state = games.get_mut(game_id)
                .ok_or_else(|| GamingError::GameNotFound { game_id: game_id.to_string() })?;

            if !game_state.all_revealed() {
                return Err(GamingError::game("Not all players have revealed"));
            }

            // Get reveals
            let reveals: Vec<&Reveal> = game_state.reveals.values().collect();
            if reveals.len() != 2 {
                return Err(GamingError::game("Expected exactly 2 reveals"));
            }

            // Determine winner
            let winner_id = CommitmentScheme::determine_winner(reveals[0], reveals[1])?;
            
            // Get winner info before we modify game_state
            let winner_address = game_state.get_player(&winner_id)
                .ok_or_else(|| GamingError::PlayerNotFound { player_id: winner_id.clone() })?
                .ark_address.to_string();

            let pot_amount = game_state.pot_amount;

            // Create game result
            let result = GameResult {
                winner: Some(winner_id.clone()),
                pot_amount,
                transaction_id: None, // Will be set after payout
                finished_at: chrono::Utc::now(),
                reason: GameEndReason::NormalCompletion,
            };

            game_state.result = Some(result);
            game_state.state = GameState::Finished;
            game_state.update_timestamp();

            (winner_id, winner_address, pot_amount)
        };

        // Execute payout (outside the lock)
        self.execute_payout(game_id, &winner_id, &winner_address, pot_amount).await?;

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

    /// Execute payout to winner
    async fn execute_payout(
        &self,
        game_id: &str,
        winner_id: &str,
        winner_address: &str,
        amount: Amount,
    ) -> Result<()> {
        // Get reveal data for both players
        let payout_data = {
            let games = self.games.read().await;
            let game_state = games.get(game_id)
                .ok_or_else(|| GamingError::GameNotFound { game_id: game_id.to_string() })?;

            let reveals: Vec<&Reveal> = game_state.reveals.values().collect();
            if reveals.len() != 2 {
                return Err(GamingError::game("Expected exactly 2 reveals"));
            }

            let reveal1 = reveals[0];
            let reveal2 = reveals[1];

            PayoutRevealData {
                winner: winner_id.to_string(),
                value1: reveal1.commitment_data.value,
                nonce1: reveal1.commitment_data.nonce,
                value2: reveal2.commitment_data.value,
                nonce2: reveal2.commitment_data.nonce,
            }
        };

        let txid = {
            let escrows = self.escrows.read().await;
            let escrow = escrows.get(game_id)
                .ok_or_else(|| GamingError::escrow("Escrow not found for game"))?;

            self.escrow_manager.execute_payout(
                escrow,
                winner_address,
                amount,
                payout_data,
            ).await?
        };

        // Update game result with txID
        {
            let mut games = self.games.write().await;
            if let Some(game_state) = games.get_mut(game_id) {
                if let Some(ref mut result) = game_state.result {
                    result.transaction_id = Some(txid.clone());
                }
                game_state.update_timestamp();
            }
        }

        {
            let games = self.games.read().await;
            if let Some(game_state) = games.get(game_id) {
                self.storage.save_game(game_state).await?;
            }
        }
        
        tracing::info!(
            "Executed payout of {} sats to {} in transaction {}",
            amount.to_sat(),
            winner_address,
            txid
        );

        Ok(())
    }

    /// Handle game timeout
    pub async fn handle_timeout(&self, game_id: &str) -> Result<()> {
        // Determine timeout action
        let timeout_action = {
            let mut games = self.games.write().await;
            let game_state = games.get_mut(game_id)
                .ok_or_else(|| GamingError::GameNotFound { game_id: game_id.to_string() })?;

            if game_state.is_finished() {
                return Err(GamingError::game("Game is already finished"));
            }

            let now = chrono::Utc::now();
            let mut timeout_action = TimeoutAction::None;

            match game_state.state {
                GameState::WaitingForCommitments => {
                    if now > game_state.timeouts.commitment_timeout {
                        // Find who committed and who didn't
                        let committed_players: Vec<_> = game_state.players
                            .iter()
                            .filter(|p| game_state.has_committed(&p.id))
                            .collect();

                        if committed_players.len() == 1 {
                            let winner_id = committed_players[0].id.clone();
                            let winner_address = committed_players[0].ark_address.to_string();
                            let pot_amount = game_state.pot_amount;

                            let result = GameResult {
                                winner: Some(winner_id.clone()),
                                pot_amount,
                                transaction_id: None,
                                finished_at: chrono::Utc::now(),
                                reason: GameEndReason::PlayerTimeout,
                            };

                            game_state.result = Some(result);
                            game_state.state = GameState::Finished;
                            game_state.update_timestamp();

                            timeout_action = TimeoutAction::WinnerPayout { winner_id, winner_address, pot_amount };
                        } else {
                            // No one committed or both committed but timeout reached
                            let result = GameResult {
                                winner: None,
                                pot_amount: Amount::ZERO,
                                transaction_id: None,
                                finished_at: chrono::Utc::now(),
                                reason: GameEndReason::PlayerAbort,
                            };

                            game_state.result = Some(result);
                            game_state.state = GameState::Aborted;
                            game_state.update_timestamp();

                            let player1_address = game_state.players[0].ark_address.to_string();
                            let player2_address = game_state.players[1].ark_address.to_string();
                            let refund_amount = game_state.bet_amount;

                            timeout_action = TimeoutAction::MutualRefund { 
                                player1_address, 
                                player2_address, 
                                refund_amount 
                            };
                        }
                    }
                }
                GameState::WaitingForReveals => {
                    if now > game_state.timeouts.reveal_timeout {
                        // Find who revealed and who didn't
                        let revealed_players: Vec<_> = game_state.players
                            .iter()
                            .filter(|p| game_state.has_revealed(&p.id))
                            .collect();

                        if revealed_players.len() == 1 {
                            let winner_id = revealed_players[0].id.clone();
                            let winner_address = revealed_players[0].ark_address.to_string();
                            let pot_amount = game_state.pot_amount;

                            let result = GameResult {
                                winner: Some(winner_id.clone()),
                                pot_amount,
                                transaction_id: None,
                                finished_at: chrono::Utc::now(),
                                reason: GameEndReason::PlayerTimeout,
                            };

                            game_state.result = Some(result);
                            game_state.state = GameState::Finished;
                            game_state.update_timestamp();

                            timeout_action = TimeoutAction::WinnerPayout { winner_id, winner_address, pot_amount };
                        } else {
                            // No one revealed or both revealed but timeout reached
                            let result = GameResult {
                                winner: None,
                                pot_amount: Amount::ZERO,
                                transaction_id: None,
                                finished_at: chrono::Utc::now(),
                                reason: GameEndReason::PlayerAbort,
                            };

                            game_state.result = Some(result);
                            game_state.state = GameState::Aborted;
                            game_state.update_timestamp();

                            let player1_address = game_state.players[0].ark_address.to_string();
                            let player2_address = game_state.players[1].ark_address.to_string();
                            let refund_amount = game_state.bet_amount;

                            timeout_action = TimeoutAction::MutualRefund { 
                                player1_address, 
                                player2_address, 
                                refund_amount 
                            };
                        }
                    }
                }
                _ => {
                    if now > game_state.timeouts.game_timeout {
                        let result = GameResult {
                            winner: None,
                            pot_amount: Amount::ZERO,
                            transaction_id: None,
                            finished_at: chrono::Utc::now(),
                            reason: GameEndReason::PlayerAbort,
                        };

                        game_state.result = Some(result);
                        game_state.state = GameState::Aborted;
                        game_state.update_timestamp();

                        let player1_address = game_state.players[0].ark_address.to_string();
                        let player2_address = game_state.players[1].ark_address.to_string();
                        let refund_amount = game_state.bet_amount;

                        timeout_action = TimeoutAction::MutualRefund { 
                            player1_address, 
                            player2_address, 
                            refund_amount 
                        };
                    }
                }
            }

            timeout_action
        };

        // Execute timeout action (outside the lock)
        match timeout_action {
            TimeoutAction::WinnerPayout { winner_id, winner_address, pot_amount } => {
                let escrows = self.escrows.read().await;
                let escrow = escrows.get(game_id)
                    .ok_or_else(|| GamingError::escrow("Escrow not found for game"))?;

                let txid = self.escrow_manager.handle_timeout(escrow, &winner_id).await?;

                // Update result with txID
                {
                    let mut games = self.games.write().await;
                    if let Some(game_state) = games.get_mut(game_id) {
                        if let Some(ref mut result) = game_state.result {
                            result.transaction_id = Some(txid);
                        }
                    }
                }

                tracing::info!("Game {} ended by timeout, winner: {}", game_id, winner_id);
            }
            TimeoutAction::MutualRefund { player1_address, player2_address, refund_amount } => {
                let escrows = self.escrows.read().await;
                let escrow = escrows.get(game_id)
                    .ok_or_else(|| GamingError::escrow("Escrow not found for game"))?;

                let txid = self.escrow_manager.handle_mutual_abort(
                    escrow,
                    &player1_address,
                    &player2_address,
                    refund_amount,
                ).await?;

                // Update result with txID
                {
                    let mut games = self.games.write().await;
                    if let Some(game_state) = games.get_mut(game_id) {
                        if let Some(ref mut result) = game_state.result {
                            result.transaction_id = Some(txid);
                        }
                    }
                }

                tracing::info!("Game {} aborted due to timeout, refunds issued", game_id);
            }
            TimeoutAction::None => {
                // No timeout action needed
            }
        }

        Ok(())
    }

    /// Get game state
    pub async fn get_game_state(&self, game_id: &str) -> Result<TwoPlayerLotteryState> {
        let games = self.games.read().await;
        let game_state = games.get(game_id)
            .ok_or_else(|| GamingError::GameNotFound { game_id: game_id.to_string() })?;

        Ok(game_state.clone())
    }

    /// List all games
    pub async fn list_games(&self) -> Result<Vec<TwoPlayerLotteryState>> {
        let games = self.games.read().await;
        Ok(games.values().cloned().collect())
    }

    /// Get games for a specific player
    pub async fn get_player_games(&self, player_wallet: &str) -> Result<Vec<TwoPlayerLotteryState>> {
        let games = self.games.read().await;
        let player_games: Vec<_> = games.values()
            .filter(|game| game.players.iter().any(|p| p.wallet_id == player_wallet))
            .cloned()
            .collect();

        Ok(player_games)
    }

    /// Cleanup expired games
    pub async fn cleanup_expired_games(&self) -> Result<usize> {
        let mut games = self.games.write().await;
        let mut escrows = self.escrows.write().await;
        
        let now = chrono::Utc::now();
        let mut expired_games = Vec::new();

        for (game_id, game_state) in games.iter() {
            if !game_state.is_finished() && now > game_state.timeouts.game_timeout {
                expired_games.push(game_id.clone());
            }
        }

        for game_id in &expired_games {
            // Handle timeout for each expired game
            if let Some(mut game_state) = games.remove(game_id) {
                game_state.state = GameState::Aborted;
                game_state.result = Some(GameResult {
                    winner: None,
                    pot_amount: Amount::ZERO,
                    transaction_id: None,
                    finished_at: now,
                    reason: GameEndReason::PlayerTimeout,
                });
                games.insert(game_id.clone(), game_state);
            }

            // Remove associated escrow
            escrows.remove(game_id);
        }

        let count = expired_games.len();
        tracing::info!("Cleaned up {} expired games", count);
        Ok(count)
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
