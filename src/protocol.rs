//! 客戶端 <-> 伺服器的 WebSocket 訊息協定（JSON，以 "type" 標籤區分）。
//!
//! 權威伺服器模型：客戶端只送「意圖（輸入）」，伺服器模擬並廣播權威狀態快照。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::combat::EnemyKind;
use crate::daynight::Phase;
use crate::gather::NodeKind;
use crate::inventory::ItemKind;

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
    /// 採集意圖：玩家想採附近的資源節點（樹／石／乙太礦）。不帶座標——伺服器一律
    /// **用玩家自己的權威位置**判定 `GATHER_REACH` 內最近的可採節點（防隔空採集）。
    Gather,
    /// 合成意圖(1-C)：玩家點合成台某配方的合成鈕。`recipe_id` 是產物 `ItemKind` 的 snake_case
    /// 名(如 "pickaxe")。伺服器查 `crafting::RECIPES`、在玩家自己背包上全有全無地扣料+產出,
    /// 產物隨既有背包快照回來(零契約變更)。
    Craft { recipe_id: String },
    /// 領地購買意圖（③ Slice D）：玩家點擊購買第一塊地。
    /// 伺服器驗證乙太足夠（PLOT_COST）且尚未擁有地塊後分配序號。
    ClaimPlot,
}

/// 伺服器送給客戶端的訊息。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// 進場成功，告訴客戶端自己的 id 與世界資訊。
    Welcome { id: Uuid, world: WorldInfo },
    /// 每個 tick 廣播一次的權威世界狀態快照（含各玩家農地當前狀態）。
    Snapshot {
        tick: u64,
        players: Vec<PlayerView>,
        /// 世界上所有玩家的地塊（per-player：每人一塊，各在不同位置）。
        /// 前端畫出全部，但只有 `owner == 自己 id` 的那塊能互動。
        fields: Vec<FieldView>,
        /// 世界裡共享的採集節點（樹／石／乙太礦,Phase 1-A）。前端畫出來、可採的標亮,
        /// 玩家走近點它送 `Gather`。
        nodes: Vec<NodeView>,
        /// 世界裡共享的敵人（戰鬥 1-F）。前端畫出來 + 血條;玩家走近會自動開打(伺服器每秒結算)。
        enemies: Vec<EnemyView>,
        /// 伺服器權威的日夜狀態（階段 + 亮度），前端依此做環境染色。
        daynight: DayNightView,
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
    /// 目前持有的乙太（收成累積）。已登入玩家重連會帶回（記憶體前置），
    /// 跨伺服器重啟才歸零（待 Phase 0-E 持久化）。
    pub ether: u32,
    /// 背包內容（採集所得）。每位玩家都帶著,前端只取「自己那位」的來畫背包面板。
    /// 條目已依 `ItemKind` 排序（`Inventory` 用 `BTreeMap`),順序穩定。
    pub inventory: Vec<ItemStack>,
    /// 目前生命值與上限（戰鬥 1-F）。前端畫血條;0 = 被打趴休息中。
    pub hp: u32,
    pub max_hp: u32,
}

/// 快照裡一個世界敵人的可見狀態。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EnemyView {
    /// 敵人種類（scrap_drone / ether_wisp）：前端據此選圖示與血色。
    pub kind: EnemyKind,
    pub x: f32,
    pub y: f32,
    /// 剩餘血量 / 上限（畫血條）。`alive=false` 表示被打倒、重生中(畫淡或不畫)。
    pub hp: u32,
    pub max_hp: u32,
    pub alive: bool,
}

/// 背包裡的一疊物品（種類 + 數量），給快照序列化用。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ItemStack {
    pub item: ItemKind,
    pub qty: u32,
}

/// 快照裡一個世界採集節點的可見狀態。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NodeView {
    /// 節點種類（tree/rock/ether_ore）：前端據此選圖示與掉落。
    pub kind: NodeKind,
    pub x: f32,
    pub y: f32,
    /// 剩餘可採次數（耐久）。0 = 採空、重生中。
    pub remaining: u32,
    /// 現在可不可以採（耐久 > 0）。前端據此把可採的標亮、採空的畫暗。
    pub harvestable: bool,
}

/// 快照裡的農地狀態：固定位置 / 大小的格陣列，讓前端能畫出每格。
#[derive(Debug, Clone, Serialize)]
pub struct FieldView {
    /// 這塊地的擁有者（per-player）。前端比對自己的 id：相同才畫成「你的地」、
    /// 才套用照顧距離回饋與互動；其餘只看得到、點不動。
    /// `Field::view()` 先填 `nil`，由廣播層（知道 HashMap key）戳上真正的擁有者。
    pub owner: Uuid,
    /// 農地左上角世界座標與每格邊長，讓前端對齊伺服器的格線。
    pub origin_x: f32,
    pub origin_y: f32,
    pub tile_size: f32,
    pub cols: usize,
    pub rows: usize,
    /// 要照顧農地，玩家離地塊矩形最近距離須在此像素內（伺服器權威 `FARM_REACH`）。
    /// 帶給前端，讓「太遠就變暗、提示走近」與伺服器的拒絕判斷用同一個來源、不會各說各話。
    pub reach: f32,
    /// row-major 的每格狀態，長度為 `cols * rows`。
    pub cells: Vec<TileView>,
}

/// 快照裡的日夜狀態：目前階段與環境亮度，讓前端疊出柔和的明暗流轉。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DayNightView {
    /// 目前階段（dawn/day/dusk/night）：前端可用來顯示或選色調。
    pub phase: Phase,
    /// 環境亮度，落在 `[daynight::MIN_LIGHT, 1.0]`：前端依此疊夜色，越暗疊越濃。
    pub light: f32,
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

    /// 前端送的 gather 訊息要能被解析成 `ClientMsg::Gather`（鎖住線上 JSON 契約）。
    #[test]
    fn parses_gather_message() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"gather"}"#).unwrap();
        assert!(matches!(msg, ClientMsg::Gather));
    }

    /// 快照序列化後要帶前端依賴的欄位名：fields / 每位玩家的 ether 與 inventory /
    /// nodes（採集節點,帶 kind/x/y/harvestable）/ 每格的 state、dry。
    #[test]
    fn snapshot_serializes_field_ether_inventory_and_nodes() {
        use crate::gather::NodeKind;
        use crate::inventory::ItemKind;
        let owner = Uuid::nil();
        let snap = ServerMsg::Snapshot {
            tick: 1,
            players: vec![PlayerView {
                id: Uuid::nil(),
                name: "測試".into(),
                species: "terran".into(),
                x: 0.0,
                y: 0.0,
                ether: 7,
                inventory: vec![ItemStack {
                    item: ItemKind::Wood,
                    qty: 3,
                }],
                hp: 18,
                max_hp: 20,
            }],
            fields: vec![FieldView {
                owner,
                origin_x: 856.0,
                origin_y: 904.0,
                tile_size: 48.0,
                cols: 6,
                rows: 4,
                reach: 48.0,
                cells: vec![TileView {
                    state: 2,
                    dry: true,
                }],
            }],
            nodes: vec![NodeView {
                kind: NodeKind::Rock,
                x: 120.0,
                y: 240.0,
                remaining: 4,
                harvestable: true,
            }],
            enemies: vec![EnemyView {
                kind: EnemyKind::ScrapDrone,
                x: 300.0,
                y: 400.0,
                hp: 5,
                max_hp: 6,
                alive: true,
            }],
            daynight: DayNightView {
                phase: Phase::Day,
                light: 0.5, // 0.5 在 f32 可精確表示，避免序列化後比對浮點誤差
            },
        };
        let v: serde_json::Value = serde_json::to_value(&snap).unwrap();
        assert_eq!(v["type"], "snapshot");
        assert_eq!(v["players"][0]["ether"], 7);
        assert_eq!(v["players"][0]["inventory"][0]["item"], "wood");
        assert_eq!(v["players"][0]["inventory"][0]["qty"], 3);
        assert_eq!(v["players"][0]["hp"], 18);
        assert_eq!(v["players"][0]["max_hp"], 20);
        assert_eq!(v["fields"][0]["owner"], owner.to_string());
        assert_eq!(v["fields"][0]["cells"][0]["state"], 2);
        assert_eq!(v["nodes"][0]["kind"], "rock");
        assert_eq!(v["nodes"][0]["x"], 120.0);
        assert_eq!(v["enemies"][0]["kind"], "scrap_drone");
        assert_eq!(v["enemies"][0]["hp"], 5);
        assert_eq!(v["enemies"][0]["alive"], true);
        assert_eq!(v["nodes"][0]["harvestable"], true);
        assert_eq!(v["daynight"]["phase"], "day");
        assert_eq!(v["daynight"]["light"], 0.5);
    }
}
