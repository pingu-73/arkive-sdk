use crate::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Trait for commitment schemes
pub trait CommitmentScheme {
    type Secret;
    type Commitment;

    fn commit(secret: Self::Secret) -> Self::Commitment;
    fn verify(commitment: &Self::Commitment, secret: &Self::Secret) -> bool;
}

/// A commitment in the lottery protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub hash: Vec<u8>,
    pub player_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub nonce: Vec<u8>, // to prevent replay
}

impl Commitment {
    pub fn new(hash: Vec<u8>, player_id: Uuid) -> Self {
        let mut nonce = vec![0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);

        Self {
            hash,
            player_id,
            timestamp: Utc::now(),
            nonce,
        }
    }

    pub fn verify_secret(&self, secret: &[u8]) -> Result<bool> {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(secret);
        hasher.update(&self.nonce);
        let computed_hash = hasher.finalize();

        Ok(computed_hash.as_slice() == self.hash)
    }

    pub fn create_with_secret(secret: &[u8], player_id: Uuid) -> Self {
        use sha2::{Digest, Sha256};

        let mut nonce = vec![0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);

        let mut hasher = Sha256::new();
        hasher.update(secret);
        hasher.update(&nonce);
        let hash = hasher.finalize().to_vec();

        Self {
            hash,
            player_id,
            timestamp: Utc::now(),
            nonce,
        }
    }
}
