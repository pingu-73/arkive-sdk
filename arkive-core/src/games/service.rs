use crate::ark::ArkService;
use crate::error::Result;
use crate::storage::Storage;
use crate::ArkAddress;
use crate::ArkWallet;
use crate::ArkiveError;
use crate::LotteryCoordinator;
use bitcoin::{Amount, Network, XOnlyPublicKey};
use std::str::FromStr;
use std::sync::Arc;

#[allow(dead_code)]
pub struct GameService {
    pub storage: Arc<Storage>,
    keypair: bitcoin::key::Keypair,
    network: Network,
    is_mutinynet: bool,
    lottery_coordinator: Arc<LotteryCoordinator>,
}

impl GameService {
    // Get actual server key based on network and mutinynet flag
    pub fn new(
        storage: Arc<Storage>,
        keypair: bitcoin::key::Keypair,
        network: Network,
        is_mutinynet: bool, // Add this parameter
    ) -> Self {
        let (coordinator_key, _) = keypair.x_only_public_key();

        // Get actual server key based on network and mutinynet flag
        let server_key = match (network, is_mutinynet) {
            (Network::Regtest, false) => {
                // For local regtest - remove the '02' prefix
                XOnlyPublicKey::from_str(
                    "4e79dab75861bf218443b5934afa2cd71d26c80104148e611f1093dbdfb4248d",
                )
                .expect("valid regtest server key")
            }
            (Network::Signet, true) => {
                // For Mutinynet - remove the '03' prefix
                XOnlyPublicKey::from_str(
                    "fa73c6e4876ffb2dfc961d763cca9abc73d4b88efcb8f5e7ff92dc55e9aa553d",
                )
                .expect("valid mutinynet server key")
            }
            (Network::Signet, false) => {
                // For regular Signet - remove the '02' prefix
                XOnlyPublicKey::from_str(
                    "8bf56160efc769112b361de4117b3c71b88ca16f1bb9f6ac7a2781929abc5e6a",
                )
                .expect("valid signet server key")
            }
            _ => {
                // Fallback for other networks
                tracing::warn!(
                    "Using coordinator key as server key for network {:?}",
                    network
                );
                coordinator_key
            }
        };

        let lottery_coordinator = Arc::new(LotteryCoordinator::new(
            storage.clone(),
            network,
            is_mutinynet,
            server_key,
            coordinator_key,
        ));

        Self {
            storage,
            keypair,
            network,
            is_mutinynet,
            lottery_coordinator,
        }
    }

    pub async fn load_lottery_escrow(&self, lottery_id: &str) -> Result<crate::LotteryEscrow> {
        self.lottery_coordinator
            .load_lottery_escrow(lottery_id)
            .await
    }

    pub fn lottery_coordinator(&self) -> &Arc<LotteryCoordinator> {
        &self.lottery_coordinator
    }

    /// Create escrow-based lottery
    pub async fn create_escrow_lottery(&self, players: usize, entry_fee: Amount) -> Result<String> {
        // Use the service owner's key as first participant
        let (owner_pubkey, _) = self.keypair.x_only_public_key();

        let participants = if players <= 1 {
            vec![owner_pubkey]
        } else {
            // For demo, use owner + generate valid keys for others
            let mut keys = vec![owner_pubkey];
            for i in 1..players {
                keys.push(self.generate_valid_demo_key(i)?);
            }
            keys
        };

        self.create_escrow_lottery_with_keys(participants, entry_fee)
            .await
    }

    pub async fn create_escrow_lottery_with_keys(
        &self,
        participants: Vec<XOnlyPublicKey>,
        entry_fee: Amount,
    ) -> Result<String> {
        if participants.is_empty() {
            return Err(ArkiveError::internal("At least one participant required"));
        }

        println!("Creating lottery with {} participants", participants.len());
        for (i, pubkey) in participants.iter().enumerate() {
            println!("  Participant {}: {}", i + 1, pubkey);
        }

        // Create lottery with escrow using real participant keys
        let lottery = self
            .lottery_coordinator
            .create_lottery_with_escrow(participants, entry_fee)
            .await?;

        Ok(lottery.lottery_id)
    }

    fn generate_valid_demo_key(&self, index: usize) -> Result<XOnlyPublicKey> {
        use bitcoin::key::Keypair;
        use bitcoin::secp256k1::{Secp256k1, SecretKey};
        use sha2::{Digest, Sha256};

        let secp = Secp256k1::new();

        // Create a valid secret key using SHA256 hash
        let mut hasher = Sha256::new();
        hasher.update(b"arkive_lottery_demo");
        hasher.update(self.keypair.public_key().serialize());
        hasher.update((index as u32).to_le_bytes());
        let hash = hasher.finalize();

        // Ensure valid secret key by clearing highest bit
        let mut secret_bytes = hash.as_slice().to_vec();
        secret_bytes[0] &= 0x7f;

        // Ensure it's not zero
        if secret_bytes.iter().all(|&b| b == 0) {
            secret_bytes[0] = 1;
        }

        // Create secret key with retry logic
        let secret = SecretKey::from_slice(&secret_bytes)
            .map_err(|e| ArkiveError::internal(format!("Failed to create secret key: {}", e)))?;

        let keypair = Keypair::from_secret_key(&secp, &secret);
        let (xonly, _) = keypair.x_only_public_key();
        Ok(xonly)
    }

    /// Submit commitment to escrow lottery
    pub async fn submit_escrow_commitment(
        &self,
        lottery_id: &str,
        participant: XOnlyPublicKey,
        commitment: super::Commitment,
    ) -> Result<()> {
        self.lottery_coordinator
            .submit_commitment_with_escrow(lottery_id, participant, commitment)
            .await
    }

    /// Submit reveal to escrow lottery
    pub async fn submit_escrow_reveal(
        &self,
        lottery_id: &str,
        participant: XOnlyPublicKey,
        reveal: super::Reveal,
    ) -> Result<Option<super::GameOutcome>> {
        self.lottery_coordinator
            .submit_reveal_with_escrow(lottery_id, participant, reveal)
            .await
    }

    pub async fn register_participation(&self, escrow_address: &str, txid: &str) -> Result<()> {
        let conn = self.storage.get_connection().await;

        conn.execute(
            "INSERT INTO game_participations (escrow_address, txid, participant_key, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                escrow_address,
                txid,
                self.keypair.x_only_public_key().0.to_string(),
                chrono::Utc::now().timestamp(),
            ],
        )?;

        Ok(())
    }

    pub fn generate_lottery_commitment(
        keypair: &bitcoin::key::Keypair,
        lottery_id: &str,
    ) -> Result<crate::Commitment> {
        use bitcoin::secp256k1::{Message, Secp256k1};
        use rand::RngCore;
        use sha2::{Digest, Sha256};

        let secp = Secp256k1::new();

        // Generate random secret and nonce
        let mut rng = rand::rng();
        let mut secret = [0u8; 32];
        let mut nonce = [0u8; 32];
        rng.fill_bytes(&mut secret);
        rng.fill_bytes(&mut nonce);

        // Create commitment hash: H(secret || nonce || lottery_id || pubkey)
        let mut hasher = Sha256::new();
        hasher.update(secret);
        hasher.update(nonce);
        hasher.update(lottery_id.as_bytes());
        hasher.update(keypair.x_only_public_key().0.serialize());
        let hash = hasher.finalize();

        // Sign the commitment
        let msg = Message::from_digest_slice(&hash)
            .map_err(|e| ArkiveError::internal(format!("Invalid message: {}", e)))?;
        let signature = secp.sign_schnorr_no_aux_rand(&msg, keypair);

        Ok(crate::Commitment {
            hash: hash.into(),
            timestamp: chrono::Utc::now().timestamp() as u64,
            signature,
        })
    }

    /// move VTXO to escrow address using real Ark transactions
    async fn move_vtxo_to_escrow(
        &self,
        participant_wallet: Arc<ArkWallet>,
        // escrow_address: bitcoin::Address,
        escrow_address_str: String,
        amount: Amount,
        _vtxo_outpoint: bitcoin::OutPoint,
    ) -> Result<bitcoin::Txid> {
        // Get the Ark service from the participant wallet
        let ark_service = participant_wallet
            .get_ark_service()
            .await
            .map_err(|e| ArkiveError::internal(format!("Failed to get Ark service: {}", e)))?;

        // Convert escrow address to ArkAddress format
        let ark_address = ArkAddress::decode(&escrow_address_str)
            .map_err(|e| ArkiveError::internal(format!("Invalid escrow address: {}", e)))?;

        // Create a real Ark transaction to send funds to escrow address
        tracing::info!(
            "Creating real Ark transaction to send {} sats to {}",
            amount.to_sat(),
            escrow_address_str
        );

        // Use the existing send method to create real Ark transaction
        let txid_str = ark_service
            .send(ark_address, amount)
            .await
            .map_err(|e| ArkiveError::internal(format!("Failed to send Ark transaction: {}", e)))?;

        // Convert string txid back to Txid
        let txid = bitcoin::Txid::from_str(&txid_str)
            .map_err(|e| ArkiveError::internal(format!("Invalid transaction ID: {}", e)))?;

        tracing::info!("Successfully created Ark transaction: {}", txid);

        // Wait for transaction to be processed by Ark network
        self.wait_for_ark_processing(ark_service.clone(), &txid)
            .await?;

        Ok(txid)
    }

    async fn wait_for_ark_processing(
        &self,
        ark_service: Arc<ArkService>,
        txid: &bitcoin::Txid,
    ) -> Result<()> {
        tracing::info!("Waiting for Ark transaction {} to be processed...", txid);

        // Poll for transaction confirmation in Ark network
        let mut attempts = 0;
        let max_attempts = 20;
        let poll_interval = std::time::Duration::from_secs(2);

        while attempts < max_attempts {
            // Check transaction status through Ark service
            match self
                .check_ark_transaction_status(ark_service.clone(), txid)
                .await
            {
                Ok(true) => {
                    tracing::info!("Ark transaction {} confirmed", txid);
                    return Ok(());
                }
                Ok(false) => {
                    // Still pending, wait and retry
                    tracing::debug!(
                        "Transaction {} still pending, attempt {}/{}",
                        txid,
                        attempts + 1,
                        max_attempts
                    );
                    tokio::time::sleep(poll_interval).await;
                    attempts += 1;
                }
                Err(e) => {
                    tracing::warn!("Failed to check transaction status: {}", e);
                    tokio::time::sleep(poll_interval).await;
                    attempts += 1;
                }
            }
        }

        // Even if we timeout, the transaction might still succeed
        tracing::warn!("Timeout waiting for Ark transaction {} confirmation, but transaction may still succeed", txid);
        Ok(())
    }

    pub async fn check_ark_transaction_status(
        &self,
        ark_service: Arc<ArkService>,
        txid: &bitcoin::Txid,
    ) -> Result<bool> {
        // Try to get transaction from storage to see if it was processed
        match ark_service.get_transaction_history().await {
            Ok(transactions) => {
                // Look for our transaction in the history
                for tx in transactions {
                    if tx.txid.contains(&txid.to_string()[..8]) || tx.txid == txid.to_string() {
                        // Found transaction, check if it's confirmed
                        match tx.status {
                            crate::types::TransactionStatus::Confirmed
                            | crate::types::TransactionStatus::Spent => {
                                return Ok(true);
                            }
                            crate::types::TransactionStatus::Pending => {
                                // Still pending
                                return Ok(false);
                            }
                            _ => {
                                // Other states
                                return Ok(false);
                            }
                        }
                    }
                }
                // Transaction not found yet, still pending
                Ok(false)
            }
            Err(_) => {
                // If we can't check status, assume still pending
                Ok(false)
            }
        }
    }

    pub async fn fund_lottery_escrow_with_wallet(
        &self,
        lottery_id: &str,
        participant_wallet: Arc<ArkWallet>,
        vtxo_outpoint: bitcoin::OutPoint,
        _amount: Amount, // Ignore the passed amount
    ) -> Result<()> {
        let (participant_pubkey, _) = participant_wallet.keypair.x_only_public_key();

        // Load the lottery escrow
        let lottery_escrow = self
            .lottery_coordinator
            .load_lottery_escrow(lottery_id)
            .await?;

        // Use the correct entry fee amount from lottery
        let amount = lottery_escrow.entry_fee;

        // Verify the participant is in the lottery
        if !lottery_escrow.participants.contains(&participant_pubkey) {
            return Err(ArkiveError::internal("Participant not in lottery"));
        }

        // Verify amount is sufficient
        if amount < lottery_escrow.entry_fee {
            return Err(ArkiveError::internal("Insufficient entry fee"));
        }

        // Get escrow address and parse it as ArkAddress (this is the key fix!)
        let escrow_address_str = &lottery_escrow.escrow_address;
        let ark_address = ArkAddress::decode(escrow_address_str)
            .map_err(|e| ArkiveError::internal(format!("Invalid escrow Ark address: {}", e)))?;

        tracing::info!(
            "Moving {} sats to escrow Ark address: {}",
            amount.to_sat(),
            escrow_address_str
        );

        let txid = self
            .move_vtxo_to_escrow(
                participant_wallet.clone(),
                // Convert ArkAddress to string for the move function
                ark_address.to_string(),
                amount,
                vtxo_outpoint,
            )
            .await?;

        // Record the actual funding with real transaction details
        let funding_record = crate::lottery_coordinator::FundingRecord {
            participant: participant_pubkey,
            vtxo_outpoint, // Original VTXO that was spent
            amount,
            timestamp: chrono::Utc::now(),
        };

        // Update lottery state by loading and modifying
        let mut updated_lottery = self
            .lottery_coordinator
            .load_lottery_escrow(lottery_id)
            .await?;
        updated_lottery.funding_vtxos.push(funding_record);

        // Update lottery state
        if updated_lottery.funding_vtxos.len() == updated_lottery.participants.len() {
            updated_lottery.state = crate::LotteryState::CommitmentPhase;
        }

        // Save updated lottery
        self.lottery_coordinator
            .update_lottery_escrow(&updated_lottery)
            .await?;

        tracing::info!(
            "Successfully moved {} sats to escrow for lottery {} (tx: {})",
            amount.to_sat(),
            lottery_id,
            txid
        );

        Ok(())
    }
}
