use crate::ark::ArkService;
use crate::bitcoin::BitcoinService;
use crate::error::{ArkiveError, Result};
use crate::storage::Storage;
use crate::types::{Address, AddressType, Balance, BatchSwapInfo, Transaction, VtxoInfo};
use crate::wallet::WalletConfig;

use ark_core::ArkAddress;
use bitcoin::key::Keypair;
use bitcoin::{Amount, Network};
use std::sync::Arc;

#[allow(dead_code)]
pub struct ArkWallet {
    id: String,
    name: String,
    keypair: Keypair,
    config: WalletConfig,
    bitcoin_service: BitcoinService,
    ark_service: ArkService,
    storage: Arc<Storage>,
}

impl ArkWallet {
    pub async fn new(
        id: String,
        name: String,
        keypair: Keypair,
        config: WalletConfig,
        storage: Arc<Storage>,
    ) -> Result<Self> {
        let bitcoin_service =
            BitcoinService::new(keypair, config.clone(), storage.clone(), id.clone()).await?;

        let ark_service =
            ArkService::new(keypair, config.clone(), storage.clone(), id.clone()).await?;

        Ok(Self {
            id,
            name,
            keypair,
            config,
            bitcoin_service,
            ark_service,
            storage,
        })
    }

    // Wallet metadata
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn network(&self) -> Network {
        self.config.network
    }

    pub fn is_mutinynet(&self) -> bool {
        self.config.is_mutinynet
    }

    pub fn network_display(&self) -> String {
        if self.config.is_mutinynet {
            "Mutinynet".to_string()
        } else {
            format!("{:?}", self.config.network)
        }
    }

    // Address generation
    pub async fn get_onchain_address(&self) -> Result<Address> {
        let address = self.bitcoin_service.get_address().await?;
        Ok(Address {
            address,
            address_type: AddressType::OnChain,
        })
    }

    pub async fn get_ark_address(&self) -> Result<Address> {
        let address = self.ark_service.get_address().await?;
        Ok(Address {
            address,
            address_type: AddressType::Ark,
        })
    }

    pub async fn get_boarding_address(&self) -> Result<Address> {
        let address = self.ark_service.get_boarding_address().await?;
        Ok(Address {
            address,
            address_type: AddressType::Boarding,
        })
    }

    // Balance operations
    pub async fn balance(&self) -> Result<Balance> {
        let onchain_balance = self.bitcoin_service.get_balance().await?;
        let (ark_confirmed, ark_pending) = self.ark_service.get_balance().await?;

        Ok(Balance::new(onchain_balance + ark_confirmed, ark_pending))
    }

    pub async fn onchain_balance(&self) -> Result<Amount> {
        self.bitcoin_service.get_balance().await
    }

    pub async fn ark_balance(&self) -> Result<(Amount, Amount)> {
        self.ark_service.get_balance().await
    }

    // Tx operations
    pub async fn send_onchain(&self, address: &str, amount: Amount) -> Result<String> {
        self.bitcoin_service.send(address, amount).await
    }

    pub async fn send_ark(&self, address: &str, amount: Amount) -> Result<String> {
        let ark_address = ArkAddress::decode(address)
            .map_err(|e| ArkiveError::InvalidAddress(format!("Invalid Ark address: {}", e)))?;

        // Check balance before sending
        let (confirmed, pending) = self.ark_service.get_balance().await?;
        let total_bal = confirmed + pending;
        if total_bal < amount {
            return Err(ArkiveError::InsufficientFunds {
                need: amount.to_sat(),
                available: total_bal.to_sat(),
            });
        }

        self.ark_service.send(ark_address, amount).await
    }

    // VTXO operations
    pub async fn list_vtxos(&self) -> Result<Vec<VtxoInfo>> {
        self.ark_service.list_vtxos().await
    }

    /// Get spendable VTXOs (confirmed, preconfirmed, and recoverable)
    pub async fn get_spendable_vtxos(&self) -> Result<Vec<VtxoInfo>> {
        let vtxos = self.ark_service.list_vtxos().await?;
        let spendable = vtxos
            .into_iter()
            .filter(|vtxo| {
                matches!(
                    vtxo.status,
                    crate::types::VtxoStatus::Confirmed
                        | crate::types::VtxoStatus::Preconfirmed
                        | crate::types::VtxoStatus::Pending
                ) || vtxo.is_recoverable
            })
            .collect();
        Ok(spendable)
    }

    /// Get preconfirmed VTXOs that need batch swap
    pub async fn get_preconfirmed_vtxos(&self) -> Result<Vec<VtxoInfo>> {
        let vtxos = self.ark_service.list_vtxos().await?;
        let preconfirmed = vtxos
            .into_iter()
            .filter(|vtxo| matches!(vtxo.status, crate::types::VtxoStatus::Preconfirmed))
            .collect();
        Ok(preconfirmed)
    }

    /// Get recoverable VTXOs
    pub async fn get_recoverable_vtxos(&self) -> Result<Vec<VtxoInfo>> {
        let vtxos = self.ark_service.list_vtxos().await?;
        let recoverable = vtxos
            .into_iter()
            .filter(|vtxo| vtxo.is_recoverable)
            .collect();
        Ok(recoverable)
    }

    /// Initiate batch swap
    pub async fn batch_swap(&self, vtxos_to_swap: Option<Vec<String>>) -> Result<Option<String>> {
        self.ark_service.batch_swap(vtxos_to_swap).await
    }

    /// Get batch swap information
    pub async fn get_batch_swap_info(&self, swap_id: &str) -> Result<Option<BatchSwapInfo>> {
        self.ark_service.get_batch_swap_info(swap_id).await
    }

    /// List all batch swaps
    pub async fn list_batch_swaps(&self) -> Result<Vec<BatchSwapInfo>> {
        self.ark_service.list_batch_swaps().await
    }

    pub async fn participate_in_round(&self) -> Result<Option<String>> {
        self.ark_service.participate_in_round().await
    }

    // Tx history
    pub async fn transaction_history(&self) -> Result<Vec<Transaction>> {
        let mut transactions = Vec::new();

        // Get onchain tx
        let onchain_txs = self.bitcoin_service.get_transaction_history().await?;
        transactions.extend(onchain_txs);

        // Get Ark tx
        let ark_txs = self.ark_service.get_transaction_history().await?;
        transactions.extend(ark_txs);

        // Sort by timestamp
        transactions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(transactions)
    }

    // Sync operations
    pub async fn sync(&self) -> Result<()> {
        // Sync both services
        self.bitcoin_service.sync().await?;
        self.ark_service.sync().await?;

        // Cleanup expired VTXOs after sync
        let cleaned = self.cleanup_expired_data().await?;
        if cleaned > 0 {
            tracing::info!("Cleaned up {} expired VTXOs", cleaned);
        }

        // Check for preconfirmed VTXOs that need batch swap
        let preconfirmed = self.get_preconfirmed_vtxos().await?;
        if !preconfirmed.is_empty() {
            tracing::info!(
                "Found {} preconfirmed VTXOs that may need batch swap",
                preconfirmed.len()
            );
        }

        Ok(())
    }

    // Utility methods
    pub async fn estimate_onchain_fee(&self, address: &str, amount: Amount) -> Result<Amount> {
        self.bitcoin_service.estimate_fee(address, amount).await
    }

    pub async fn estimate_ark_fee(&self, amount: Amount) -> Result<Amount> {
        self.ark_service.estimate_fee(amount).await
    }

    /// Get backup manager for this wallet
    pub fn get_backup_manager(&self) -> crate::backup::BackupManager {
        crate::backup::BackupManager::new(self.storage.clone())
    }

    /// Get sync manager for this wallet
    pub fn get_sync_manager(&self) -> crate::sync::SyncManager {
        crate::sync::SyncManager::new(self.storage.clone())
    }

    /// Create encrypted backup
    pub async fn create_backup(&self, password: &str) -> Result<crate::backup::EncryptedBackup> {
        let backup_manager = self.get_backup_manager();
        backup_manager.create_backup(&self.id, password).await
    }

    /// Export backup to file
    pub async fn export_backup(&self, password: &str, file_path: &str) -> Result<()> {
        let backup_manager = self.get_backup_manager();
        backup_manager
            .export_to_file(&self.id, password, file_path)
            .await
    }

    /// Initialize sync for this wallet
    pub async fn init_sync(&self) -> Result<()> {
        let sync_manager = self.get_sync_manager();
        sync_manager.init_sync(&self.id).await
    }

    /// Create sync package for sharing with other devices
    pub async fn create_sync_package(&self) -> Result<crate::sync::SyncPackage> {
        let sync_manager = self.get_sync_manager();
        sync_manager.create_sync_package(&self.id).await
    }

    /// Get sync conflicts that need resolution
    pub async fn get_sync_conflicts(&self) -> Result<Vec<crate::sync::SyncConflict>> {
        let sync_manager = self.get_sync_manager();
        sync_manager.get_conflicts(&self.id).await
    }

    /// Get VTXOs approaching expiry (for proactive management)
    pub async fn get_expiring_vtxos(
        &self,
        hours_threshold: i64,
    ) -> Result<Vec<crate::storage::vtxo_store::VtxoState>> {
        // let vtxo_store = crate::storage::VtxoStore::new(&self.storage);
        // vtxo_store
        //     .get_expiring_vtxos(&self.id, hours_threshold)
        //     .await
        self.ark_service.get_expiring_vtxos(hours_threshold).await
    }

    /// Clean up expired VTXOs and old data
    pub async fn cleanup_expired_data(&self) -> Result<usize> {
        let vtxo_store = crate::storage::VtxoStore::new(&self.storage);
        vtxo_store.cleanup_expired(&self.id).await
    }

    pub async fn auto_manage_vtxos(&self, hours_threshold: i64) -> Result<Option<String>> {
        let expiring_vtxos = self.get_expiring_vtxos(hours_threshold).await?;

        if expiring_vtxos.is_empty() {
            return Ok(None);
        }

        tracing::info!(
            "Found {} VTXOs expiring within {} hours, initiating batch swap",
            expiring_vtxos.len(),
            hours_threshold
        );

        let vtxo_outpoints: Vec<String> =
            expiring_vtxos.iter().map(|v| v.outpoint.clone()).collect();

        self.batch_swap(Some(vtxo_outpoints)).await
    }

    /// New v0.7 method: Get detailed VTXO statistics
    pub async fn get_vtxo_statistics(&self) -> Result<VtxoStatistics> {
        let vtxos = self.list_vtxos().await?;

        let mut stats = VtxoStatistics::default();

        for vtxo in vtxos {
            stats.total_count += 1;
            stats.total_value += vtxo.amount;

            match vtxo.status {
                crate::types::VtxoStatus::Confirmed => {
                    stats.confirmed_count += 1;
                    stats.confirmed_value += vtxo.amount;
                }
                crate::types::VtxoStatus::Preconfirmed => {
                    stats.preconfirmed_count += 1;
                    stats.preconfirmed_value += vtxo.amount;
                }
                crate::types::VtxoStatus::Pending => {
                    stats.pending_count += 1;
                    stats.pending_value += vtxo.amount;
                }
                crate::types::VtxoStatus::Spent => {
                    stats.spent_count += 1;
                    stats.spent_value += vtxo.amount;
                }
                crate::types::VtxoStatus::Expired => {
                    stats.expired_count += 1;
                    stats.expired_value += vtxo.amount;
                }
                _ => {}
            }

            if vtxo.is_recoverable {
                stats.recoverable_count += 1;
                stats.recoverable_value += vtxo.amount;
            }

            // Check if expiring soon (within 24 hours)
            let now = chrono::Utc::now();
            let threshold = now + chrono::Duration::hours(24);
            if vtxo.expiry <= threshold {
                stats.expiring_soon_count += 1;
                stats.expiring_soon_value += vtxo.amount;
            }
        }

        Ok(stats)
    }
}

#[derive(Debug, Clone, Default)]
pub struct VtxoStatistics {
    pub total_count: usize,
    pub total_value: Amount,
    pub confirmed_count: usize,
    pub confirmed_value: Amount,
    pub preconfirmed_count: usize,
    pub preconfirmed_value: Amount,
    pub pending_count: usize,
    pub pending_value: Amount,
    pub spent_count: usize,
    pub spent_value: Amount,
    pub expired_count: usize,
    pub expired_value: Amount,
    pub recoverable_count: usize,
    pub recoverable_value: Amount,
    pub expiring_soon_count: usize,
    pub expiring_soon_value: Amount,
}
