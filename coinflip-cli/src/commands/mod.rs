#![allow(unused_variables)]
use arkive_core::{Amount, WalletManager};
use arkive_lottery::TwoPlayerGame;
use comfy_table::{presets::UTF8_FULL, Table};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GameStorage {
    games: HashMap<String, GameData>,
    player_secrets: HashMap<String, Vec<u8>>, // key: "game_id:player_id"
    pot_wallets: HashMap<String, String>,     // game_id -> pot_wallet_name
    player_wallets: HashMap<String, String>,  // "game_id:player_id" -> wallet_name
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GameData {
    id: String,
    bet_amount: u64,
    state: String,
    players: Vec<String>, // player IDs
    total_pot: u64,
    commitment_deadline: Option<i64>,
    reveal_deadline: Option<i64>,
    player_commitments: HashMap<String, bool>,
    player_reveals: HashMap<String, bool>,
    collected_bets: HashMap<String, BetData>,
    winner: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BetData {
    player_id: String,
    amount: u64,
    txid: String,
    timestamp: i64,
}

impl Default for GameStorage {
    fn default() -> Self {
        Self {
            games: HashMap::new(),
            player_secrets: HashMap::new(),
            pot_wallets: HashMap::new(),
            player_wallets: HashMap::new(),
        }
    }
}

fn get_storage_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arkive")
        .join("coinflip_games.json")
}

fn load_storage() -> GameStorage {
    let path = get_storage_path();
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(storage) = serde_json::from_str(&content) {
                return storage;
            }
        }
    }
    GameStorage::default()
}

fn save_storage(storage: &GameStorage) -> Result<(), Box<dyn std::error::Error>> {
    let path = get_storage_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(storage)?;
    std::fs::write(path, content)?;
    Ok(())
}

pub async fn create_game(
    wallet_manager: &WalletManager,
    wallet_name: &str,
    amount: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let player_wallet = wallet_manager.load_wallet(wallet_name).await?;
    let bet_amount = Amount::from_sat(amount);

    // Create a dedicated pot wallet for this game
    let pot_wallet_name = format!("pot_{}", Uuid::new_v4());
    let (pot_wallet, _mnemonic) = if player_wallet.is_mutinynet() {
        wallet_manager
            .create_wallet_mutinynet(&pot_wallet_name)
            .await?
    } else {
        wallet_manager
            .create_wallet(&pot_wallet_name, player_wallet.network())
            .await?
    };

    // Create new game with pot wallet
    let mut game = TwoPlayerGame::new(bet_amount, pot_wallet.clone()).await?;
    let game_id = game.id();

    // Add the creator as first player
    let creator_player_id = game.add_player(player_wallet).await?;

    let info = game.get_info();

    // Store game data
    let game_data = GameData {
        id: game_id.to_string(),
        bet_amount: amount,
        state: format!("{:?}", info.state),
        players: vec![creator_player_id.to_string()],
        total_pot: info.total_pot.to_sat(),
        commitment_deadline: info.commitment_deadline.map(|d| d.timestamp()),
        reveal_deadline: info.reveal_deadline.map(|d| d.timestamp()),
        player_commitments: HashMap::new(),
        player_reveals: HashMap::new(),
        collected_bets: HashMap::new(),
        winner: None,
    };

    let mut storage = load_storage();
    storage.games.insert(game_id.to_string(), game_data);
    storage
        .pot_wallets
        .insert(game_id.to_string(), pot_wallet_name.clone());

    // Store player-wallet mapping
    let player_key = format!("{}:{}", game_id, creator_player_id);
    storage
        .player_wallets
        .insert(player_key, wallet_name.to_string());

    save_storage(&storage)?;

    println!("Created new game with real betting!");
    println!("Game ID: {}", game_id);
    println!("Your Player ID: {}", creator_player_id);
    println!("Bet Amount: {} sats", amount);
    println!("Pot Wallet: {}", pot_wallet_name);
    println!("Pot Address: {}", game.get_pot_address().await?);
    println!("Waiting for second player to join...");
    println!();
    println!("Share this command with another player:");
    println!("coinflip join <their-wallet> {}", game_id);

    Ok(())
}

pub async fn join_game(
    wallet_manager: &WalletManager,
    wallet_name: &str,
    game_id_str: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let player_wallet = wallet_manager.load_wallet(wallet_name).await?;
    let game_id = Uuid::parse_str(game_id_str)?;

    let mut storage = load_storage();

    // Get pot wallet for this game
    let pot_wallet_name = storage
        .pot_wallets
        .get(&game_id.to_string())
        .ok_or("Game not found or pot wallet missing")?;

    let pot_wallet = wallet_manager.load_wallet(pot_wallet_name).await?;

    let (player_id, is_ready) = {
        let game_data = storage
            .games
            .get_mut(&game_id.to_string())
            .ok_or("Game not found")?;

        if game_data.players.len() >= 2 {
            return Err("Game is full".into());
        }

        // Generate new player ID for the joining player
        let player_id = Uuid::new_v4();
        game_data.players.push(player_id.to_string());

        let is_ready = if game_data.players.len() == 2 {
            game_data.state = "WaitingForBets".to_string();
            true
        } else {
            false
        };

        (player_id, is_ready)
    };

    // Store player-wallet mapping
    let player_key = format!("{}:{}", game_id, player_id);
    storage
        .player_wallets
        .insert(player_key, wallet_name.to_string());

    save_storage(&storage)?;

    println!("Joined game {}!", game_id);
    println!("Your Player ID: {}", player_id);

    if is_ready {
        println!("Game is now ready for betting!");
        println!();
        println!("Both players must now place their bets:");
        println!("coinflip bet {} {}", wallet_name, game_id);
    } else {
        println!("Waiting for one more player...");
    }

    Ok(())
}

pub async fn place_bet(
    wallet_manager: &WalletManager,
    wallet_name: &str,
    game_id_str: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let player_wallet = wallet_manager.load_wallet(wallet_name).await?;
    let game_id = Uuid::parse_str(game_id_str)?;

    let mut storage = load_storage();

    // Get pot wallet
    let pot_wallet_name = storage
        .pot_wallets
        .get(&game_id.to_string())
        .ok_or("Game not found")?;

    let pot_wallet = wallet_manager.load_wallet(pot_wallet_name).await?;

    // Find this player's ID
    let player_id = storage
        .player_wallets
        .iter()
        .find(|(key, wallet)| key.starts_with(&format!("{}:", game_id)) && wallet == &wallet_name)
        .map(|(key, _)| key.split(':').nth(1).unwrap().to_string())
        .ok_or("Player not found in this game")?;

    let (txid, bet_amount, is_commitment_ready) = {
        let game_data = storage
            .games
            .get_mut(&game_id.to_string())
            .ok_or("Game not found")?;

        if game_data.state != "WaitingForBets" {
            return Err(format!(
                "Game not in betting phase. Current state: {}",
                game_data.state
            )
            .into());
        }

        // Check if player already bet
        if game_data.collected_bets.contains_key(&player_id) {
            return Err("You have already placed your bet".into());
        }

        // Check balance
        let balance = player_wallet.balance().await?;
        let bet_amount = Amount::from_sat(game_data.bet_amount);

        // [Unsure refer back again] currently both pending and confirmed VTXO's
        let available_balance = balance.confirmed + balance.pending;

        if available_balance < bet_amount {
            return Err(format!(
                "Insufficient balance: need {} sats, have {} sats (confirmed: {}, pending: {})",
                bet_amount.to_sat(),
                available_balance.to_sat(),
                balance.confirmed.to_sat(),
                balance.pending.to_sat()
            )
            .into());
        }

        let pot_address = pot_wallet.get_ark_address().await?;
        println!("DEBUG: Pot address: '{}'", pot_address.address);
        println!("DEBUG: Player wallet: {}", wallet_name);
        println!("DEBUG: Bet amount: {} sats", bet_amount.to_sat());

        // Place the bet
        let txid = player_wallet
            .send_ark(&pot_address.address, bet_amount)
            .await?;
        println!("DEBUG: Transaction successful: {}", txid);

        // Update storage
        let bet_data = BetData {
            player_id: player_id.clone(),
            amount: game_data.bet_amount,
            txid: txid.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        game_data.collected_bets.insert(player_id, bet_data);
        game_data.total_pot += game_data.bet_amount;

        let is_commitment_ready = if game_data.collected_bets.len() == 2 {
            game_data.state = "BetsCollected".to_string();
            true
        } else {
            false
        };

        (txid, game_data.bet_amount, is_commitment_ready)
    };

    save_storage(&storage)?;

    println!("Bet placed successfully!");
    println!("Amount: {} sats", bet_amount);
    println!("Transaction ID: {}", txid);
    println!();

    if is_commitment_ready {
        println!("Both players have placed their bets!");
        println!("The commitment phase can now begin:");
        println!("coinflip commit {} {}", wallet_name, game_id);
    } else {
        println!("Waiting for other player to place their bet...");
    }

    Ok(())
}

pub async fn commit_to_game(
    wallet_manager: &WalletManager,
    wallet_name: &str,
    game_id_str: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let _wallet = wallet_manager.load_wallet(wallet_name).await?;
    let game_id = Uuid::parse_str(game_id_str)?;

    let mut storage = load_storage();

    // Find this player's ID
    let player_id = storage
        .player_wallets
        .iter()
        .find(|(key, wallet)| key.starts_with(&format!("{}:", game_id)) && wallet == &wallet_name)
        .map(|(key, _)| key.split(':').nth(1).unwrap().to_string())
        .ok_or("Player not found in this game")?;

    let (secret, is_reveal_phase) = {
        let game_data = storage
            .games
            .get_mut(&game_id.to_string())
            .ok_or("Game not found")?;

        // Check if all bets are collected
        if game_data.state == "BetsCollected" {
            game_data.state = "CommitmentPhase".to_string();
            game_data.commitment_deadline = Some(chrono::Utc::now().timestamp() + 300);
            // 5 minutes
        }

        if game_data.state != "CommitmentPhase" {
            return Err(format!(
                "Game not in commitment phase. Current state: {}",
                game_data.state
            )
            .into());
        }

        // Check if player already committed
        if *game_data
            .player_commitments
            .get(&player_id)
            .unwrap_or(&false)
        {
            return Err("You have already committed".into());
        }

        let secret = arkive_lottery::commitment::generate_secret();

        // Mark player as committed
        game_data.player_commitments.insert(player_id.clone(), true);

        // Check if both players have committed
        let is_reveal_phase = if game_data.player_commitments.len() == 2
            && game_data
                .player_commitments
                .values()
                .all(|&committed| committed)
        {
            game_data.state = "RevealPhase".to_string();
            game_data.reveal_deadline = Some(chrono::Utc::now().timestamp() + 300); // 5 minutes
            true
        } else {
            false
        };

        (secret, is_reveal_phase)
    };

    // Store secret
    let secret_key = format!("{}:{}", game_id, player_id);
    storage.player_secrets.insert(secret_key, secret.clone());

    save_storage(&storage)?;

    println!("Commitment submitted for game {}!", game_id);
    println!("Your Player ID: {}", player_id);
    println!(
        "Your secret (save this for reveal): {}",
        hex::encode(&secret)
    );
    println!();

    if is_reveal_phase {
        println!("Both players have committed! Now reveal your commitment:");
        println!(
            "coinflip reveal {} {} {}",
            wallet_name,
            game_id,
            hex::encode(&secret)
        );
    } else {
        println!("Waiting for other player to commit...");
    }

    Ok(())
}

pub async fn reveal_commitment(
    wallet_manager: &WalletManager,
    wallet_name: &str,
    game_id_str: &str,
    secret_hex: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let _wallet = wallet_manager.load_wallet(wallet_name).await?;
    let game_id = Uuid::parse_str(game_id_str)?;
    let secret = hex::decode(secret_hex)?;

    let mut storage = load_storage();

    // Get pot wallet for payout
    let pot_wallet_name = storage
        .pot_wallets
        .get(&game_id.to_string())
        .ok_or("Game not found")?;

    let pot_wallet = wallet_manager.load_wallet(pot_wallet_name).await?;

    // Find this player's ID
    let player_id = storage
        .player_wallets
        .iter()
        .find(|(key, wallet)| key.starts_with(&format!("{}:", game_id)) && wallet == &wallet_name)
        .map(|(key, _)| key.split(':').nth(1).unwrap().to_string())
        .ok_or("Player not found in this game")?;

    let (winner_id, bet_amount, total_pot) = {
        let game_data = storage
            .games
            .get_mut(&game_id.to_string())
            .ok_or("Game not found")?;

        if game_data.state != "RevealPhase" {
            return Err(format!(
                "Game not in reveal phase. Current state: {}",
                game_data.state
            )
            .into());
        }

        // Verify this player's secret
        let secret_key = format!("{}:{}", game_id, player_id);
        let stored_secret = storage
            .player_secrets
            .get(&secret_key)
            .ok_or("Secret not found for this player")?;

        if stored_secret != &secret {
            return Err("Invalid secret provided".into());
        }

        // Mark player as revealed
        game_data.player_reveals.insert(player_id.clone(), true);

        // Check if both players have revealed
        let (winner_id, total_pot) = if game_data.player_reveals.len() == 2
            && game_data.player_reveals.values().all(|&revealed| revealed)
        {
            // Determine winner using XOR
            let player1_id = &game_data.players[0];
            let player2_id = &game_data.players[1];

            let secret1_key = format!("{}:{}", game_id, player1_id);
            let secret2_key = format!("{}:{}", game_id, player2_id);

            let secret1 = storage.player_secrets.get(&secret1_key).unwrap();
            let secret2 = storage.player_secrets.get(&secret2_key).unwrap();

            let player1_wins = arkive_lottery::commitment::determine_winner(secret1, secret2);
            let winner_id = if player1_wins { player1_id } else { player2_id };

            game_data.winner = Some(winner_id.clone());
            game_data.state = "Completed".to_string();

            (Some(winner_id.clone()), game_data.total_pot)
        } else {
            (None, game_data.total_pot)
        };

        (winner_id, game_data.bet_amount, total_pot)
    };

    save_storage(&storage)?;

    println!("Commitment revealed for game {}!", game_id);

    if let Some(winner) = winner_id {
        println!();
        println!("------ GAME COMPLETED! ------");
        println!("═══════════════════════════════════");
        println!("Winner: {}", winner);
        println!("Prize: {} sats", total_pot);
        println!();

        println!("Processing payout...");

        let winner_wallet_key = format!("{}:{}", game_id, winner);
        let winner_wallet_name = storage
            .player_wallets
            .get(&winner_wallet_key)
            .ok_or("Winner wallet not found")?;

        let winner_wallet = wallet_manager.load_wallet(winner_wallet_name).await?;
        let winner_address = winner_wallet.get_ark_address().await?;

        // payout to winner
        match pot_wallet
            .send_ark(&winner_address.address, Amount::from_sat(total_pot))
            .await
        {
            Ok(payout_txid) => {
                println!("Payout successful!");
                println!("{} sats transferred to winner", total_pot);
                println!("Transaction ID: {}", payout_txid);
            }
            Err(e) => {
                println!("Payout failed ????: {}", e);
                println!(
                    "Winner can claim manually from pot wallet: {}",
                    pot_wallet_name
                );
            }
        }

        // Show pot wallet balance after payout
        let pot_balance = pot_wallet.balance().await?;
        println!("Remaining pot balance: {} sats", pot_balance.total.to_sat());
    } else {
        println!("Waiting for other player to reveal...");
    }

    Ok(())
}

pub async fn show_game_status(game_id_str: &str) -> Result<(), Box<dyn std::error::Error>> {
    let game_id = Uuid::parse_str(game_id_str)?;
    let storage = load_storage();

    let game_data = storage
        .games
        .get(&game_id.to_string())
        .ok_or("Game not found")?;

    println!("Game Status: {}", game_id);
    println!("═══════════════════════════════════");
    println!("State: {}", game_data.state);
    println!("Bet Amount: {} sats", game_data.bet_amount);
    println!("Total Pot: {} sats", game_data.total_pot);
    println!("Players: {}/2", game_data.players.len());

    if let Some(pot_wallet_name) = storage.pot_wallets.get(&game_id.to_string()) {
        println!("Pot Wallet: {}", pot_wallet_name);
    }

    if let Some(deadline) = game_data.commitment_deadline {
        let dt = chrono::DateTime::from_timestamp(deadline, 0).unwrap();
        println!(
            "Commitment Deadline: {}",
            dt.format("%Y-%m-%d %H:%M:%S UTC")
        );
    }

    if let Some(deadline) = game_data.reveal_deadline {
        let dt = chrono::DateTime::from_timestamp(deadline, 0).unwrap();
        println!("Reveal Deadline: {}", dt.format("%Y-%m-%d %H:%M:%S UTC"));
    }

    if let Some(winner) = &game_data.winner {
        println!("Winner!: {}", winner);
    }

    println!();

    // Player details with wallet names
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        "Player ID",
        "Wallet",
        "Bet Placed",
        "Committed",
        "Revealed",
    ]);

    for player_id in &game_data.players {
        let wallet_key = format!("{}:{}", game_id, player_id);
        let wallet_name = storage
            .player_wallets
            .get(&wallet_key)
            .map(|w| w.as_str())
            .unwrap_or("unknown");

        let bet_placed = game_data.collected_bets.contains_key(player_id);
        let committed = game_data
            .player_commitments
            .get(player_id)
            .unwrap_or(&false);
        let revealed = game_data.player_reveals.get(player_id).unwrap_or(&false);

        table.add_row(vec![
            &player_id[..8],
            wallet_name,
            &bet_placed.to_string(),
            &committed.to_string(),
            &revealed.to_string(),
        ]);
    }

    println!("{}", table);

    // Bet details if any
    if !game_data.collected_bets.is_empty() {
        println!();
        println!("Bet Details:");
        let mut bet_table = Table::new();
        bet_table.load_preset(UTF8_FULL);
        bet_table.set_header(vec!["Player ID", "Amount", "Transaction ID", "Timestamp"]);

        for (player_id, bet_info) in &game_data.collected_bets {
            let dt = chrono::DateTime::from_timestamp(bet_info.timestamp, 0).unwrap();
            bet_table.add_row(vec![
                &player_id[..8],
                &format!("{} sats", bet_info.amount),
                &bet_info.txid[..16],
                &dt.format("%H:%M:%S").to_string(),
            ]);
        }

        println!("{}", bet_table);
    }

    Ok(())
}

pub async fn list_games() -> Result<(), Box<dyn std::error::Error>> {
    let storage = load_storage();

    if storage.games.is_empty() {
        println!("No active games.");
        return Ok(());
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        "Game ID",
        "State",
        "Players",
        "Bet Amount",
        "Total Pot",
    ]);

    for (game_id, game_data) in &storage.games {
        table.add_row(vec![
            &game_id[..8],
            &game_data.state,
            &format!("{}/2", game_data.players.len()),
            &format!("{} sats", game_data.bet_amount),
            &format!("{} sats", game_data.total_pot),
        ]);
    }

    println!("Active Games:");
    println!("{}", table);

    Ok(())
}
