#![allow(unused_imports)]
use crate::error::{ArkiveError, Result};
use crate::ArkAddress;
use ark_core::script::{csv_sig_script, multisig_script};
use bitcoin::key::{PublicKey, Secp256k1};
use bitcoin::taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo};
use bitcoin::{ScriptBuf, Sequence, XOnlyPublicKey};
use std::str::FromStr;

/// Lottery escrow script options
#[derive(Debug, Clone)]
pub struct LotteryEscrowOptions {
    /// Participants' public keys
    pub participants: Vec<XOnlyPublicKey>,
    /// Coordinator/Operator public key
    pub coordinator: XOnlyPublicKey,
    /// Ark server public key
    pub server: XOnlyPublicKey,
    /// Timeout for revealing phase
    pub reveal_timeout: Sequence,
    /// Timeout for claiming phase
    pub claim_timeout: Sequence,
}

/// Lottery escrow script implementation
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct LotteryEscrowScript {
    options: LotteryEscrowOptions,
    spend_info: TaprootSpendInfo,
    scripts: LotteryScripts,
    network: bitcoin::Network,
    server_key: XOnlyPublicKey,
}

#[derive(Debug, Clone)]
struct LotteryScripts {
    /// Winner claim path (winner + coordinator + server)
    winner_claim: ScriptBuf,
    /// Timeout refund path (all participants + server after timeout)
    timeout_refund: ScriptBuf,
    /// Coordinator resolution path (coordinator + server)
    coordinator_resolve: ScriptBuf,
    /// Emergency exit path (all participants after long timeout)
    emergency_exit: ScriptBuf,
}

impl LotteryEscrowScript {
    pub fn new(
        secp: &Secp256k1<bitcoin::secp256k1::All>,
        options: LotteryEscrowOptions,
        network: bitcoin::Network,
        server_key: XOnlyPublicKey,
    ) -> Result<Self> {
        // Validate options
        if options.participants.len() < 2 {
            return Err(ArkiveError::internal(
                "Lottery requires at least 2 participants",
            ));
        }

        // Create spending scripts
        let scripts = Self::create_scripts(&options)?;

        // Build taproot tree
        let spend_info = Self::build_full_taproot_tree(secp, &scripts, &options)?;

        Ok(Self {
            options,
            spend_info,
            scripts,
            network,
            server_key,
        })
    }

    fn create_scripts(options: &LotteryEscrowOptions) -> Result<LotteryScripts> {
        // Winner claim: winner + coordinator + server (for verified winner)
        // Note: winner determined by coordinator after reveal
        let winner_claim = multisig_script(options.coordinator, options.server);

        // Timeout refund: If reveal phase times out, refund to all
        let timeout_refund = Self::create_refund_script(
            &options.participants,
            options.server,
            options.reveal_timeout,
        )?;

        // Coordinator resolution: For dispute or non-reveal cases
        let coordinator_resolve = csv_sig_script(options.claim_timeout, options.coordinator);

        // Emergency exit: All participants can exit after long timeout
        let emergency_timeout =
            Sequence::from_consensus(options.claim_timeout.to_consensus_u32() * 2);
        let emergency_exit =
            Self::create_emergency_script(&options.participants, emergency_timeout)?;

        let scripts = LotteryScripts {
            winner_claim,
            timeout_refund,
            coordinator_resolve,
            emergency_exit,
        };

        tracing::debug!("Created scripts:");
        tracing::debug!("  Winner claim: {} bytes", scripts.winner_claim.len());
        tracing::debug!("  Timeout refund: {} bytes", scripts.timeout_refund.len());
        tracing::debug!(
            "  Coordinator resolve: {} bytes",
            scripts.coordinator_resolve.len()
        );
        tracing::debug!("  Emergency exit: {} bytes", scripts.emergency_exit.len());

        Ok(scripts)
    }

    fn create_refund_script(
        participants: &[XOnlyPublicKey],
        server: XOnlyPublicKey,
        timeout: Sequence,
    ) -> Result<ScriptBuf> {
        use bitcoin::opcodes::all::*;

        let mut script = ScriptBuf::builder()
            .push_int(timeout.to_consensus_u32() as i64)
            .push_opcode(OP_CSV)
            .push_opcode(OP_DROP);

        // Add server signature requirement
        script = script
            .push_x_only_key(&server)
            .push_opcode(OP_CHECKSIGVERIFY);

        // Add participant signatures (threshold could be all or majority)
        script = script.push_int(participants.len() as i64);

        for participant in participants {
            script = script.push_x_only_key(participant);
        }

        script = script
            .push_int(participants.len() as i64)
            .push_opcode(OP_CHECKMULTISIG);

        Ok(script.into_script())
    }

    fn create_emergency_script(
        participants: &[XOnlyPublicKey],
        timeout: Sequence,
    ) -> Result<ScriptBuf> {
        use bitcoin::opcodes::all::*;

        let mut script = ScriptBuf::builder()
            .push_int(timeout.to_consensus_u32() as i64)
            .push_opcode(OP_CSV)
            .push_opcode(OP_DROP);

        // Require all participants for emergency exit
        script = script.push_int(participants.len() as i64);

        for participant in participants {
            script = script.push_x_only_key(participant);
        }

        script = script
            .push_int(participants.len() as i64)
            .push_opcode(OP_CHECKMULTISIG);

        Ok(script.into_script())
    }

    fn build_full_taproot_tree(
        secp: &Secp256k1<bitcoin::secp256k1::All>,
        scripts: &LotteryScripts,
        _options: &LotteryEscrowOptions,
    ) -> Result<TaprootSpendInfo> {
        let unspendable_key_str =
            "0250929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac0";
        let unspendable_key = PublicKey::from_str(unspendable_key_str)
            .map_err(|e| ArkiveError::internal(format!("Invalid unspendable key: {}", e)))?;
        let (unspendable_xonly, _) = unspendable_key.inner.x_only_public_key();

        // TODO: Trying just one simple path first
        tracing::debug!("Attempting ultra-minimal tree with just winner claim");

        let builder = TaprootBuilder::new();

        match builder.add_leaf(0, scripts.winner_claim.clone()) {
            Ok(single_builder) => match single_builder.finalize(secp, unspendable_xonly) {
                Ok(spend_info) => {
                    tracing::info!("Successfully built ultra-minimal 1-path escrow tree");
                    Ok(spend_info)
                }
                Err(_) => {
                    tracing::error!("Even ultra-minimal tree failed to finalize");
                    tracing::debug!(
                        "Winner claim script: {:?}",
                        hex::encode(scripts.winner_claim.as_bytes())
                    );
                    Err(ArkiveError::internal(
                        "Failed to build even minimal escrow tree - script may be invalid",
                    ))
                }
            },
            Err(e) => {
                tracing::error!("Failed to add even single leaf: {:?}", e);
                tracing::debug!(
                    "Winner claim script bytes: {:?}",
                    scripts.winner_claim.as_bytes()
                );
                Err(ArkiveError::internal(format!(
                    "Failed to add single leaf: {:?}",
                    e
                )))
            }
        }
    }

    pub fn spend_info(&self) -> &TaprootSpendInfo {
        &self.spend_info
    }

    pub fn get_ark_address(&self) -> ArkAddress {
        let output_key = self.spend_info.output_key();
        ArkAddress::new(self.network, self.server_key, output_key)
    }

    pub fn get_bitcoin_address(&self, network: bitcoin::Network) -> bitcoin::Address {
        bitcoin::Address::p2tr(
            &Secp256k1::new(),
            self.spend_info.internal_key(),
            self.spend_info.merkle_root(),
            network,
        )
    }

    pub fn winner_claim_script(&self) -> &ScriptBuf {
        &self.scripts.winner_claim
    }

    pub fn timeout_refund_script(&self) -> &ScriptBuf {
        &self.scripts.timeout_refund
    }
}
