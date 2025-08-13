#![allow(unused_imports)]
use crate::error::{ArkiveError, Result};
use crate::storage::vtxo_store::{VtxoState, VtxoTreeData};
use crate::storage::{BoardingOutputState, BoardingStore};
use crate::storage::{Storage, VtxoStore};
use crate::types::{
    BatchSwapInfo, BatchSwapStatus, Transaction, TransactionSource, TransactionStatus,
    TransactionType, VtxoInfo, VtxoStatus,
};
use crate::wallet::WalletConfig;

use ark_client::{Blockchain, Client, ExplorerUtxo, OfflineClient, SpendStatus};
use ark_core::batch::{
    create_and_sign_forfeit_txs, generate_nonce_tree, sign_batch_tree, sign_commitment_psbt,
    OnChainInput, VtxoInput,
};
use ark_core::coin_select::select_vtxos;
use ark_core::server::{GetVtxosRequest, Info, ListVtxo, VirtualTxOutPoint};
use ark_core::UtxoCoinSelection;
use ark_core::{ArkAddress, Vtxo};
use bip39::rand::rngs::StdRng;
use bip39::rand::SeedableRng;
use bitcoin::key::Keypair;
use bitcoin::{Amount, Network, OutPoint, Psbt};
use chrono::{DateTime, Utc};
use rusqlite::params;
use std::collections::HashMap;
use std::sync::Arc;

// Blockchain implementation for Esplora
pub struct EsploraBlockchain {
    client: esplora_client::AsyncClient,
}

impl EsploraBlockchain {
    pub fn new(url: &str) -> Result<Self> {
        let client = esplora_client::Builder::new(url)
            .build_async()
            .map_err(|e| ArkiveError::esplora(format!("Failed to create esplora client: {}", e)))?;
        Ok(Self { client })
    }
}

impl Blockchain for EsploraBlockchain {
    async fn find_outpoints(
        &self,
        address: &bitcoin::Address,
    ) -> std::result::Result<Vec<ExplorerUtxo>, ark_client::Error> {
        let script_pubkey = address.script_pubkey();

        let txs = self
            .client
            .scripthash_txs(&script_pubkey, None)
            .await
            .map_err(|e| ark_client::Error::wallet(anyhow::anyhow!("Esplora error: {}", e)))?;

        let mut utxos = Vec::new();
        for tx in txs {
            for (vout, output) in tx.vout.iter().enumerate() {
                if output.scriptpubkey == script_pubkey {
                    let outpoint = bitcoin::OutPoint {
                        txid: tx.txid,
                        vout: vout as u32,
                    };

                    let is_spent = match self.client.get_output_status(&tx.txid, vout as u64).await
                    {
                        Ok(Some(status)) => status.spent,
                        Ok(None) => false,
                        Err(_) => false,
                    };

                    utxos.push(ExplorerUtxo {
                        outpoint,
                        amount: bitcoin::Amount::from_sat(output.value),
                        confirmation_blocktime: tx.status.block_time,
                        is_spent,
                    });
                }
            }
        }

        Ok(utxos)
    }

    async fn find_tx(
        &self,
        txid: &bitcoin::Txid,
    ) -> std::result::Result<Option<bitcoin::Transaction>, ark_client::Error> {
        match self.client.get_tx(txid).await {
            Ok(Some(tx)) => {
                let tx_bytes = bitcoin::consensus::serialize(&tx);
                match bitcoin::consensus::deserialize(&tx_bytes) {
                    Ok(tx) => Ok(Some(tx)),
                    Err(e) => Err(ark_client::Error::wallet(anyhow::anyhow!(
                        "Deserialization error: {}",
                        e
                    ))),
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(ark_client::Error::wallet(anyhow::anyhow!(
                "Esplora error: {}",
                e
            ))),
        }
    }

    async fn get_output_status(
        &self,
        txid: &bitcoin::Txid,
        vout: u32,
    ) -> std::result::Result<SpendStatus, ark_client::Error> {
        let status = self
            .client
            .get_output_status(txid, vout as u64)
            .await
            .map_err(|e| ark_client::Error::wallet(anyhow::anyhow!("Esplora error: {}", e)))?;

        Ok(SpendStatus {
            spend_txid: status.and_then(|s| s.txid),
        })
    }

    async fn broadcast(
        &self,
        tx: &bitcoin::Transaction,
    ) -> std::result::Result<(), ark_client::Error> {
        self.client
            .broadcast(tx)
            .await
            .map_err(|e| ark_client::Error::wallet(anyhow::anyhow!("Broadcast error: {}", e)))?;
        Ok(())
    }

    async fn get_fee_rate(&self) -> std::result::Result<f64, ark_client::Error> {
        // TODO: query the fee estimation API
        Ok(10.0)
    }

    async fn broadcast_package(
        &self,
        txs: &[&bitcoin::Transaction],
    ) -> std::result::Result<(), ark_client::Error> {
        // Broadcast multiple tx in sequence
        for tx in txs {
            self.broadcast(tx).await?;
        }
        Ok(())
    }
}

// Wallet implementation for Ark
pub struct ArkWalletImpl {
    keypair: Keypair,
    network: Network,
    storage: Arc<Storage>,
    wallet_id: String,
}

impl ArkWalletImpl {
    pub fn new(
        keypair: Keypair,
        network: Network,
        storage: Arc<Storage>,
        wallet_id: String,
    ) -> Self {
        Self {
            keypair,
            network,
            storage,
            wallet_id,
        }
    }
}

impl ark_client::wallet::BoardingWallet for ArkWalletImpl {
    fn new_boarding_output(
        &self,
        server_pk: bitcoin::XOnlyPublicKey,
        exit_delay: bitcoin::Sequence,
        network: Network,
    ) -> std::result::Result<ark_core::BoardingOutput, ark_client::Error> {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let (owner_pk, _) = self.keypair.x_only_public_key();

        ark_core::BoardingOutput::new(&secp, server_pk, owner_pk, exit_delay, network).map_err(
            |e| {
                ark_client::Error::wallet(anyhow::anyhow!(
                    "Failed to create boarding output: {}",
                    e
                ))
            },
        )
    }

    fn get_boarding_outputs(
        &self,
    ) -> std::result::Result<Vec<ark_core::BoardingOutput>, ark_client::Error> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let boarding_store = BoardingStore::new(&self.storage);
                let boarding_states = boarding_store
                    .load_unspent_boarding_outputs(&self.wallet_id)
                    .await
                    .map_err(|e| {
                        ark_client::Error::wallet(anyhow::anyhow!(
                            "Failed to load boarding outputs: {}",
                            e
                        ))
                    })?;

                let mut boarding_outputs = Vec::new();

                for state in boarding_states {
                    // Use stored params
                    let boarding_output = state.to_boarding_output(self.network).map_err(|e| {
                        ark_client::Error::wallet(anyhow::anyhow!(
                            "Failed to recreate boarding output: {}",
                            e
                        ))
                    })?;

                    // Verify addr
                    if boarding_output.address().to_string() == state.address {
                        boarding_outputs.push(boarding_output);
                        tracing::debug!("Successfully loaded boarding output: {}", state.outpoint);
                    } else {
                        tracing::error!("Address mismatch for boarding output: {}", state.outpoint);
                        tracing::error!("Expected: {}", state.address);
                        tracing::error!("Got: {}", boarding_output.address());
                        // Don't include mismatched boarding outputs
                    }
                }

                tracing::info!("Loaded {} valid boarding outputs", boarding_outputs.len());
                Ok(boarding_outputs)
            })
        })
    }

    fn sign_for_pk(
        &self,
        _pk: &bitcoin::XOnlyPublicKey,
        msg: &bitcoin::secp256k1::Message,
    ) -> std::result::Result<bitcoin::secp256k1::schnorr::Signature, ark_client::Error> {
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let sig = secp.sign_schnorr_no_aux_rand(msg, &self.keypair);
        Ok(sig)
    }
}

impl ark_client::wallet::OnchainWallet for ArkWalletImpl {
    fn get_onchain_address(&self) -> std::result::Result<bitcoin::Address, ark_client::Error> {
        let pubkey = self.keypair.public_key();
        let pubkey_bytes = pubkey.serialize();
        let wpkh = bitcoin::key::CompressedPublicKey::from_slice(&pubkey_bytes).map_err(|e| {
            ark_client::Error::wallet(anyhow::anyhow!("Failed to create WPKH: {}", e))
        })?;
        let address = bitcoin::Address::p2wpkh(&wpkh, self.network);
        Ok(address)
    }

    async fn sync(&self) -> std::result::Result<(), ark_client::Error> {
        // [TODO] Placeholder
        Ok(())
    }

    fn balance(&self) -> std::result::Result<ark_client::wallet::Balance, ark_client::Error> {
        // [TODO] Placeholder
        Ok(ark_client::wallet::Balance {
            confirmed: Amount::ZERO,
            trusted_pending: Amount::ZERO,
            untrusted_pending: Amount::ZERO,
            immature: Amount::ZERO,
        })
    }

    fn prepare_send_to_address(
        &self,
        _address: bitcoin::Address,
        _amount: Amount,
        _fee_rate: bitcoin::FeeRate,
    ) -> std::result::Result<bitcoin::Psbt, ark_client::Error> {
        Err(ark_client::Error::wallet(anyhow::anyhow!(
            "Not implemented"
        )))
    }

    fn sign(&self, _psbt: &mut bitcoin::Psbt) -> std::result::Result<bool, ark_client::Error> {
        Err(ark_client::Error::wallet(anyhow::anyhow!(
            "Not implemented"
        )))
    }

    fn select_coins(
        &self,
        _amount: Amount,
    ) -> std::result::Result<UtxoCoinSelection, ark_client::Error> {
        // TODO: Placeholder
        todo!("to be implemented")
    }
}

pub struct ArkService {
    client: Option<Client<EsploraBlockchain, ArkWalletImpl>>,
    keypair: Keypair,
    config: WalletConfig,
    storage: Arc<Storage>,
    wallet_id: String,
    tx_manager: TransactionManager,
}

impl ArkService {
    pub fn tx_manager(&self) -> &TransactionManager {
        &self.tx_manager
    }
    
    pub async fn new(
        keypair: Keypair,
        config: WalletConfig,
        storage: Arc<Storage>,
        wallet_id: String,
    ) -> Result<Self> {
        let tx_manager = TransactionManager::new(storage.clone(), wallet_id.clone());

        let mut service = Self {
            client: None,
            keypair,
            config,
            storage: storage.clone(),
            wallet_id: wallet_id.clone(),
            tx_manager,
        };

        // Try to connect to Ark server
        if let Err(e) = service.connect().await {
            tracing::warn!("Failed to connect to Ark server: {}", e);
        }

        Ok(service)
    }

    async fn connect(&mut self) -> Result<()> {
        let blockchain = Arc::new(EsploraBlockchain::new(&self.config.esplora_url)?);
        let wallet = Arc::new(ArkWalletImpl::new(
            self.keypair,
            self.config.network,
            self.storage.clone(),
            self.wallet_id.clone(),
        ));

        let offline_client = OfflineClient::new(
            "arkive-sdk".to_string(),
            self.keypair,
            blockchain,
            wallet,
            self.config.ark_server_url.clone(),
        );

        match offline_client.connect().await {
            Ok(client) => {
                self.client = Some(client);
                tracing::info!("Connected to Ark server");
                Ok(())
            }
            Err(e) => Err(ArkiveError::ark(format!(
                "Failed to connect to Ark server: {}",
                e
            ))),
        }
    }

    pub async fn send(&self, address: ArkAddress, amount: Amount) -> Result<String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ArkiveError::internal("Ark server not connected"))?;

        // 1. Get available VTXOs
        let available_vtxos = self.get_spendable_vtxos().await?;
        tracing::info!("Spendable VTXOs found: {}", available_vtxos.len());

        if available_vtxos.is_empty() {
            tracing::error!("No spendable VTXOs found!");

            return Err(ArkiveError::InsufficientFunds {
                need: amount.to_sat(),
                available: 0,
            });
        }

        // 2. Select VTXOs for this transaction
        let vtxo_outpoints: Result<Vec<_>> = available_vtxos
            .iter()
            .map(|v| -> Result<ark_core::coin_select::VirtualTxOutPoint> {
                Ok(ark_core::coin_select::VirtualTxOutPoint {
                    outpoint: bitcoin::OutPoint::from_str(&v.outpoint)
                        .map_err(|e| ArkiveError::internal(format!("Invalid outpoint: {}", e)))?,
                    expire_at: v.expiry.timestamp(),
                    amount: v.amount,
                })
            })
            .collect();

        let vtxo_outpoints = vtxo_outpoints?;

        let selected_outpoints = select_vtxos(
            vtxo_outpoints,
            amount,
            Amount::from_sat(333), // dust limit
            true,                  // allow change
        )
        .map_err(|e| ArkiveError::ark(format!("VTXO selection failed: {}", e)))?;

        let total_input: Amount = selected_outpoints.iter().map(|o| o.amount).sum();
        if total_input < amount {
            return Err(ArkiveError::InsufficientFunds {
                need: amount.to_sat(),
                available: total_input.to_sat(),
            });
        }

        // 3. Submit tx, update vtxo, record tx
        let txid = client
            .send_vtxo(address, amount)
            .await
            .map_err(|e| ArkiveError::ark(format!("Failed to submit transaction: {}", e)))?;

        let txid_str = txid.to_string();
        self.update_vtxo_states_after_send(&selected_outpoints, &txid_str)
            .await?;

        self.tx_manager
            .record_transaction_if_new(
                &txid_str,
                -(amount.to_sat() as i64),
                TransactionType::Ark,
                TransactionSource::LocalRound,
            )
            .await?;

        tracing::info!(
            "Sent {} sats via Ark transaction: {}",
            amount.to_sat(),
            txid_str
        );
        Ok(txid_str)
    }

    pub async fn batch_swap(&self, vtxos_to_swap: Option<Vec<String>>) -> Result<Option<String>> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ArkiveError::internal("Ark server not connected"))?;

        // 1. Get VTXOs that need swapping
        let vtxos_to_swap = if let Some(specific_vtxos) = vtxos_to_swap {
            let all_vtxos = self.get_all_vtxos().await?;
            all_vtxos
                .into_iter()
                .filter(|v| specific_vtxos.contains(&v.outpoint))
                .collect()
        } else {
            let preconfirmed_vtxos = self.get_preconfirmed_vtxos().await?;
            if !preconfirmed_vtxos.is_empty() {
                preconfirmed_vtxos
            } else {
                self.get_expiring_vtxos(24).await?
            }
        };

        if vtxos_to_swap.is_empty() {
            tracing::info!("No VTXOs need batch swap");
            return Ok(None);
        }

        tracing::info!("Initiating batch swap for {} VTXOs", vtxos_to_swap.len());

        // 2. Create batch swap request
        let swap_id = uuid::Uuid::new_v4().to_string();
        let vtxo_outpoints: Vec<String> =
            vtxos_to_swap.iter().map(|v| v.outpoint.clone()).collect();

        let vtxo_store = VtxoStore::new(&self.storage);
        let expires_at = Utc::now() + chrono::Duration::hours(2);
        vtxo_store
            .save_batch_swap(&self.wallet_id, &swap_id, &vtxo_outpoints, expires_at)
            .await?;

        // 3. Use v0.7 settle method for batch swap (this will include existing VTXOs)
        let mut rng = StdRng::from_entropy();

        match client.settle(&mut rng, true).await {
            // true = include recoverable VTXOs
            Ok(Some(commitment_txid)) => {
                let txid_str = commitment_txid.to_string();

                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                self.force_sync_with_server().await?;

                vtxo_store
                    .update_batch_swap_status(
                        &self.wallet_id,
                        &swap_id,
                        BatchSwapStatus::Confirmed,
                        Some(&txid_str),
                    )
                    .await?;

                for vtxo in &vtxos_to_swap {
                    vtxo_store
                        .mark_vtxo_spent(&self.wallet_id, &vtxo.outpoint, Some(&swap_id), None)
                        .await?;
                }

                tracing::info!("Batch swap completed successfully: {}", swap_id);
                tracing::info!("Commitment transaction: {}", txid_str);
                Ok(Some(swap_id))
            }
            Ok(None) => {
                tracing::info!("No batch swap needed");
                Ok(None)
            }
            Err(e) => {
                vtxo_store
                    .update_batch_swap_status(
                        &self.wallet_id,
                        &swap_id,
                        BatchSwapStatus::Failed,
                        None,
                    )
                    .await?;

                Err(ArkiveError::ark(format!("Batch swap failed: {}", e)))
            }
        }
    }

    async fn get_spendable_vtxos(&self) -> Result<Vec<VtxoState>> {
        let vtxo_store = VtxoStore::new(&self.storage);
        vtxo_store.get_spendable_vtxos(&self.wallet_id).await
    }

    /// Get preconfirmed VTXOs
    async fn get_preconfirmed_vtxos(&self) -> Result<Vec<VtxoState>> {
        let vtxo_store = VtxoStore::new(&self.storage);
        vtxo_store.get_preconfirmed_vtxos(&self.wallet_id).await
    }

    /// Get recoverable VTXOs
    #[allow(dead_code)]
    async fn get_recoverable_vtxos(&self) -> Result<Vec<VtxoState>> {
        let vtxo_store = VtxoStore::new(&self.storage);
        vtxo_store.get_recoverable_vtxos(&self.wallet_id).await
    }

    async fn update_vtxo_states_after_send(
        &self,
        spent_outpoints: &[ark_core::coin_select::VirtualTxOutPoint],
        txid: &str,
    ) -> Result<()> {
        let vtxo_store = VtxoStore::new(&self.storage);

        for outpoint in spent_outpoints {
            vtxo_store
                .mark_vtxo_spent(
                    &self.wallet_id,
                    &outpoint.outpoint.to_string(),
                    Some(txid),
                    Some(txid),
                )
                .await?;
        }

        tracing::info!(
            "Updated {} VTXO states after transaction {}",
            spent_outpoints.len(),
            txid
        );
        Ok(())
    }

    pub async fn participate_in_round(&self) -> Result<Option<String>> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ArkiveError::internal("Ark server not connected"))?;

        // 1. Check for preconfirmed VTXOs first
        let preconfirmed_vtxos = self.get_preconfirmed_vtxos().await?;
        if !preconfirmed_vtxos.is_empty() {
            tracing::info!(
                "Found {} preconfirmed VTXOs, initiating batch swap",
                preconfirmed_vtxos.len()
            );
            return self.batch_swap(None).await;
        }

        // 2. Check for boarding outputs
        self.detect_and_store_boarding_outputs().await?;
        let boarding_store = BoardingStore::new(&self.storage);
        let boarding_states = boarding_store
            .load_unspent_boarding_outputs(&self.wallet_id)
            .await?;

        if boarding_states.is_empty() {
            tracing::info!("No boarding outputs or preconfirmed VTXOs to process");
            return Ok(None);
        }

        tracing::info!(
            "Found {} boarding outputs to process",
            boarding_states.len()
        );

        // 3. Use the proper v0.7 settle method
        let mut rng = StdRng::from_entropy();

        match client.settle(&mut rng, false).await {
            // false = don't include recoverable VTXOs
            Ok(Some(commitment_txid)) => {
                let round_id = format!("round_{}", chrono::Utc::now().timestamp());
                let txid_str = commitment_txid.to_string();

                // Wait for settlement to complete
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

                // Sync to get new VTXOs
                self.force_sync_with_server().await?;

                let new_vtxos = self.get_spendable_vtxos().await?;

                if !new_vtxos.is_empty() {
                    // Mark boarding outputs as spent
                    let boarding_outpoints: Vec<bitcoin::OutPoint> =
                        boarding_states.iter().map(|s| s.outpoint).collect();

                    self.tx_manager
                        .mark_boarding_outputs_spent(&boarding_outpoints, &round_id)
                        .await?;

                    for state in &boarding_states {
                        boarding_store
                            .mark_boarding_output_spent(&self.wallet_id, &state.outpoint)
                            .await?;
                    }

                    tracing::info!("Successfully participated in round: {}", round_id);
                    tracing::info!("Commitment transaction: {}", txid_str);
                    Ok(Some(round_id))
                } else {
                    tracing::warn!("Settlement completed but no new VTXOs found");
                    Ok(Some(round_id))
                }
            }
            Ok(None) => {
                tracing::info!("No settlement needed - no inputs to process");
                Ok(None)
            }
            Err(e) => {
                tracing::error!("Settlement failed: {}", e);
                Err(ArkiveError::ark(format!("Settlement failed: {}", e)))
            }
        }
    }

    pub async fn force_sync_with_server(&self) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ArkiveError::internal("Ark server not connected"))?;

        // 1. Get current VTXOs from server using v0.7 APIs
        let ark_address = self.get_address().await?;
        let _address = ArkAddress::decode(&ark_address)?;

        let list_vtxo = client
            .list_vtxos(true)
            .await
            .map_err(|e| ArkiveError::ark(format!("Failed to get VTXOs from server: {}", e)))?;

        // 2. Process spendable VTXOs
        let vtxo_store = VtxoStore::new(&self.storage);
        let existing_vtxos = self.get_all_vtxos().await?;
        let existing_outpoints: std::collections::HashSet<String> =
            existing_vtxos.iter().map(|v| v.outpoint.clone()).collect();

        let mut new_vtxo_count = 0;

        // Process spendable VTXOs
        for (vtxo_outpoints, _vtxo) in &list_vtxo.spendable {
            for vtxo_outpoint in vtxo_outpoints {
                if existing_outpoints.contains(&vtxo_outpoint.outpoint.to_string()) {
                    continue;
                }

                let vtxo_state = self.virtual_outpoint_to_vtxo_state(vtxo_outpoint).await?;
                vtxo_store
                    .save_vtxo_state(&self.wallet_id, &vtxo_state)
                    .await?;

                new_vtxo_count += 1;
                tracing::info!(
                    "Added new spendable VTXO: {} with {} sats (preconfirmed: {})",
                    vtxo_state.outpoint,
                    vtxo_state.amount.to_sat(),
                    vtxo_state.is_preconfirmed
                );
            }
        }

        // Process recoverable VTXOs
        for (vtxo_outpoints, _vtxo) in list_vtxo.spent.iter() {
            for vtxo_outpoint in vtxo_outpoints.iter().filter(|v| v.is_recoverable()) {
                if existing_outpoints.contains(&vtxo_outpoint.outpoint.to_string()) {
                    continue;
                }

                let mut vtxo_state = self.virtual_outpoint_to_vtxo_state(vtxo_outpoint).await?;
                vtxo_state.is_recoverable = true;
                vtxo_state.status = VtxoStatus::Spent;

                vtxo_store
                    .save_vtxo_state(&self.wallet_id, &vtxo_state)
                    .await?;

                new_vtxo_count += 1;
                tracing::info!(
                    "Added new recoverable VTXO: {} with {} sats",
                    vtxo_state.outpoint,
                    vtxo_state.amount.to_sat()
                );
            }
        }

        tracing::info!("Added {} new VTXOs from server during sync", new_vtxo_count);

        Ok(())
    }

    /// Convert VirtualTxOutPoint to VtxoState (issues with expiry calculation)
    async fn virtual_outpoint_to_vtxo_state(
        &self,
        vtxo_outpoint: &VirtualTxOutPoint,
    ) -> Result<VtxoState> {
        let status = if vtxo_outpoint.is_preconfirmed {
            VtxoStatus::Preconfirmed
        } else if vtxo_outpoint.is_spent {
            VtxoStatus::Spent
        } else {
            VtxoStatus::Confirmed
        };

        // Fix expiry calculation - expires_at is likely a duration from creation, not absolute timestamp
        let expiry = if vtxo_outpoint.expires_at < 1_000_000_000 {
            // If it's a small number, treat it as seconds from now
            Utc::now() + chrono::Duration::seconds(vtxo_outpoint.expires_at)
        } else {
            // If it's a large number, treat it as a timestamp
            DateTime::from_timestamp(vtxo_outpoint.expires_at, 0).unwrap_or_else(|| {
                // Fallback: use server's vtxo_tree_expiry
                let client = self.client.as_ref().unwrap();
                let expiry_seconds = client.server_info.vtxo_tree_expiry.to_consensus_u32() as i64;
                Utc::now() + chrono::Duration::seconds(expiry_seconds)
            })
        };

        Ok(VtxoState {
            outpoint: vtxo_outpoint.outpoint.to_string(),
            amount: vtxo_outpoint.amount,
            status,
            expiry,
            address: bitcoin::Address::from_script(&vtxo_outpoint.script, self.config.network)
                .map_err(|e| ArkiveError::internal(format!("Invalid script: {}", e)))?
                .to_string(),
            batch_id: format!("batch_{}", vtxo_outpoint.expires_at),
            tree_path: Vec::new(),
            exit_transactions: Vec::new(),
            is_preconfirmed: vtxo_outpoint.is_preconfirmed,
            is_recoverable: vtxo_outpoint.is_recoverable(),
            commitment_txids: vtxo_outpoint
                .commitment_txids
                .iter()
                .map(|txid| txid.to_string())
                .collect(),
            spent_by: vtxo_outpoint.spent_by.map(|txid| txid.to_string()),
            settled_by: vtxo_outpoint.settled_by.map(|txid| txid.to_string()),
            ark_txid: vtxo_outpoint.ark_txid.map(|txid| txid.to_string()),
        })
    }

    async fn detect_and_store_boarding_outputs(&self) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ArkiveError::internal("Ark server not connected"))?;

        // Get boarding address from the client (this uses correct parameters)
        let boarding_address = self.get_boarding_address().await?;
        let address = bitcoin::Address::from_str(&boarding_address)
            .map_err(|e| ArkiveError::internal(format!("Invalid boarding address: {}", e)))?
            .assume_checked();

        // Use blockchain client to find UTXOs at boarding address
        let blockchain = Arc::new(EsploraBlockchain::new(&self.config.esplora_url)?);
        let utxos = blockchain
            .find_outpoints(&address)
            .await
            .map_err(|e| ArkiveError::ark(format!("Failed to find boarding outputs: {}", e)))?;

        let boarding_store = BoardingStore::new(&self.storage);

        // Store confirmed, unspent boarding outputs
        for utxo in utxos {
            if !utxo.is_spent && utxo.confirmation_blocktime.is_some() {
                let server_pk = client.server_info.pk.x_only_public_key().0;
                let (user_pk, _) = self.keypair.x_only_public_key();

                let exit_delay = client.server_info.boarding_exit_delay.to_consensus_u32();

                let boarding_state = BoardingOutputState {
                    outpoint: utxo.outpoint,
                    amount: utxo.amount,
                    address: boarding_address.clone(),
                    script_pubkey: address.script_pubkey().to_hex_string(),
                    exit_delay,
                    server_pubkey: server_pk.to_string(),
                    user_pubkey: user_pk.to_string(),
                    confirmation_blocktime: utxo
                        .confirmation_blocktime
                        .and_then(|t| DateTime::from_timestamp(t as i64, 0)),
                    is_spent: false,
                    is_mutinynet: self.config.is_mutinynet,
                };

                boarding_store
                    .save_boarding_output(&self.wallet_id, &boarding_state)
                    .await?;

                self.tx_manager
                    .record_transaction_if_new(
                        &utxo.outpoint.txid.to_string(),
                        utxo.amount.to_sat() as i64,
                        TransactionType::Boarding,
                        TransactionSource::Blockchain,
                    )
                    .await?;

                tracing::info!(
                    "Detected and stored boarding output: {} with {} sats",
                    utxo.outpoint,
                    utxo.amount.to_sat()
                );
            }
        }

        Ok(())
    }

    pub async fn sync_with_server(&self) -> Result<()> {
        self.detect_and_store_boarding_outputs().await?;

        self.force_sync_with_server().await?;

        tracing::info!("Synced wallet {} with Ark server", self.wallet_id);
        Ok(())
    }

    pub async fn get_balance(&self) -> Result<(Amount, Amount)> {
        if let Some(client) = &self.client {
            match client.offchain_balance().await {
                Ok(balance) => {
                    tracing::info!(
                        "Server balance - Confirmed: {}, Pending: {}",
                        balance.confirmed().to_sat(),
                        balance.pending().to_sat()
                    );

                    if balance.confirmed().to_sat() == 0 && balance.pending().to_sat() == 0 {
                        let local_balance = self.calculate_local_balance().await?;
                        if local_balance.0.to_sat() > 0 || local_balance.1.to_sat() > 0 {
                            tracing::info!(
                                "Server reports zero balance but local VTXOs found, using local balance"
                            );
                            return Ok(local_balance);
                        }
                    }

                    Ok((balance.confirmed(), balance.pending()))
                }
                Err(e) => {
                    tracing::warn!("Failed to get server balance: {}, falling back to local", e);
                    self.calculate_local_balance().await
                }
            }
        } else {
            self.calculate_local_balance().await
        }
    }

    async fn calculate_local_balance(&self) -> Result<(Amount, Amount)> {
        let vtxos = self.get_all_vtxos().await?;

        let mut confirmed = Amount::ZERO;
        let mut pending = Amount::ZERO;

        for vtxo in vtxos {
            match vtxo.status {
                VtxoStatus::Confirmed => confirmed += vtxo.amount,
                VtxoStatus::Preconfirmed | VtxoStatus::Unconfirmed | VtxoStatus::Pending => {
                    pending += vtxo.amount
                }
                _ => {} // Skip spent/expired/replaced
            }

            // Add recoverable VTXOs to confirmed balance
            if vtxo.is_recoverable && !matches!(vtxo.status, VtxoStatus::Replaced) {
                confirmed += vtxo.amount;
            }
        }

        Ok((confirmed, pending))
    }

    async fn get_all_vtxos(&self) -> Result<Vec<VtxoState>> {
        let vtxo_store = VtxoStore::new(&self.storage);
        vtxo_store.load_vtxo_states(&self.wallet_id).await
    }

    pub async fn list_vtxos(&self) -> Result<Vec<VtxoInfo>> {
        let vtxos = self.get_all_vtxos().await?;

        let vtxo_infos = vtxos
            .into_iter()
            .map(|vtxo| VtxoInfo {
                outpoint: vtxo.outpoint,
                amount: vtxo.amount,
                status: vtxo.status,
                expiry: vtxo.expiry,
                address: vtxo.address,
                is_preconfirmed: vtxo.is_preconfirmed,
                is_recoverable: vtxo.is_recoverable,
                batch_id: Some(vtxo.batch_id),
                commitment_txids: vtxo.commitment_txids,
            })
            .collect();

        Ok(vtxo_infos)
    }

    pub async fn get_transaction_history(&self) -> Result<Vec<Transaction>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT txid, amount, timestamp, tx_type, status, fee, source, ark_round_id, batch_swap_id
             FROM transactions 
             WHERE wallet_id = ?1 
             ORDER BY timestamp DESC",
        )?;

        let transactions = stmt
            .query_map([&self.wallet_id], |row| {
                let tx_type_str: String = row.get(3)?;
                let status_str: String = row.get(4)?;
                let source_str: String = row.get(6)?;

                let tx_type: TransactionType =
                    serde_json::from_str(&tx_type_str).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            3,
                            "tx_type".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?;

                let status: TransactionStatus =
                    serde_json::from_str(&status_str).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            4,
                            "status".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?;

                Ok(Transaction {
                    txid: row.get(0)?,
                    amount: row.get(1)?,
                    timestamp: chrono::DateTime::from_timestamp(row.get::<_, i64>(2)?, 0)
                        .unwrap_or_else(Utc::now),
                    tx_type,
                    status,
                    fee: row
                        .get::<_, Option<i64>>(5)?
                        .map(|f| Amount::from_sat(f as u64)),
                    source: serde_json::from_str(&source_str).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            6,
                            "source".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?,
                    ark_round_id: row.get::<_, Option<String>>(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(transactions)
    }

    pub async fn estimate_fee(&self, amount: Amount) -> Result<Amount> {
        let base_fee = Amount::from_sat(100); // 100 sats base
        let amount_fee = Amount::from_sat(amount.to_sat() / 10000); // 0.01% of amount
        Ok(base_fee + amount_fee)
    }

    pub async fn get_address(&self) -> Result<String> {
        if let Some(client) = &self.client {
            let (address, _) = client
                .get_offchain_address()
                .map_err(|e| ArkiveError::ark(format!("Failed to get address: {}", e)))?;
            Ok(address.to_string())
        } else {
            // Generate address offline
            let secp = bitcoin::secp256k1::Secp256k1::new();
            let (owner_pk, _) = self.keypair.x_only_public_key();

            // Use placeholder server key for offline mode
            let server_pk = bitcoin::XOnlyPublicKey::from_str(
                "33ffb3dee353b1a9ebe4ced64b946238d0a4ac364f275d771da6ad2445d07ae0",
            )
            .map_err(|e| ArkiveError::internal(format!("Invalid server key: {}", e)))?;

            let vtxo = ark_core::Vtxo::new_default(
                &secp,
                server_pk,
                owner_pk,
                bitcoin::Sequence::from_consensus(3600),
                self.config.network,
            )
            .map_err(|e| ArkiveError::internal(format!("Failed to create VTXO: {}", e)))?;

            Ok(vtxo.to_ark_address().to_string())
        }
    }

    pub async fn get_boarding_address(&self) -> Result<String> {
        if let Some(client) = &self.client {
            let address = client
                .get_boarding_address()
                .map_err(|e| ArkiveError::ark(format!("Failed to get boarding address: {}", e)))?;
            Ok(address.to_string())
        } else {
            Err(ArkiveError::internal("Ark server not connected"))
        }
    }

    pub async fn sync(&self) -> Result<()> {
        if self.client.is_some() {
            self.sync_with_server().await
        } else {
            // Try to reconnect
            tracing::warn!("Ark client not connected, skipping sync");
            Ok(())
        }
    }

    // Cleanup expired VTXOs
    pub async fn cleanup_expired_vtxos(&self) -> Result<usize> {
        let vtxo_store = VtxoStore::new(&self.storage);
        vtxo_store.cleanup_expired(&self.wallet_id).await
    }

    // Get VTXOs approaching expiry
    pub async fn get_expiring_vtxos(&self, hours_threshold: i64) -> Result<Vec<VtxoState>> {
        let vtxo_store = VtxoStore::new(&self.storage);
        vtxo_store
            .get_expiring_vtxos(&self.wallet_id, hours_threshold)
            .await
    }

    /// Get batch swap information
    pub async fn get_batch_swap_info(&self, swap_id: &str) -> Result<Option<BatchSwapInfo>> {
        let conn = self.storage.get_connection().await;

        let result = conn.query_row(
            "SELECT swap_id, status, input_vtxos, output_vtxos, commitment_txid, created_at, expires_at
             FROM batch_swaps WHERE wallet_id = ?1 AND swap_id = ?2",
            params![self.wallet_id, swap_id],
            |row| {
                let status_str: String = row.get(1)?;
                let input_vtxos_str: String = row.get(2)?;
                let output_vtxos_str: Option<String> = row.get(3)?;

                let status: BatchSwapStatus = serde_json::from_str(&status_str).map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        1,
                        "status".to_string(),
                        rusqlite::types::Type::Text,
                    )
                })?;

                let input_vtxos: Vec<String> = serde_json::from_str(&input_vtxos_str).map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        2,
                        "input_vtxos".to_string(),
                        rusqlite::types::Type::Text,
                    )
                })?;

                let output_vtxos: Vec<String> = if let Some(output_str) = output_vtxos_str {
                    serde_json::from_str(&output_str).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            3,
                            "output_vtxos".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?
                } else {
                    Vec::new()
                };

                Ok(BatchSwapInfo {
                    swap_id: row.get(0)?,
                    status,
                    input_vtxos,
                    output_vtxos,
                    commitment_txid: row.get(4)?,
                    created_at: DateTime::from_timestamp(row.get::<_, i64>(5)?, 0)
                        .unwrap_or_else(Utc::now),
                    expires_at: DateTime::from_timestamp(row.get::<_, i64>(6)?, 0)
                        .unwrap_or_else(Utc::now),
                })
            },
        );

        match result {
            Ok(info) => Ok(Some(info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(ArkiveError::Storage(e)),
        }
    }

    /// List all batch swaps
    pub async fn list_batch_swaps(&self) -> Result<Vec<BatchSwapInfo>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT swap_id, status, input_vtxos, output_vtxos, commitment_txid, created_at, expires_at
             FROM batch_swaps WHERE wallet_id = ?1 ORDER BY created_at DESC",
        )?;

        let swaps = stmt
            .query_map([&self.wallet_id], |row| {
                let status_str: String = row.get(1)?;
                let input_vtxos_str: String = row.get(2)?;
                let output_vtxos_str: Option<String> = row.get(3)?;

                let status: BatchSwapStatus = serde_json::from_str(&status_str).map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        1,
                        "status".to_string(),
                        rusqlite::types::Type::Text,
                    )
                })?;

                let input_vtxos: Vec<String> =
                    serde_json::from_str(&input_vtxos_str).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            2,
                            "input_vtxos".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?;

                let output_vtxos: Vec<String> = if let Some(output_str) = output_vtxos_str {
                    serde_json::from_str(&output_str).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            3,
                            "output_vtxos".to_string(),
                            rusqlite::types::Type::Text,
                        )
                    })?
                } else {
                    Vec::new()
                };

                Ok(BatchSwapInfo {
                    swap_id: row.get(0)?,
                    status,
                    input_vtxos,
                    output_vtxos,
                    commitment_txid: row.get(4)?,
                    created_at: DateTime::from_timestamp(row.get::<_, i64>(5)?, 0)
                        .unwrap_or_else(Utc::now),
                    expires_at: DateTime::from_timestamp(row.get::<_, i64>(6)?, 0)
                        .unwrap_or_else(Utc::now),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(swaps)
    }
}

pub struct TransactionManager {
    storage: Arc<Storage>,
    wallet_id: String,
}

impl TransactionManager {
    pub fn new(storage: Arc<Storage>, wallet_id: String) -> Self {
        Self { storage, wallet_id }
    }

    pub async fn record_transaction_if_new(
        &self,
        txid: &str,
        amount: i64,
        tx_type: TransactionType,
        source: TransactionSource,
    ) -> Result<bool> {
        let conn = self.storage.get_connection().await;

        // Check if exists
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM transactions WHERE wallet_id = ?1 AND txid = ?2",
            params![self.wallet_id, txid],
            |row| row.get(0),
        )?;

        if exists {
            tracing::debug!("Transaction {} already exists, preserving state", txid);
            return Ok(false);
        }

        // Insert new tx
        conn.execute(
            "INSERT INTO transactions 
             (wallet_id, txid, amount, timestamp, tx_type, status, source, last_updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                self.wallet_id,
                txid,
                amount,
                Utc::now().timestamp(),
                serde_json::to_string(&tx_type)?,
                serde_json::to_string(&TransactionStatus::Pending)?,
                serde_json::to_string(&source)?,
                Utc::now().timestamp(),
            ],
        )?;

        tracing::info!(
            "Recorded new {} transaction: {} ({} sats)",
            format!("{:?}", tx_type),
            txid,
            amount
        );
        Ok(true)
    }

    // Update status with validation
    pub async fn update_transaction_status(
        &self,
        txid: &str,
        new_status: TransactionStatus,
        round_id: Option<String>,
    ) -> Result<bool> {
        let conn = self.storage.get_connection().await;

        let rows_affected = conn.execute(
            "UPDATE transactions 
             SET status = ?1, last_updated = ?2, ark_round_id = COALESCE(?3, ark_round_id)
             WHERE wallet_id = ?4 AND txid = ?5",
            params![
                serde_json::to_string(&new_status)?,
                Utc::now().timestamp(),
                round_id,
                self.wallet_id,
                txid,
            ],
        )?;

        if rows_affected > 0 {
            tracing::info!("Updated transaction {} status to {:?}", txid, new_status);
        }

        Ok(rows_affected > 0)
    }

    // Mark boarding outputs as spent in round
    pub async fn mark_boarding_outputs_spent(
        &self,
        outpoints: &[bitcoin::OutPoint],
        round_id: &str,
    ) -> Result<()> {
        for outpoint in outpoints {
            self.update_transaction_status(
                &outpoint.txid.to_string(),
                TransactionStatus::Spent,
                Some(round_id.to_string()),
            )
            .await?;
        }
        Ok(())
    }

    // Get tx history for specific tx type
    pub async fn get_transaction_history_by_type(
        &self,
        tx_type: TransactionType,
    ) -> Result<Vec<Transaction>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT txid, amount, timestamp, tx_type, status, fee, source, ark_round_id, batch_swap_id
             FROM transactions 
             WHERE wallet_id = ?1 AND tx_type = ?2
             ORDER BY timestamp DESC",
        )?;

        let transactions = stmt
            .query_map(
                [&self.wallet_id, &serde_json::to_string(&tx_type)?],
                |row| {
                    let tx_type_str: String = row.get(3)?;
                    let status_str: String = row.get(4)?;
                    let source_str: String = row.get(6)?;

                    let tx_type: TransactionType =
                        serde_json::from_str(&tx_type_str).map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                3,
                                "tx_type".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?;

                    let status: TransactionStatus =
                        serde_json::from_str(&status_str).map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                4,
                                "status".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?;

                    Ok(Transaction {
                        txid: row.get(0)?,
                        amount: row.get(1)?,
                        timestamp: chrono::DateTime::from_timestamp(row.get::<_, i64>(2)?, 0)
                            .unwrap_or_else(Utc::now),
                        tx_type,
                        status,
                        fee: row
                            .get::<_, Option<i64>>(5)?
                            .map(|f| Amount::from_sat(f as u64)),
                        source: serde_json::from_str(&source_str).map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                6,
                                "source".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?,
                        ark_round_id: row.get::<_, Option<String>>(7)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(transactions)
    }
}
use std::str::FromStr;
