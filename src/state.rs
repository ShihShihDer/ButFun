//! 伺服器的共享狀態：權威世界、玩家清單、廣播頻道。
//!
//! 目前狀態存在記憶體裡。持久化（Postgres）刻意藏在這層之後——之後把 `players`
//! 換成「啟動時從 DB 載入、變動時寫回」即可，不用動 WebSocket / 遊戲迴圈的程式。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::protocol::{PlayerView, WorldInfo};
use crate::suggestions::SuggestionStore;

/// 世界大小（像素）。MVP：一張單一地圖。
pub const WORLD_WIDTH: f32 = 2000.0;
pub const WORLD_HEIGHT: f32 = 2000.0;
/// 玩家移動速度（像素 / 秒）。
pub const PLAYER_SPEED: f32 = 200.0;

/// 一名玩家在伺服器上的權威狀態。
#[derive(Debug, Clone)]
pub struct Player {
    pub id: Uuid,
    pub name: String,
    pub species: String,
    pub x: f32,
    pub y: f32,
    pub input: Input,
}

impl Player {
    pub fn view(&self) -> PlayerView {
        PlayerView {
            id: self.id,
            name: self.name.clone(),
            species: self.species.clone(),
            x: self.x,
            y: self.y,
        }
    }
}

/// 玩家目前按住的方向鍵（移動意圖）。
#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

/// 整個應用程式共享的狀態。用 `Arc` 包起來在 handler 間共用。
#[derive(Clone)]
pub struct AppState {
    /// 權威玩家清單。
    pub players: Arc<RwLock<HashMap<Uuid, Player>>>,
    /// 廣播頻道：tick 快照與聊天都走這裡，內容是已序列化的 JSON 字串
    /// （只序列化一次，再扇出給所有連線）。
    pub tx: broadcast::Sender<String>,
    /// 遊戲內建議箱（玩家回饋迴圈的伺服器端）。
    pub suggestions: SuggestionStore,
}

impl AppState {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(256);
        Self {
            players: Arc::new(RwLock::new(HashMap::new())),
            tx,
            suggestions: SuggestionStore::new(),
        }
    }

    pub fn world_info(&self) -> WorldInfo {
        WorldInfo {
            width: WORLD_WIDTH,
            height: WORLD_HEIGHT,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
