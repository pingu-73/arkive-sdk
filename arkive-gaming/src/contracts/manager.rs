use crate::contracts::compiler::{ArkadeCompiler, CompiledContract};
use crate::error::{GamingError, Result};
use bitcoin::key::Keypair;
use bitcoin::secp256k1::{Message, Secp256k1};
use std::collections::HashMap;
use std::str::FromStr;
use sha2::{Digest, Sha256};

pub struct ContractManager {
    compiler: ArkadeCompiler,
    compiled_contracts: HashMap<String, CompiledContract>,
}

impl ContractManager {
    pub fn new(compiler_path: Option<String>) -> Self {
        Self {
            compiler: ArkadeCompiler::new(compiler_path),
            compiled_contracts: HashMap::new(),
        }
    }

    /// Initialize and compile the two-player lottery contract
    pub async fn initialize(&mut self) -> Result<()> {
        // Load the embedded contract instead of compiling 'coz arkade-os is not yet working properly
        let compiled = self.compiler.load_embedded_lottery_contract()?;
        
        self.compiled_contracts.insert("TwoPlayerLottery".to_string(), compiled);
        
        tracing::info!("Loaded TwoPlayerLottery contract successfully");
        Ok(())
    }

    /// Create a new lottery contract instance
    pub fn create_lottery_contract(
        &self,
        player1_keypair: &Keypair,
        player2_keypair: &Keypair,
        server_keypair: &Keypair,
        commitment1: &str,
        commitment2: &str,
        bet_amount: u64,
        game_timeout: u64,
    ) -> Result<LotteryContract> {
        let contract = self.compiled_contracts
            .get("TwoPlayerLottery")
            .ok_or_else(|| GamingError::internal("TwoPlayerLottery contract not compiled"))?;

        let player1_pubkey = hex::encode(player1_keypair.public_key().serialize());
        let player2_pubkey = hex::encode(player2_keypair.public_key().serialize());
        let server_pubkey = hex::encode(server_keypair.public_key().serialize());

        let params = self.compiler.generate_lottery_params(
            &player1_pubkey,
            &player2_pubkey,
            &server_pubkey,
            commitment1,
            commitment2,
            bet_amount,
            game_timeout,
        );

        Ok(LotteryContract {
            compiled_contract: contract.clone(),
            params,
            player1_keypair: *player1_keypair,
            player2_keypair: *player2_keypair,
            server_keypair: *server_keypair,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LotteryContract {
    pub compiled_contract: CompiledContract,
    pub params: HashMap<String, String>,
    pub player1_keypair: Keypair,
    pub player2_keypair: Keypair,
    pub server_keypair: Keypair,
}

impl LotteryContract {
    /// Get the Bitcoin Script for player 1 winning
    pub fn get_player1_wins_script(&self, server_variant: bool) -> Result<Vec<String>> {
        let compiler = ArkadeCompiler::new(None);
        compiler.get_function_script(&self.compiled_contract, "player1Wins", server_variant)
    }

    /// Get the Bitcoin Script for player 2 winning
    pub fn get_player2_wins_script(&self, server_variant: bool) -> Result<Vec<String>> {
        let compiler = ArkadeCompiler::new(None);
        compiler.get_function_script(&self.compiled_contract, "player2Wins", server_variant)
    }

    /// Get the Bitcoin Script for player 1 timeout win
    pub fn get_player1_timeout_script(&self, server_variant: bool) -> Result<Vec<String>> {
        let compiler = ArkadeCompiler::new(None);
        compiler.get_function_script(&self.compiled_contract, "player1TimeoutWin", server_variant)
    }

    /// Get the Bitcoin Script for player 2 timeout win
    pub fn get_player2_timeout_script(&self, server_variant: bool) -> Result<Vec<String>> {
        let compiler = ArkadeCompiler::new(None);
        compiler.get_function_script(&self.compiled_contract, "player2TimeoutWin", server_variant)
    }

    /// Get the Bitcoin Script for mutual abort
    pub fn get_mutual_abort_script(&self, server_variant: bool) -> Result<Vec<String>> {
        let compiler = ArkadeCompiler::new(None);
        compiler.get_function_script(&self.compiled_contract, "mutualAbort", server_variant)
    }

    /// Generate the Taproot address for this lottery contract
    pub fn get_contract_address(&self) -> Result<String> {

        // deterministic seed from contract parameters
        let mut hasher = Sha256::new();
        hasher.update("lottery_contract");
        hasher.update(self.params.get("player1").unwrap_or(&"".to_string()));
        hasher.update(self.params.get("player2").unwrap_or(&"".to_string()));
        hasher.update(self.params.get("betAmount").unwrap_or(&"0".to_string()));
        
        let seed = hasher.finalize();
        
        // keypair from the seed
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let secret_key = bitcoin::secp256k1::SecretKey::from_slice(&seed[..32])
            .map_err(|e| GamingError::internal(format!("Failed to create secret key: {}", e)))?;
        let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &secret_key);
        
        // compressed public key to x-only public key
        const DEFAULT_SERVER_PK: &str = "0209fc225f75331e20b2f2e01e290f2096ac96175e193cb701029e55cda5e7d5f6";
        let server_pk_str = self.params.get("server")
            .map(|s| s.as_str())
            .unwrap_or(DEFAULT_SERVER_PK);
        
        let server_pk = if server_pk_str.len() == 66 && (server_pk_str.starts_with("02") || server_pk_str.starts_with("03")) {
            let compressed_bytes = hex::decode(server_pk_str)
                .map_err(|e| GamingError::internal(format!("Invalid hex in server pubkey: {}", e)))?;
            
            let compressed_pk = bitcoin::secp256k1::PublicKey::from_slice(&compressed_bytes)
                .map_err(|e| GamingError::internal(format!("Invalid compressed public key: {}", e)))?;
            
            compressed_pk.x_only_public_key().0
        } else if server_pk_str.len() == 64 {
            bitcoin::XOnlyPublicKey::from_str(server_pk_str)
                .map_err(|e| GamingError::internal(format!("Invalid x-only public key: {}", e)))?
        } else {
            return Err(GamingError::internal("Invalid server public key format"));
        };
        
        let (owner_pk, _) = keypair.x_only_public_key();
        
        let exit_delay_seconds = 3600;
        let exit_delay = bitcoin::Sequence::from_seconds_floor(exit_delay_seconds)
            .map_err(|e| GamingError::internal(format!("Invalid exit delay: {}", e)))?;
        
        let vtxo = ark_core::Vtxo::new_default(
            &secp,
            server_pk,
            owner_pk,
            exit_delay,
            bitcoin::Network::Regtest,
        ).map_err(|e| GamingError::internal(format!("Failed to create VTXO: {}", e)))?;
        
        Ok(vtxo.to_ark_address().to_string())
    }

    /// Create witness data for player 1 winning
    pub fn create_player1_wins_witness(
        &self,
        value1: u64,
        nonce1: [u8; 32],
        value2: u64,
        nonce2: [u8; 32],
    ) -> Result<Vec<Vec<u8>>> {
        let mut witness = Vec::new();
        
        // TODO: actual sig
        // let signature = self.sign_for_player1(&[])?;
        // witness.push(signature);
        let message = format!("player1_wins_{}_{}", value1, value2);
        let signature = self.sign_for_player1(message.as_bytes())?;
        witness.push(signature);
        
        witness.push(value1.to_le_bytes().to_vec());
        witness.push(nonce1.to_vec());
        witness.push(value2.to_le_bytes().to_vec());
        witness.push(nonce2.to_vec());
        
        Ok(witness)
    }

    /// Create witness data for player 2 winning
    pub fn create_player2_wins_witness(
        &self,
        value1: u64,
        nonce1: [u8; 32],
        value2: u64,
        nonce2: [u8; 32],
    ) -> Result<Vec<Vec<u8>>> {
        let mut witness = Vec::new();
        
        // TODO: actual sig
        // let signature = self.sign_for_player2(&[])?;
        // witness.push(signature);
        let message = format!("player2_wins_{}_{}", value1, value2);
        let signature = self.sign_for_player2(message.as_bytes())?;
        witness.push(signature);
        
        witness.push(value1.to_le_bytes().to_vec());
        witness.push(nonce1.to_vec());
        witness.push(value2.to_le_bytes().to_vec());
        witness.push(nonce2.to_vec());
        
        Ok(witness)
    }

    /// Sign a message for player 1
    fn sign_for_player1(&self, message: &[u8]) -> Result<Vec<u8>> {
        use bitcoin::secp256k1::{Message, Secp256k1};
        use sha2::{Digest, Sha256};
        
        let secp = Secp256k1::new();
        
        // hash msg to ensure 32 bytes
        let mut hasher = Sha256::new();
        hasher.update(message);
        let hash: [u8; 32] = hasher.finalize().into();
        
        let msg = Message::from_digest_slice(&hash)
            .map_err(|e| GamingError::internal(format!("Invalid message: {}", e)))?;
        
        let signature = secp.sign_schnorr_no_aux_rand(&msg, &self.player1_keypair);
        Ok(signature.as_ref().to_vec())
    }

    /// Sign a message for player 2
    fn sign_for_player2(&self, message: &[u8]) -> Result<Vec<u8>> {
        use bitcoin::secp256k1::{Message, Secp256k1};
        use sha2::{Digest, Sha256};
        
        let secp = Secp256k1::new();
        
        // hash msg to ensure 32 bytes
        let mut hasher = Sha256::new();
        hasher.update(message);
        let hash: [u8; 32] = hasher.finalize().into();
        
        let msg = Message::from_digest_slice(&hash)
            .map_err(|e| GamingError::internal(format!("Invalid message: {}", e)))?;
        
        let signature = secp.sign_schnorr_no_aux_rand(&msg, &self.player2_keypair);
        Ok(signature.as_ref().to_vec())
    }

    /// Sign a message for the server
    #[allow(dead_code)]
    fn sign_for_server(&self, message: &[u8]) -> Result<Vec<u8>> {
        let secp = Secp256k1::new();
        let msg = Message::from_digest_slice(message)
            .map_err(|e| GamingError::internal(format!("Invalid message: {}", e)))?;
        
        let signature = secp.sign_schnorr_no_aux_rand(&msg, &self.server_keypair);
        Ok(signature.as_ref().to_vec())
    }
}