use crate::{GamingError, Result};
use arkive_core::{Amount, ArkWallet};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub type EscrowId = Uuid;

/// Escrow manager for handling game funds
pub struct EscrowManager {
    wallet: Arc<ArkWallet>,
    active_escrows: HashMap<EscrowId, EscrowState>,
    audit_trail: super::audit::AuditTrail,
}

impl std::fmt::Debug for EscrowManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EscrowManager")
            .field("wallet", &"<ArkWallet>")
            .field("active_escrows", &self.active_escrows)
            .field("audit_trail", &self.audit_trail)
            .finish()
    }
}

/// State of an escrow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowState {
    pub id: EscrowId,
    pub participants: Vec<Uuid>,
    pub total_amount: Amount,
    pub individual_amounts: HashMap<Uuid, Amount>,
    pub conditions: EscrowConditions,
    pub status: EscrowStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Conditions for escrow release
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EscrowConditions {
    GameCompletion { game_id: Uuid },
    TimeoutExpiry { deadline: DateTime<Utc> },
    ManualRelease,
}

/// Status of escrow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EscrowStatus {
    WaitingForDeposits,
    Active,
    Released { winner: Option<Uuid> },
    Refunded { reason: String },
}

impl EscrowManager {
    pub fn new(wallet: Arc<ArkWallet>) -> Self {
        Self {
            wallet,
            active_escrows: HashMap::new(),
            audit_trail: super::audit::AuditTrail::new(),
        }
    }

    /// Create new escrow
    pub async fn create_escrow(
        &mut self,
        participants: Vec<Uuid>,
        conditions: EscrowConditions,
    ) -> Result<EscrowId> {
        let escrow_id = Uuid::new_v4();
        let now = Utc::now();

        let escrow_state = EscrowState {
            id: escrow_id,
            participants,
            total_amount: Amount::ZERO,
            individual_amounts: HashMap::new(),
            conditions,
            status: EscrowStatus::WaitingForDeposits,
            created_at: now,
            updated_at: now,
        };

        self.active_escrows.insert(escrow_id, escrow_state);

        self.audit_trail.add_entry(super::audit::AuditEntry {
            escrow_id,
            action: "created".to_string(),
            amount: None,
            participant: None,
            timestamp: now,
            details: "Escrow created".to_string(),
        });

        tracing::info!("Created escrow {}", escrow_id);
        Ok(escrow_id)
    }

    /// Get escrow address for deposits
    pub async fn get_escrow_address(&self) -> Result<String> {
        let ark_addr = self.wallet.get_ark_address().await?;
        Ok(ark_addr.address)
    }

    /// Record a deposit to escrow
    pub async fn record_deposit(
        &mut self,
        escrow_id: EscrowId,
        participant: Uuid,
        amount: Amount,
        txid: String,
    ) -> Result<()> {
        let escrow = self
            .active_escrows
            .get_mut(&escrow_id)
            .ok_or_else(|| GamingError::Escrow("Escrow not found".to_string()))?;

        escrow.individual_amounts.insert(participant, amount);
        escrow.total_amount += amount;
        escrow.updated_at = Utc::now();

        // Check if all participants have deposited
        if escrow.individual_amounts.len() == escrow.participants.len() {
            escrow.status = EscrowStatus::Active;
        }

        self.audit_trail.add_entry(super::audit::AuditEntry {
            escrow_id,
            action: "deposit".to_string(),
            amount: Some(amount),
            participant: Some(participant),
            timestamp: Utc::now(),
            details: format!("Deposit recorded: {}", txid),
        });

        tracing::info!(
            "Recorded deposit of {} sats from {} to escrow {}: {}",
            amount.to_sat(),
            participant,
            escrow_id,
            txid
        );

        Ok(())
    }

    /// Release escrow to winner
    pub async fn release_to_winner(
        &mut self,
        escrow_id: EscrowId,
        winner: Uuid,
        winner_address: &str,
    ) -> Result<String> {
        let escrow = self
            .active_escrows
            .get_mut(&escrow_id)
            .ok_or_else(|| GamingError::Escrow("Escrow not found".to_string()))?;

        if !matches!(escrow.status, EscrowStatus::Active) {
            return Err(GamingError::Escrow("Escrow not active".to_string()));
        }

        let payout_amount = escrow.total_amount;
        let txid = self.wallet.send_ark(winner_address, payout_amount).await?;

        escrow.status = EscrowStatus::Released {
            winner: Some(winner),
        };
        escrow.updated_at = Utc::now();

        self.audit_trail.add_entry(super::audit::AuditEntry {
            escrow_id,
            action: "release".to_string(),
            amount: Some(payout_amount),
            participant: Some(winner),
            timestamp: Utc::now(),
            details: format!("Released to winner: {}", txid),
        });

        tracing::info!(
            "Released {} sats from escrow {} to winner {}: {}",
            payout_amount.to_sat(),
            escrow_id,
            winner,
            txid
        );

        Ok(txid)
    }

    /// Refund escrow to all participants
    pub async fn refund_escrow(
        &mut self,
        escrow_id: EscrowId,
        reason: String,
    ) -> Result<Vec<String>> {
        let escrow = self
            .active_escrows
            .get_mut(&escrow_id)
            .ok_or_else(|| GamingError::Escrow("Escrow not found".to_string()))?;

        let refund_txids = Vec::new();

        for (participant, amount) in &escrow.individual_amounts {
            // TODO: Get participant's addr (this would need to be stored or retrieved)
            // For now, we'll need to get it from the participant somehow
            tracing::warn!(
                "Refund needed for participant {} amount {}",
                participant,
                amount.to_sat()
            );
            // TODO: actual refund logic when we have participant addr
        }

        escrow.status = EscrowStatus::Refunded {
            reason: reason.clone(),
        };
        escrow.updated_at = Utc::now();

        self.audit_trail.add_entry(super::audit::AuditEntry {
            escrow_id,
            action: "refund".to_string(),
            amount: Some(escrow.total_amount),
            participant: None,
            timestamp: Utc::now(),
            details: format!("Refunded: {}", reason),
        });

        tracing::info!("Refunded escrow {}: {}", escrow_id, reason);
        Ok(refund_txids)
    }

    /// Get escrow state
    pub fn get_escrow(&self, escrow_id: EscrowId) -> Option<&EscrowState> {
        self.active_escrows.get(&escrow_id)
    }

    /// Check escrow balance
    pub async fn check_balance(&self) -> Result<arkive_core::Balance> {
        self.wallet.balance().await.map_err(GamingError::from)
    }

    /// Get audit trail
    pub fn get_audit_trail(&self) -> &super::audit::AuditTrail {
        &self.audit_trail
    }
}
