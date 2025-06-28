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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GameData {
    id: String,
    bet_amount: u64,
    state: String,
    players: Vec<String>, // player IDs
    pot_address: String,
    commitment_deadline: Option<i64>,
    reveal_deadline: Option<i64>,
    player_commitments: HashMap<String, bool>,
    player_reveals: HashMap<String, bool>,
    winner: Option<String>,
}

impl Default for GameStorage {
    fn default() -> Self {
        Self {
            games: HashMap::new(),
            player_secrets: HashMap::new(),
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
    let _wallet = wallet_manager.load_wallet(wallet_name).await?;
    let bet_amount = Amount::from_sat(amount);

    // Create new game
    let game = TwoPlayerGame::new(bet_amount).await?;
    let game_id = game.id();
    let info = game.get_info();

    // player ID for the creator
    let creator_player_id = Uuid::new_v4();

    // [TODO]
    // simplified game data for storage
    let game_data = GameData {
        id: game_id.to_string(),
        bet_amount: amount,
        state: "WaitingForPlayers".to_string(),
        players: vec![creator_player_id.to_string()], // creator as first player
        pot_address: info.pot_address,
        commitment_deadline: None,
        reveal_deadline: None,
        player_commitments: HashMap::new(),
        player_reveals: HashMap::new(),
        winner: None,
    };

    let mut storage = load_storage();
    storage.games.insert(game_id.to_string(), game_data);
    save_storage(&storage)?;

    println!("Created new game!");
    println!("Game ID: {}", game_id);
    println!("Your Player ID: {}", creator_player_id);
    println!("Bet Amount: {} sats", amount);
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
    let _wallet = wallet_manager.load_wallet(wallet_name).await?;
    let game_id = Uuid::parse_str(game_id_str)?;

    let mut storage = load_storage();

    let (player_id, _players_len, is_ready) = {
        let game_data = storage
            .games
            .get_mut(&game_id.to_string())
            .ok_or("Game not found")?;

        if game_data.players.len() >= 2 {
            return Err("Game is full".into());
        }

        // player ID for this wallet
        let player_id = Uuid::new_v4();
        game_data.players.push(player_id.to_string());

        // If we have 2 players, move to betting phase
        let is_ready = if game_data.players.len() == 2 {
            game_data.state = "WaitingForBets".to_string();
            true
        } else {
            false
        };

        (player_id, game_data.players.len(), is_ready)
    };

    save_storage(&storage)?;

    println!("Joined game {}!", game_id);
    println!("Your Player ID: {}", player_id);

    if is_ready {
        println!("Game is now ready!");
        println!();
        println!("Both players can now commit:");
        println!("coinflip commit alice-wallet {}", game_id);
        println!("coinflip commit bob-wallet {}", game_id);
    } else {
        println!("Waiting for one more player...");
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

    let (player_id, secret, is_reveal_phase) = {
        let game_data = storage
            .games
            .get_mut(&game_id.to_string())
            .ok_or("Game not found")?;

        if game_data.players.len() != 2 {
            return Err("Game needs 2 players".into());
        }

        // Start commitment phase if not already started
        if game_data.state == "WaitingForBets" {
            game_data.state = "CommitmentPhase".to_string();
            game_data.commitment_deadline = Some(chrono::Utc::now().timestamp() + 300);
            // 5 minutes
        }

        if game_data.state != "CommitmentPhase" {
            return Err("Not in commitment phase".into());
        }

        // Find a player who hasn't committed yet
        let available_player = game_data
            .players
            .iter()
            .find(|pid| !game_data.player_commitments.get(*pid).unwrap_or(&false))
            .ok_or("All players have already committed")?;

        let player_id = available_player.clone();

        // Generate secret
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

        (player_id, secret, is_reveal_phase)
    };

    // Store secret after releasing the mutable borrow
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
        println!("Wait for other player to commit...");
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

    let (_player_id, winner_id, bet_amount) = {
        let game_data = storage
            .games
            .get_mut(&game_id.to_string())
            .ok_or("Game not found")?;

        if game_data.state != "RevealPhase" {
            return Err("Not in reveal phase".into());
        }

        // Find the player who provided this secret
        let player_id = game_data
            .players
            .iter()
            .find(|pid| {
                let secret_key = format!("{}:{}", game_id, pid);
                storage.player_secrets.get(&secret_key) == Some(&secret)
            })
            .ok_or("Invalid secret or player not found")?
            .clone();

        // Mark player as revealed
        game_data.player_reveals.insert(player_id.clone(), true);

        // Check if both players have revealed
        let winner_id = if game_data.player_reveals.len() == 2
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

            Some(winner_id.clone())
        } else {
            None
        };

        (player_id, winner_id, game_data.bet_amount)
    };

    save_storage(&storage)?;

    println!("Commitment revealed for game {}!", game_id);

    if let Some(winner) = winner_id {
        println!();
        println!("======GAME COMPLETED!======");
        println!("Winner: {}", winner);
        println!("The winner receives {} sats!", bet_amount * 2);
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
    println!("Players: {}/2", game_data.players.len());
    println!("Pot Address: {}", game_data.pot_address);

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
        println!("Winner: {}", winner);
    }

    println!();

    // Show player details
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec!["Player ID", "Committed", "Revealed"]);

    for player_id in &game_data.players {
        let committed = game_data
            .player_commitments
            .get(player_id)
            .unwrap_or(&false);
        let revealed = game_data.player_reveals.get(player_id).unwrap_or(&false);

        table.add_row(vec![
            &player_id[..8],
            &committed.to_string(),
            &revealed.to_string(),
        ]);
    }

    println!("{}", table);

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
    table.set_header(vec!["Game ID", "State", "Players", "Bet Amount"]);

    for (game_id, game_data) in &storage.games {
        table.add_row(vec![
            &game_id[..8],
            &game_data.state,
            &format!("{}/2", game_data.players.len()),
            &format!("{} sats", game_data.bet_amount),
        ]);
    }

    println!("Active Games:");
    println!("{}", table);

    Ok(())
}
