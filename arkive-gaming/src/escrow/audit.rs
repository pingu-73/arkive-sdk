use arkive_core::Amount;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Audit trail for escrow operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditTrail {
    entries: Vec<AuditEntry>,
    escrow_summaries: HashMap<super::EscrowId, EscrowSummary>,
}

/// Individual audit entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub escrow_id: super::EscrowId,
    pub action: String,
    pub amount: Option<Amount>,
    pub participant: Option<Uuid>,
    pub timestamp: DateTime<Utc>,
    pub details: String,
}

/// Summary of escrow activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowSummary {
    pub escrow_id: super::EscrowId,
    pub total_deposits: Amount,
    pub total_releases: Amount,
    pub total_refunds: Amount,
    pub participant_count: usize,
    pub created_at: DateTime<Utc>,
    pub final_status: Option<String>,
}

impl AuditTrail {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            escrow_summaries: HashMap::new(),
        }
    }

    pub fn add_entry(&mut self, entry: AuditEntry) {
        let summary = self
            .escrow_summaries
            .entry(entry.escrow_id)
            .or_insert_with(|| EscrowSummary {
                escrow_id: entry.escrow_id,
                total_deposits: Amount::ZERO,
                total_releases: Amount::ZERO,
                total_refunds: Amount::ZERO,
                participant_count: 0,
                created_at: entry.timestamp,
                final_status: None,
            });

        match entry.action.as_str() {
            "deposit" => {
                if let Some(amount) = entry.amount {
                    summary.total_deposits += amount;
                }
            }
            "release" => {
                if let Some(amount) = entry.amount {
                    summary.total_releases += amount;
                }
                summary.final_status = Some("released".to_string());
            }
            "refund" => {
                if let Some(amount) = entry.amount {
                    summary.total_refunds += amount;
                }
                summary.final_status = Some("refunded".to_string());
            }
            _ => {}
        }

        self.entries.push(entry);
    }

    pub fn get_entries_for_escrow(&self, escrow_id: super::EscrowId) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.escrow_id == escrow_id)
            .collect()
    }

    pub fn get_summary(&self, escrow_id: super::EscrowId) -> Option<&EscrowSummary> {
        self.escrow_summaries.get(&escrow_id)
    }

    pub fn get_all_entries(&self) -> &[AuditEntry] {
        &self.entries
    }
}

impl Default for AuditTrail {
    fn default() -> Self {
        Self::new()
    }
}
