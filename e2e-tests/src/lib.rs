use anyhow::{Context, Result};
use arkive_core::{Amount, Network, WalletManager};
use reqwest::Client;
use serde_json::Value;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;
use tracing::{debug, info, warn};

pub const ARK_SERVER_URL: &str = "http://localhost:7070";
pub const NIGIRI_RPC_URL: &str = "http://localhost:18443";
pub const NIGIRI_USER: &str = "admin1";
pub const NIGIRI_PASS: &str = "123";
pub const ARK_TRANSACTION_FEE: u64 = 0;

pub struct TestEnvironment {
    pub temp_dir: TempDir,
    pub manager: WalletManager,
    pub http_client: Client,
    pub initialized: bool,
}

impl TestEnvironment {
    pub async fn new() -> Result<Self> {
        let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
        let manager = WalletManager::new(temp_dir.path())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create wallet manager: {}", e))?;

        let http_client = Client::new();

        let mut env = Self {
            temp_dir,
            manager,
            http_client,
            initialized: false,
        };

        env.initialize().await?;

        Ok(env)
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }

        info!("Initializing test environment...");

        self.check_services().await?;

        info!("  Mining initial 506 blocks...");
        self.mine_blocks(506).await?;

        self.fund_ark_server().await?;

        self.initialized = true;
        info!(" Test environment initialized successfully");
        Ok(())
    }

    async fn check_services(&self) -> Result<()> {
        let nigiri_response = self
            .http_client
            .post(NIGIRI_RPC_URL)
            .basic_auth(NIGIRI_USER, Some(NIGIRI_PASS))
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "getblockchaininfo",
                "params": [],
                "id": 1
            }))
            .send()
            .await
            .context("Failed to connect to Nigiri - is it running with 'nigiri start --ark'?")?;

        if !nigiri_response.status().is_success() {
            anyhow::bail!(
                "Nigiri is not responding properly. Make sure to run 'nigiri start --ark'"
            );
        }

        let ark_response = self
            .http_client
            .get(format!("{}/v1/info", ARK_SERVER_URL))
            .send()
            .await
            .context("Failed to connect to Ark server - is Nigiri running with --ark flag?")?;

        if !ark_response.status().is_success() {
            anyhow::bail!("Ark server is not responding properly");
        }

        info!(" Both Nigiri and Ark server are running");
        Ok(())
    }

    async fn get_forfeit_addr(&self) -> Result<String> {
        let response = self
            .http_client
            .get(format!("{}/v1/info", ARK_SERVER_URL))
            .send()
            .await?;

        let info: Value = response.json().await?;
        let forfeit_address = info["forfeitAddress"]
            .as_str()
            .context("Failed to get forfeit address from Ark server")?;

        Ok(forfeit_address.to_string())
    }

    async fn get_admin_wallet_addr(&self) -> Result<String> {
        let response = self
            .http_client
            .get(format!("{}/v1/admin/wallet/address", ARK_SERVER_URL))
            .send()
            .await?;

        let info: Value = response.json().await?;
        let forfeit_address = info["address"]
            .as_str()
            .context("Failed to get admin wallet address from Ark server")?;

        Ok(forfeit_address.to_string())
    }

    async fn fund_ark_server(&self) -> Result<()> {
        let forfeit_addr = self.get_forfeit_addr().await?;
        let admin_wallet_addr = self.get_admin_wallet_addr().await?;

        info!(" Funding Ark server forfeit address: {}", forfeit_addr);
        self.nigiri_faucet(&forfeit_addr, 100.0).await?;

        info!(" Funding admin wallet: {}", admin_wallet_addr);
        self.nigiri_faucet(&admin_wallet_addr, 100.0).await?;

        info!(" Mining block to confirm the funding");
        self.mine_blocks(4).await?;

        info!(" Ark server funded successfully");
        Ok(())
    }

    pub async fn nigiri_faucet(&self, address: &str, amount_btc: f64) -> Result<String> {
        let output = Command::new("nigiri")
            .args(["faucet", address, &amount_btc.to_string()])
            .output()
            .context("Failed to execute nigiri faucet command - is nigiri in PATH?")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Nigiri faucet failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let txid = stdout
            .lines()
            .find(|line| line.starts_with("txId:"))
            .and_then(|line| line.split(':').nth(1))
            .map(|s| s.trim().to_string())
            .context("Failed to parse transaction ID from nigiri output")?;

        info!(
            " Funded {} with {} BTC, txid: {}",
            address, amount_btc, txid
        );
        Ok(txid)
    }

    pub async fn mine_blocks(&self, count: u32) -> Result<Vec<String>> {
        let new_addr_output = Command::new("nigiri")
            .args(["rpc", "getnewaddress"])
            .output()
            .context("Failed to get new address from nigiri")?;

        if !new_addr_output.status.success() {
            let stderr = String::from_utf8_lossy(&new_addr_output.stderr);
            anyhow::bail!("Failed to get new address: {}", stderr);
        }

        let new_address = String::from_utf8_lossy(&new_addr_output.stdout)
            .trim()
            .to_string();

        // mine blocks to that addr
        let output = Command::new("nigiri")
            .args(["rpc", "generatetoaddress", &count.to_string(), &new_address])
            .output()
            .context("Failed to execute nigiri mine command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Mining blocks failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // parse json array of block hashes
        let block_hashes: Vec<String> = serde_json::from_str(&stdout)
            .context("Failed to parse block hashes from nigiri output")?;

        info!("  Mined {} blocks", block_hashes.len());
        Ok(block_hashes)
    }
}

pub struct TestWallet {
    pub name: String,
    pub wallet: std::sync::Arc<arkive_core::ArkWallet>,
    pub mnemonic: String,
}

impl TestWallet {
    pub async fn create(manager: &WalletManager, name: &str) -> Result<Self> {
        let (wallet, mnemonic) = manager
            .create_wallet(name, Network::Regtest)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create wallet: {}", e))?;

        Ok(Self {
            name: name.to_string(),
            wallet,
            mnemonic,
        })
    }

    pub async fn get_boarding_address(&self) -> Result<String> {
        let addr = self
            .wallet
            .get_boarding_address()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get boarding address: {}", e))?;
        Ok(addr.address)
    }

    pub async fn get_ark_address(&self) -> Result<String> {
        let addr = self
            .wallet
            .get_ark_address()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get ark address: {}", e))?;
        Ok(addr.address)
    }

    pub async fn get_balance(&self) -> Result<(Amount, Amount)> {
        self.wallet
            .ark_balance()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get balance: {}", e))
    }

    pub async fn participate_in_round(&self) -> Result<Option<String>> {
        self.wallet
            .participate_in_round()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to participate in round: {}", e))
    }

    pub async fn send_ark(&self, to_address: &str, amount_sats: u64) -> Result<String> {
        self.wallet
            .send_ark(to_address, Amount::from_sat(amount_sats))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send ark transaction: {}", e))
    }

    pub async fn sync(&self) -> Result<()> {
        self.wallet
            .sync()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to sync wallet: {}", e))
    }

    pub async fn board_funds(&self, env: &TestEnvironment, amount_btc: f64) -> Result<String> {
        let boarding_addr = self.get_boarding_address().await?;
        info!(
            " Funding {} with {} BTC at boarding address: {}",
            self.name, amount_btc, boarding_addr
        );

        let txid = env.nigiri_faucet(&boarding_addr, amount_btc).await?;
        info!(" Transaction ID: {}", txid);

        info!("  Mining blocks to confirm transaction...");
        env.mine_blocks(2).await?;

        info!("⏳ Waiting 10 seconds for transaction to be reflected...");
        sleep(Duration::from_secs(10)).await;

        info!(" Syncing {} to detect boarding outputs...", self.name);
        self.sync().await?;

        info!("⏳ Waiting 5 seconds after sync...");
        sleep(Duration::from_secs(70)).await;

        // Try to participate in round with retries
        for attempt in 1..=3 {
            info!(
                " {} participating in round (attempt {})...",
                self.name, attempt
            );

            match self.participate_in_round().await {
                Ok(Some(round_id)) => {
                    info!(
                        " {} successfully participated in round: {}",
                        self.name, round_id
                    );
                    return Ok(round_id);
                }
                Ok(None) => {
                    info!(
                        "  {} - No round participation needed (attempt {})",
                        self.name, attempt
                    );
                    if attempt < 3 {
                        info!("⏳ Waiting 5 seconds before retry...");
                        sleep(Duration::from_secs(5)).await;
                        // Try syncing again
                        info!(" Syncing again...");
                        self.sync().await?;
                        sleep(Duration::from_secs(60)).await;
                        continue;
                    }
                }
                Err(e) => {
                    warn!(
                        "⚠️  {} round participation failed (attempt {}): {}",
                        self.name, attempt, e
                    );
                    if attempt < 3 {
                        info!("⏳ Waiting 5 seconds before retry...");
                        sleep(Duration::from_secs(5)).await;
                        continue;
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        anyhow::bail!("Failed to participate in round after 3 attempts - boarding output may not have been detected")
    }

    pub async fn wait_for_balance(
        &self,
        expected_confirmed: u64,
        expected_pending: u64,
        timeout_secs: u64,
    ) -> Result<()> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        info!(
            " {} waiting for balance: {} confirmed, {} pending",
            self.name, expected_confirmed, expected_pending
        );

        while start.elapsed() < timeout {
            let (confirmed, pending) = self.get_balance().await?;
            debug!(
                " {} balance check: {} confirmed, {} pending (expecting {}/{})",
                self.name,
                confirmed.to_sat(),
                pending.to_sat(),
                expected_confirmed,
                expected_pending
            );

            if confirmed.to_sat() == expected_confirmed && pending.to_sat() == expected_pending {
                info!("✅ {} reached expected balance!", self.name);
                return Ok(());
            }
            sleep(Duration::from_millis(1000)).await;
        }

        let (confirmed, pending) = self.get_balance().await?;
        anyhow::bail!(
            "{} balance not reached within {} seconds. Expected: {}/{}, Got: {}/{}",
            self.name,
            timeout_secs,
            expected_confirmed,
            expected_pending,
            confirmed.to_sat(),
            pending.to_sat()
        );
    }

    pub async fn wait_for_incoming_transaction(
        &self,
        expected_pending: u64,
        timeout_secs: u64,
    ) -> Result<()> {
        info!(" {} syncing to receive incoming transactions...", self.name);
        self.sync().await?;

        // Wait a bit after sync
        sleep(Duration::from_secs(60)).await;

        self.wait_for_balance(0, expected_pending, timeout_secs)
            .await
    }
}

pub fn init_test_tracing() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            "info,arkive=debug,e2e_tests=info",
        ))
        .with(tracing_subscriber::fmt::layer().with_test_writer())
        .try_init();
}

pub async fn setup_test_environment() -> Result<TestEnvironment> {
    init_test_tracing();
    TestEnvironment::new().await
}
