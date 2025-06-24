#![allow(unused_imports)]
use crate::error::{ArkiveError, Result};
use crate::storage::vtxo_store::{VtxoState, VtxoTreeData};
use crate::storage::{BoardingOutputState, BoardingStore};
use crate::storage::{Storage, VtxoStore};
use crate::types::{Transaction, TransactionStatus, TransactionType, VtxoInfo, VtxoStatus};
use crate::wallet::WalletConfig;
use ark_client::{Blockchain, Client, ExplorerUtxo, OfflineClient, SpendStatus};
use ark_core::coin_select::select_vtxos;
use ark_core::redeem::{build_redeem_transaction, sign_redeem_transaction, VtxoInput};
use ark_core::{ArkAddress, ArkTransaction};
use bip39::rand::rngs::StdRng;
use bip39::rand::SeedableRng;
use bitcoin::key::Keypair;
use bitcoin::{Amount, Network, Psbt};
use chrono::{DateTime, Utc};
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
}

// Wallet implementation for Ark
pub struct ArkWalletImpl {
    keypair: Keypair,
    network: Network,
}

impl ArkWalletImpl {
    pub fn new(keypair: Keypair, network: Network) -> Self {
        Self { keypair, network }
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
        // [TODO] this would load from storage
        Ok(Vec::new())
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
}

pub struct ArkService {
    client: Option<Client<EsploraBlockchain, ArkWalletImpl>>,
    keypair: Keypair,
    config: WalletConfig,
    storage: Arc<Storage>,
    wallet_id: String,
}

impl ArkService {
    pub async fn new(
        keypair: Keypair,
        config: WalletConfig,
        storage: Arc<Storage>,
        wallet_id: String,
    ) -> Result<Self> {
        let mut service = Self {
            client: None,
            keypair,
            config,
            storage,
            wallet_id,
        };

        // Try to connect to Ark server
        if let Err(e) = service.connect().await {
            tracing::warn!("Failed to connect to Ark server: {}", e);
        }

        Ok(service)
    }

    async fn connect(&mut self) -> Result<()> {
        let blockchain = Arc::new(EsploraBlockchain::new(&self.config.esplora_url)?);
        let wallet = Arc::new(ArkWalletImpl::new(self.keypair, self.config.network));

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

    async fn record_transaction(
        &self,
        txid: &str,
        amount: i64,
        tx_type: TransactionType,
    ) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "INSERT OR REPLACE INTO transactions (wallet_id, txid, amount, timestamp, tx_type, status, fee, raw_data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                self.wallet_id,
                txid,
                amount,
                Utc::now().timestamp(),
                serde_json::to_string(&tx_type)?,
                serde_json::to_string(&TransactionStatus::Pending)?,
                None::<i64>, // fee
                None::<String>, // raw_data
            ],
        )?;

        tracing::info!(
            "Recorded transaction: {} with amount: {} sats",
            txid,
            amount
        );
        Ok(())
    }

    pub async fn send(&self, address: ArkAddress, amount: Amount) -> Result<String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ArkiveError::internal("Ark server not connected"))?;

        // 1. Get available VTXOs
        let available_vtxos = self.get_spendable_vtxos().await?;
        if available_vtxos.is_empty() {
            return Err(ArkiveError::InsufficientFunds {
                need: amount.to_sat(),
                available: 0,
            });
        }

        // 2. Select VTXOs for this transaction
        let vtxo_outpoints: Result<Vec<_>> = available_vtxos
            .iter()
            .map(|v| -> Result<ark_core::coin_select::VtxoOutPoint> {
                Ok(ark_core::coin_select::VtxoOutPoint {
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
            Amount::from_sat(546), // dust limit
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

        // 3. Build VTXO inputs
        let vtxo_inputs: Vec<VtxoInput> = selected_outpoints
            .iter()
            .filter_map(|outpoint| {
                available_vtxos
                    .iter()
                    .find(|v| v.outpoint == outpoint.outpoint.to_string())
                    .map(|_vtxo_state| {
                        // Create VTXO from stored state
                        let secp = bitcoin::secp256k1::Secp256k1::new();
                        let server_pk = client.server_info.pk.x_only_public_key().0;
                        let (owner_pk, _) = self.keypair.x_only_public_key();

                        let vtxo = ark_core::Vtxo::new_default(
                            &secp,
                            server_pk,
                            owner_pk,
                            client.server_info.unilateral_exit_delay,
                            self.config.network,
                        )
                        .expect("Valid VTXO");

                        VtxoInput::new(vtxo, outpoint.amount, outpoint.outpoint)
                    })
            })
            .collect();

        // 4. Create change address if needed
        let change_amount = total_input - amount;
        let change_address = if change_amount > Amount::from_sat(546) {
            Some(self.get_address().await?)
        } else {
            None
        };

        // 5. Build redeem transaction
        let mut redeem_psbt = build_redeem_transaction(
            &[(&address, amount)],
            change_address
                .as_ref()
                .map(|addr| ArkAddress::decode(addr).expect("Valid change address"))
                .as_ref(),
            &vtxo_inputs,
        )
        .map_err(|e| ArkiveError::ark(format!("Failed to build transaction: {}", e)))?;

        // 6. Sign the transaction
        let sign_fn = |msg: bitcoin::secp256k1::Message| -> std::result::Result<
            (
                bitcoin::secp256k1::schnorr::Signature,
                bitcoin::XOnlyPublicKey,
            ),
            ark_core::Error,
        > {
            let secp = bitcoin::secp256k1::Secp256k1::new();
            let sig = secp.sign_schnorr_no_aux_rand(&msg, &self.keypair);
            let pk = self.keypair.x_only_public_key().0;
            Ok((sig, pk))
        };

        for (i, _) in vtxo_inputs.iter().enumerate() {
            sign_redeem_transaction(sign_fn, &mut redeem_psbt, &vtxo_inputs, i)
                .map_err(|e| ArkiveError::ark(format!("Failed to sign transaction: {}", e)))?;
        }

        // 7. Submit to server
        let signed_psbt = client
            .send_vtxo(address, amount)
            .await
            .map_err(|e| ArkiveError::ark(format!("Failed to submit transaction: {}", e)))?;

        let tx = signed_psbt
            .extract_tx()
            .map_err(|e| ArkiveError::internal(format!("Failed to extract transaction: {}", e)))?;
        let txid = tx.compute_txid().to_string();

        // 8. Update VTXO states in storage
        self.update_vtxo_states_after_send(&selected_outpoints, &txid)
            .await?;

        // 9. Record transaction
        self.record_transaction(&txid, -(amount.to_sat() as i64), TransactionType::Ark)
            .await?;

        tracing::info!(
            "Sent {} sats via Ark transaction: {}",
            amount.to_sat(),
            txid
        );
        Ok(txid)
    }

    async fn get_spendable_vtxos(&self) -> Result<Vec<VtxoState>> {
        let vtxo_store = VtxoStore::new(&self.storage);
        let all_vtxos = vtxo_store.load_vtxo_states(&self.wallet_id).await?;

        // Filter for spendable VTXOs (confirmed and not expired)
        let now = Utc::now();
        let spendable: Vec<VtxoState> = all_vtxos
            .into_iter()
            .filter(|vtxo| matches!(vtxo.status, VtxoStatus::Confirmed) && vtxo.expiry > now)
            .collect();

        Ok(spendable)
    }

    async fn update_vtxo_states_after_send(
        &self,
        spent_outpoints: &[ark_core::coin_select::VtxoOutPoint],
        txid: &str,
    ) -> Result<()> {
        let vtxo_store = VtxoStore::new(&self.storage);

        for outpoint in spent_outpoints {
            // Mark VTXO as spent
            let mut vtxo_state = vtxo_store
                .load_vtxo_states(&self.wallet_id)
                .await?
                .into_iter()
                .find(|v| v.outpoint == outpoint.outpoint.to_string())
                .ok_or_else(|| ArkiveError::internal("VTXO not found in storage"))?;

            vtxo_state.status = VtxoStatus::Spent;
            vtxo_store
                .save_vtxo_state(&self.wallet_id, &vtxo_state)
                .await?;
        }

        tracing::info!(
            "Updated {} VTXO states after transaction {}",
            spent_outpoints.len(),
            txid
        );
        Ok(())
    }

    // [TODO] Not working correctly.
    pub async fn participate_in_round(&self) -> Result<Option<String>> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ArkiveError::internal("Ark server not connected"))?;

        // First, sync to detect any new boarding outputs
        self.detect_and_store_boarding_outputs().await?;

        // Check if we have any VTXOs or boarding outputs to settle
        let vtxos = self.get_spendable_vtxos().await?;
        let boarding_outputs = self.get_boarding_outputs().await?;

        if vtxos.is_empty() && boarding_outputs.is_empty() {
            tracing::info!("No VTXOs or boarding outputs to settle");
            return Ok(None);
        }

        tracing::info!(
            "Participating in round with {} VTXOs and {} boarding outputs",
            vtxos.len(),
            boarding_outputs.len()
        );

        let mut rng = StdRng::from_entropy();

        tracing::info!("Calling client.board()...");
        match client.board(&mut rng).await {
            Ok(_) => {
                tracing::info!("client.board() returned success");
                // Check if we actually got VTXOs
                let new_vtxos = client.spendable_vtxos().await.map_err(|e| {
                    ArkiveError::ark(format!("Failed to get VTXOs after round: {}", e))
                })?;

                tracing::info!("VTXOs after round: {}", new_vtxos.len());
                // --------
                // // Mark boarding outputs as spent
                // #[allow(unused_variables)]
                // let boarding_store = BoardingStore::new(&self.storage);
                // #[allow(unused_variables)]
                // for boarding_output in &boarding_outputs {
                //     // Extract outpoint from boarding output (you may need to store this)
                //     // For now, we'll mark all as potentially spent and let sync clean up
                // }

                // Sync with server to get updated state
                self.sync_with_server().await?;

                // let round_id = format!("round_{}", Utc::now().timestamp());
                // tracing::info!("Successfully participated in round: {}", round_id);
                // Ok(Some(round_id))

                // ---------
                if new_vtxos.is_empty() {
                    tracing::warn!("Round participation claimed success but no VTXOs created");
                    Ok(None)
                } else {
                    let round_id = format!("round_{}", Utc::now().timestamp());
                    tracing::info!("Successfully participated in round: {}", round_id);
                    Ok(Some(round_id))
                }
            }
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("No boarding outputs")
                    || error_msg.contains("No VTXOs")
                    || error_msg.contains("no inputs")
                {
                    tracing::info!("No round participation needed: {}", error_msg);
                    Ok(None)
                } else {
                    Err(ArkiveError::ark(format!(
                        "Round participation failed: {}",
                        e
                    )))
                }
            }
        }
    }

    async fn get_boarding_outputs(&self) -> Result<Vec<ark_core::BoardingOutput>> {
        let boarding_store = BoardingStore::new(&self.storage);
        let boarding_states = boarding_store
            .load_boarding_outputs(&self.wallet_id)
            .await?;

        let mut boarding_outputs = Vec::new();
        for state in boarding_states {
            let boarding_output = state.to_boarding_output(self.config.network)?;
            boarding_outputs.push(boarding_output);
        }

        Ok(boarding_outputs)
    }

    async fn detect_and_store_boarding_outputs(&self) -> Result<()> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ArkiveError::internal("Ark server not connected"))?;

        // Get boarding address
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

                let boarding_state = BoardingOutputState {
                    outpoint: utxo.outpoint,
                    amount: utxo.amount,
                    address: boarding_address.clone(),
                    script_pubkey: address.script_pubkey().to_hex_string(),
                    exit_delay: client.server_info.unilateral_exit_delay.to_consensus_u32(),
                    server_pubkey: server_pk.to_string(),
                    user_pubkey: user_pk.to_string(),
                    confirmation_blocktime: utxo
                        .confirmation_blocktime
                        .and_then(|t| DateTime::from_timestamp(t as i64, 0)),
                    is_spent: false,
                };

                boarding_store
                    .save_boarding_output(&self.wallet_id, &boarding_state)
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
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ArkiveError::internal("Ark server not connected"))?;

        // 1. Detect and store boarding outputs
        self.detect_and_store_boarding_outputs().await?;

        // 2. Get current VTXOs from server
        let server_vtxos = client
            .spendable_vtxos()
            .await
            .map_err(|e| ArkiveError::ark(format!("Failed to get VTXOs from server: {}", e)))?;

        // 3. Update local VTXO storage
        let vtxo_store = VtxoStore::new(&self.storage);

        for (outpoints, vtxo) in server_vtxos {
            for outpoint in outpoints {
                let vtxo_state = VtxoState {
                    outpoint: outpoint.outpoint.to_string(),
                    amount: outpoint.amount,
                    status: if outpoint.is_pending {
                        VtxoStatus::Pending
                    } else {
                        VtxoStatus::Confirmed
                    },
                    expiry: chrono::DateTime::from_timestamp(outpoint.expire_at, 0)
                        .unwrap_or_else(Utc::now),
                    address: vtxo.address().to_string(),
                    batch_id: format!("batch_{}", outpoint.expire_at),
                    tree_path: Vec::new(), // [TODO] Extract from VTXO tree
                    exit_transactions: Vec::new(), // [TODO] Store exit transactions
                };

                vtxo_store
                    .save_vtxo_state(&self.wallet_id, &vtxo_state)
                    .await?;
            }
        }

        // 4. Get and store transaction history
        let history = client
            .transaction_history()
            .await
            .map_err(|e| ArkiveError::ark(format!("Failed to get transaction history: {}", e)))?;

        for tx in history {
            let (txid, amount, _timestamp, tx_type) = match tx {
                ArkTransaction::Boarding {
                    txid,
                    amount,
                    confirmed_at,
                } => (
                    txid.to_string(),
                    amount.to_sat() as i64,
                    confirmed_at.unwrap_or(Utc::now().timestamp()),
                    TransactionType::Boarding,
                ),
                ArkTransaction::Round {
                    txid,
                    amount,
                    created_at,
                } => (
                    txid.to_string(),
                    amount.to_sat(),
                    created_at,
                    TransactionType::Ark,
                ),
                ArkTransaction::Redeem {
                    txid,
                    amount,
                    created_at,
                    ..
                } => (
                    txid.to_string(),
                    amount.to_sat(),
                    created_at,
                    TransactionType::Ark,
                ),
            };

            self.record_transaction(&txid, amount, tx_type).await?;
        }
        tracing::info!("Synced wallet {} with Ark server", self.wallet_id);
        Ok(())
    }

    pub async fn get_balance(&self) -> Result<(Amount, Amount)> {
        if let Some(client) = &self.client {
            // Get balance from server
            match client.offchain_balance().await {
                Ok(balance) => {
                    tracing::info!(
                        "Server balance - Confirmed: {}, Pending: {}",
                        balance.confirmed().to_sat(),
                        balance.pending().to_sat()
                    );
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
                VtxoStatus::Pending => pending += vtxo.amount,
                _ => {} // Skip spent/expired
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
            })
            .collect();

        Ok(vtxo_infos)
    }

    pub async fn get_transaction_history(&self) -> Result<Vec<Transaction>> {
        let conn = self.storage.get_connection().await;

        let mut stmt = conn.prepare(
            "SELECT txid, amount, timestamp, tx_type, status, fee 
             FROM transactions 
             WHERE wallet_id = ?1 
             ORDER BY timestamp DESC",
        )?;

        let transactions = stmt
            .query_map([&self.wallet_id], |row| {
                let tx_type_str: String = row.get(3)?;
                let status_str: String = row.get(4)?;

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
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(transactions)
    }

    pub async fn estimate_fee(&self, amount: Amount) -> Result<Amount> {
        // Ark transaction fees are typically very low
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
}

use std::str::FromStr;
