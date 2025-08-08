use bitcoin::Amount;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub confirmed: Amount,
    pub pending: Amount,
    pub total: Amount,
}

impl Balance {
    pub fn new(confirmed: Amount, pending: Amount) -> Self {
        Self {
            confirmed,
            pending,
            total: confirmed + pending,
        }
    }

    pub fn zero() -> Self {
        Self::new(Amount::ZERO, Amount::ZERO)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub txid: String,
    pub amount: i64, // +ve for incoming, -ve for outgoing
    pub timestamp: DateTime<Utc>,
    pub tx_type: TransactionType,
    pub status: TransactionStatus,
    pub fee: Option<Amount>,
    pub source: TransactionSource,
    pub ark_round_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionType {
    OnChain,
    Ark,
    Boarding,
    Exit,
    BatchSwap,
    ConnectorSpend,
    ForfeitTx,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionStatus {
    Pending,
    Confirmed,
    Failed,
    Spent,
    Replaced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Address {
    pub address: String,
    pub address_type: AddressType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AddressType {
    OnChain,
    Ark,
    Boarding,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VtxoInfo {
    pub outpoint: String,
    pub amount: Amount,
    pub status: VtxoStatus,
    pub expiry: DateTime<Utc>,
    pub address: String,
    pub is_preconfirmed: bool,
    pub is_recoverable: bool,
    pub batch_id: Option<String>,
    pub commitment_txids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VtxoStatus {
    Preconfirmed,
    Unconfirmed,
    Pending,
    Confirmed,
    Spent,
    Expired,
    Replaced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchSwapInfo {
    pub swap_id: String,
    pub status: BatchSwapStatus,
    pub input_vtxos: Vec<String>,
    pub output_vtxos: Vec<String>,
    pub commitment_txid: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BatchSwapStatus {
    Pending,
    Signed,
    Committed,
    Confirmed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorInfo {
    pub outpoint: String,
    pub amount: Amount,
    pub associated_vtxo: String,
    pub status: ConnectorStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectorStatus {
    Active,
    Spent,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionSource {
    Blockchain,
    ArkServer,
    LocalRound,
}
