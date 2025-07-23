pub mod game_trait;
pub mod player;
pub mod state;

pub use game_trait::{Game, GameAction, GameResult};
pub use player::{Player, PlayerId, PlayerState};
pub use state::{GameState, StateManager};
