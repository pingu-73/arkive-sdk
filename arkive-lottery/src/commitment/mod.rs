pub mod scheme;

pub use scheme::{Commitment, CommitmentScheme};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Hash based commitment impl
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashCommitment {
    hash: Vec<u8>,
    #[serde(skip)]
    secret: Option<Vec<u8>>,
}

impl HashCommitment {
    pub fn new(secret: Vec<u8>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(&secret);
        let hash = hasher.finalize().to_vec();

        Self {
            hash,
            secret: Some(secret),
        }
    }

    pub fn from_hash(hash: Vec<u8>) -> Self {
        Self { hash, secret: None }
    }

    pub fn hash(&self) -> &[u8] {
        &self.hash
    }

    pub fn verify(&self, secret: &[u8]) -> bool {
        let mut hasher = Sha256::new();
        hasher.update(secret);
        let computed_hash = hasher.finalize();
        computed_hash.as_slice() == self.hash
    }

    pub fn reveal(self) -> Option<Vec<u8>> {
        self.secret
    }
}

/// Rnd secret for commitment
pub fn generate_secret() -> Vec<u8> {
    let mut secret = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut secret);
    secret
}

/// Winner from two secrets using XOR
pub fn determine_winner(secret1: &[u8], secret2: &[u8]) -> bool {
    let combined = secret1
        .iter()
        .zip(secret2.iter())
        .map(|(a, b)| a ^ b)
        .collect::<Vec<u8>>();

    let winner_bit = combined.iter().fold(0u8, |acc, &byte| acc ^ byte) & 1;
    winner_bit == 0 // true = player1 wins, false = player2 wins
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commitment_scheme() {
        let secret = generate_secret();
        let commitment = HashCommitment::new(secret.clone());

        assert!(commitment.verify(&secret));
        assert!(!commitment.verify(b"wrong secret"));
    }

    #[test]
    fn test_winner_determination() {
        let secret1 = vec![0x00, 0x00, 0x00, 0x00];
        let secret2 = vec![0x00, 0x00, 0x00, 0x01];

        let winner = determine_winner(&secret1, &secret2);
        assert!(!winner);
    }
}
