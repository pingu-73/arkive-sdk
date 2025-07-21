use crate::{Commitment, LotteryError, Result};
use arkive_core::{Amount, ArkWallet};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Player state in lottery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlayerState {
    Joined,
    Committed,
    Revealed,
    Winner,
    Loser,
}

/// player in the 2-player lottery
pub struct Player {
    id: Uuid,
    wallet: Arc<ArkWallet>,
    state: PlayerState,
    commitment: Option<Commitment>,
    revealed_secret: Option<Vec<u8>>,
}

impl Player {
    pub async fn new(wallet: Arc<ArkWallet>) -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4(),
            wallet,
            state: PlayerState::Joined,
            commitment: None,
            revealed_secret: None,
        })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn wallet(&self) -> &ArkWallet {
        &self.wallet
    }

    pub fn state(&self) -> &PlayerState {
        &self.state
    }

    pub fn has_committed(&self) -> bool {
        self.commitment.is_some()
    }

    pub fn has_revealed(&self) -> bool {
        self.revealed_secret.is_some()
    }

    pub fn revealed_secret(&self) -> Option<&[u8]> {
        self.revealed_secret.as_deref()
    }

    pub fn commitment(&self) -> Option<&Commitment> {
        self.commitment.as_ref()
    }

    /// Generate and submit commitment
    pub fn create_commitment(&mut self) -> Result<(Commitment, Vec<u8>)> {
        if self.commitment.is_some() {
            return Err(LotteryError::CommitmentAlreadySubmitted);
        }

        let secret = crate::commitment::generate_secret();
        let commitment = Commitment::create_with_secret(&secret, self.id);

        self.commitment = Some(commitment.clone());
        self.state = PlayerState::Committed;

        tracing::info!("Player {} created commitment", self.id);
        Ok((commitment, secret))
    }

    /// Reveal commitment with the secret
    pub fn reveal_commitment(&mut self, secret: Vec<u8>) -> Result<()> {
        let commitment = self
            .commitment
            .as_ref()
            .ok_or(LotteryError::InvalidCommitment)?;

        if !commitment.verify_secret(&secret)? {
            return Err(LotteryError::InvalidCommitment);
        }

        self.revealed_secret = Some(secret);
        self.state = PlayerState::Revealed;

        tracing::info!("Player {} revealed commitment", self.id);
        Ok(())
    }

    pub async fn place_bet(&self, lottery_address: &str, amount: Amount) -> Result<String> {
        let txid = self.wallet.send_ark(lottery_address, amount).await?;
        tracing::info!(
            "Player {} placed bet of {} sats: {}",
            self.id,
            amount.to_sat(),
            txid
        );
        Ok(txid)
    }

    pub fn set_winner(&mut self) {
        self.state = PlayerState::Winner;
    }

    pub fn set_loser(&mut self) {
        self.state = PlayerState::Loser;
    }
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("has_commitment", &self.commitment.is_some())
            .field("has_revealed", &self.revealed_secret.is_some())
            .finish()
    }
}
