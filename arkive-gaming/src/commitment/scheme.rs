#![allow(unused_imports)]
use crate::error::{GamingError, Result};
use bitcoin::hashes::{sha256, Hash, HashEngine};
use rand::{thread_rng, RngCore};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub hash: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub commitment_type: CommitmentType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CommitmentType {
    RandomValue,
    GameMove,
    WinnerDetermination,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitmentData {
    pub value: u64,
    pub nonce: [u8; 32],
    pub player_id: String,
    pub game_id: String,
    pub commitment_type: CommitmentType,
    pub additional_entropy: [u8; 32], // Extra randomness for security
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reveal {
    pub commitment_data: CommitmentData,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub proof_of_work: Option<u64>, // Optional PoW for anti-spam
}

pub struct CommitmentScheme;

impl CommitmentScheme {
    /// Create a cryptographically secure commitment
    pub fn commit(
        value: u64,
        player_id: &str,
        game_id: &str,
        commitment_type: CommitmentType,
    ) -> Result<(Commitment, CommitmentData)> {
        let mut nonce = [0u8; 32];
        let mut additional_entropy = [0u8; 32];
        thread_rng().fill_bytes(&mut nonce);
        thread_rng().fill_bytes(&mut additional_entropy);

        let commitment_data = CommitmentData {
            value,
            nonce,
            player_id: player_id.to_string(),
            game_id: game_id.to_string(),
            commitment_type: commitment_type.clone(),
            additional_entropy,
        };

        let hash = Self::hash_commitment(&commitment_data)?;

        let commitment = Commitment {
            hash,
            timestamp: chrono::Utc::now(),
            commitment_type,
        };

        Ok((commitment, commitment_data))
    }

    /// Verify a revealed commitment with enhanced security checks
    pub fn verify(commitment: &Commitment, reveal: &Reveal) -> Result<bool> {
        // Verify hash matches
        let calculated_hash = Self::hash_commitment(&reveal.commitment_data)?;
        if calculated_hash != commitment.hash {
            return Ok(false);
        }

        // Verify timestamp ordering
        if reveal.timestamp <= commitment.timestamp {
            return Ok(false);
        }

        // Verify commitment types match
        if commitment.commitment_type != reveal.commitment_data.commitment_type {
            return Ok(false);
        }

        // Verify minimum time delay (prevents immediate reveals)
        let min_delay = chrono::Duration::seconds(5);
        if reveal.timestamp - commitment.timestamp < min_delay {
            return Ok(false);
        }

        Ok(true)
    }

    /// Enhanced hash function with domain separation
    fn hash_commitment(data: &CommitmentData) -> Result<String> {
        let mut engine = sha256::HashEngine::default();

        // Domain separation
        engine.input(b"ARK_LOTTERY_COMMITMENT_V1");

        // Serialize commitment data deterministically
        engine.input(&data.value.to_le_bytes());
        engine.input(&data.nonce);
        engine.input(data.player_id.as_bytes());
        engine.input(data.game_id.as_bytes());
        engine.input(&data.additional_entropy);

        // Add commitment type
        let type_bytes = match data.commitment_type {
            CommitmentType::RandomValue => b"RANDOM",
            CommitmentType::GameMove => b"GAMEMV",
            CommitmentType::WinnerDetermination => b"WINNER",
        };
        engine.input(type_bytes);

        let hash = sha256::Hash::from_engine(engine);
        Ok(hash.to_string())
    }

    /// Cryptographically secure random value generation
    pub fn generate_random_value() -> u64 {
        let mut bytes = [0u8; 8];
        thread_rng().fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    /// Determine winner using verifiable randomness
    pub fn determine_winner(reveal1: &Reveal, reveal2: &Reveal) -> Result<String> {
        // Combine randomness from both players
        let combined = reveal1.commitment_data.value ^ reveal2.commitment_data.value;

        // Add additional entropy for extra security
        let entropy1 = u64::from_le_bytes([
            reveal1.commitment_data.additional_entropy[0],
            reveal1.commitment_data.additional_entropy[1],
            reveal1.commitment_data.additional_entropy[2],
            reveal1.commitment_data.additional_entropy[3],
            reveal1.commitment_data.additional_entropy[4],
            reveal1.commitment_data.additional_entropy[5],
            reveal1.commitment_data.additional_entropy[6],
            reveal1.commitment_data.additional_entropy[7],
        ]);

        let entropy2 = u64::from_le_bytes([
            reveal2.commitment_data.additional_entropy[0],
            reveal2.commitment_data.additional_entropy[1],
            reveal2.commitment_data.additional_entropy[2],
            reveal2.commitment_data.additional_entropy[3],
            reveal2.commitment_data.additional_entropy[4],
            reveal2.commitment_data.additional_entropy[5],
            reveal2.commitment_data.additional_entropy[6],
            reveal2.commitment_data.additional_entropy[7],
        ]);

        let final_randomness = combined ^ entropy1 ^ entropy2;

        // Player 1 wins if result is even, Player 2 wins if odd
        if final_randomness % 2 == 0 {
            Ok(reveal1.commitment_data.player_id.clone())
        } else {
            Ok(reveal2.commitment_data.player_id.clone())
        }
    }

    /// Verify that commitments are cryptographically different
    pub fn verify_different_commitments(c1: &Commitment, c2: &Commitment) -> bool {
        c1.hash != c2.hash
    }

    /// Generate commitment hash for tapscript (32 bytes)
    pub fn commitment_hash_for_script(commitment: &Commitment) -> [u8; 32] {
        let hash = sha256::Hash::from_str(&commitment.hash)
            .unwrap_or_else(|_| sha256::Hash::hash(commitment.hash.as_bytes()));
        hash.to_byte_array()
    }
}
