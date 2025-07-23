use crate::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use uuid::Uuid;

pub type GameId = Uuid;

/// Generic game interface for 2-player games
#[async_trait]
pub trait Game: Debug + Send + Sync {
    type GameState: Clone + Debug + Send + Sync;
    type Player: Clone + Debug + Send + Sync;
    type GameResult: Clone + Debug + Send + Sync;

    /// Get the game ID
    fn id(&self) -> GameId;

    /// Get current game state
    fn state(&self) -> &Self::GameState;

    /// Add a player to the game
    async fn add_player(&mut self, player: Self::Player) -> Result<Uuid>;

    /// Check if game can start
    fn can_start(&self) -> bool;

    /// Start the game
    async fn start(&mut self) -> Result<()>;

    /// Execute a player action
    async fn execute_action(&mut self, player_id: Uuid, action: GameAction) -> Result<()>;

    /// Check for timeouts and handle them
    async fn check_timeouts(&mut self) -> Result<()>;

    /// Get game result if completed
    fn get_result(&self) -> Option<&Self::GameResult>;

    /// Check if game is completed
    fn is_completed(&self) -> bool;
}

/// Generic game actions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameAction {
    PlaceBet,
    SubmitCommitment { secret: Vec<u8> },
    RevealCommitment { secret: Vec<u8> },
    Forfeit,
}

/// Game result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameResult {
    pub winner: Option<Uuid>,
    pub payout_amount: arkive_core::Amount,
    pub game_completed_at: chrono::DateTime<chrono::Utc>,
}
