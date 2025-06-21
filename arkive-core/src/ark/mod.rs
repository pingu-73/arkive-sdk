use crate::error::{ArkiveError, Result};
use crate::types::{Transaction, TransactionStatus, TransactionType, VtxoInfo, VtxoStatus};
use crate::wallet::WalletConfig;
use ark_client::{Blockchain, Client, ExplorerUtxo, OfflineClient, SpendStatus};
use ark_core::ArkAddress;
use bip39::rand::rngs::StdRng;
use bip39::rand::SeedableRng;
use bitcoin::key::Keypair;
use bitcoin::{Amount, Network};
use chrono::Utc;
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
}

impl ArkService {
    pub async fn new(keypair: Keypair, config: WalletConfig) -> Result<Self> {
        let mut service = Self {
            client: None,
            keypair,
            config,
        };

        // Try to connect to Ark server
        if let Err(e) = service.connect().await {
            tracing::warn!("Failed to connect to Ark server: {}", e);
            // Continue without connection - will retry later
        }

        Ok(service)
    }

    async fn connect(&mut self) -> Result<()> {
        let blockchain = Arc::new(EsploraBlockchain::new(&self.config.esplora_url)?);
        let wallet = Arc::new(ArkWalletImpl::new(
            self.keypair.clone(),
            self.config.network,
        ));

        let offline_client = OfflineClient::new(
            "arkive-sdk".to_string(),
            self.keypair.clone(),
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

            // [TODO] placeholder server key for offline mode
            let server_pk = bitcoin::XOnlyPublicKey::from_str(
                "33ffb3dee353b1a9ebe4ced64b946238d0a4ac364f275d771da6ad2445d07ae0",
            )
            .map_err(|e| ArkiveError::internal(format!("Invalid server key: {}", e)))?;

            let vtxo = ark_core::Vtxo::new_default(
                &secp,
                server_pk,
                owner_pk,
                bitcoin::Sequence::from_consensus(3600), // 1 hour delay
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

    pub async fn get_balance(&self) -> Result<(Amount, Amount)> {
        if let Some(client) = &self.client {
            let balance = client
                .offchain_balance()
                .await
                .map_err(|e| ArkiveError::ark(format!("Failed to get balance: {}", e)))?;
            Ok((balance.confirmed(), balance.pending()))
        } else {
            Ok((Amount::ZERO, Amount::ZERO))
        }
    }

    pub async fn send(&self, address: ArkAddress, amount: Amount) -> Result<String> {
        if let Some(client) = &self.client {
            let psbt = client
                .send_vtxo(address, amount)
                .await
                .map_err(|e| ArkiveError::ark(format!("Failed to send: {}", e)))?;
            let tx = psbt.extract_tx().map_err(|e| {
                ArkiveError::internal(format!("Failed to extract transaction: {}", e))
            })?;
            Ok(tx.compute_txid().to_string())
        } else {
            Err(ArkiveError::internal("Ark server not connected"))
        }
    }

    pub async fn list_vtxos(&self) -> Result<Vec<VtxoInfo>> {
        if let Some(client) = &self.client {
            let vtxos = client
                .spendable_vtxos()
                .await
                .map_err(|e| ArkiveError::ark(format!("Failed to list VTXOs: {}", e)))?;

            let mut vtxo_infos = Vec::new();
            for (outpoints, vtxo) in vtxos {
                for outpoint in outpoints {
                    vtxo_infos.push(VtxoInfo {
                        outpoint: outpoint.outpoint.to_string(),
                        amount: outpoint.amount,
                        status: if outpoint.is_pending {
                            VtxoStatus::Pending
                        } else {
                            VtxoStatus::Confirmed
                        },
                        expiry: chrono::DateTime::from_timestamp(outpoint.expire_at, 0)
                            .unwrap_or_else(|| Utc::now()),
                        address: vtxo.address().to_string(),
                    });
                }
            }

            Ok(vtxo_infos)
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn participate_in_round(&self) -> Result<Option<String>> {
        if let Some(client) = &self.client {
            let mut rng = StdRng::from_entropy();
            client
                .board(&mut rng)
                .await
                .map_err(|e| ArkiveError::ark(format!("Failed to participate in round: {}", e)))?;
            // [TODO] Return actual round txid
            Ok(Some("round_completed".to_string()))
        } else {
            Err(ArkiveError::internal("Ark server not connected"))
        }
    }

    pub async fn get_transaction_history(&self) -> Result<Vec<Transaction>> {
        if let Some(client) = &self.client {
            let history = client.transaction_history().await.map_err(|e| {
                ArkiveError::ark(format!("Failed to get transaction history: {}", e))
            })?;

            let mut transactions = Vec::new();
            for tx in history {
                let (txid, amount, timestamp, tx_type, is_settled) = match tx {
                    ark_core::ArkTransaction::Boarding {
                        txid,
                        amount,
                        confirmed_at,
                    } => (
                        txid.to_string(),
                        amount.to_sat() as i64,
                        confirmed_at.unwrap_or(Utc::now().timestamp()),
                        TransactionType::Boarding,
                        confirmed_at.is_some(),
                    ),
                    ark_core::ArkTransaction::Round {
                        txid,
                        amount,
                        created_at,
                    } => (
                        txid.to_string(),
                        amount.to_sat() as i64,
                        created_at,
                        TransactionType::Ark,
                        true,
                    ),
                    ark_core::ArkTransaction::Redeem {
                        txid,
                        amount,
                        is_settled,
                        created_at,
                    } => (
                        txid.to_string(),
                        amount.to_sat() as i64,
                        created_at,
                        TransactionType::Ark,
                        is_settled,
                    ),
                };

                transactions.push(Transaction {
                    txid,
                    amount,
                    timestamp: chrono::DateTime::from_timestamp(timestamp, 0)
                        .unwrap_or_else(|| Utc::now()),
                    tx_type,
                    status: if is_settled {
                        TransactionStatus::Confirmed
                    } else {
                        TransactionStatus::Pending
                    },
                    fee: None,
                });
            }

            Ok(transactions)
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn sync(&self) -> Result<()> {
        // Try to reconnect if not connected
        if self.client.is_none() {
            // [TODO] requires &mut self, so we'll skip reconnection for now
            tracing::warn!("Ark client not connected, skipping sync");
        }
        Ok(())
    }

    pub async fn estimate_fee(&self, amount: Amount) -> Result<Amount> {
        // [TODO] Ark tx fees
        let base_fee = Amount::from_sat(100);
        let amount_fee = Amount::from_sat(amount.to_sat() / 10000); // 0.01%
        Ok(base_fee + amount_fee)
    }
}

use std::str::FromStr;
