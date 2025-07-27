use crate::error::{GamingError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledContract {
    #[serde(rename = "contractName")]
    pub contract_name: String,
    #[serde(rename = "constructorInputs")]
    pub constructor_inputs: Vec<ContractInput>,
    pub functions: Vec<ContractFunction>,
    pub source: String,
    pub compiler: CompilerInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractInput {
    pub name: String,
    #[serde(rename = "type")]
    pub input_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractFunction {
    pub name: String,
    #[serde(rename = "functionInputs")]
    pub function_inputs: Vec<ContractInput>,
    #[serde(rename = "serverVariant")]
    pub server_variant: bool,
    pub require: Vec<serde_json::Value>,
    pub asm: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerInfo {
    pub name: String,
    pub version: String,
}

#[allow(dead_code)]
pub struct ArkadeCompiler {
    compiler_path: String,
}

impl ArkadeCompiler {
    pub fn new(compiler_path: Option<String>) -> Self {
        let compiler_path = compiler_path.unwrap_or_else(|| "arkadec".to_string());
        Self { compiler_path }
    }

    pub async fn compile_contract(&self, contract_source: &str) -> Result<CompiledContract> {
        if contract_source.contains("TwoPlayerLottery") {
            return self.load_embedded_lottery_contract();
        }

        // If it's not the lottery contract, return an error for now
        Err(GamingError::internal("Only TwoPlayerLottery contract is supported"))
    }
    
    pub fn load_embedded_lottery_contract(&self) -> Result<CompiledContract> {
        let contract_json = include_str!("../../contracts/two_player_lottery.json");
        let compiled_contract: CompiledContract = serde_json::from_str(contract_json)
            .map_err(|e| GamingError::internal(format!("Failed to parse embedded contract: {}", e)))?;
        Ok(compiled_contract)
    }

    /// Get the Bitcoin Script for a specific function
    pub fn get_function_script(&self, contract: &CompiledContract, function_name: &str, server_variant: bool) -> Result<Vec<String>> {
        let function = contract.functions
            .iter()
            .find(|f| f.name == function_name && f.server_variant == server_variant)
            .ok_or_else(|| GamingError::internal(format!("Function {} not found", function_name)))?;

        Ok(function.asm.clone())
    }

    /// Generate contract parameters for two-player lottery
    pub fn generate_lottery_params(
        &self,
        player1_pubkey: &str,
        player2_pubkey: &str,
        server_pubkey: &str,
        commitment1: &str,
        commitment2: &str,
        bet_amount: u64,
        game_timeout: u64,
    ) -> std::collections::HashMap<String, String> {
        let mut params = std::collections::HashMap::new();
        params.insert("player1".to_string(), player1_pubkey.to_string());
        params.insert("player2".to_string(), player2_pubkey.to_string());
        params.insert("server".to_string(), server_pubkey.to_string());
        params.insert("commitment1".to_string(), commitment1.to_string());
        params.insert("commitment2".to_string(), commitment2.to_string());
        params.insert("betAmount".to_string(), bet_amount.to_string());
        params.insert("gameTimeout".to_string(), game_timeout.to_string());
        params
    }
}