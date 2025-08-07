use thiserror::Error;

pub type Result<T> = std::result::Result<T, GamingError>;

#[derive(Error, Debug)]
pub enum GamingError {
    #[error("Game error: {0}")]
    Game(String),

    #[error("Commitment error: {0}")]
    Commitment(String),

    #[error("Escrow error: {0}")]
    Escrow(String),

    #[error("Player error: {0}")]
    Player(String),

    #[error("Invalid game state: expected {expected}, found {found}")]
    InvalidState { expected: String, found: String },

    #[error("Game not found: {game_id}")]
    GameNotFound { game_id: String },

    #[error("Player not found: {player_id}")]
    PlayerNotFound { player_id: String },

    #[error("Insufficient funds: need {need}, have {available}")]
    InsufficientFunds { need: u64, available: u64 },

    #[error("Commitment verification failed: {reason}")]
    CommitmentVerificationFailed { reason: String },

    #[error("Game timeout: {phase}")]
    GameTimeout { phase: String },

    #[error("Invalid bet amount: {amount}")]
    InvalidBetAmount { amount: u64 },

    #[error("Game already started")]
    GameAlreadyStarted,

    #[error("Game already finished")]
    GameAlreadyFinished,

    #[error("Not player's turn")]
    NotPlayerTurn,

    #[error("ARKive core error: {0}")]
    ArkiveCore(#[from] arkive_core::ArkiveError),

    #[error("Ark core error: {0}")]
    ArkCore(#[from] ark_core::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl GamingError {
    pub fn game(msg: impl Into<String>) -> Self {
        Self::Game(msg.into())
    }

    pub fn commitment(msg: impl Into<String>) -> Self {
        Self::Commitment(msg.into())
    }

    pub fn escrow(msg: impl Into<String>) -> Self {
        Self::Escrow(msg.into())
    }

    pub fn player(msg: impl Into<String>) -> Self {
        Self::Player(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}
