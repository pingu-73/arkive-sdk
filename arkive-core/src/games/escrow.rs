#![allow(unused_imports)]
use super::*;
use crate::error::{ArkiveError, Result};
use ark_core::conversions::{from_musig_xonly, to_musig_pk};
use bitcoin::key::UntweakedPublicKey;
use bitcoin::secp256k1::{PublicKey, Secp256k1};
use bitcoin::taproot::{TaprootBuilder, TaprootSpendInfo};
use bitcoin::{opcodes::all::*, ScriptBuf, Sequence};
use std::fmt;
// use musig::musig::{KeyAggCache, PublicKey as MusigPublicKey};

pub struct EscrowManager {
    network: bitcoin::Network,
    coordinator_key: XOnlyPublicKey,
    timeout_blocks: u32,
}

impl EscrowManager {
    pub fn new(
        network: bitcoin::Network,
        coordinator_key: XOnlyPublicKey,
        timeout_blocks: u32,
    ) -> Self {
        Self {
            network,
            coordinator_key,
            timeout_blocks,
        }
    }

    /// Create a multi-party game escrow
    pub fn create_game_escrow(
        &self,
        participants: &[Participant],
        entry_fee: Amount,
        game_rules: &GameRules,
    ) -> Result<GameEscrow> {
        let secp = bitcoin::secp256k1::Secp256k1::new();

        // Generate escrow ID
        let escrow_id = self.generate_escrow_id(participants, entry_fee)?;

        // Create script tree
        let script_tree = self.build_escrow_scripts(participants, game_rules)?;

        // Build Taproot tree
        let (internal_key, _participant_pubkeys) = self.compute_aggregate_key(participants)?;
        let taproot_info = self.build_taproot_tree(&secp, internal_key, &script_tree)?;

        // Create address
        let address = bitcoin::Address::p2tr(
            &secp,
            internal_key,
            taproot_info.merkle_root(),
            self.network,
        );

        Ok(GameEscrow {
            escrow_id,
            taproot_address: address.to_string(),
            internal_key,
            script_tree,
            total_stake: entry_fee * participants.len() as u64,
            participants: participants.to_vec(),
            timeout: self.current_block_height()? + self.timeout_blocks,
            state: EscrowState::Gathering,
        })
    }

    /// Build escrow script tree
    fn build_escrow_scripts(
        &self,
        participants: &[Participant],
        rules: &GameRules,
    ) -> Result<EscrowScriptTree> {
        // 1. Cooperative payout path (all participants agree)
        let cooperative_payout = self.build_cooperative_script(participants)?;

        // 2. Timeout refund path (after timeout, refund all)
        let timeout_refund = self.build_timeout_script(participants, self.timeout_blocks)?;

        // 3. Dispute resolution path (coordinator + majority)
        let dispute_resolution =
            self.build_dispute_script(participants, self.coordinator_key, rules.dispute_threshold)?;

        // 4. Emergency key path (optional, for critical issues)
        let emergency_key = if let Some(key) = rules.emergency_key {
            Some(self.build_emergency_script(key, self.timeout_blocks * 2)?)
        } else {
            None
        };

        Ok(EscrowScriptTree {
            cooperative_payout,
            timeout_refund,
            dispute_resolution,
            emergency_key,
        })
    }

    /// Build cooperative payout script (all participants must sign)
    fn build_cooperative_script(&self, participants: &[Participant]) -> Result<ScriptBuf> {
        let mut script = ScriptBuf::builder();

        // Require all participants to sign
        for participant in participants {
            script = script
                .push_x_only_key(&participant.pubkey)
                .push_opcode(OP_CHECKSIGVERIFY);
        }

        // Final OP_TRUE for completion
        script = script.push_opcode(OP_PUSHNUM_1);

        Ok(script.into_script())
    }

    /// Build timeout refund script
    fn build_timeout_script(
        &self,
        participants: &[Participant],
        timeout_blocks: u32,
    ) -> Result<ScriptBuf> {
        let mut script = ScriptBuf::builder()
            // Check timeout has passed
            .push_int(timeout_blocks as i64)
            .push_opcode(OP_CSV)
            .push_opcode(OP_DROP);

        // For each participant, add their refund output
        for participant in participants {
            // Push participant pubkey
            script = script.push_x_only_key(&participant.pubkey);
            // Push their stake amount
            script = script.push_int(participant.stake.to_sat() as i64);
        }

        // Add final verification
        script = script
            .push_int(participants.len() as i64)
            .push_opcode(OP_PUSHNUM_1);

        Ok(script.into_script())
    }

    /// Build dispute resolution script
    fn build_dispute_script(
        &self,
        participants: &[Participant],
        coordinator: XOnlyPublicKey,
        threshold: usize,
    ) -> Result<ScriptBuf> {
        // Coordinator + threshold of participants can resolve disputes
        let script = ScriptBuf::builder()
            // Coordinator must sign
            .push_x_only_key(&coordinator)
            .push_opcode(OP_CHECKSIGVERIFY)
            // Threshold signatures from participants
            .push_int(threshold as i64);

        let mut script = script;
        for participant in participants {
            script = script.push_x_only_key(&participant.pubkey);
        }

        script = script
            .push_int(participants.len() as i64)
            .push_opcode(OP_CHECKMULTISIG);

        Ok(script.into_script())
    }

    /// Build emergency script
    fn build_emergency_script(
        &self,
        emergency_key: XOnlyPublicKey,
        delay_blocks: u32,
    ) -> Result<ScriptBuf> {
        let script = ScriptBuf::builder()
            .push_int(delay_blocks as i64)
            .push_opcode(OP_CSV)
            .push_opcode(OP_DROP)
            .push_x_only_key(&emergency_key)
            .push_opcode(OP_CHECKSIG)
            .into_script();

        Ok(script)
    }

    /// Compute aggregate key for Taproot internal key
    fn compute_aggregate_key(
        &self,
        participants: &[Participant],
    ) -> Result<(XOnlyPublicKey, Vec<PublicKey>)> {
        if participants.is_empty() {
            return Err(ArkiveError::internal("No participants for key aggregation"));
        }

        let secp_musig = musig::Secp256k1::new();

        // Collect all pubkeys that will participate in the aggregate
        let mut pubkeys: Vec<PublicKey> = Vec::with_capacity(participants.len());

        // Add all participant keys
        for participant in participants {
            let pubkey =
                PublicKey::from_x_only_public_key(participant.pubkey, bitcoin::key::Parity::Even);
            pubkeys.push(pubkey);
        }

        // Sort pubkeys for deterministic aggregation
        pubkeys.sort_by_key(|k| k.serialize());

        // Convert to musig public keys
        let musig_pubkeys: Vec<musig::PublicKey> =
            pubkeys.iter().map(|pk| to_musig_pk(*pk)).collect();

        // Create key aggregation cache
        let key_agg_cache =
            musig::musig::KeyAggCache::new(&secp_musig, &musig_pubkeys.iter().collect::<Vec<_>>());

        // Get the aggregated public key
        let agg_pk = key_agg_cache.agg_pk();

        // Convert back to XOnlyPublicKey
        let aggregate_key = from_musig_xonly(agg_pk);

        Ok((aggregate_key, pubkeys))
    }

    /// Build Taproot tree from scripts
    fn build_taproot_tree(
        &self,
        secp: &bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
        internal_key: XOnlyPublicKey,
        scripts: &EscrowScriptTree,
    ) -> Result<TaprootSpendInfo> {
        let mut builder = TaprootBuilder::new();

        // Add cooperative payout at depth 1 (most likely)
        builder = builder
            .add_leaf(1, scripts.cooperative_payout.clone())
            .expect("valid cooperative script");

        // Add timeout refund at depth 2
        builder = builder
            .add_leaf(2, scripts.timeout_refund.clone())
            .expect("valid timeout script");

        // Add dispute resolution at depth 2
        builder = builder
            .add_leaf(2, scripts.dispute_resolution.clone())
            .expect("valid dispute script");

        // Add emergency key at depth 3 if present
        if let Some(ref emergency_script) = scripts.emergency_key {
            builder = builder
                .add_leaf(3, emergency_script.clone())
                .expect("valid emergency script");
        }

        let taproot_info = builder
            .finalize(secp, internal_key)
            .expect("valid taproot tree");

        Ok(taproot_info)
    }

    fn generate_escrow_id(
        &self,
        participants: &[Participant],
        entry_fee: Amount,
    ) -> Result<[u8; 32]> {
        use bitcoin::hashes::{sha256, Hash};

        let mut data = Vec::new();
        for p in participants {
            data.extend_from_slice(&p.pubkey.serialize());
        }
        data.extend_from_slice(&entry_fee.to_sat().to_le_bytes());
        data.extend_from_slice(&self.timeout_blocks.to_le_bytes());

        Ok(sha256::Hash::hash(&data).to_byte_array())
    }

    #[allow(dead_code)]
    fn encode_refund_outputs(&self, participants: &[Participant]) -> Result<Vec<u8>> {
        // Encode refund distribution
        let mut encoded = Vec::new();
        for participant in participants {
            encoded.extend_from_slice(&participant.pubkey.serialize());
            encoded.extend_from_slice(&participant.stake.to_sat().to_le_bytes());
        }
        Ok(encoded)
    }

    fn current_block_height(&self) -> Result<u32> {
        // TODO: Query actual block height
        Ok(800_000)
    }
}

/// Game rules configuration
#[derive(Debug, Clone)]
pub struct GameRules {
    pub min_participants: usize,
    pub max_participants: usize,
    pub entry_fee: Amount,
    pub payout_distribution: PayoutDistribution,
    pub dispute_threshold: usize,
    pub emergency_key: Option<XOnlyPublicKey>,
    pub commitment_timeout: u32,
    pub reveal_timeout: u32,
}

/// Custom payout function type
pub type PayoutFunction = fn(&[Participant]) -> HashMap<XOnlyPublicKey, Amount>;

/// Payout distribution rules
#[derive(Clone)]
pub enum PayoutDistribution {
    WinnerTakeAll,
    TopN { n: usize, percentages: Vec<f64> },
    Proportional,
    Custom(PayoutFunction),
}

impl fmt::Debug for PayoutDistribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PayoutDistribution::WinnerTakeAll => write!(f, "WinnerTakeAll"),
            PayoutDistribution::TopN { n, percentages } => f
                .debug_struct("TopN")
                .field("n", n)
                .field("percentages", percentages)
                .finish(),
            PayoutDistribution::Proportional => write!(f, "Proportional"),
            PayoutDistribution::Custom(_) => write!(f, "Custom(<function>)"),
        }
    }
}
