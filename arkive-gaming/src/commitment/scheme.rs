use crate::error::{GamingError, Result};
use bitcoin::hashes::{sha256, Hash};
use rand::{thread_rng, RngCore};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub hash: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitmentData {
    pub value: u64,
    pub nonce: [u8; 32],
    pub player_id: String,
    pub game_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reveal {
    pub commitment_data: CommitmentData,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub struct CommitmentScheme;

impl CommitmentScheme {
    /// Create a new commitment to a value
    pub fn commit(
        value: u64,
        player_id: &str,
        game_id: &str,
    ) -> Result<(Commitment, CommitmentData)> {
        let mut nonce = [0u8; 32];
        thread_rng().fill_bytes(&mut nonce);

        let commitment_data = CommitmentData {
            value,
            nonce,
            player_id: player_id.to_string(),
            game_id: game_id.to_string(),
        };

        let hash = Self::hash_commitment(&commitment_data)?;

        let commitment = Commitment {
            hash,
            timestamp: chrono::Utc::now(),
        };

        Ok((commitment, commitment_data))
    }

    /// Verify a revealed commitment
    pub fn verify(commitment: &Commitment, reveal: &Reveal) -> Result<bool> {
        let calculated_hash = Self::hash_commitment(&reveal.commitment_data)?;
        
        if calculated_hash != commitment.hash {
            return Ok(false);
        }

        // Verify timestamp ordering (reveal must come after commitment)
        if reveal.timestamp <= commitment.timestamp {
            return Ok(false);
        }

        Ok(true)
    }

    /// Hash the commitment data using SHA256
    fn hash_commitment(data: &CommitmentData) -> Result<String> {
        let serialized = serde_json::to_vec(data)
            .map_err(|e| GamingError::commitment(format!("Failed to serialize commitment: {}", e)))?;
        
        let hash = sha256::Hash::hash(&serialized);
        Ok(hash.to_string())
    }

    /// Generate random value for commitment (used for randomness)
    pub fn generate_random_value() -> u64 {
        thread_rng().next_u64()
    }

    /// Combine two revealed values to determine winner (XOR-based)
    pub fn determine_winner(reveal1: &Reveal, reveal2: &Reveal) -> Result<String> {
        let combined = reveal1.commitment_data.value ^ reveal2.commitment_data.value;
        
        // Player 1 wins if result is even, Player 2 wins if odd
        if combined % 2 == 0 {
            Ok(reveal1.commitment_data.player_id.clone())
        } else {
            Ok(reveal2.commitment_data.player_id.clone())
        }
    }

    /// Verify that two commitments are different (prevent replay attacks)
    pub fn verify_different_commitments(c1: &Commitment, c2: &Commitment) -> bool {
        c1.hash != c2.hash
    }
}