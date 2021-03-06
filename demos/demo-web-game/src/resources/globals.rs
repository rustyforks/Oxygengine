use crate::components::player::PlayerType;
use oxygengine::prelude::*;

#[derive(Debug, Copy, Clone)]
pub enum GamePhase {
    Start,
    Game,
    End(Option<PlayerType>),
    Restart,
}

impl Default for GamePhase {
    fn default() -> Self {
        Self::Start
    }
}

impl GamePhase {
    pub fn is_game(self) -> bool {
        if let Self::Game = self {
            true
        } else {
            false
        }
    }

    pub fn is_restart(self) -> bool {
        if let Self::Restart = self {
            true
        } else {
            false
        }
    }
}

#[derive(Default)]
pub struct Globals {
    pub camera: Option<Entity>,
    pub map_size: Option<Vec2>,
    pub phase: GamePhase,
}

impl Globals {
    pub fn start(&mut self, camera: Entity) {
        self.reset();
        self.camera = Some(camera);
    }

    pub fn reset(&mut self) {
        self.camera = None;
        self.map_size = None;
        self.phase = GamePhase::Start;
    }
}
