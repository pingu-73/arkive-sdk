use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Generic game state management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameState {
    WaitingForPlayers,
    WaitingForBets,
    BetsCollected,
    InProgress,
    Completed { winner: Option<Uuid> },
    Aborted { reason: String },
}

/// State manager for tracking game progression
#[derive(Debug)]
pub struct StateManager {
    current_state: GameState,
    state_history: Vec<(GameState, DateTime<Utc>)>,
    timeouts: HashMap<String, DateTime<Utc>>,
}

impl StateManager {
    pub fn new() -> Self {
        let initial_state = GameState::WaitingForPlayers;
        let mut state_history = Vec::new();
        state_history.push((initial_state.clone(), Utc::now()));

        Self {
            current_state: initial_state,
            state_history,
            timeouts: HashMap::new(),
        }
    }

    pub fn current_state(&self) -> &GameState {
        &self.current_state
    }

    pub fn transition_to(&mut self, new_state: GameState) {
        tracing::info!(
            "State transition: {:?} -> {:?}",
            self.current_state,
            new_state
        );
        self.state_history.push((new_state.clone(), Utc::now()));
        self.current_state = new_state;
    }

    pub fn set_timeout(&mut self, name: String, deadline: DateTime<Utc>) {
        self.timeouts.insert(name, deadline);
    }

    pub fn check_timeout(&self, name: &str) -> bool {
        if let Some(deadline) = self.timeouts.get(name) {
            Utc::now() > *deadline
        } else {
            false
        }
    }

    pub fn get_timeout(&self, name: &str) -> Option<DateTime<Utc>> {
        self.timeouts.get(name).copied()
    }

    pub fn clear_timeout(&mut self, name: &str) {
        self.timeouts.remove(name);
    }
}

impl Default for StateManager {
    fn default() -> Self {
        Self::new()
    }
}
