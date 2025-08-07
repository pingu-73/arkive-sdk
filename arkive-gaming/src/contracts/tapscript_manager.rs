#![allow(unused_imports)]
use crate::error::{GamingError, Result};
use bitcoin::{Amount, XOnlyPublicKey};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

pub struct TapscriptManager {
    // scripts_dir: String,
}

impl TapscriptManager {
    // pub fn new() -> Self {
    //     Self {
    //         scripts_dir: "scripts".to_string(),
    //     }
    // }

    pub fn new() -> Self {
        Self {}
    }

    pub fn get_embedded_scripts() -> HashMap<&'static str, &'static str> {
        let mut scripts = HashMap::new();

        scripts.insert(
            "lottery_escrow",
            include_str!("../../scripts/miniscript/lottery_escrow.miniscript"),
        );
        scripts.insert(
            "winner_payout",
            include_str!("../../scripts/miniscript/winner_payout.miniscript"),
        );
        scripts.insert(
            "timeout_refund",
            include_str!("../../scripts/miniscript/timeout_refund.miniscript"),
        );
        scripts.insert(
            "mutual_abort",
            include_str!("../../scripts/miniscript/mutual_abort.miniscript"),
        );

        scripts
    }

    /// Compile lottery tapscripts by reading from script files and substituting parameters
    pub fn compile_lottery_scripts(
        &self,
        player1_pk: XOnlyPublicKey,
        player2_pk: XOnlyPublicKey,
        asp_pk: XOnlyPublicKey,
        bet_amount: Amount,
        commitment1_hash: [u8; 32],
        commitment2_hash: [u8; 32],
        timeout_delay_seconds: u32,
    ) -> Result<LotteryTapscripts> {
        let scripts = Self::get_embedded_scripts();

        // Create parameter substitution map
        let mut params = HashMap::new();
        params.insert(
            "player1_pubkey".to_string(),
            hex::encode(player1_pk.serialize()),
        );
        params.insert(
            "player2_pubkey".to_string(),
            hex::encode(player2_pk.serialize()),
        );
        params.insert("asp_pubkey".to_string(), hex::encode(asp_pk.serialize()));
        params.insert("bet_amount".to_string(), bet_amount.to_sat().to_string());
        params.insert(
            "total_amount".to_string(),
            (bet_amount * 2).to_sat().to_string(),
        );
        
        // params.insert("timeout_delay".to_string(), timeout_delay.to_string());
        let sequence_value = bitcoin::Sequence::from_seconds_ceil(timeout_delay_seconds)
        .map_err(|e| GamingError::internal(format!("Invalid timeout delay: {}", e)))?;
        params.insert("timeout_delay".to_string(), sequence_value.to_consensus_u32().to_string());

        params.insert(
            "commitment1_hash".to_string(),
            hex::encode(commitment1_hash),
        );
        params.insert(
            "commitment2_hash".to_string(),
            hex::encode(commitment2_hash),
        );
        params.insert(
            "winner_pubkey".to_string(),
            hex::encode(player1_pk.serialize()),
        );

        // Compile each embedded script
        let escrow_script = self.compile_embedded_script(scripts["lottery_escrow"], &params)?;
        let winner_script = self.compile_embedded_script(scripts["winner_payout"], &params)?;
        let timeout_script = self.compile_embedded_script(scripts["timeout_refund"], &params)?;
        let abort_script = self.compile_embedded_script(scripts["mutual_abort"], &params)?;

        Ok(LotteryTapscripts {
            escrow_script,
            winner_payout_script: winner_script,
            timeout_refund_script: timeout_script,
            mutual_abort_script: abort_script,
        })
    }

    pub fn validate_scripts(&self) -> Result<()> {
        let scripts = Self::get_embedded_scripts();

        if scripts.len() != 4 {
            return Err(GamingError::internal("Missing embedded scripts"));
        }

        for (name, content) in scripts {
            if content.is_empty() {
                return Err(GamingError::internal(format!(
                    "Empty embedded script: {}",
                    name
                )));
            }
        }

        tracing::info!("All required lottery scripts embedded and validated");
        Ok(())
    }

    /// Compile a specific script for a known winner
    pub fn compile_winner_script(
        &self,
        winner_pk: XOnlyPublicKey,
        asp_pk: XOnlyPublicKey,
        total_amount: Amount,
        commitment1_hash: [u8; 32],
        commitment2_hash: [u8; 32],
    ) -> Result<CompiledScript> {
        let scripts = Self::get_embedded_scripts();

        let mut params = HashMap::new();
        params.insert(
            "winner_pubkey".to_string(),
            hex::encode(winner_pk.serialize()),
        );
        params.insert("asp_pubkey".to_string(), hex::encode(asp_pk.serialize()));
        params.insert(
            "total_amount".to_string(),
            total_amount.to_sat().to_string(),
        );
        params.insert(
            "commitment1_hash".to_string(),
            hex::encode(commitment1_hash),
        );
        params.insert(
            "commitment2_hash".to_string(),
            hex::encode(commitment2_hash),
        );

        // Use embedded script instead of file path
        self.compile_embedded_script(scripts["winner_payout"], &params)
    }

    fn compile_embedded_script(
        &self,
        template: &str,
        params: &HashMap<String, String>,
    ) -> Result<CompiledScript> {
        // Substitute parameters in the embedded template
        let mut compiled_content = template.to_string();
        for (key, value) in params {
            compiled_content = compiled_content.replace(key, value);
        }

        // Try external compiler, fall back to template
        match self.compile_with_external_compiler(&compiled_content) {
            Ok(compiled_output) => Ok(CompiledScript {
                source: template.to_string(),
                compiled: compiled_output,
            }),
            Err(e) => {
                tracing::warn!("External compiler failed: {}, using template", e);
                Ok(CompiledScript {
                    source: template.to_string(),
                    compiled: compiled_content,
                })
            }
        }
    }

    fn compile_with_external_compiler(&self, script_content: &str) -> Result<String> {
        // Try to use the external miniscript compiler
        let output = Command::new("miniscript-compiler")
            .arg("descriptor")
            .arg(script_content)
            .output()
            .map_err(|e| {
                GamingError::internal(format!("Failed to run miniscript compiler: {}", e))
            })?;

        if !output.status.success() {
            return Err(GamingError::internal(format!(
                "Miniscript compilation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let compiled_output = String::from_utf8(output.stdout)
            .map_err(|e| GamingError::internal(format!("Invalid compiler output: {}", e)))?;

        Ok(compiled_output.trim().to_string())
    }
}

#[derive(Debug, Clone)]
pub struct LotteryTapscripts {
    pub escrow_script: CompiledScript,
    pub winner_payout_script: CompiledScript,
    pub timeout_refund_script: CompiledScript,
    pub mutual_abort_script: CompiledScript,
}

#[derive(Debug, Clone)]
pub struct CompiledScript {
    pub source: String,
    pub compiled: String,
}
