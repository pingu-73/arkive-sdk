use crate::Result;
use arkive_core::{Amount, ArkWallet};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

pub type PlayerId = Uuid;

/// Player state in any game
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlayerState {
    Joined,
    BetPlaced,
    Committed,
    Revealed,
    Winner,
    Loser,
    Forfeited,
}

/// Generic player for 2-player games
#[derive(Clone)]
pub struct Player {
    id: PlayerId,
    wallet: Arc<ArkWallet>,
    state: PlayerState,
}

impl Player {
    pub async fn new(wallet: Arc<ArkWallet>) -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4(),
            wallet,
            state: PlayerState::Joined,
        })
    }

    pub fn id(&self) -> PlayerId {
        self.id
    }

    pub fn wallet(&self) -> &ArkWallet {
        &self.wallet
    }

    pub fn state(&self) -> &PlayerState {
        &self.state
    }

    pub fn set_state(&mut self, state: PlayerState) {
        self.state = state;
    }

    /// Place a bet by sending to escrow addr
    pub async fn place_bet(&self, escrow_address: &str, amount: Amount) -> Result<String> {
        let txid = self.wallet.send_ark(escrow_address, amount).await?;
        tracing::info!(
            "Player {} placed bet of {} sats: {}",
            self.id,
            amount.to_sat(),
            txid
        );
        Ok(txid)
    }

    /// Get player's current balance
    pub async fn get_balance(&self) -> Result<arkive_core::Balance> {
        Ok(self.wallet.balance().await?)
    }

    /// Get player's Ark addr for payouts
    pub async fn get_ark_address(&self) -> Result<String> {
        let addr = self.wallet.get_ark_address().await?;
        Ok(addr.address)
    }
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("wallet", &"<ArkWallet>")
            .finish()
    }
}
