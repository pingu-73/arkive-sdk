use crate::commitment::{Commitment, Reveal};
use crate::core::Player;
use bitcoin::Amount;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameState {
    WaitingForPlayers,
    WaitingForCommitments,
    WaitingForReveals,
    Finished,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamePhaseTimeouts {
    pub commitment_timeout: chrono::DateTime<chrono::Utc>,
    pub reveal_timeout: chrono::DateTime<chrono::Utc>,
    pub game_timeout: chrono::DateTime<chrono::Utc>,
}

// for runtime use
#[derive(Debug, Clone)]
pub struct GameResult {
    pub winner: Option<String>, // Player ID
    pub pot_amount: Amount,
    pub transaction_id: Option<String>,
    pub finished_at: chrono::DateTime<chrono::Utc>,
    pub reason: GameEndReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableGameResult {
    pub winner: Option<String>, // Player ID
    pub pot_amount: u64,        // u64 for serialization
    pub transaction_id: Option<String>,
    pub finished_at: chrono::DateTime<chrono::Utc>,
    pub reason: GameEndReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameEndReason {
    NormalCompletion,
    PlayerTimeout,
    PlayerAbort,
    InsufficientFunds,
    CommitmentFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableTwoPlayerLotteryState {
    pub game_id: String,
    pub state: GameState,
    pub bet_amount: u64,
    pub pot_amount: u64,
    pub players: Vec<crate::core::player::SerializablePlayer>,
    pub commitments: std::collections::HashMap<String, Commitment>,
    pub reveals: std::collections::HashMap<String, Reveal>,
    pub timeouts: GamePhaseTimeouts,
    pub result: Option<SerializableGameResult>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub escrow_address: Option<String>,
}

// for runtime use
#[derive(Debug, Clone)]
pub struct TwoPlayerLotteryState {
    pub game_id: String,
    pub state: GameState,
    pub bet_amount: Amount,
    pub pot_amount: Amount,
    pub players: Vec<Player>,
    pub commitments: std::collections::HashMap<String, Commitment>, // player_id -> commitment
    pub reveals: std::collections::HashMap<String, Reveal>,         // player_id -> reveal
    pub timeouts: GamePhaseTimeouts,
    pub result: Option<GameResult>, // Keep original GameResult
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub escrow_address: Option<String>,
}

impl GameResult {
    pub fn to_serializable(&self) -> SerializableGameResult {
        SerializableGameResult {
            winner: self.winner.clone(),
            pot_amount: self.pot_amount.to_sat(),
            transaction_id: self.transaction_id.clone(),
            finished_at: self.finished_at,
            reason: self.reason.clone(),
        }
    }

    pub fn from_serializable(serializable: SerializableGameResult) -> Self {
        Self {
            winner: serializable.winner,
            pot_amount: Amount::from_sat(serializable.pot_amount),
            transaction_id: serializable.transaction_id,
            finished_at: serializable.finished_at,
            reason: serializable.reason,
        }
    }
}

impl TwoPlayerLotteryState {
    pub fn new(game_id: String, bet_amount: Amount) -> Self {
        let now = chrono::Utc::now();

        Self {
            game_id,
            state: GameState::WaitingForPlayers,
            bet_amount,
            pot_amount: Amount::ZERO,
            players: Vec::new(),
            commitments: std::collections::HashMap::new(),
            reveals: std::collections::HashMap::new(),
            timeouts: GamePhaseTimeouts {
                commitment_timeout: now + chrono::Duration::minutes(10),
                reveal_timeout: now + chrono::Duration::minutes(15),
                game_timeout: now + chrono::Duration::hours(1),
            },
            result: None,
            created_at: now,
            updated_at: now,
            escrow_address: None,
        }
    }

    pub fn to_serializable(&self) -> SerializableTwoPlayerLotteryState {
        SerializableTwoPlayerLotteryState {
            game_id: self.game_id.clone(),
            state: self.state.clone(),
            bet_amount: self.bet_amount.to_sat(),
            pot_amount: self.pot_amount.to_sat(),
            players: self.players.iter().map(|p| p.to_serializable()).collect(),
            commitments: self.commitments.clone(),
            reveals: self.reveals.clone(),
            timeouts: self.timeouts.clone(),
            result: self.result.as_ref().map(|r| r.to_serializable()),
            created_at: self.created_at,
            updated_at: self.updated_at,
            escrow_address: self.escrow_address.clone(),
        }
    }

    pub fn from_serializable(
        serializable: SerializableTwoPlayerLotteryState,
    ) -> Result<Self, String> {
        let players: Result<Vec<Player>, String> = serializable
            .players
            .into_iter()
            .map(Player::from_serializable)
            .collect();

        let players = players?;

        Ok(Self {
            game_id: serializable.game_id,
            state: serializable.state,
            bet_amount: Amount::from_sat(serializable.bet_amount),
            pot_amount: Amount::from_sat(serializable.pot_amount),
            players,
            commitments: serializable.commitments,
            reveals: serializable.reveals,
            timeouts: serializable.timeouts,
            result: serializable.result.map(GameResult::from_serializable),
            created_at: serializable.created_at,
            updated_at: serializable.updated_at,
            escrow_address: serializable.escrow_address,
        })
    }

    pub fn is_player(&self, player_id: &str) -> bool {
        self.players.iter().any(|p| p.id == player_id)
    }

    pub fn get_player(&self, player_id: &str) -> Option<&Player> {
        self.players.iter().find(|p| p.id == player_id)
    }

    pub fn can_join(&self) -> bool {
        matches!(self.state, GameState::WaitingForPlayers) && self.players.len() < 2
    }

    pub fn is_ready_for_commitments(&self) -> bool {
        self.players.len() == 2 && matches!(self.state, GameState::WaitingForPlayers)
    }

    pub fn has_committed(&self, player_id: &str) -> bool {
        self.commitments.contains_key(player_id)
    }

    pub fn has_revealed(&self, player_id: &str) -> bool {
        self.reveals.contains_key(player_id)
    }

    pub fn all_committed(&self) -> bool {
        self.players.len() == 2 && self.commitments.len() == 2
    }

    pub fn all_revealed(&self) -> bool {
        self.players.len() == 2 && self.reveals.len() == 2
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.state, GameState::Finished | GameState::Aborted)
    }

    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now();
        now > self.timeouts.game_timeout
    }

    pub fn update_timestamp(&mut self) {
        self.updated_at = chrono::Utc::now();
    }
}
