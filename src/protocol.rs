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
    /// 農地互動：玩家點地表某個世界座標。伺服器換算成耕地格後，依該格目前狀態
    /// 自動決定動作（翻土 / 播種 / 澆水 / 收成）——「一鍵照顧」。
    Farm { x: f32, y: f32 },
}

/// 伺服器送給客戶端的訊息。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// 進場成功，告訴客戶端自己的 id 與世界資訊。
    Welcome { id: Uuid, world: WorldInfo },
    /// 每個 tick 廣播一次的權威世界狀態快照（含農地當前狀態）。
    Snapshot {
        tick: u64,
        players: Vec<PlayerView>,
        field: FieldView,
    },
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
    /// 目前持有的乙太（收成累積；持久化待 Phase 0-E，目前重連歸零）。
    pub ether: u32,
}

/// 快照裡的農地狀態：固定位置 / 大小的格陣列，讓前端能畫出每格。
#[derive(Debug, Clone, Serialize)]
pub struct FieldView {
    /// 農地左上角世界座標與每格邊長，讓前端對齊伺服器的格線。
    pub origin_x: f32,
    pub origin_y: f32,
    pub tile_size: f32,
    pub cols: usize,
    pub rows: usize,
    /// row-major 的每格狀態，長度為 `cols * rows`。
    pub cells: Vec<TileView>,
}

/// 一格耕地對前端的可見狀態。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TileView {
    /// 0=自然地 1=空土 2=種子 3=發芽 4=成熟。
    pub state: u8,
    /// 種了作物但已乾、需要澆水（給前端做「該澆水了」提示）。
    pub dry: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 前端送的 farm 訊息要能被解析成 `ClientMsg::Farm`（鎖住線上 JSON 契約）。
    #[test]
    fn parses_farm_message() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"farm","x":12.5,"y":-3.0}"#).unwrap();
        match msg {
            ClientMsg::Farm { x, y } => {
                assert_eq!(x, 12.5);
                assert_eq!(y, -3.0);
            }
            other => panic!("解析成非預期變體：{other:?}"),
        }
    }

    /// 快照序列化後要帶前端依賴的欄位名：field / 每位玩家的 ether / 每格的 state、dry。
    #[test]
    fn snapshot_serializes_field_and_ether() {
        let snap = ServerMsg::Snapshot {
            tick: 1,
            players: vec![PlayerView {
                id: Uuid::nil(),
                name: "測試".into(),
                species: "terran".into(),
                x: 0.0,
                y: 0.0,
                ether: 7,
            }],
            field: FieldView {
                origin_x: 856.0,
                origin_y: 904.0,
                tile_size: 48.0,
                cols: 6,
                rows: 4,
                cells: vec![TileView {
                    state: 2,
                    dry: true,
                }],
            },
        };
        let v: serde_json::Value = serde_json::to_value(&snap).unwrap();
        assert_eq!(v["type"], "snapshot");
        assert_eq!(v["players"][0]["ether"], 7);
        assert_eq!(v["field"]["tile_size"], 48.0);
        assert_eq!(v["field"]["cells"][0]["state"], 2);
        assert_eq!(v["field"]["cells"][0]["dry"], true);
    }
}
