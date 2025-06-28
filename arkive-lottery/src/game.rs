use crate::{commitment::determine_winner, LotteryError, Player, Result};
use arkive_core::{Amount, ArkWallet};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Game state for 2-player lottery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameState {
    WaitingForPlayers,
    WaitingForBets,
    CommitmentPhase,
    RevealPhase,
    Completed { winner: Uuid },
    Aborted { reason: String },
}

#[derive(Debug)]
pub struct TwoPlayerGame {
    id: Uuid,
    bet_amount: Amount,
    state: GameState,
    players: HashMap<Uuid, Player>,
    commitment_deadline: Option<DateTime<Utc>>,
    reveal_deadline: Option<DateTime<Utc>>,
    pot_address: String,
}

impl TwoPlayerGame {
    pub async fn new(bet_amount: Amount) -> Result<Self> {
        let pot_address = format!("game_pot_{}", Uuid::new_v4());

        Ok(Self {
            id: Uuid::new_v4(),
            bet_amount,
            state: GameState::WaitingForPlayers,
            players: HashMap::new(),
            commitment_deadline: None,
            reveal_deadline: None,
            pot_address,
        })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn bet_amount(&self) -> Amount {
        self.bet_amount
    }

    pub fn state(&self) -> &GameState {
        &self.state
    }

    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    pub fn pot_address(&self) -> &str {
        &self.pot_address
    }

    pub fn players(&self) -> Vec<Uuid> {
        self.players.keys().cloned().collect()
    }

    /// Add a player to the game
    pub async fn add_player(&mut self, wallet: Arc<ArkWallet>) -> Result<Uuid> {
        if self.players.len() >= 2 {
            return Err(LotteryError::GameFull);
        }

        if !matches!(self.state, GameState::WaitingForPlayers) {
            return Err(LotteryError::InvalidState(
                "Game not accepting players".to_string(),
            ));
        }

        let player = Player::new(wallet).await?;
        let player_id = player.id();
        self.players.insert(player_id, player);

        tracing::info!("Player {} joined game {}", player_id, self.id);

        // If we have 2 players, move to betting phase
        if self.players.len() == 2 {
            self.state = GameState::WaitingForBets;
            tracing::info!("Game {} ready for betting phase", self.id);
        }

        Ok(player_id)
    }

    /// Start the commitment phase after both players have placed bets
    pub async fn start_commitment_phase(&mut self) -> Result<()> {
        if self.players.len() != 2 {
            return Err(LotteryError::GameNotReady);
        }

        if !matches!(self.state, GameState::WaitingForBets) {
            return Err(LotteryError::InvalidState(
                "Not ready for commitment phase".to_string(),
            ));
        }

        // commitment deadline 5 minutes from now
        self.commitment_deadline = Some(Utc::now() + Duration::minutes(5));
        self.state = GameState::CommitmentPhase;

        tracing::info!("Game {} started commitment phase", self.id);
        Ok(())
    }

    /// Submit a commitment from a player
    pub async fn submit_commitment(&mut self, player_id: Uuid) -> Result<()> {
        if !matches!(self.state, GameState::CommitmentPhase) {
            return Err(LotteryError::InvalidState(
                "Not in commitment phase".to_string(),
            ));
        }

        // Check deadline
        if let Some(deadline) = self.commitment_deadline {
            if Utc::now() > deadline {
                self.abort_game("Commitment deadline expired".to_string())
                    .await?;
                return Err(LotteryError::TimeoutExpired);
            }
        }

        let player = self
            .players
            .get_mut(&player_id)
            .ok_or(LotteryError::PlayerNotFound(player_id))?;

        let (_commitment, _secret) = player.create_commitment()?;

        // Check if both players have committed
        if self.players.values().all(|p| p.has_committed()) {
            self.start_reveal_phase().await?;
        }

        Ok(())
    }

    /// Start the reveal phase
    async fn start_reveal_phase(&mut self) -> Result<()> {
        // reveal deadline 5 minutes from now
        self.reveal_deadline = Some(Utc::now() + Duration::minutes(5));
        self.state = GameState::RevealPhase;

        tracing::info!("Game {} started reveal phase", self.id);
        Ok(())
    }

    /// Reveal a commitment
    pub async fn reveal_commitment(&mut self, player_id: Uuid, secret: Vec<u8>) -> Result<()> {
        if !matches!(self.state, GameState::RevealPhase) {
            return Err(LotteryError::InvalidState(
                "Not in reveal phase".to_string(),
            ));
        }

        if let Some(deadline) = self.reveal_deadline {
            if Utc::now() > deadline {
                self.abort_game("Reveal deadline expired".to_string())
                    .await?;
                return Err(LotteryError::TimeoutExpired);
            }
        }

        let player = self
            .players
            .get_mut(&player_id)
            .ok_or(LotteryError::PlayerNotFound(player_id))?;

        player.reveal_commitment(secret)?;

        if self.players.values().all(|p| p.has_revealed()) {
            self.determine_winner().await?;
        }

        Ok(())
    }

    /// winner is determined using XOR of revealed secrets
    async fn determine_winner(&mut self) -> Result<()> {
        let player_ids: Vec<Uuid> = self.players.keys().cloned().collect();
        if player_ids.len() != 2 {
            return Err(LotteryError::Internal("Invalid player count".to_string()));
        }

        let player1_id = player_ids[0];
        let player2_id = player_ids[1];

        let player1 = &self.players[&player1_id];
        let player2 = &self.players[&player2_id];

        let secret1 = player1
            .revealed_secret()
            .ok_or(LotteryError::CommitmentNotRevealed(player1_id))?;
        let secret2 = player2
            .revealed_secret()
            .ok_or(LotteryError::CommitmentNotRevealed(player2_id))?;

        let player1_wins = determine_winner(secret1, secret2);
        let winner_id = if player1_wins { player1_id } else { player2_id };
        let loser_id = if player1_wins { player2_id } else { player1_id };

        self.players.get_mut(&winner_id).unwrap().set_winner();
        self.players.get_mut(&loser_id).unwrap().set_loser();

        self.state = GameState::Completed { winner: winner_id };

        tracing::info!("Game {} completed. Winner: {}", self.id, winner_id);

        // [TODO] transfer the pot to the winner
        self.payout_winner(winner_id).await?;

        Ok(())
    }

    /// Payout the winner
    async fn payout_winner(&self, winner_id: Uuid) -> Result<()> {
        let total_pot = self.bet_amount * 2u64;

        tracing::info!(
            "Game {} payout: Player {} wins {} sats",
            self.id,
            winner_id,
            total_pot.to_sat()
        );

        // [TODO] Ark Tx to transfer pot to winner
        // 1. Getting winner's Ark addr
        // 2. Creating Tx from pot addr to winner
        // 3. Broadcasting the Tx

        Ok(())
    }

    /// Abort the game
    async fn abort_game(&mut self, reason: String) -> Result<()> {
        self.state = GameState::Aborted {
            reason: reason.clone(),
        };

        tracing::warn!("Game {} aborted: {}", self.id, reason);

        // [TODO] refund
        // Return bets to players if they were placed

        Ok(())
    }

    /// Check if game has expired and handle timeouts
    pub async fn check_timeouts(&mut self) -> Result<()> {
        let now = Utc::now();

        match &self.state {
            GameState::CommitmentPhase => {
                if let Some(deadline) = self.commitment_deadline {
                    if now > deadline {
                        // Find players who haven't committed and forfeit them
                        let non_committed: Vec<Uuid> = self
                            .players
                            .iter()
                            .filter(|(_, player)| !player.has_committed())
                            .map(|(id, _)| *id)
                            .collect();

                        if non_committed.len() == 1 {
                            // One player didn't commit, other wins by default
                            let winner_id = self
                                .players
                                .iter()
                                .find(|(_, player)| player.has_committed())
                                .map(|(id, _)| *id)
                                .ok_or(LotteryError::Internal(
                                    "No committed players".to_string(),
                                ))?;

                            self.players.get_mut(&winner_id).unwrap().set_winner();
                            for &loser_id in &non_committed {
                                self.players.get_mut(&loser_id).unwrap().set_loser();
                            }

                            self.state = GameState::Completed { winner: winner_id };
                            self.payout_winner(winner_id).await?;
                        } else {
                            self.abort_game("Commitment deadline expired".to_string())
                                .await?;
                        }
                    }
                }
            }
            GameState::RevealPhase => {
                if let Some(deadline) = self.reveal_deadline {
                    if now > deadline {
                        // Find players who haven't revealed and forfeit them
                        let non_revealed: Vec<Uuid> = self
                            .players
                            .iter()
                            .filter(|(_, player)| !player.has_revealed())
                            .map(|(id, _)| *id)
                            .collect();

                        if non_revealed.len() == 1 {
                            // One player didn't reveal, other wins by default
                            let winner_id = self
                                .players
                                .iter()
                                .find(|(_, player)| player.has_revealed())
                                .map(|(id, _)| *id)
                                .ok_or(LotteryError::Internal("No revealed players".to_string()))?;

                            self.players.get_mut(&winner_id).unwrap().set_winner();
                            for &loser_id in &non_revealed {
                                self.players.get_mut(&loser_id).unwrap().set_loser();
                            }

                            self.state = GameState::Completed { winner: winner_id };
                            self.payout_winner(winner_id).await?;
                        } else {
                            self.abort_game("Reveal deadline expired".to_string())
                                .await?;
                        }
                    }
                }
            }
            _ => {} // No timeouts for other states
        }

        Ok(())
    }

    pub fn get_info(&self) -> GameInfo {
        GameInfo {
            id: self.id,
            bet_amount: self.bet_amount,
            state: self.state.clone(),
            player_count: self.players.len(),
            pot_address: self.pot_address.clone(),
            commitment_deadline: self.commitment_deadline,
            reveal_deadline: self.reveal_deadline,
        }
    }

    pub fn get_player(&self, player_id: Uuid) -> Option<&Player> {
        self.players.get(&player_id)
    }

    pub fn can_place_bet(&self, player_id: Uuid) -> bool {
        matches!(self.state, GameState::WaitingForBets) && self.players.contains_key(&player_id)
    }

    pub fn can_commit(&self, player_id: Uuid) -> bool {
        matches!(self.state, GameState::CommitmentPhase)
            && self
                .players
                .get(&player_id)
                .map_or(false, |p| !p.has_committed())
    }

    pub fn can_reveal(&self, player_id: Uuid) -> bool {
        matches!(self.state, GameState::RevealPhase)
            && self
                .players
                .get(&player_id)
                .map_or(false, |p| p.has_committed() && !p.has_revealed())
    }
}

/// Game info for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    pub id: Uuid,
    pub bet_amount: Amount,
    pub state: GameState,
    pub player_count: usize,
    pub pot_address: String,
    pub commitment_deadline: Option<DateTime<Utc>>,
    pub reveal_deadline: Option<DateTime<Utc>>,
}
