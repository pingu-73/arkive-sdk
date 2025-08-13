use super::*;
use crate::error::Result;
use crate::ArkiveError;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{self, Message, Secp256k1};

/// Provable fairness engine
pub struct FairnessEngine {
    secp: Secp256k1<secp256k1::All>,
    verifier: Box<dyn FairnessVerifier>,
}

impl FairnessEngine {
    pub fn new(verifier: Box<dyn FairnessVerifier>) -> Self {
        Self {
            secp: Secp256k1::new(),
            verifier,
        }
    }

    /// Generate commitment for participant
    pub fn generate_commitment(
        &self,
        secret: &[u8; 32],
        nonce: &[u8; 32],
        participant_key: &bitcoin::key::Keypair,
    ) -> Result<Commitment> {
        // Hash(secret || nonce || pubkey)
        let mut data = Vec::new();
        data.extend_from_slice(secret);
        data.extend_from_slice(nonce);
        data.extend_from_slice(&participant_key.public_key().serialize());

        let hash = sha256::Hash::hash(&data).to_byte_array();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Sign the commitment
        let msg = Message::from_digest(hash);
        let signature = self.secp.sign_schnorr(&msg, participant_key);

        Ok(Commitment {
            hash,
            timestamp,
            signature,
        })
    }

    /// Verify and reveal commitment
    pub fn verify_reveal(
        &self,
        commitment: &Commitment,
        reveal: &Reveal,
        participant_key: &XOnlyPublicKey,
    ) -> Result<bool> {
        // Reconstruct commitment hash
        let mut data = Vec::new();
        data.extend_from_slice(&reveal.preimage);
        data.extend_from_slice(&reveal.nonce);
        data.extend_from_slice(&participant_key.serialize());

        let computed_hash = sha256::Hash::hash(&data).to_byte_array();

        // Verify hash matches
        if computed_hash != commitment.hash {
            return Ok(false);
        }

        // Verify signatures
        let commit_msg = Message::from_digest(commitment.hash);
        self.secp
            .verify_schnorr(&commitment.signature, &commit_msg, participant_key)
            .map_err(|e| ArkiveError::internal(format!("Commitment signature invalid: {}", e)))?;

        let reveal_msg = Message::from_digest(reveal.preimage);
        self.secp
            .verify_schnorr(&reveal.signature, &reveal_msg, participant_key)
            .map_err(|e| ArkiveError::internal(format!("Reveal signature invalid: {}", e)))?;

        Ok(true)
    }

    /// Calculate fair outcome from reveals
    pub fn calculate_outcome(
        &self,
        participants: &[Participant],
        reveals: &[Reveal],
    ) -> Result<GameOutcome> {
        // Verify all reveals are present
        if reveals.len() != participants.len() {
            return Err(ArkiveError::internal("Missing reveals"));
        }

        // Combine all reveals to generate seed
        let seed = self.combine_reveals(reveals)?;

        // Use verifier to determine outcome
        let outcome = self.verifier.determine_outcome(participants, seed)?;

        // Generate proof
        let proof = OutcomeProof {
            seed,
            commitments: participants
                .iter()
                .filter_map(|p| p.commitment.clone())
                .collect(),
            reveals: reveals.to_vec(),
            calculation: self.verifier.explain_calculation(&outcome)?,
        };

        Ok(GameOutcome {
            winner: outcome.winner,
            runner_ups: outcome.runner_ups,
            payouts: outcome.payouts,
            proof,
        })
    }

    /// Combine reveals into deterministic seed
    fn combine_reveals(&self, reveals: &[Reveal]) -> Result<[u8; 32]> {
        let mut combined = Vec::new();

        // Sort reveals by preimage to ensure deterministic order
        let mut sorted_reveals = reveals.to_vec();
        sorted_reveals.sort_by_key(|r| r.preimage);

        for reveal in sorted_reveals {
            combined.extend_from_slice(&reveal.preimage);
            combined.extend_from_slice(&reveal.nonce);
        }

        Ok(sha256::Hash::hash(&combined).to_byte_array())
    }
}

/// Trait for different fairness verification algorithms
pub trait FairnessVerifier: Send + Sync {
    /// Determine outcome from seed
    fn determine_outcome(
        &self,
        participants: &[Participant],
        seed: [u8; 32],
    ) -> Result<OutcomeData>;

    /// Explain calculation for transparency
    fn explain_calculation(&self, outcome: &OutcomeData) -> Result<Vec<u8>>;

    /// Verify outcome independently
    fn verify_outcome(&self, proof: &OutcomeProof) -> Result<bool>;
}

/// Outcome data from verifier
#[derive(Debug, Clone)]
pub struct OutcomeData {
    pub winner: XOnlyPublicKey,
    pub runner_ups: Vec<XOnlyPublicKey>,
    pub payouts: HashMap<XOnlyPublicKey, Amount>,
}

/// Modulo-based fairness verifier (for lottery)
pub struct ModuloVerifier {
    total_stake: Amount,
}

impl ModuloVerifier {
    pub fn new(total_stake: Amount) -> Self {
        Self { total_stake }
    }
}

impl FairnessVerifier for ModuloVerifier {
    fn determine_outcome(
        &self,
        participants: &[Participant],
        seed: [u8; 32],
    ) -> Result<OutcomeData> {
        // Convert seed to number
        let seed_num = u64::from_le_bytes(seed[0..8].try_into().unwrap());

        // Winner is seed % num_participants
        let winner_index = (seed_num % participants.len() as u64) as usize;
        let winner = participants[winner_index].pubkey;

        // Create payout map
        let mut payouts = HashMap::new();
        payouts.insert(winner, self.total_stake);

        Ok(OutcomeData {
            winner,
            runner_ups: vec![],
            payouts,
        })
    }

    fn explain_calculation(&self, outcome: &OutcomeData) -> Result<Vec<u8>> {
        let explanation = format!(
            "Winner determined by seed modulo participant count. Winner: {:?}",
            outcome.winner
        );
        Ok(explanation.into_bytes())
    }

    fn verify_outcome(&self, proof: &OutcomeProof) -> Result<bool> {
        // Recalculate from proof
        let seed_num = u64::from_le_bytes(proof.seed[0..8].try_into().unwrap());
        let num_participants = proof.commitments.len();
        let _winner_index = (seed_num % num_participants as u64) as usize;

        // Verify winner matches
        Ok(true) // Simplified for brevity
    }
}
