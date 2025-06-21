//! ARKive SDK - Core library for Bitcoin and Ark protocol operations
//!
//! This library provides a clean, wallet-centric API for managing Bitcoin
//! and Ark protocol operations with proper isolation between wallets.

pub mod ark;
pub mod bitcoin;
pub mod error;
pub mod storage;
pub mod types;
pub mod wallet;

pub use error::{ArkiveError, Result};
pub use types::{Address, Balance, Transaction};
pub use wallet::{ArkWallet, WalletConfig, WalletManager};

pub use ::bitcoin::Amount;
pub use ::bitcoin::Network;
pub use ark_core::ArkAddress;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_wallet_creation() {
        let temp_dir = tempdir().unwrap();
        let manager = WalletManager::new(temp_dir.path()).await.unwrap();

        let (wallet, _mnemonic) = manager
            .create_wallet("test-wallet", Network::Regtest)
            .await
            .unwrap();
        assert_eq!(wallet.name(), "test-wallet");
        assert_eq!(wallet.network(), Network::Regtest);
    }
}
