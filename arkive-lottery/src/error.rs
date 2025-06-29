use thiserror::Error;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, LotteryError>;

#[derive(Error, Debug)]
pub enum LotteryError {
    #[error("Arkive core error: {0}")]
    ArkiveCore(#[from] arkive_core::ArkiveError),

    #[error("Invalid game state: {0}")]
    InvalidState(String),

    #[error("Player not found: {0}")]
    PlayerNotFound(Uuid),

    #[error("Game is full")]
    GameFull,

    #[error("Game not ready")]
    GameNotReady,

    #[error("Commitment already submitted")]
    CommitmentAlreadySubmitted,

    #[error("Commitment not revealed for player: {0}")]
    CommitmentNotRevealed(Uuid),

    #[error("Invalid commitment")]
    InvalidCommitment,

    #[error("Timeout expired")]
    TimeoutExpired,

    #[error("Bet already placed by player: {0}")]
    BetAlreadyPlaced(Uuid),

    #[error("Insufficient balance: need {need} sats, have {available} sats")]
    InsufficientBalance { need: u64, available: u64 },

    #[error("Bet placement failed: {0}")]
    BetPlacementFailed(String),

    #[error("Payout failed: {0}")]
    PayoutFailed(String),

    #[error("Refund failed: {0}")]
    RefundFailed(String),

    #[error("Pot wallet error: {0}")]
    PotWalletError(String),

    #[error("Cryptographic error: {0}")]
    Crypto(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}
