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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionType {
    OnChain,
    Ark,
    Boarding,
    Exit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionStatus {
    Pending,
    Confirmed,
    Failed,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VtxoStatus {
    Pending,
    Confirmed,
    Spent,
    Expired,
}
