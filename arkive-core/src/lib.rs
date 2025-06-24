//! ARKive SDK - Core library for Bitcoin and Ark protocol operations
//!
//! This library provides a clean, wallet-centric API for managing Bitcoin
//! and Ark protocol operations with proper isolation between wallets.

pub mod ark;
pub mod backup;
pub mod bitcoin;
pub mod error;
pub mod storage;
pub mod sync;
pub mod types;
pub mod wallet;

pub use error::{ArkiveError, Result};
pub use types::{Address, Balance, Transaction};
pub use wallet::{ArkWallet, WalletConfig, WalletManager};

pub use backup::{BackupManager, EncryptedBackup, WalletBackup};
pub use sync::{SyncConflict, SyncManager, SyncPackage};

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

    #[tokio::test]
    async fn test_backup_restore() {
        let temp_dir = tempdir().unwrap();
        let manager = WalletManager::new(temp_dir.path()).await.unwrap();

        let (wallet, _) = manager
            .create_wallet("backup-test", Network::Regtest)
            .await
            .unwrap();

        // Create backup
        let backup = wallet.create_backup("test_password").await.unwrap();
        assert_eq!(backup.encryption_method, "ChaCha20Poly1305");

        // Test backup manager
        let backup_manager = wallet.get_backup_manager();
        let restored_id = backup_manager
            .restore_backup(&backup, "test_password")
            .await
            .unwrap();
        assert_eq!(restored_id, wallet.id());
    }

    #[tokio::test]
    #[ignore]
    async fn test_sync_initialization() {
        let temp_dir = tempdir().unwrap();
        let manager = WalletManager::new(temp_dir.path()).await.unwrap();

        let (wallet, _) = manager
            .create_wallet("sync-test", Network::Regtest)
            .await
            .unwrap();

        // Initialize sync
        wallet.init_sync().await.unwrap();

        // Check sync state
        let sync_manager = wallet.get_sync_manager();
        let state = sync_manager.get_sync_state(wallet.id()).await.unwrap();
        assert!(state.is_some());
    }
}
