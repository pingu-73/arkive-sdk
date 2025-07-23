use crate::{
    commitment::{determine_winner, generate_secret, Commitment},
    core::{Game, GameAction, GameResult, GameState, Player, PlayerId, PlayerState, StateManager},
    escrow::{EscrowConditions, EscrowManager},
    GamingError, Result,
};
use arkive_core::{Amount, ArkWallet};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// 2-Player Lottery Game impl
#[derive(Debug)]
pub struct TwoPlayerLottery {
    id: Uuid,
    bet_amount: Amount,
    state_manager: StateManager,
    players: HashMap<PlayerId, LotteryPlayer>,
    escrow_manager: EscrowManager,
    escrow_id: Option<crate::escrow::EscrowId>,
    game_result: Option<GameResult>,
}

/// Lottery-specific player data
#[derive(Debug, Clone)]
pub struct LotteryPlayer {
    core_player: Player,
    commitment: Option<Commitment>,
    revealed_secret: Option<Vec<u8>>,
    secret: Option<Vec<u8>>, // Store secret for reveal phase
}

// impl std::fmt::Debug for LotteryPlayer {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.debug_struct("LotteryPlayer")
//             .field("core_player", &self.core_player)
//             .field("has_commitment", &self.commitment.is_some())
//             .field("has_revealed", &self.revealed_secret.is_some())
//             .field("has_secret", &self.secret.is_some())
//             .finish()
//     }
// }

impl TwoPlayerLottery {
    pub async fn new(bet_amount: Amount, escrow_wallet: Arc<ArkWallet>) -> Result<Self> {
        let game_id = Uuid::new_v4();

        Ok(Self {
            id: game_id,
            bet_amount,
            state_manager: StateManager::new(),
            players: HashMap::new(),
            escrow_manager: EscrowManager::new(escrow_wallet),
            escrow_id: None,
            game_result: None,
        })
    }

    pub fn bet_amount(&self) -> Amount {
        self.bet_amount
    }

    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    pub async fn get_escrow_address(&self) -> Result<String> {
        self.escrow_manager.get_escrow_address().await
    }

    pub fn get_player(&self, player_id: PlayerId) -> Option<&LotteryPlayer> {
        self.players.get(&player_id)
    }

    pub fn can_place_bet(&self, player_id: PlayerId) -> bool {
        matches!(
            self.state_manager.current_state(),
            GameState::WaitingForBets
        ) && self.players.contains_key(&player_id)
            && matches!(
                self.players.get(&player_id).unwrap().core_player.state(),
                PlayerState::Joined
            )
    }

    pub fn can_commit(&self, player_id: PlayerId) -> bool {
        matches!(self.state_manager.current_state(), GameState::InProgress)
            && self
                .players
                .get(&player_id)
                .is_some_and(|p| p.commitment.is_none())
    }

    pub fn can_reveal(&self, player_id: PlayerId) -> bool {
        matches!(self.state_manager.current_state(), GameState::InProgress)
            && self
                .players
                .get(&player_id)
                .is_some_and(|p| p.commitment.is_some() && p.revealed_secret.is_none())
    }

    /// Place bet for a player
    async fn place_bet_internal(&mut self, player_id: PlayerId) -> Result<String> {
        // check player balance and get escrow addr
        let (balance, escrow_address) = {
            let player = self
                .players
                .get(&player_id)
                .ok_or(GamingError::PlayerNotFound(player_id))?;

            let balance = player.core_player.get_balance().await?;
            let escrow_address = self.get_escrow_address().await?;
            (balance, escrow_address)
        };

        if balance.confirmed < self.bet_amount {
            return Err(GamingError::InsufficientBalance {
                need: self.bet_amount.to_sat(),
                available: balance.confirmed.to_sat(),
            });
        }

        // get mutable ref to player and place bet
        let player = self
            .players
            .get_mut(&player_id)
            .ok_or(GamingError::PlayerNotFound(player_id))?;

        // Place bet
        let txid = player
            .core_player
            .place_bet(&escrow_address, self.bet_amount)
            .await?;

        // Update player state
        player.core_player.set_state(PlayerState::BetPlaced);

        // Record deposit in escrow
        if let Some(escrow_id) = self.escrow_id {
            self.escrow_manager
                .record_deposit(escrow_id, player_id, self.bet_amount, txid.clone())
                .await?;
        }

        // Check if both players have bet
        if self
            .players
            .values()
            .all(|p| matches!(p.core_player.state(), PlayerState::BetPlaced))
        {
            self.state_manager.transition_to(GameState::BetsCollected);
            self.start_commitment_phase().await?;
        }

        Ok(txid)
    }

    /// Start commitment phase
    async fn start_commitment_phase(&mut self) -> Result<()> {
        // Set commitment deadline (5 minutes)
        let deadline = Utc::now() + Duration::minutes(5);
        self.state_manager
            .set_timeout("commitment".to_string(), deadline);

        self.state_manager.transition_to(GameState::InProgress);

        tracing::info!("Game {} started commitment phase", self.id);
        Ok(())
    }

    /// Submit commitment
    async fn submit_commitment_internal(&mut self, player_id: PlayerId) -> Result<()> {
        let player = self
            .players
            .get_mut(&player_id)
            .ok_or(GamingError::PlayerNotFound(player_id))?;

        if player.commitment.is_some() {
            return Err(GamingError::CommitmentAlreadySubmitted);
        }

        // Generate secret and commitment
        let secret = generate_secret();
        let commitment = Commitment::create_with_secret(&secret, player_id);

        player.commitment = Some(commitment);
        player.secret = Some(secret);
        player.core_player.set_state(PlayerState::Committed);

        tracing::info!("Player {} submitted commitment", player_id);

        // Check if both players have committed
        if self.players.values().all(|p| p.commitment.is_some()) {
            self.start_reveal_phase().await?;
        }

        Ok(())
    }

    /// Start reveal phase
    async fn start_reveal_phase(&mut self) -> Result<()> {
        // Clear commitment timeout and set reveal timeout
        self.state_manager.clear_timeout("commitment");
        let deadline = Utc::now() + Duration::minutes(5);
        self.state_manager
            .set_timeout("reveal".to_string(), deadline);

        tracing::info!("Game {} started reveal phase", self.id);
        Ok(())
    }

    /// Reveal commitment
    async fn reveal_commitment_internal(
        &mut self,
        player_id: PlayerId,
        secret: Vec<u8>,
    ) -> Result<()> {
        let player = self
            .players
            .get_mut(&player_id)
            .ok_or(GamingError::PlayerNotFound(player_id))?;

        let commitment = player
            .commitment
            .as_ref()
            .ok_or(GamingError::InvalidCommitment)?;

        // Verify secret matches commitment
        if !commitment.verify_secret(&secret)? {
            return Err(GamingError::InvalidCommitment);
        }

        player.revealed_secret = Some(secret);
        player.core_player.set_state(PlayerState::Revealed);

        tracing::info!("Player {} revealed commitment", player_id);

        // Check if both players have revealed
        if self.players.values().all(|p| p.revealed_secret.is_some()) {
            self.determine_winner().await?;
        }

        Ok(())
    }

    /// Determine winner and complete game
    async fn determine_winner(&mut self) -> Result<()> {
        let player_ids: Vec<PlayerId> = self.players.keys().cloned().collect();
        if player_ids.len() != 2 {
            return Err(GamingError::Internal("Invalid player count".to_string()));
        }

        let player1_id = player_ids[0];
        let player2_id = player_ids[1];

        let player1 = &self.players[&player1_id];
        let player2 = &self.players[&player2_id];

        let secret1 = player1
            .revealed_secret
            .as_ref()
            .ok_or(GamingError::CommitmentNotRevealed(player1_id))?;
        let secret2 = player2
            .revealed_secret
            .as_ref()
            .ok_or(GamingError::CommitmentNotRevealed(player2_id))?;

        // Determine winner using XOR
        let player1_wins = determine_winner(secret1, secret2);
        let winner_id = if player1_wins { player1_id } else { player2_id };
        let loser_id = if player1_wins { player2_id } else { player1_id };

        // Update player states
        self.players
            .get_mut(&winner_id)
            .unwrap()
            .core_player
            .set_state(PlayerState::Winner);
        self.players
            .get_mut(&loser_id)
            .unwrap()
            .core_player
            .set_state(PlayerState::Loser);

        // Create game result
        let payout_amount = self.bet_amount * 2u64;
        self.game_result = Some(GameResult {
            winner: Some(winner_id),
            payout_amount,
            game_completed_at: Utc::now(),
        });

        // Update state
        self.state_manager.transition_to(GameState::Completed {
            winner: Some(winner_id),
        });

        // Payout winner
        self.payout_winner(winner_id).await?;

        tracing::info!("Game {} completed. Winner: {}", self.id, winner_id);
        Ok(())
    }

    /// Payout winner
    async fn payout_winner(&mut self, winner_id: PlayerId) -> Result<()> {
        let winner = self
            .players
            .get(&winner_id)
            .ok_or(GamingError::PlayerNotFound(winner_id))?;

        let winner_address = winner.core_player.get_ark_address().await?;
        let payout_amount = self.bet_amount * 2u64;

        if let Some(escrow_id) = self.escrow_id {
            let txid = self
                .escrow_manager
                .release_to_winner(escrow_id, winner_id, &winner_address)
                .await?;

            tracing::info!(
                "Paid out {} sats to winner {}: {}",
                payout_amount.to_sat(),
                winner_id,
                txid
            );
        }

        Ok(())
    }

    /// Handle timeout scenarios
    async fn handle_timeout(&mut self, timeout_type: &str) -> Result<()> {
        match timeout_type {
            "commitment" => {
                // Find players who haven't committed
                let non_committed: Vec<PlayerId> = self
                    .players
                    .iter()
                    .filter(|(_, player)| player.commitment.is_none())
                    .map(|(id, _)| *id)
                    .collect();

                if non_committed.len() == 1 {
                    // One player didn't commit, other wins by default
                    let winner_id = self
                        .players
                        .iter()
                        .find(|(_, player)| player.commitment.is_some())
                        .map(|(id, _)| *id)
                        .ok_or(GamingError::Internal("No committed players".to_string()))?;

                    self.forfeit_players(&non_committed, winner_id).await?;
                } else {
                    self.abort_game("Commitment deadline expired".to_string())
                        .await?;
                }
            }
            "reveal" => {
                // Find players who haven't revealed
                let non_revealed: Vec<PlayerId> = self
                    .players
                    .iter()
                    .filter(|(_, player)| player.revealed_secret.is_none())
                    .map(|(id, _)| *id)
                    .collect();

                if non_revealed.len() == 1 {
                    // One player didn't reveal, other wins by default
                    let winner_id = self
                        .players
                        .iter()
                        .find(|(_, player)| player.revealed_secret.is_some())
                        .map(|(id, _)| *id)
                        .ok_or(GamingError::Internal("No revealed players".to_string()))?;

                    self.forfeit_players(&non_revealed, winner_id).await?;
                } else {
                    self.abort_game("Reveal deadline expired".to_string())
                        .await?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Forfeit players and award winner
    async fn forfeit_players(&mut self, forfeited: &[PlayerId], winner_id: PlayerId) -> Result<()> {
        // Update player states
        for &player_id in forfeited {
            if let Some(player) = self.players.get_mut(&player_id) {
                player.core_player.set_state(PlayerState::Forfeited);
            }
        }

        if let Some(winner) = self.players.get_mut(&winner_id) {
            winner.core_player.set_state(PlayerState::Winner);
        }

        // Create game result
        let payout_amount = self.bet_amount * 2u64;
        self.game_result = Some(GameResult {
            winner: Some(winner_id),
            payout_amount,
            game_completed_at: Utc::now(),
        });

        // Update state
        self.state_manager.transition_to(GameState::Completed {
            winner: Some(winner_id),
        });

        // Payout winner
        self.payout_winner(winner_id).await?;

        Ok(())
    }

    /// Abort game and refund
    async fn abort_game(&mut self, reason: String) -> Result<()> {
        self.state_manager.transition_to(GameState::Aborted {
            reason: reason.clone(),
        });

        // Refund through escrow
        if let Some(escrow_id) = self.escrow_id {
            self.escrow_manager.refund_escrow(escrow_id, reason).await?;
        }

        tracing::warn!("Game {} aborted", self.id);
        Ok(())
    }
}

impl LotteryPlayer {
    pub fn new(core_player: Player) -> Self {
        Self {
            core_player,
            commitment: None,
            revealed_secret: None,
            secret: None,
        }
    }
}

#[async_trait]
impl Game for TwoPlayerLottery {
    type GameState = GameState;
    type Player = Player;
    type GameResult = GameResult;

    fn id(&self) -> Uuid {
        self.id
    }

    fn state(&self) -> &Self::GameState {
        self.state_manager.current_state()
    }

    async fn add_player(&mut self, player: Self::Player) -> Result<Uuid> {
        if self.players.len() >= 2 {
            return Err(GamingError::GameFull);
        }

        if !matches!(
            self.state_manager.current_state(),
            GameState::WaitingForPlayers
        ) {
            return Err(GamingError::InvalidState(
                "Game not accepting players".to_string(),
            ));
        }

        let player_id = player.id();
        let lottery_player = LotteryPlayer::new(player);
        self.players.insert(player_id, lottery_player);

        tracing::info!("Player {} joined game {}", player_id, self.id);

        // If we have 2 players, create escrow and move to betting phase
        if self.players.len() == 2 {
            let player_ids: Vec<Uuid> = self.players.keys().cloned().collect();
            let escrow_id = self
                .escrow_manager
                .create_escrow(
                    player_ids,
                    EscrowConditions::GameCompletion { game_id: self.id },
                )
                .await?;

            self.escrow_id = Some(escrow_id);
            self.state_manager.transition_to(GameState::WaitingForBets);
            tracing::info!("Game {} ready for betting phase", self.id);
        }

        Ok(player_id)
    }

    fn can_start(&self) -> bool {
        self.players.len() == 2
            && matches!(
                self.state_manager.current_state(),
                GameState::WaitingForBets
            )
    }

    async fn start(&mut self) -> Result<()> {
        if !self.can_start() {
            return Err(GamingError::GameNotReady);
        }

        // Game starts when both players place bets
        // This is handled automatically in place_bet_internal
        Ok(())
    }

    async fn execute_action(&mut self, player_id: Uuid, action: GameAction) -> Result<()> {
        match action {
            GameAction::PlaceBet => {
                if !self.can_place_bet(player_id) {
                    return Err(GamingError::InvalidState(
                        "Cannot place bet in current state".to_string(),
                    ));
                }
                self.place_bet_internal(player_id).await?;
            }
            GameAction::SubmitCommitment { .. } => {
                if !self.can_commit(player_id) {
                    return Err(GamingError::InvalidState(
                        "Cannot commit in current state".to_string(),
                    ));
                }
                self.submit_commitment_internal(player_id).await?;
            }
            GameAction::RevealCommitment { secret } => {
                if !self.can_reveal(player_id) {
                    return Err(GamingError::InvalidState(
                        "Cannot reveal in current state".to_string(),
                    ));
                }
                self.reveal_commitment_internal(player_id, secret).await?;
            }
            GameAction::Forfeit => {
                // Handle forfeit
                let other_player_id = self
                    .players
                    .keys()
                    .find(|&&id| id != player_id)
                    .copied()
                    .ok_or(GamingError::Internal("Other player not found".to_string()))?;

                self.forfeit_players(&[player_id], other_player_id).await?;
            }
        }

        Ok(())
    }

    async fn check_timeouts(&mut self) -> Result<()> {
        let timeouts_to_check = vec!["commitment", "reveal"];

        for timeout_name in timeouts_to_check {
            if self.state_manager.check_timeout(timeout_name) {
                self.handle_timeout(timeout_name).await?;
                break; // Only handle one timeout at a time
            }
        }

        Ok(())
    }

    fn get_result(&self) -> Option<&Self::GameResult> {
        self.game_result.as_ref()
    }

    fn is_completed(&self) -> bool {
        matches!(
            self.state_manager.current_state(),
            GameState::Completed { .. } | GameState::Aborted { .. }
        )
    }
}

/// Game information for external queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LotteryGameInfo {
    pub id: Uuid,
    pub bet_amount: Amount,
    pub state: GameState,
    pub player_count: usize,
    pub escrow_address: Option<String>,
    pub commitment_deadline: Option<DateTime<Utc>>,
    pub reveal_deadline: Option<DateTime<Utc>>,
    pub result: Option<GameResult>,
}

impl TwoPlayerLottery {
    /// Get game information for display
    pub async fn get_info(&self) -> Result<LotteryGameInfo> {
        let escrow_address = if self.escrow_id.is_some() {
            Some(self.get_escrow_address().await?)
        } else {
            None
        };

        Ok(LotteryGameInfo {
            id: self.id,
            bet_amount: self.bet_amount,
            state: self.state_manager.current_state().clone(),
            player_count: self.players.len(),
            escrow_address,
            commitment_deadline: self.state_manager.get_timeout("commitment"),
            reveal_deadline: self.state_manager.get_timeout("reveal"),
            result: self.game_result.clone(),
        })
    }

    /// Get escrow audit trail
    pub fn get_audit_trail(&self) -> &crate::escrow::audit::AuditTrail {
        self.escrow_manager.get_audit_trail()
    }

    /// Check escrow balance
    pub async fn check_escrow_balance(&self) -> Result<arkive_core::Balance> {
        self.escrow_manager.check_balance().await
    }
}
