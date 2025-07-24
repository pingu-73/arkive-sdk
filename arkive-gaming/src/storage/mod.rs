use crate::{GamingError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
use uuid::Uuid;

/// File-based storage for games
pub struct GameStorage {
    storage_dir: PathBuf,
}

impl GameStorage {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        let storage_dir = data_dir.join("games");
        fs::create_dir_all(&storage_dir).await?;

        Ok(Self { storage_dir })
    }

    /// Save game state to file
    pub async fn save_game<T>(&self, game_id: Uuid, game_data: &T) -> Result<()>
    where
        T: Serialize,
    {
        let file_path = self.storage_dir.join(format!("{}.json", game_id));
        let json_data = serde_json::to_string_pretty(game_data)?;
        fs::write(file_path, json_data).await?;
        Ok(())
    }

    /// Load game state from file
    pub async fn load_game<T>(&self, game_id: Uuid) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let file_path = self.storage_dir.join(format!("{}.json", game_id));

        if !file_path.exists() {
            return Err(GamingError::Internal(format!("Game {} not found", game_id)));
        }

        let json_data = fs::read_to_string(file_path).await?;
        let game_data = serde_json::from_str(&json_data)?;
        Ok(game_data)
    }

    /// List all saved games
    pub async fn list_games(&self) -> Result<Vec<Uuid>> {
        let mut games = Vec::new();
        let mut entries = fs::read_dir(&self.storage_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            if let Some(file_name) = entry.file_name().to_str() {
                if file_name.ends_with(".json") {
                    if let Some(game_id_str) = file_name.strip_suffix(".json") {
                        if let Ok(game_id) = Uuid::parse_str(game_id_str) {
                            games.push(game_id);
                        }
                    }
                }
            }
        }

        Ok(games)
    }

    /// Delete game file
    pub async fn delete_game(&self, game_id: Uuid) -> Result<()> {
        let file_path = self.storage_dir.join(format!("{}.json", game_id));
        if file_path.exists() {
            fs::remove_file(file_path).await?;
        }
        Ok(())
    }

    /// Check if game exists
    pub async fn game_exists(&self, game_id: Uuid) -> bool {
        let file_path = self.storage_dir.join(format!("{}.json", game_id));
        file_path.exists()
    }
}

/// Serializable game state for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredGameState {
    pub game_id: Uuid,
    pub game_type: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub state: serde_json::Value,
}

impl StoredGameState {
    pub fn new(game_id: Uuid, game_type: String, state: serde_json::Value) -> Self {
        let now = chrono::Utc::now();
        Self {
            game_id,
            game_type,
            created_at: now,
            updated_at: now,
            state,
        }
    }

    pub fn update_state(&mut self, state: serde_json::Value) {
        self.state = state;
        self.updated_at = chrono::Utc::now();
    }
}
