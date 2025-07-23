use arkive_core::Amount;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Types of escrow tx
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionType {
    Deposit,
    Release,
    Refund,
}

/// Escrow tx record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowTransaction {
    pub id: Uuid,
    pub escrow_id: super::EscrowId,
    pub transaction_type: TransactionType,
    pub amount: Amount,
    pub from_address: Option<String>,
    pub to_address: Option<String>,
    pub txid: String,
    pub participant: Option<Uuid>,
    pub timestamp: DateTime<Utc>,
    pub confirmed: bool,
}

impl EscrowTransaction {
    pub fn new_deposit(
        escrow_id: super::EscrowId,
        amount: Amount,
        from_address: String,
        txid: String,
        participant: Uuid,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            escrow_id,
            transaction_type: TransactionType::Deposit,
            amount,
            from_address: Some(from_address),
            to_address: None,
            txid,
            participant: Some(participant),
            timestamp: Utc::now(),
            confirmed: false,
        }
    }

    pub fn new_release(
        escrow_id: super::EscrowId,
        amount: Amount,
        to_address: String,
        txid: String,
        participant: Uuid,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            escrow_id,
            transaction_type: TransactionType::Release,
            amount,
            from_address: None,
            to_address: Some(to_address),
            txid,
            participant: Some(participant),
            timestamp: Utc::now(),
            confirmed: false,
        }
    }

    pub fn mark_confirmed(&mut self) {
        self.confirmed = true;
    }
}
