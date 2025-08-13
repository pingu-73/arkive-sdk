#![allow(unused_imports)]
use super::escrow_scripts::{LotteryEscrowScript, LotteryEscrowOptions};
use super::fairness::{FairnessEngine, ModuloVerifier};
use super::{Commitment, GameOutcome, Participant, Reveal};
use crate::ark::ArkService;
use crate::error::{ArkiveError, Result};
use crate::storage::Storage;
use crate::types::VtxoInfo;
use ark_core::Vtxo;
use crate::games::XOnlyPublicKey;
use bitcoin::{Amount, Network, OutPoint, Psbt, Sequence};
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::str::FromStr;

#[allow(dead_code)]
pub struct LotteryCoordinator {
    storage: Arc<Storage>,
    network: Network,
    is_mutinynet: bool,
    server_key: XOnlyPublicKey,
    coordinator_key: XOnlyPublicKey,
    fairness_engine: Arc<FairnessEngine>,
}

impl LotteryCoordinator {
    pub async fn list_lotteries(&self) -> Result<Vec<LotteryEscrow>> {
        let conn = self.storage.get_connection().await;
        
        let mut stmt = conn.prepare(
            "SELECT escrow_data FROM lottery_escrows ORDER BY created_at DESC"
        )?;
        
        let lotteries = stmt
            .query_map([], |row| {
                let escrow_data: String = row.get(0)?;
                let lottery: LotteryEscrow = serde_json::from_str(&escrow_data)
                    .map_err(|_e| rusqlite::Error::InvalidColumnType(
                        0, 
                        "escrow_data".to_string(), 
                        rusqlite::types::Type::Text
                    ))?;
                
                Ok(lottery)
            })?
            .collect::<std::result::Result<Vec<LotteryEscrow>, _>>()
            .map_err(|e| ArkiveError::internal(format!("Failed to deserialize lotteries: {}", e)))?;
            
        Ok(lotteries)
    }
    
    pub fn new(
        storage: Arc<Storage>,
        network: Network,
        is_mutinynet: bool,
        server_key: XOnlyPublicKey,
        coordinator_key: XOnlyPublicKey,
    ) -> Self {
        let verifier = Box::new(ModuloVerifier::new(Amount::ZERO));
        let fairness_engine = Arc::new(FairnessEngine::new(verifier));

        Self {
            storage,
            network,
            is_mutinynet,
            server_key,
            coordinator_key,
            fairness_engine,
        }
    }

    /// Create a lottery with escrow script
    pub async fn create_lottery_with_escrow(
        &self,
        participants: Vec<XOnlyPublicKey>,
        entry_fee: Amount,
    ) -> Result<LotteryEscrow> {
        // Create escrow options
        let options = LotteryEscrowOptions {
            participants: participants.clone(),
            coordinator: self.coordinator_key,
            server: self.server_key,
            reveal_timeout: Sequence::from_consensus(144), // ~1 day
            claim_timeout: Sequence::from_consensus(288),  // ~2 days
        };
    
        // Create escrow script
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let escrow_script = LotteryEscrowScript::new(&secp, options.clone(), self.network, self.server_key)?;
    
        // Get escrow address
        let ark_address = escrow_script.get_ark_address();
        let escrow_address_string = ark_address.to_string();

        // Store script parameters for reconstruction
        let script_parameters = LotteryEscrowParameters {
            coordinator: self.coordinator_key,
            server: self.server_key,
            reveal_timeout: options.reveal_timeout.to_consensus_u32(),
            claim_timeout: options.claim_timeout.to_consensus_u32(),
        };
    
        // Create lottery escrow
        let lottery_id = format!("lottery_{}", Utc::now().timestamp());
        let lottery_escrow = LotteryEscrow {
            lottery_id: lottery_id.clone(),
            escrow_script: Some(escrow_script),  // Store as Some
            escrow_address: escrow_address_string.to_string(),
            participants: participants.clone(),
            script_parameters,
            entry_fee,
            total_pot: entry_fee * participants.len() as u64,
            state: LotteryState::AwaitingFunding,
            commitments: HashMap::new(),
            reveals: HashMap::new(),
            funding_vtxos: Vec::new(),
            created_at: Utc::now(),
            reveal_deadline: Utc::now() + chrono::Duration::hours(24),
            claim_deadline: Utc::now() + chrono::Duration::hours(48),
        };
    
        // Store in database
        self.store_lottery_escrow(&lottery_escrow).await?;
    
        Ok(lottery_escrow)
    }

    /// Process participant funding into escrow
    pub async fn fund_escrow(
        &self,
        lottery_id: &str,
        participant: XOnlyPublicKey,
        vtxo_outpoint: OutPoint,
        amount: Amount,
    ) -> Result<()> {
        let mut lottery = self.load_lottery_escrow(lottery_id).await?;

        // Verify participant
        if !lottery.participants.contains(&participant) {
            return Err(ArkiveError::internal("Not a participant in this lottery"));
        }

        // Verify amount
        if amount < lottery.entry_fee {
            return Err(ArkiveError::internal("Insufficient entry fee"));
        }

        // Record funding
        lottery.funding_vtxos.push(FundingRecord {
            participant,
            vtxo_outpoint,
            amount,
            timestamp: Utc::now(),
        });

        // Check if all funded
        if lottery.funding_vtxos.len() == lottery.participants.len() {
            lottery.state = LotteryState::CommitmentPhase;
        }

        self.update_lottery_escrow(&lottery).await?;
        Ok(())
    }

    /// Submit commitment with escrow coordination
    pub async fn submit_commitment_with_escrow(
        &self,
        lottery_id: &str,
        participant: XOnlyPublicKey,
        commitment: Commitment,
    ) -> Result<()> {
        let mut lottery = self.load_lottery_escrow(lottery_id).await?;

        // Verify state
        if lottery.state != LotteryState::CommitmentPhase {
            return Err(ArkiveError::internal("Not in commitment phase"));
        }

        // Store commitment
        lottery.commitments.insert(participant, commitment);

        // Check if all committed
        if lottery.commitments.len() == lottery.participants.len() {
            lottery.state = LotteryState::RevealPhase;
        }

        self.update_lottery_escrow(&lottery).await?;
        Ok(())
    }

    /// Submit reveal with escrow coordination
    pub async fn submit_reveal_with_escrow(
        &self,
        lottery_id: &str,
        participant: XOnlyPublicKey,
        reveal: Reveal,
    ) -> Result<Option<GameOutcome>> {
        let mut lottery = self.load_lottery_escrow(lottery_id).await?;

        // Verify state
        if lottery.state != LotteryState::RevealPhase {
            return Err(ArkiveError::internal("Not in reveal phase"));
        }

        // Verify reveal against commitment
        let commitment = lottery.commitments.get(&participant)
            .ok_or_else(|| ArkiveError::internal("No commitment found"))?;

        let valid = self.fairness_engine.verify_reveal(
            commitment,
            &reveal,
            &participant,
        )?;

        if !valid {
            return Err(ArkiveError::internal("Invalid reveal"));
        }

        // Store reveal
        lottery.reveals.insert(participant, reveal);

        // Check if all revealed
        if lottery.reveals.len() == lottery.participants.len() {
            // Calculate winner
            let outcome = self.determine_winner(&lottery).await?;
            lottery.state = LotteryState::WinnerDetermined;
            
            self.update_lottery_escrow(&lottery).await?;
            
            return Ok(Some(outcome));
        }

        self.update_lottery_escrow(&lottery).await?;
        Ok(None)
    }

    /// Execute winner payout using forfeit transactions
    pub async fn execute_winner_payout(
        &self,
        lottery_id: &str,
        winner: XOnlyPublicKey,
        ark_service: &ArkService,
    ) -> Result<String> {
        let lottery = self.load_lottery_escrow(lottery_id).await?;

        if lottery.state != LotteryState::WinnerDetermined {
            return Err(ArkiveError::internal("Winner not yet determined"));
        }

        // Create forfeit transactions for losers
        let mut forfeit_txs = Vec::new();
        
        for funding in &lottery.funding_vtxos {
            if funding.participant != winner {
                // This VTXO will be forfeited to winner
                // create actual forfeit transaction
                forfeit_txs.push(funding.vtxo_outpoint);
            }
        }

        // TODO: Execute batch swap to consolidate winnings
        let swap_id = ark_service.batch_swap(Some(
            forfeit_txs.iter().map(|o| o.to_string()).collect()
        )).await?;

        Ok(swap_id.unwrap_or_else(|| "pending".to_string()))
    }

    async fn determine_winner(&self, lottery: &LotteryEscrow) -> Result<GameOutcome> {
        let participants: Vec<Participant> = lottery.participants.iter().map(|pk| {
            Participant {
                pubkey: *pk,
                ark_address: format!("ark1{}", hex::encode(pk.serialize())),
                stake: lottery.entry_fee,
                commitment: lottery.commitments.get(pk).cloned(),
                reveal: lottery.reveals.get(pk).cloned(),
                payout_script: None,
            }
        }).collect();

        let reveals: Vec<Reveal> = lottery.reveals.values().cloned().collect();

        self.fairness_engine.calculate_outcome(&participants, &reveals)
    }

    async fn store_lottery_escrow(&self, lottery: &LotteryEscrow) -> Result<()> {
        let conn = self.storage.get_connection().await;
        
        let escrow_data = serde_json::to_string(lottery)
        .map_err(|e| ArkiveError::internal(format!("Failed to serialize lottery: {}", e)))?;
        
        conn.execute(
            "INSERT OR REPLACE INTO lottery_escrows 
             (lottery_id, escrow_data, escrow_address, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
             rusqlite::params![
                lottery.lottery_id,
                escrow_data,
                lottery.escrow_address,
                serde_json::to_string(&lottery.state).map_err(|e| ArkiveError::internal(format!("Failed to serialize state: {}", e)))?,
                lottery.created_at.timestamp(),
                Utc::now().timestamp(),
            ],
        ).map_err(|e| ArkiveError::Storage(e))?;

        Ok(())
    }

    pub async fn load_lottery_escrow(&self, lottery_id: &str) -> Result<LotteryEscrow> {
        let conn = self.storage.get_connection().await;
        
        let escrow_data: String = conn.query_row(
            "SELECT escrow_data FROM lottery_escrows WHERE lottery_id = ?1",
            rusqlite::params![lottery_id],
            |row| row.get(0),
        )?;

        let lottery: LotteryEscrow = serde_json::from_str(&escrow_data)?;
        Ok(lottery)
    }

    pub async fn update_lottery_escrow(&self, lottery: &LotteryEscrow) -> Result<()> {
        self.store_lottery_escrow(lottery).await
    }
}

/// Enhanced lottery escrow structure
#[derive(Debug, Clone)]
pub struct LotteryEscrow {
    pub lottery_id: String,
    // #[serde(skip)]
    pub escrow_script: Option<LotteryEscrowScript>,
    pub escrow_address: String,
    pub participants: Vec<XOnlyPublicKey>,
    pub entry_fee: Amount,
    pub total_pot: Amount,
    pub state: LotteryState,
    pub commitments: HashMap<XOnlyPublicKey, Commitment>,
    pub reveals: HashMap<XOnlyPublicKey, Reveal>,
    pub funding_vtxos: Vec<FundingRecord>,
    pub created_at: DateTime<Utc>,
    pub reveal_deadline: DateTime<Utc>,
    pub claim_deadline: DateTime<Utc>,
    pub script_parameters: LotteryEscrowParameters,
}

impl LotteryEscrow {
    pub fn recreate_escrow_script(&self, network: Network, is_mutinynet: bool) -> Result<super::escrow_scripts::LotteryEscrowScript> {
        let _effective_network = if is_mutinynet && network == Network::Signet {
            Network::Signet
        } else {
            network
        };
        
        let options = super::escrow_scripts::LotteryEscrowOptions {
            participants: self.participants.clone(),
            coordinator: self.script_parameters.coordinator,
            server: self.script_parameters.server,
            reveal_timeout: bitcoin::Sequence::from_consensus(self.script_parameters.reveal_timeout),
            claim_timeout: bitcoin::Sequence::from_consensus(self.script_parameters.claim_timeout),
        };
        
        let secp = bitcoin::secp256k1::Secp256k1::new();
        super::escrow_scripts::LotteryEscrowScript::new(&secp, options, _effective_network, self.script_parameters.server)
    }

    pub fn get_escrow_address(&self) -> Result<crate::ArkAddress> {
        if let Some(ref escrow_script) = self.escrow_script {
            Ok(escrow_script.get_ark_address())
        } else {
            // Try to recreate from stored parameters
            let options = super::escrow_scripts::LotteryEscrowOptions {
                participants: self.participants.clone(),
                coordinator: self.script_parameters.coordinator,
                server: self.script_parameters.server,
                reveal_timeout: bitcoin::Sequence::from_consensus(self.script_parameters.reveal_timeout),
                claim_timeout: bitcoin::Sequence::from_consensus(self.script_parameters.claim_timeout),
            };
            
            let secp = bitcoin::secp256k1::Secp256k1::new();
            let script = super::escrow_scripts::LotteryEscrowScript::new(
                &secp, 
                options, 
                Network::Regtest, // TODO: change
                self.script_parameters.server
            )?;
            Ok(script.get_ark_address())
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LotteryEscrowParameters {
    pub coordinator: XOnlyPublicKey,
    pub server: XOnlyPublicKey,
    pub reveal_timeout: u32,
    pub claim_timeout: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FundingRecord {
    pub participant: XOnlyPublicKey,
    pub vtxo_outpoint: OutPoint,
    pub amount: Amount,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LotteryState {
    AwaitingFunding,
    CommitmentPhase,
    RevealPhase,
    WinnerDetermined,
    Completed,
    TimedOut,
    Disputed,
}


// Manual serialization
impl serde::Serialize for LotteryEscrow {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        
        let mut state = serializer.serialize_struct("LotteryEscrow", 13)?;
        state.serialize_field("lottery_id", &self.lottery_id)?;
        state.serialize_field("escrow_address", &self.escrow_address)?;
        state.serialize_field("participants", &self.participants)?;
        state.serialize_field("entry_fee", &self.entry_fee.to_sat())?;
        state.serialize_field("total_pot", &self.total_pot.to_sat())?;
        state.serialize_field("state", &self.state)?;
        state.serialize_field("commitments", &self.commitments)?;
        state.serialize_field("reveals", &self.reveals)?;
        state.serialize_field("funding_vtxos", &self.funding_vtxos)?;
        state.serialize_field("created_at", &self.created_at.timestamp())?;
        state.serialize_field("reveal_deadline", &self.reveal_deadline.timestamp())?;
        state.serialize_field("claim_deadline", &self.claim_deadline.timestamp())?;
        state.serialize_field("script_parameters", &self.script_parameters)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for LotteryEscrow {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Helper {
            lottery_id: String,
            escrow_address: String,
            participants: Vec<XOnlyPublicKey>,
            entry_fee: u64,
            total_pot: u64,
            state: LotteryState,
            commitments: HashMap<XOnlyPublicKey, Commitment>,
            reveals: HashMap<XOnlyPublicKey, Reveal>,
            funding_vtxos: Vec<FundingRecord>,
            created_at: i64,
            reveal_deadline: i64,
            claim_deadline: i64,
            script_parameters: LotteryEscrowParameters,
        }

        let helper = Helper::deserialize(deserializer)?;
        
        Ok(LotteryEscrow {
            lottery_id: helper.lottery_id,
            escrow_script: None, // Will be recreated when needed
            escrow_address: helper.escrow_address,
            participants: helper.participants,
            entry_fee: Amount::from_sat(helper.entry_fee),
            total_pot: Amount::from_sat(helper.total_pot),
            state: helper.state,
            commitments: helper.commitments,
            reveals: helper.reveals,
            funding_vtxos: helper.funding_vtxos,
            created_at: DateTime::from_timestamp(helper.created_at, 0).unwrap_or_else(Utc::now),
            reveal_deadline: DateTime::from_timestamp(helper.reveal_deadline, 0).unwrap_or_else(Utc::now),
            claim_deadline: DateTime::from_timestamp(helper.claim_deadline, 0).unwrap_or_else(Utc::now),
            script_parameters: helper.script_parameters,
        })
    }
}