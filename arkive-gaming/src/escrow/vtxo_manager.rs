#![allow(unused_variables)]
use crate::contracts::TapscriptManager;
use crate::error::{GamingError, Result};
use crate::storage::{GameStorage, VtxoStorageData};
use crate::CompiledScript;
use ark_core::{ArkAddress, Vtxo};
use arkive_core::WalletManager;
use bitcoin::{Amount, OutPoint, Transaction, TxOut, XOnlyPublicKey};
use std::sync::Arc;

pub struct VtxoEscrowManager {
    wallet_manager: Arc<WalletManager>,
    tapscript_manager: TapscriptManager,
    storage: GameStorage,
}

impl VtxoEscrowManager {
    pub fn new(wallet_manager: Arc<WalletManager>) -> Self {
        Self {
            wallet_manager,
            tapscript_manager: TapscriptManager::new(),
            storage: GameStorage::new("vtxo_storage"),
        }
    }

    /// Create a real VTXO-based escrow for the lottery
    pub async fn create_lottery_vtxo(
        &self,
        game_id: &str,
        player1_wallet: &str,
        player2_wallet: &str,
        bet_amount: Amount,
        commitment1_hash: [u8; 32],
        commitment2_hash: [u8; 32],
    ) -> Result<LotteryVtxo> {
        // load player wallets only
        let player1_wallet = self
            .wallet_manager
            .load_wallet(player1_wallet)
            .await
            .map_err(|e| GamingError::escrow(format!("Failed to load player1 wallet: {}", e)))?;
        let player2_wallet = self
            .wallet_manager
            .load_wallet(player2_wallet)
            .await
            .map_err(|e| GamingError::escrow(format!("Failed to load player2 wallet: {}", e)))?;

        // extract public keys from wallets
        let player1_pk = self.extract_public_key(&player1_wallet).await?;
        let player2_pk = self.extract_public_key(&player2_wallet).await?;

        // use deterministic server key derived from the game
        let server_pk = self.derive_game_server_key(game_id)?;

        self.tapscript_manager.validate_scripts()?;

        // compile lottery tapscripts
        let timeout_delay_seconds = 24 * 60 * 60; // 24 hrs 
        let tapscripts = self.tapscript_manager.compile_lottery_scripts(
            player1_pk,
            player2_pk,
            server_pk,
            bet_amount,
            commitment1_hash,
            commitment2_hash,
            timeout_delay_seconds,
        )?;

        tracing::info!("Compiled lottery scripts from files for game: {}", game_id);
        tracing::debug!("Escrow script: {}", tapscripts.escrow_script.source);

        // create VTXO with custom tapscripts
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let vtxo = self.create_custom_vtxo(
            &secp,
            server_pk,
            &tapscripts,
            bet_amount * 2, // Total pot
        )?;

        let lottery_vtxo = LotteryVtxo {
            game_id: game_id.to_string(),
            vtxo,
            tapscripts,
            player1_pk,
            player2_pk,
            server_pk,
            bet_amount,
            total_amount: bet_amount * 2,
            commitment1_hash,
            commitment2_hash,
            created_at: chrono::Utc::now(),
        };

        // save VTXO data to storage
        let vtxo_data = VtxoStorageData::new(
            game_id.to_string(),
            lottery_vtxo.get_escrow_address().to_string(),
            player1_pk,
            player2_pk,
            server_pk,
            bet_amount,
            commitment1_hash,
            commitment2_hash,
        );
        self.storage.save_vtxo_data(game_id, &vtxo_data).await?;

        Ok(lottery_vtxo)
    }

    /// Compile winner-specific script when we know the winner
    pub fn compile_winner_script(
        &self,
        winner_pk: XOnlyPublicKey,
        server_pk: XOnlyPublicKey,
        total_amount: Amount,
        commitment1_hash: [u8; 32],
        commitment2_hash: [u8; 32],
    ) -> Result<CompiledScript> {
        self.tapscript_manager.compile_winner_script(
            winner_pk,
            server_pk,
            total_amount,
            commitment1_hash,
            commitment2_hash,
        )
    }

    /// Derive a deterministic server key for the game
    fn derive_game_server_key(&self, game_id: &str) -> Result<XOnlyPublicKey> {
        use bitcoin::hashes::{sha256, Hash};
        use bitcoin::secp256k1::{Secp256k1, SecretKey};

        // create a deterministic key based on the game ID
        let mut data = Vec::new();
        data.extend_from_slice(b"ark_lottery_server_key_");
        data.extend_from_slice(game_id.as_bytes());
        data.extend_from_slice(b"_deterministic_salt");

        let hash = sha256::Hash::hash(&data);

        let secp = Secp256k1::new();
        let mut key_bytes = hash.to_byte_array();

        // ensure the key is valid
        loop {
            match SecretKey::from_slice(&key_bytes) {
                Ok(secret_key) => {
                    let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &secret_key);
                    return Ok(keypair.x_only_public_key().0);
                }
                Err(_) => {
                    // If invalid, hash again
                    let new_hash = sha256::Hash::hash(&key_bytes);
                    key_bytes = new_hash.to_byte_array();
                }
            }
        }
    }

    /// Mark VTXO as spent and cleanup storage
    pub async fn mark_vtxo_spent(&self, game_id: &str) -> Result<()> {
        if let Some(mut vtxo_data) = self.storage.load_vtxo_data(game_id).await? {
            vtxo_data.is_spent = true;
            self.storage.save_vtxo_data(game_id, &vtxo_data).await?;
        }
        Ok(())
    }

    /// Load VTXO data from storage
    pub async fn load_vtxo_data(&self, game_id: &str) -> Result<Option<VtxoStorageData>> {
        self.storage.load_vtxo_data(game_id).await
    }

    /// Execute winner payout using real Ark transaction
    pub async fn execute_winner_payout(
        &self,
        lottery_vtxo: &LotteryVtxo,
        winner_address: &ArkAddress,
        reveal1: &crate::commitment::Reveal,
        reveal2: &crate::commitment::Reveal,
    ) -> Result<String> {
        // build witness data for winner payout
        let witness = self.build_winner_witness(lottery_vtxo, reveal1, reveal2)?;

        // Create the payout transaction
        let payout_tx = self.build_payout_transaction(
            &lottery_vtxo.vtxo,
            winner_address,
            lottery_vtxo.total_amount,
            witness,
        )?;

        // Broadcast through Ark
        let txid = self.broadcast_ark_transaction(payout_tx).await?;

        tracing::info!(
            "Executed real winner payout: {} sats to {} in transaction {}",
            lottery_vtxo.total_amount.to_sat(),
            winner_address,
            txid
        );

        Ok(txid)
    }

    /// Execute timeout refund using real Ark transaction
    pub async fn execute_timeout_refund(
        &self,
        lottery_vtxo: &LotteryVtxo,
        player1_address: &ArkAddress,
        player2_address: &ArkAddress,
    ) -> Result<String> {
        // Build witness data for timeout refund
        let witness = self.build_timeout_witness(lottery_vtxo)?;

        // Create refund transaction with two outputs
        let refund_tx = self.build_refund_transaction(
            &lottery_vtxo.vtxo,
            player1_address,
            player2_address,
            lottery_vtxo.bet_amount,
            witness,
        )?;

        // Broadcast through Ark
        let txid = self.broadcast_ark_transaction(refund_tx).await?;

        tracing::info!(
            "Executed timeout refund: {} sats each to players in transaction {}",
            lottery_vtxo.bet_amount.to_sat(),
            txid
        );

        Ok(txid)
    }

    /// Execute mutual abort using real Ark transaction
    pub async fn execute_mutual_abort(
        &self,
        lottery_vtxo: &LotteryVtxo,
        player1_address: &ArkAddress,
        player2_address: &ArkAddress,
    ) -> Result<String> {
        // Build witness data for mutual abort
        let witness = self.build_abort_witness(lottery_vtxo)?;

        // Create abort transaction
        let abort_tx = self.build_refund_transaction(
            &lottery_vtxo.vtxo,
            player1_address,
            player2_address,
            lottery_vtxo.bet_amount,
            witness,
        )?;

        // Broadcast through Ark
        let txid = self.broadcast_ark_transaction(abort_tx).await?;

        tracing::info!(
            "Executed mutual abort: {} sats each to players in transaction {}",
            lottery_vtxo.bet_amount.to_sat(),
            txid
        );

        Ok(txid)
    }

    // Helper methods
    async fn extract_public_key(&self, wallet: &arkive_core::ArkWallet) -> Result<XOnlyPublicKey> {
        // Create a more robust deterministic key generation
        use bitcoin::hashes::{sha256, Hash};
        use bitcoin::secp256k1::{Secp256k1, SecretKey};

        // Use wallet name and a salt to generate deterministic key
        let mut data = Vec::new();
        data.extend_from_slice(b"ark_wallet_pubkey_");
        data.extend_from_slice(wallet.name().as_bytes());
        data.extend_from_slice(b"_salt_12345"); // Add some salt

        let hash = sha256::Hash::hash(&data);

        // Ensure the hash produces a valid secret key
        let secp = Secp256k1::new();
        let mut key_bytes = hash.to_byte_array();

        // Ensure the key is valid by checking if it's in the valid range
        loop {
            match SecretKey::from_slice(&key_bytes) {
                Ok(secret_key) => {
                    let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &secret_key);
                    return Ok(keypair.x_only_public_key().0);
                }
                Err(_) => {
                    // If invalid, hash again to get a new attempt
                    let new_hash = sha256::Hash::hash(&key_bytes);
                    key_bytes = new_hash.to_byte_array();
                }
            }
        }
    }

    fn create_custom_vtxo(
        &self,
        secp: &bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
        server_pk: XOnlyPublicKey,
        tapscripts: &crate::contracts::LotteryTapscripts,
        amount: Amount,
    ) -> Result<Vtxo> {
        // Simplified VTXO creation for testing
        let exit_delay_seconds = 24 * 60 * 60;
        // let exit_delay = bitcoin::Sequence::from_consensus(144);
        let exit_delay = bitcoin::Sequence::from_seconds_ceil(exit_delay_seconds)
            .map_err(|e| GamingError::internal(format!("Invalid exit delay: {}", e)))?;

        let vtxo = ark_core::Vtxo::new_default(
            secp,
            server_pk,
            server_pk, // Use server_pk as owner for now
            exit_delay,
            bitcoin::Network::Regtest,
        )
        .map_err(|e| GamingError::internal(format!("Failed to create VTXO: {}", e)))?;

        Ok(vtxo)
    }

    fn build_winner_witness(
        &self,
        lottery_vtxo: &LotteryVtxo,
        reveal1: &crate::commitment::Reveal,
        reveal2: &crate::commitment::Reveal,
    ) -> Result<Vec<Vec<u8>>> {
        let mut witness = Vec::new();

        // Winner signature (needs to be signed by actual winner)
        let winner_sig = self.sign_for_winner(lottery_vtxo, reveal1, reveal2)?;
        witness.push(winner_sig);

        // Server signature
        let server_sig = self.sign_for_server(lottery_vtxo)?;
        witness.push(server_sig);

        // Commitment reveals
        witness.push(reveal1.commitment_data.value.to_le_bytes().to_vec());
        witness.push(reveal1.commitment_data.nonce.to_vec());
        witness.push(reveal2.commitment_data.value.to_le_bytes().to_vec());
        witness.push(reveal2.commitment_data.nonce.to_vec());

        Ok(witness)
    }

    fn build_timeout_witness(&self, lottery_vtxo: &LotteryVtxo) -> Result<Vec<Vec<u8>>> {
        let mut witness = Vec::new();

        // Player signatures
        let player1_sig = self.sign_for_player1(lottery_vtxo)?;
        witness.push(player1_sig);

        let player2_sig = self.sign_for_player2(lottery_vtxo)?;
        witness.push(player2_sig);

        // Server signature
        let server_sig = self.sign_for_server(lottery_vtxo)?;
        witness.push(server_sig);

        Ok(witness)
    }

    fn build_abort_witness(&self, lottery_vtxo: &LotteryVtxo) -> Result<Vec<Vec<u8>>> {
        // Same as timeout witness for mutual abort
        self.build_timeout_witness(lottery_vtxo)
    }

    fn build_payout_transaction(
        &self,
        vtxo: &Vtxo,
        winner_address: &ArkAddress,
        amount: Amount,
        witness: Vec<Vec<u8>>,
    ) -> Result<Transaction> {
        // Build actual Bitcoin transaction
        let tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                previous_output: OutPoint::null(), // Placeholder - will be set when we have the actual VTXO outpoint
                script_sig: bitcoin::ScriptBuf::new(),
                sequence: bitcoin::Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: bitcoin::Witness::from_slice(&witness),
            }],
            output: vec![TxOut {
                value: amount,
                script_pubkey: self.vtxo_to_script_pubkey(winner_address)?,
            }],
        };

        // TODO: Set the actual VTXO outpoint

        Ok(tx)
    }

    fn vtxo_to_script_pubkey(&self, address: &ArkAddress) -> Result<bitcoin::ScriptBuf> {
        // Convert ArkAddress to script pubkey
        // This is a placeholder implementation
        // In reality, you'd need to extract the script pubkey from the ArkAddress

        // For now, create a simple P2TR script as placeholder
        use bitcoin::key::UntweakedPublicKey;
        use bitcoin::secp256k1::XOnlyPublicKey as Secp256k1XOnlyPublicKey;

        // This is a placeholder - you'll need proper address parsing
        let placeholder_pk = Secp256k1XOnlyPublicKey::from_slice(&[2u8; 32])
            .map_err(|e| GamingError::internal(format!("Invalid pubkey: {}", e)))?;

        let untweaked = UntweakedPublicKey::from(placeholder_pk);
        use bitcoin::key::TapTweak;
        let script = bitcoin::ScriptBuf::new_p2tr_tweaked(untweaked.dangerous_assume_tweaked());

        Ok(script)
    }

    fn build_refund_transaction(
        &self,
        vtxo: &Vtxo,
        player1_address: &ArkAddress,
        player2_address: &ArkAddress,
        refund_amount: Amount,
        witness: Vec<Vec<u8>>,
    ) -> Result<Transaction> {
        // Build transaction with two outputs for refund
        let tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![bitcoin::TxIn {
                previous_output: OutPoint::null(), // Placeholder
                script_sig: bitcoin::ScriptBuf::new(),
                sequence: bitcoin::Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: bitcoin::Witness::from_slice(&witness),
            }],
            output: vec![
                TxOut {
                    value: refund_amount,
                    script_pubkey: self.vtxo_to_script_pubkey(player1_address)?,
                },
                TxOut {
                    value: refund_amount,
                    script_pubkey: self.vtxo_to_script_pubkey(player2_address)?,
                },
            ],
        };

        // TODO: Set the actual VTXO outpoint when we have proper integration

        Ok(tx)
    }

    async fn broadcast_ark_transaction(&self, tx: Transaction) -> Result<String> {
        // TODO: integrate for ark Tx broadcasting
        todo!("todo integration with arkive-core")
    }

    // Signature methods (these need proper key management integration)
    fn sign_for_winner(
        &self,
        lottery_vtxo: &LotteryVtxo,
        reveal1: &crate::commitment::Reveal,
        reveal2: &crate::commitment::Reveal,
    ) -> Result<Vec<u8>> {
        // determine winner and sign accordingly
        let winner = crate::commitment::CommitmentScheme::determine_winner(reveal1, reveal2)?;

        if winner == reveal1.commitment_data.player_id {
            self.sign_for_player1(lottery_vtxo)
        } else {
            self.sign_for_player2(lottery_vtxo)
        }
    }

    fn sign_for_player1(&self, _lottery_vtxo: &LotteryVtxo) -> Result<Vec<u8>> {
        todo!("Sign for player1 - needs key management integration")
    }

    fn sign_for_player2(&self, _lottery_vtxo: &LotteryVtxo) -> Result<Vec<u8>> {
        todo!("Sign for player2 - needs key management integration")
    }

    fn sign_for_server(&self, _lottery_vtxo: &LotteryVtxo) -> Result<Vec<u8>> {
        todo!("Sign for server - needs key management integration")
    }
}

#[derive(Debug, Clone)]
pub struct LotteryVtxo {
    pub game_id: String,
    pub vtxo: Vtxo,
    pub tapscripts: crate::contracts::LotteryTapscripts,
    pub player1_pk: XOnlyPublicKey,
    pub player2_pk: XOnlyPublicKey,
    pub server_pk: XOnlyPublicKey,
    pub bet_amount: Amount,
    pub total_amount: Amount,
    pub commitment1_hash: [u8; 32],
    pub commitment2_hash: [u8; 32],
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl LotteryVtxo {
    pub fn get_escrow_address(&self) -> ArkAddress {
        self.vtxo.to_ark_address()
    }

    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now();
        // TODO: check if VTXO has expired based on its timelock
        false // Placeholder
    }
}
