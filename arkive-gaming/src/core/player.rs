use ark_core::ArkAddress;
use bitcoin::key::Keypair;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Player {
    pub id: String,
    pub wallet_id: String,
    pub ark_address: ArkAddress,
    pub public_key: String, // Hex-encoded public key
    pub joined_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializablePlayer {
    pub id: String,
    pub wallet_id: String,
    pub ark_address: String, // string 'coz ArkAddress isn't serializable
    pub public_key: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
}

impl Player {
    pub fn new(
        id: String,
        wallet_id: String,
        ark_address: ArkAddress,
        keypair: &Keypair,
    ) -> Self {
        let public_key = hex::encode(keypair.public_key().serialize());
        
        Self {
            id,
            wallet_id,
            ark_address,
            public_key,
            joined_at: chrono::Utc::now(),
        }
    }

    pub fn to_serializable(&self) -> SerializablePlayer {
        SerializablePlayer {
            id: self.id.clone(),
            wallet_id: self.wallet_id.clone(),
            ark_address: self.ark_address.to_string(),
            public_key: self.public_key.clone(),
            joined_at: self.joined_at,
        }
    }

    pub fn from_serializable(serializable: SerializablePlayer) -> Result<Self, String> {
        let ark_address = ArkAddress::decode(&serializable.ark_address)
            .map_err(|e| format!("Invalid ark address: {}", e))?;

        Ok(Self {
            id: serializable.id,
            wallet_id: serializable.wallet_id,
            ark_address,
            public_key: serializable.public_key,
            joined_at: serializable.joined_at,
        })
    }
}