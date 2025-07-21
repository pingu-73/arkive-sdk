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
    BetsCollected,
    CommitmentPhase,
    RevealPhase,
    Completed { winner: Uuid },
    Aborted { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BetInfo {
    pub player_id: Uuid,
    pub amount: Amount,
    pub txid: String,
    pub timestamp: DateTime<Utc>,
}

pub struct TwoPlayerGame {
    id: Uuid,
    bet_amount: Amount,
    state: GameState,
    players: HashMap<Uuid, Player>,
    pot_wallet: Arc<ArkWallet>, // wallet for the pot
    collected_bets: HashMap<Uuid, BetInfo>,
    commitment_deadline: Option<DateTime<Utc>>,
    reveal_deadline: Option<DateTime<Utc>>,
    total_pot: Amount,
}

impl TwoPlayerGame {
    pub async fn new(bet_amount: Amount, pot_wallet: Arc<ArkWallet>) -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4(),
            bet_amount,
            state: GameState::WaitingForPlayers,
            players: HashMap::new(),
            pot_wallet,
            collected_bets: HashMap::new(),
            commitment_deadline: None,
            reveal_deadline: None,
            total_pot: Amount::ZERO,
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

    pub async fn get_pot_address(&self) -> Result<String> {
        let ark_addr = self.pot_wallet.get_ark_address().await?;
        Ok(ark_addr.address)
    }

    pub fn total_pot(&self) -> Amount {
        self.total_pot
    }

    pub fn players(&self) -> Vec<Uuid> {
        self.players.keys().cloned().collect()
    }

    pub fn get_bet_info(&self, player_id: Uuid) -> Option<&BetInfo> {
        self.collected_bets.get(&player_id)
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

    /// Player places their bet
    pub async fn place_bet(&mut self, player_id: Uuid) -> Result<String> {
        if !matches!(self.state, GameState::WaitingForBets) {
            return Err(LotteryError::InvalidState(
                "Not in betting phase".to_string(),
            ));
        }

        // Check if player already placed bet
        if self.collected_bets.contains_key(&player_id) {
            return Err(LotteryError::InvalidState(
                "Player already placed bet".to_string(),
            ));
        }

        let player = self
            .players
            .get(&player_id)
            .ok_or(LotteryError::PlayerNotFound(player_id))?;

        // Check player has sufficient balance
        let balance = player.wallet().balance().await?;

        if balance.confirmed < self.bet_amount {
            return Err(LotteryError::Internal(format!(
                "Insufficient balance: need {} sats, have {} sats",
                self.bet_amount.to_sat(),
                balance.confirmed.to_sat()
            )));
        }

        // Get pot address
        let pot_address = self.get_pot_address().await?;

        // Send bet to pot
        let txid = player.place_bet(&pot_address, self.bet_amount).await?;

        // Record the bet
        let bet_info = BetInfo {
            player_id,
            amount: self.bet_amount,
            txid: txid.clone(),
            timestamp: Utc::now(),
        };

        self.collected_bets.insert(player_id, bet_info);
        self.total_pot += self.bet_amount;

        tracing::info!(
            "Player {} placed bet of {} sats in game {}: {}",
            player_id,
            self.bet_amount.to_sat(),
            self.id,
            txid
        );

        // Check if both players have bet
        if self.collected_bets.len() == 2 {
            self.state = GameState::BetsCollected;
            tracing::info!("All bets collected for game {}", self.id);
        }

        Ok(txid)
    }

    /// Start the commitment phase after both players have placed bets
    pub async fn start_commitment_phase(&mut self) -> Result<()> {
        if self.players.len() != 2 {
            return Err(LotteryError::GameNotReady);
        }

        if !matches!(self.state, GameState::BetsCollected) {
            return Err(LotteryError::InvalidState(
                "Bets not collected yet".to_string(),
            ));
        }

        // Verify pot wallet has received the bets
        let pot_balance = self.pot_wallet.balance().await?;
        let expected_pot = self.bet_amount * 2u64;

        if pot_balance.confirmed < expected_pot {
            tracing::warn!(
                "Pot balance {} less than expected {}. Waiting for confirmations...",
                pot_balance.confirmed.to_sat(),
                expected_pot.to_sat()
            );
            // Could wait for confirmations or proceed with pending balance
        }

        // Set commitment deadline (5 minutes from now)
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
        // Set reveal deadline (5 minutes from now)
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

    /// Determine winner using XOR of revealed secrets
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

        // Payout the winner
        self.payout_winner(winner_id).await?;

        Ok(())
    }

    /// Payout the winner with actual Ark transaction
    async fn payout_winner(&self, winner_id: Uuid) -> Result<()> {
        let winner = self
            .players
            .get(&winner_id)
            .ok_or(LotteryError::PlayerNotFound(winner_id))?;

        // Get winner's Ark address
        let winner_address = winner.wallet().get_ark_address().await?;

        // Send entire pot to winner
        let payout_amount = self.total_pot;

        tracing::info!(
            "Paying out {} sats to winner {} at address {}",
            payout_amount.to_sat(),
            winner_id,
            winner_address.address
        );

        // Send from pot wallet to winner
        let txid = self
            .pot_wallet
            .send_ark(&winner_address.address, payout_amount)
            .await?;

        tracing::info!(
            "Game {} payout completed. Winner {} received {} sats: {}",
            self.id,
            winner_id,
            payout_amount.to_sat(),
            txid
        );

        Ok(())
    }

    /// Abort the game and refund bets
    async fn abort_game(&mut self, reason: String) -> Result<()> {
        self.state = GameState::Aborted {
            reason: reason.clone(),
        };

        tracing::warn!("Game {} aborted: {}", self.id, reason);

        // Refund bets to players
        self.refund_bets().await?;

        Ok(())
    }

    /// Refund bets to all players
    async fn refund_bets(&self) -> Result<()> {
        for (player_id, bet_info) in &self.collected_bets {
            let player = self
                .players
                .get(player_id)
                .ok_or(LotteryError::PlayerNotFound(*player_id))?;

            let player_address = player.wallet().get_ark_address().await?;

            tracing::info!(
                "Refunding {} sats to player {} at address {}",
                bet_info.amount.to_sat(),
                player_id,
                player_address.address
            );

            let txid = self
                .pot_wallet
                .send_ark(&player_address.address, bet_info.amount)
                .await?;

            tracing::info!(
                "Refunded {} sats to player {}: {}",
                bet_info.amount.to_sat(),
                player_id,
                txid
            );
        }

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
            total_pot: self.total_pot,
            commitment_deadline: self.commitment_deadline,
            reveal_deadline: self.reveal_deadline,
            collected_bets: self.collected_bets.clone(),
        }
    }

    pub fn get_player(&self, player_id: Uuid) -> Option<&Player> {
        self.players.get(&player_id)
    }

    pub fn can_place_bet(&self, player_id: Uuid) -> bool {
        matches!(self.state, GameState::WaitingForBets)
            && self.players.contains_key(&player_id)
            && !self.collected_bets.contains_key(&player_id)
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
    pub total_pot: Amount,
    pub commitment_deadline: Option<DateTime<Utc>>,
    pub reveal_deadline: Option<DateTime<Utc>>,
    pub collected_bets: HashMap<Uuid, BetInfo>,
}
