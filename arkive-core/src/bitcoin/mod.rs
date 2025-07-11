use crate::ark::TransactionManager;
use crate::error::{ArkiveError, Result};
use crate::storage::Storage;
use crate::types::{Transaction, TransactionSource, TransactionStatus, TransactionType};
use crate::wallet::WalletConfig;

use bitcoin::key::Keypair;
use bitcoin::Amount;
use esplora_client::AsyncClient;
use std::sync::Arc;

pub struct BitcoinService {
    keypair: Keypair,
    config: WalletConfig,
    client: AsyncClient,
    tx_manager: TransactionManager,
}

impl BitcoinService {
    pub async fn new(
        keypair: Keypair,
        config: WalletConfig,
        storage: Arc<Storage>,
        wallet_id: String,
    ) -> Result<Self> {
        let client = esplora_client::Builder::new(&config.esplora_url)
            .build_async()
            .map_err(|e| ArkiveError::esplora(format!("Failed to create esplora client: {}", e)))?;

        let tx_manager = TransactionManager::new(storage, wallet_id.clone());

        Ok(Self {
            keypair,
            config,
            client,
            tx_manager,
        })
    }

    pub async fn get_address(&self) -> Result<String> {
        let pubkey = self.keypair.public_key();
        let pubkey_bytes = pubkey.serialize();
        let wpkh = bitcoin::key::CompressedPublicKey::from_slice(&pubkey_bytes)
            .map_err(|e| ArkiveError::internal(format!("Failed to create WPKH: {}", e)))?;
        let address = bitcoin::Address::p2wpkh(&wpkh, self.config.network);
        Ok(address.to_string())
    }

    pub async fn get_balance(&self) -> Result<Amount> {
        let address_str = self.get_address().await?;
        let address = bitcoin::Address::from_str(&address_str)
            .map_err(|e| ArkiveError::bitcoin(format!("Invalid address: {}", e)))?
            .assume_checked();

        let script_pubkey = address.script_pubkey();

        // Get UTXOs for this address
        let txs = self
            .client
            .scripthash_txs(&script_pubkey, None)
            .await
            .map_err(|e| ArkiveError::esplora(format!("Failed to get transactions: {}", e)))?;

        let mut balance = Amount::ZERO;

        for tx in txs {
            for (vout, output) in tx.vout.iter().enumerate() {
                if output.scriptpubkey == script_pubkey {
                    // Check if this output is unspent
                    let is_spent = match self.client.get_output_status(&tx.txid, vout as u64).await
                    {
                        Ok(Some(status)) => status.spent,
                        Ok(None) => false,
                        Err(_) => false, // Assume unspent if we can't check
                    };

                    if !is_spent {
                        balance += Amount::from_sat(output.value);
                    }
                }
            }
        }

        Ok(balance)
    }

    #[allow(unused_variables)]
    pub async fn send(&self, address: &str, amount: Amount) -> Result<String> {
        // This is a simplified implementation
        // In a real implementation, you would:
        // 1. Build a proper transaction with UTXO selection
        // 2. Sign the transaction
        // 3. Broadcast it

        // For now, return a placeholder
        Err(ArkiveError::internal("Bitcoin sending not yet implemented"))
    }

    pub async fn get_transaction_history(&self) -> Result<Vec<Transaction>> {
        let address_str = self.get_address().await?;
        let address = bitcoin::Address::from_str(&address_str)
            .map_err(|e| ArkiveError::bitcoin(format!("Invalid address: {}", e)))?
            .assume_checked();

        let script_pubkey = address.script_pubkey();

        let txs = self
            .client
            .scripthash_txs(&script_pubkey, None)
            .await
            .map_err(|e| ArkiveError::esplora(format!("Failed to get transactions: {}", e)))?;

        // Record tx using tx manager
        for tx in &txs {
            let mut net_amount = 0i64;

            // Calculate net amount for this tx
            for output in &tx.vout {
                if output.scriptpubkey == script_pubkey {
                    net_amount += output.value as i64;
                }
            }

            if net_amount != 0 {
                // Use tx manager to record if new
                self.tx_manager
                    .record_transaction_if_new(
                        &tx.txid.to_string(),
                        net_amount,
                        TransactionType::OnChain,
                        TransactionSource::Blockchain,
                    )
                    .await?;

                // Update status based on confirmation
                let status = if tx.status.confirmed {
                    TransactionStatus::Confirmed
                } else {
                    TransactionStatus::Pending
                };

                self.tx_manager
                    .update_transaction_status(&tx.txid.to_string(), status, None)
                    .await?;
            }
        }

        self.tx_manager
            .get_transaction_history_by_type(TransactionType::OnChain)
            .await
    }

    pub async fn sync(&self) -> Result<()> {
        // [TODO] Placeholder for sync logic
        Ok(())
    }

    pub async fn estimate_fee(&self, _address: &str, _amount: Amount) -> Result<Amount> {
        // [TODO] Placeholder fee estimation
        Ok(Amount::from_sat(1000))
    }
}

use std::str::FromStr;
