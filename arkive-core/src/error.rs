use thiserror::Error;

pub type Result<T> = std::result::Result<T, ArkiveError>;

#[derive(Error, Debug)]
pub enum ArkiveError {
    #[error("Wallet error: {0}")]
    Wallet(String),

    #[error("Bitcoin error: {0}")]
    Bitcoin(String),

    #[error("Ark protocol error: {0}")]
    Ark(String),

    #[error("Storage error: {0}")]
    Storage(#[from] rusqlite::Error),

    #[error("Network connection error: {0}")]
    NetworkConnection(String),

    #[error("Esplora error: {0}")]
    Esplora(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Invalid configuration: {0}")]
    Config(String),

    #[error("Insufficient funds: need {need}, have {available}")]
    InsufficientFunds { need: u64, available: u64 },

    #[error("Wallet not found: {name}")]
    WalletNotFound { name: String },

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Operation timeout: {0}")]
    Timeout(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Dialog error: {0}")]
    Dialog(String),
}

impl ArkiveError {
    pub fn wallet(msg: impl Into<String>) -> Self {
        Self::Wallet(msg.into())
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    pub fn bitcoin(msg: impl Into<String>) -> Self {
        Self::Bitcoin(msg.into())
    }

    pub fn ark(msg: impl Into<String>) -> Self {
        Self::Ark(msg.into())
    }

    pub fn network_connection(msg: impl Into<String>) -> Self {
        Self::NetworkConnection(msg.into())
    }

    pub fn esplora(msg: impl Into<String>) -> Self {
        Self::Esplora(msg.into())
    }

    pub fn dialog(msg: impl Into<String>) -> Self {
        Self::Dialog(msg.into())
    }
}

// conversion from dialoguer::Error
impl From<dialoguer::Error> for ArkiveError {
    fn from(err: dialoguer::Error) -> Self {
        ArkiveError::Dialog(err.to_string())
    }
}
