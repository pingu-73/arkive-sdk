use crate::error::{ArkiveError, Result};
use crate::storage::wallet_store::WalletData;
use crate::storage::{Storage, WalletStore};
use crate::wallet::{generate_mnemonic, mnemonic_to_keypair, ArkWallet, WalletConfig};
use bitcoin::Network;
use chrono::Utc;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

pub struct WalletManager {
    storage: Arc<Storage>,
    wallets: Arc<RwLock<HashMap<String, Arc<ArkWallet>>>>,
}

impl WalletManager {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("arkive.db");
        let storage = Arc::new(Storage::new(&db_path).await?);

        Ok(Self {
            storage,
            wallets: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn create_wallet(
        &self,
        name: &str,
        network: Network,
    ) -> Result<(Arc<ArkWallet>, String)> {
        // Check if wallet already exists
        let wallet_store = WalletStore::new(&self.storage);
        if wallet_store.wallet_exists(name).await? {
            return Err(ArkiveError::config(format!(
                "Wallet '{}' already exists",
                name
            )));
        }

        // Generate mnemonic and keypair
        let mnemonic = generate_mnemonic()?;
        let keypair = mnemonic_to_keypair(&mnemonic, network)?;

        // Create wallet config
        let config = WalletConfig::new(network);
        config.validate()?;

        // Create wallet data
        let wallet_id = Uuid::new_v4().to_string();
        let wallet_data = WalletData {
            id: wallet_id.clone(),
            name: name.to_string(),
            network,
            created_at: Utc::now(),
            encrypted_seed: self.encrypt_seed(&mnemonic)?,
            config: Some(serde_json::to_string(&config)?),
        };

        // Save to storage
        wallet_store.save_wallet(&wallet_data).await?;

        // Create wallet instance
        let wallet = Arc::new(
            ArkWallet::new(
                wallet_id.clone(),
                name.to_string(),
                keypair,
                config,
                self.storage.clone(),
            )
            .await?,
        );

        // Cache the wallet
        {
            let mut wallets = self.wallets.write();
            wallets.insert(wallet_id, wallet.clone());
        }

        tracing::info!("Created wallet '{}' with ID: {}", name, wallet.id());
        Ok((wallet, mnemonic))
    }

    pub async fn create_wallet_mutinynet(&self, name: &str) -> Result<(Arc<ArkWallet>, String)> {
        // Check if wallet already exists
        let wallet_store = WalletStore::new(&self.storage);
        if wallet_store.wallet_exists(name).await? {
            return Err(ArkiveError::config(format!(
                "Wallet '{}' already exists",
                name
            )));
        }

        // Generate mnemonic and keypair
        let mnemonic = generate_mnemonic()?;
        let keypair = mnemonic_to_keypair(&mnemonic, Network::Signet)?;

        // Create Mutinynet config
        let config = WalletConfig::new_mutinynet();
        config.validate()?;

        // Create wallet data
        let wallet_id = Uuid::new_v4().to_string();
        let wallet_data = WalletData {
            id: wallet_id.clone(),
            name: name.to_string(),
            network: Network::Signet,
            created_at: Utc::now(),
            encrypted_seed: self.encrypt_seed(&mnemonic)?,
            config: Some(serde_json::to_string(&config)?),
        };

        // Save to storage
        wallet_store.save_wallet(&wallet_data).await?;

        // Create wallet instance
        let wallet = Arc::new(
            ArkWallet::new(
                wallet_id.clone(),
                name.to_string(),
                keypair,
                config,
                self.storage.clone(),
            )
            .await?,
        );

        // Cache the wallet
        {
            let mut wallets = self.wallets.write();
            wallets.insert(wallet_id, wallet.clone());
        }

        tracing::info!(
            "Created Mutinynet wallet '{}' with ID: {}",
            name,
            wallet.id()
        );
        Ok((wallet, mnemonic))
    }

    pub async fn import_wallet_mutinynet(
        &self,
        name: &str,
        mnemonic: &str,
    ) -> Result<Arc<ArkWallet>> {
        // Check if wallet already exists
        let wallet_store = WalletStore::new(&self.storage);
        if wallet_store.wallet_exists(name).await? {
            return Err(ArkiveError::config(format!(
                "Wallet '{}' already exists",
                name
            )));
        }

        // Validate mnemonic and create keypair
        let keypair = mnemonic_to_keypair(mnemonic, Network::Signet)?;

        // Create Mutinynet config
        let config = WalletConfig::new_mutinynet();
        config.validate()?;

        // Create wallet data
        let wallet_id = Uuid::new_v4().to_string();
        let wallet_data = WalletData {
            id: wallet_id.clone(),
            name: name.to_string(),
            network: Network::Signet,
            created_at: Utc::now(),
            encrypted_seed: self.encrypt_seed(mnemonic)?,
            config: Some(serde_json::to_string(&config)?),
        };

        // Save to storage
        wallet_store.save_wallet(&wallet_data).await?;

        // Create wallet instance
        let wallet = Arc::new(
            ArkWallet::new(
                wallet_id.clone(),
                name.to_string(),
                keypair,
                config,
                self.storage.clone(),
            )
            .await?,
        );

        // Cache the wallet
        {
            let mut wallets = self.wallets.write();
            wallets.insert(wallet_id, wallet.clone());
        }

        tracing::info!(
            "Imported Mutinynet wallet '{}' with ID: {}",
            name,
            wallet.id()
        );
        Ok(wallet)
    }

    pub async fn load_wallet(&self, name: &str) -> Result<Arc<ArkWallet>> {
        // Check cache first
        {
            let wallets = self.wallets.read();
            for wallet in wallets.values() {
                if wallet.name() == name {
                    return Ok(wallet.clone());
                }
            }
        }

        // Load from storage
        let wallet_store = WalletStore::new(&self.storage);
        let wallets_data = wallet_store.list_wallets().await?;

        let wallet_data = wallets_data
            .into_iter()
            .find(|w| w.name == name)
            .ok_or_else(|| ArkiveError::WalletNotFound {
                name: name.to_string(),
            })?;

        // Decrypt seed and create keypair
        let mnemonic = self.decrypt_seed(&wallet_data.encrypted_seed)?;
        let keypair = mnemonic_to_keypair(&mnemonic, wallet_data.network)?;

        // Parse config
        let config = if let Some(config_str) = &wallet_data.config {
            serde_json::from_str(config_str)?
        } else {
            WalletConfig::new(wallet_data.network)
        };

        // Create wallet instance
        let wallet = Arc::new(
            ArkWallet::new(
                wallet_data.id.clone(),
                wallet_data.name.clone(),
                keypair,
                config,
                self.storage.clone(),
            )
            .await?,
        );

        // Cache the wallet
        {
            let mut wallets = self.wallets.write();
            wallets.insert(wallet_data.id, wallet.clone());
        }

        Ok(wallet)
    }

    pub async fn list_wallets(&self) -> Result<Vec<String>> {
        let wallet_store = WalletStore::new(&self.storage);
        let wallets_data = wallet_store.list_wallets().await?;
        Ok(wallets_data.into_iter().map(|w| w.name).collect())
    }

    pub async fn delete_wallet(&self, name: &str) -> Result<()> {
        let wallet_store = WalletStore::new(&self.storage);
        let wallets_data = wallet_store.list_wallets().await?;

        let wallet_data = wallets_data
            .into_iter()
            .find(|w| w.name == name)
            .ok_or_else(|| ArkiveError::WalletNotFound {
                name: name.to_string(),
            })?;

        // Remove from cache
        {
            let mut wallets = self.wallets.write();
            wallets.remove(&wallet_data.id);
        }

        // Delete from storage
        wallet_store.delete_wallet(&wallet_data.id).await?;

        tracing::info!("Deleted wallet '{}'", name);
        Ok(())
    }

    pub async fn import_wallet(
        &self,
        name: &str,
        mnemonic: &str,
        network: Network,
    ) -> Result<Arc<ArkWallet>> {
        // Check if wallet already exists
        let wallet_store = WalletStore::new(&self.storage);
        if wallet_store.wallet_exists(name).await? {
            return Err(ArkiveError::config(format!(
                "Wallet '{}' already exists",
                name
            )));
        }

        // Validate mnemonic and create keypair
        let keypair = mnemonic_to_keypair(mnemonic, network)?;

        // Create wallet config
        let config = WalletConfig::new(network);
        config.validate()?;

        // Create wallet data
        let wallet_id = Uuid::new_v4().to_string();
        let wallet_data = WalletData {
            id: wallet_id.clone(),
            name: name.to_string(),
            network,
            created_at: Utc::now(),
            encrypted_seed: self.encrypt_seed(mnemonic)?,
            config: Some(serde_json::to_string(&config)?),
        };

        // Save to storage
        wallet_store.save_wallet(&wallet_data).await?;

        // Create wallet instance
        let wallet = Arc::new(
            ArkWallet::new(
                wallet_id.clone(),
                name.to_string(),
                keypair,
                config,
                self.storage.clone(),
            )
            .await?,
        );

        // Cache the wallet
        {
            let mut wallets = self.wallets.write();
            wallets.insert(wallet_id, wallet.clone());
        }

        tracing::info!("Imported wallet '{}' with ID: {}", name, wallet.id());
        Ok(wallet)
    }

    fn encrypt_seed(&self, mnemonic: &str) -> Result<Vec<u8>> {
        // [TODO] Impl proper encryption with user password/keychain
        Ok(mnemonic.as_bytes().to_vec())
    }

    fn decrypt_seed(&self, encrypted_seed: &[u8]) -> Result<String> {
        // [TODO] Impl proper decryption
        String::from_utf8(encrypted_seed.to_vec())
            .map_err(|e| ArkiveError::internal(format!("Failed to decrypt seed: {}", e)))
    }
}
