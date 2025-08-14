pub mod escrow_scripts;
pub mod lottery_coordinator;
pub mod service;

pub use escrow_scripts::{LotteryEscrowOptions, LotteryEscrowScript};
pub use lottery_coordinator::{LotteryCoordinator, LotteryEscrow, LotteryState};
pub use service::GameService;

use crate::error::Result;
use async_trait::async_trait;
use bitcoin::{Amount, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

/// Base trait for all trustless games
#[async_trait]
pub trait TrustlessGame: Send + Sync {
    /// Unique game identifier
    fn game_id(&self) -> &str;

    /// Minimum number of players
    fn min_players(&self) -> usize;

    /// Maximum number of players
    fn max_players(&self) -> usize;

    /// Create game escrow script
    async fn create_escrow_script(
        &self,
        participants: &[XOnlyPublicKey],
        entry_fee: Amount,
    ) -> Result<GameEscrow>;

    /// Generate fairness proof
    async fn generate_fairness_proof(&self) -> Result<FairnessProof>;

    /// Verify game outcome
    async fn verify_outcome(
        &self,
        commits: &[Commitment],
        reveals: &[Reveal],
    ) -> Result<GameOutcome>;

    /// Execute payout according to game rules
    async fn execute_payout(
        &self,
        escrow: &GameEscrow,
        outcome: &GameOutcome,
    ) -> Result<PayoutTransaction>;
}

/// Game escrow holding funds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEscrow {
    pub escrow_id: [u8; 32],
    pub taproot_address: String,
    #[serde(with = "xonly_pubkey_serde")]
    pub internal_key: XOnlyPublicKey,
    pub script_tree: EscrowScriptTree,
    pub total_stake: Amount,
    pub participants: Vec<Participant>,
    pub timeout: u32,
    pub state: EscrowState,
}

/// Participant in a game
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Participant {
    #[serde(with = "xonly_pubkey_serde")]
    pub pubkey: XOnlyPublicKey,
    pub ark_address: String,
    pub stake: Amount,
    pub commitment: Option<Commitment>,
    pub reveal: Option<Reveal>,
    #[serde(with = "optional_script_buf_serde")]
    pub payout_script: Option<bitcoin::ScriptBuf>,
}

/// Commitment for provable fairness
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub hash: [u8; 32],
    pub timestamp: u64,
    #[serde(with = "schnorr_sig_serde")]
    pub signature: bitcoin::secp256k1::schnorr::Signature,
}

/// Reveal for commitment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reveal {
    pub preimage: [u8; 32],
    pub nonce: [u8; 32],
    #[serde(with = "schnorr_sig_serde")]
    pub signature: bitcoin::secp256k1::schnorr::Signature,
}

/// Game outcome
#[derive(Debug, Clone)]
pub struct GameOutcome {
    pub winner: XOnlyPublicKey,
    pub runner_ups: Vec<XOnlyPublicKey>,
    pub payouts: HashMap<XOnlyPublicKey, Amount>,
    pub proof: OutcomeProof,
}

/// Proof of fair outcome
#[derive(Debug, Clone)]
pub struct OutcomeProof {
    pub seed: [u8; 32],
    pub commitments: Vec<Commitment>,
    pub reveals: Vec<Reveal>,
    pub calculation: Vec<u8>,
}

/// Fairness proof for the game
#[derive(Debug, Clone)]
pub struct FairnessProof {
    pub algorithm: String,
    pub parameters: HashMap<String, String>,
    pub verifier_script: bitcoin::ScriptBuf,
}

/// Payout transaction
#[derive(Debug, Clone)]
pub struct PayoutTransaction {
    pub psbt: bitcoin::Psbt,
    pub signatures_required: Vec<XOnlyPublicKey>,
    pub timeout_block: u32,
}

/// Escrow state
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EscrowState {
    Gathering,
    Locked,
    CommitPhase,
    RevealPhase,
    Resolved,
    Refunded,
    Expired,
}

/// Escrow script tree for Taproot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowScriptTree {
    #[serde(with = "script_buf_serde")]
    pub cooperative_payout: bitcoin::ScriptBuf,
    #[serde(with = "script_buf_serde")]
    pub timeout_refund: bitcoin::ScriptBuf,
    #[serde(with = "script_buf_serde")]
    pub dispute_resolution: bitcoin::ScriptBuf,
    #[serde(with = "optional_script_buf_serde")]
    pub emergency_key: Option<bitcoin::ScriptBuf>,
}

// Serialization helpers
mod xonly_pubkey_serde {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(key: &XOnlyPublicKey, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&key.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<XOnlyPublicKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        XOnlyPublicKey::from_str(&s).map_err(serde::de::Error::custom)
    }
}

mod schnorr_sig_serde {
    use super::*;
    use bitcoin::hashes::hex::FromHex;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(
        sig: &bitcoin::secp256k1::schnorr::Signature,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(sig.as_ref()))
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> std::result::Result<bitcoin::secp256k1::schnorr::Signature, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = Vec::<u8>::from_hex(&s).map_err(serde::de::Error::custom)?;
        bitcoin::secp256k1::schnorr::Signature::from_slice(&bytes).map_err(serde::de::Error::custom)
    }
}

mod script_buf_serde {
    use super::*;
    use bitcoin::hashes::hex::FromHex;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(
        script: &bitcoin::ScriptBuf,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(script.to_bytes()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<bitcoin::ScriptBuf, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = Vec::<u8>::from_hex(&s).map_err(serde::de::Error::custom)?;
        Ok(bitcoin::ScriptBuf::from(bytes))
    }
}

mod optional_script_buf_serde {
    use super::*;
    use bitcoin::hashes::hex::FromHex;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(
        script: &Option<bitcoin::ScriptBuf>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match script {
            Some(s) => serializer.serialize_str(&hex::encode(s.to_bytes())),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> std::result::Result<Option<bitcoin::ScriptBuf>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt = Option::<String>::deserialize(deserializer)?;
        match opt {
            Some(s) => {
                let bytes = Vec::<u8>::from_hex(&s).map_err(serde::de::Error::custom)?;
                Ok(Some(bitcoin::ScriptBuf::from(bytes)))
            }
            None => Ok(None),
        }
    }
}
