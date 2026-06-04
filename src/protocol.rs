//! 客戶端 <-> 伺服器的 WebSocket 訊息協定（JSON，以 "type" 標籤區分）。
//!
//! 權威伺服器模型：客戶端只送「意圖（輸入）」，伺服器模擬並廣播權威狀態快照。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 客戶端送給伺服器的訊息。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// 進場：選好名字與種族（起源系統的第一步，目前 MVP 只有「地球人」）。
    Join { name: String, species: String },
    /// 移動意圖：目前按住的方向鍵。伺服器在每個 tick 依此整合位置。
    Input {
        #[serde(default)]
        up: bool,
        #[serde(default)]
        down: bool,
        #[serde(default)]
        left: bool,
        #[serde(default)]
        right: bool,
    },
    /// 聊天訊息。
    Chat { text: String },
}

/// 伺服器送給客戶端的訊息。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// 進場成功，告訴客戶端自己的 id 與世界資訊。
    Welcome { id: Uuid, world: WorldInfo },
    /// 每個 tick 廣播一次的權威世界狀態快照。
    Snapshot { tick: u64, players: Vec<PlayerView> },
    /// 廣播聊天訊息。
    Chat { from: String, text: String },
    /// 有玩家離線。
    PlayerLeft { id: Uuid },
}

/// 世界的基本參數，讓客戶端知道地圖邊界。
#[derive(Debug, Clone, Serialize)]
pub struct WorldInfo {
    pub width: f32,
    pub height: f32,
}

/// 快照裡每個玩家的可見狀態。
#[derive(Debug, Clone, Serialize)]
pub struct PlayerView {
    pub id: Uuid,
    pub name: String,
    pub species: String,
    pub x: f32,
    pub y: f32,
}
