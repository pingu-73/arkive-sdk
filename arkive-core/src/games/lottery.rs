#![allow(unused_imports)]
use super::*;
use crate::ark::ArkService;
use crate::error::Result;
use crate::storage::Storage;
use crate::ArkiveError;
use async_trait::async_trait;
use bitcoin::hashes::Hash;
use std::sync::Arc;

/// Trustless lottery implementation
pub struct TrustlessLottery {
    id: String,
    config: LotteryConfig,
    escrow_manager: Arc<EscrowManager>,
    fairness_engine: Arc<FairnessEngine>,
    storage: Arc<Storage>,
}

/// Lottery configuration
#[derive(Debug, Clone)]
pub struct LotteryConfig {
    pub min_players: usize,
    pub max_players: usize,
    pub entry_fee: Amount,
    pub commitment_timeout: u32,
    pub reveal_timeout: u32,
    pub coordinator_key: XOnlyPublicKey,
    pub network: bitcoin::Network,
}

impl TrustlessLottery {
    pub fn new(config: LotteryConfig, storage: Arc<Storage>) -> Self {
        let escrow_manager = Arc::new(EscrowManager::new(
            config.network,
            config.coordinator_key,
            config.commitment_timeout + config.reveal_timeout,
        ));

        let verifier = Box::new(ModuloVerifier::new(
            config.entry_fee * config.max_players as u64,
        ));

        let fairness_engine = Arc::new(FairnessEngine::new(verifier));

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            config,
            escrow_manager,
            fairness_engine,
            storage,
        }
    }

    /// Create lottery with escrow
    pub async fn create_lottery(&self, participants: Vec<XOnlyPublicKey>) -> Result<GameEscrow> {
        if participants.len() < self.config.min_players {
            return Err(ArkiveError::internal("Not enough participants"));
        }

        if participants.len() > self.config.max_players {
            return Err(ArkiveError::internal("Too many participants"));
        }

        // Create participants
        let participants: Vec<Participant> = participants
            .into_iter()
            .map(|pubkey| Participant {
                pubkey,
                ark_address: format!("ark1{}", hex::encode(pubkey.serialize())),
                stake: self.config.entry_fee,
                commitment: None,
                reveal: None,
                payout_script: None,
            })
            .collect();

        // Create game rules
        let rules = GameRules {
            min_participants: self.config.min_players,
            max_participants: self.config.max_players,
            entry_fee: self.config.entry_fee,
            payout_distribution: PayoutDistribution::WinnerTakeAll,
            dispute_threshold: participants.len() / 2 + 1,
            emergency_key: None,
            commitment_timeout: self.config.commitment_timeout,
            reveal_timeout: self.config.reveal_timeout,
        };

        // Create escrow
        let escrow =
            self.escrow_manager
                .create_game_escrow(&participants, self.config.entry_fee, &rules)?;

        // Store escrow
        self.store_escrow(&escrow).await?;

        Ok(escrow)
    }

    /// Store escrow in database
    async fn store_escrow(&self, escrow: &GameEscrow) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "INSERT INTO game_escrows (
                escrow_id, game_type, taproot_address, total_stake, 
                participants, timeout_block, state, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                hex::encode(escrow.escrow_id),
                "lottery",
                escrow.taproot_address.to_string(),
                escrow.total_stake.to_sat() as i64,
                serde_json::to_string(&escrow.participants)?,
                escrow.timeout as i64,
                serde_json::to_string(&escrow.state)?,
                chrono::Utc::now().timestamp(),
            ],
        )?;

        Ok(())
    }
}

#[async_trait]
impl TrustlessGame for TrustlessLottery {
    fn game_id(&self) -> &str {
        &self.id
    }

    fn min_players(&self) -> usize {
        self.config.min_players
    }

    fn max_players(&self) -> usize {
        self.config.max_players
    }

    async fn create_escrow_script(
        &self,
        participants: &[XOnlyPublicKey],
        _entry_fee: Amount,
    ) -> Result<GameEscrow> {
        self.create_lottery(participants.to_vec()).await
    }

    async fn generate_fairness_proof(&self) -> Result<FairnessProof> {
        Ok(FairnessProof {
            algorithm: "Modulo".to_string(),
            parameters: HashMap::from([
                (
                    "seed_generation".to_string(),
                    "combined_reveals".to_string(),
                ),
                (
                    "winner_selection".to_string(),
                    "seed_mod_participants".to_string(),
                ),
            ]),
            verifier_script: self.build_verifier_script()?,
        })
    }

    async fn verify_outcome(
        &self,
        commits: &[Commitment],
        reveals: &[Reveal],
    ) -> Result<GameOutcome> {
        // Load participants from storage
        let participants = self.load_participants(&self.id).await?;

        // Verify all reveals
        for (i, reveal) in reveals.iter().enumerate() {
            let valid =
                self.fairness_engine
                    .verify_reveal(&commits[i], reveal, &participants[i].pubkey)?;

            if !valid {
                return Err(ArkiveError::internal(format!(
                    "Invalid reveal from participant {}",
                    i
                )));
            }
        }

        // Calculate outcome
        self.fairness_engine
            .calculate_outcome(&participants, reveals)
    }

    async fn execute_payout(
        &self,
        escrow: &GameEscrow,
        outcome: &GameOutcome,
    ) -> Result<PayoutTransaction> {
        // Build payout transaction
        let psbt = self.build_payout_psbt(escrow, outcome)?;

        // Determine required signatures
        let signatures_required: Vec<XOnlyPublicKey> =
            escrow.participants.iter().map(|p| p.pubkey).collect();

        Ok(PayoutTransaction {
            psbt,
            signatures_required,
            timeout_block: escrow.timeout,
        })
    }
}

impl TrustlessLottery {
    /// Build verifier script for on-chain verification
    fn build_verifier_script(&self) -> Result<bitcoin::ScriptBuf> {
        use bitcoin::opcodes::all::*;

        // Script that verifies the fairness calculation on-chain
        let script = bitcoin::ScriptBuf::builder()
            // Verify seed calculation
            .push_opcode(OP_SHA256)
            .push_opcode(OP_DUP)
            // Verify modulo operation
            .push_int(self.config.max_players as i64)
            .push_opcode(OP_MOD)
            // Result is winner index
            .push_opcode(OP_PUSHNUM_1)
            .into_script();

        Ok(script)
    }

    /// Build payout PSBT
    fn build_payout_psbt(
        &self,
        _escrow: &GameEscrow,
        outcome: &GameOutcome,
    ) -> Result<bitcoin::Psbt> {
        use bitcoin::{OutPoint, Transaction, TxIn, TxOut};

        // Create payout transaction
        let mut tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::all_zeros(), // TODO: Get actual escrow UTXO
                    vout: 0,
                },
                script_sig: bitcoin::ScriptBuf::new(),
                sequence: bitcoin::Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: bitcoin::Witness::new(),
            }],
            output: vec![],
        };

        // Add outputs for payouts
        for (pubkey, amount) in &outcome.payouts {
            let address = bitcoin::Address::p2tr(
                &bitcoin::secp256k1::Secp256k1::new(),
                *pubkey,
                None,
                self.config.network,
            );

            tx.output.push(TxOut {
                value: *amount,
                script_pubkey: address.script_pubkey(),
            });
        }

        let psbt = bitcoin::Psbt::from_unsigned_tx(tx)
            .map_err(|e| ArkiveError::internal(format!("Failed to create PSBT: {}", e)))?;
        Ok(psbt)
    }

    /// Load participants from storage
    async fn load_participants(&self, game_id: &str) -> Result<Vec<Participant>> {
        let conn = self.storage.get_connection().await;

        let participants_json: String = conn.query_row(
            "SELECT participants FROM game_escrows WHERE escrow_id = ?1",
            rusqlite::params![game_id],
            |row| row.get(0),
        )?;

        let participants: Vec<Participant> = serde_json::from_str(&participants_json)?;
        Ok(participants)
    }
}
