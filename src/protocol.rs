//! 客戶端 <-> 伺服器的 WebSocket 訊息協定（JSON，以 "type" 標籤區分）。
//!
//! 權威伺服器模型：客戶端只送「意圖（輸入）」，伺服器模擬並廣播權威狀態快照。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::combat::EnemyKind;
use crate::daynight::Phase;
use crate::gather::NodeKind;
use crate::inventory::ItemKind;
use world_core::TileKind;

/// 地形格種類的協定表示（序列化為小寫字串，與 world-core TileKind 對齊）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TileKindView {
    Empty,
    Dirt,
    Stone,
    Ore,
    Crystal,
    Mushroom,
    AncientRuin,
    CoralReef,
    WildFlower,
}

impl From<TileKind> for TileKindView {
    fn from(k: TileKind) -> Self {
        match k {
            TileKind::Empty       => TileKindView::Empty,
            TileKind::Dirt        => TileKindView::Dirt,
            TileKind::Stone       => TileKindView::Stone,
            TileKind::Ore         => TileKindView::Ore,
            TileKind::Crystal     => TileKindView::Crystal,
            TileKind::Mushroom    => TileKindView::Mushroom,
            TileKind::AncientRuin => TileKindView::AncientRuin,
            TileKind::CoralReef   => TileKindView::CoralReef,
            TileKind::WildFlower  => TileKindView::WildFlower,
        }
    }
}

/// 玩家挖 / 建造後與確定性生成的差異（delta-save）。
/// C-1 快照裡此陣列為空；C-2 挖掘後才開始廣播真實差異。
/// 前端收到後應覆蓋 `tileKindAt` 的本地計算，讓挖掘視覺即時反映。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TileDeltaView {
    /// 所在 chunk 的格座標（世界像素 / 512 取 floor）。
    pub cx: i32,
    pub cy: i32,
    /// Chunk 內格座標（0..=15）。
    pub tx: u8,
    pub ty: u8,
    pub kind: TileKindView,
}

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
    /// 農地擴張意圖：玩家點「擴地」按鈕，花乙太把自己農地多開一列。
    /// 伺服器驗餘額（economy::expansion_cost）、扣乙太、農地 grow()；結果隨下一次快照回來。
    BuyExpansion,
    /// 市場掛單：從背包取出 qty 個 item 以每單位 price_per 乙太掛單出售。
    /// 伺服器驗背包庫存後移出物品並建立掛單；失敗靜默忽略（量不夠 / 未登入）。
    PostListing { item: ItemKind, qty: u32, price_per: u32 },
    /// 購買掛單：花 total = qty * price_per 乙太換取掛單內物品。
    /// 乙太不足 / 掛單不存在靜默忽略。不能買自己的掛單。
    BuyListing { listing_id: Uuid },
    /// 取消自己的掛單：托管物品退還背包（非本人掛單靜默忽略）。
    CancelListing { listing_id: Uuid },
    /// 向新手村商人 NPC 賣出 qty 個 item（背包扣物品 → 得乙太）。
    /// 伺服器驗距離、物品在收購清單、背包有貨；失敗靜默忽略。
    ShopSell { item: ItemKind, qty: u32 },
    /// 向新手村商人 NPC 購買 qty 個 item（花乙太 → 背包加物品）。
    /// 伺服器驗距離、物品在販售清單、乙太足夠；失敗靜默忽略。
    ShopBuy { item: ItemKind, qty: u32 },
    /// 挖掘地形格（C-2）：玩家點擊世界座標 (wx, wy)。
    /// 伺服器換算成 cell 座標，驗可及距離（DIG_REACH）、目標為實心格後：
    /// delta 設 Empty、對應材料入背包、廣播差異（随下一次快照帶出）。
    Dig { wx: f32, wy: f32 },
    /// 建造——放置地形格（C-4）：玩家右鍵點擊世界座標 (wx, wy)。
    /// `material` 為 ItemKind 的 snake_case 名（目前可放：`"dirt"` / `"stone"`）。
    /// 伺服器驗可及距離、目標為 Empty、背包有該材料後：
    /// 背包扣 1、delta 設對應 TileKind、廣播差異。
    Place { wx: f32, wy: f32, material: String },
    /// 主動攻擊：玩家按下攻擊鍵，打 ATTACK_REACH 內最近的存活敵人。
    /// 伺服器驗：未倒地、冷卻已到期（ATTACK_COOLDOWN_SECS），再結算傷害 + 掉落。
    /// 冷卻期間送出靜默忽略（不懲罰多按）。
    Attack,
    /// 回城：把玩家傳回新手村（出生點 / 安全區中心）。便利功能，無代價、無冷卻。
    ReturnHome,
    /// 使用道具：消耗背包裡一個 `item` 並立即觸發其效果。
    /// 目前支援：`HealingPotion`（活力藥水）→ 立即回復 6 HP。
    /// 倒地中 / 背包不足靜默忽略。
    UseItem { item: ItemKind },
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
        /// 附近玩家的市場掛單（AOI 剔除後，距離內才送）。
        listings: Vec<ListingView>,
        /// 世界裡的 NPC（目前只有新手村商人）。前端畫出 NPC 圖示、靠近時顯示商店面板。
        npcs: Vec<NpcView>,
        /// 玩家挖 / 建造後偏離確定性生成的地形差異（delta-save）。
        /// C-1：永遠為空陣列；C-2 起有挖掘記錄時才非空。
        /// 前端本地用 `tileKindAt` 生成初始地形，收到 delta 後覆蓋對應格子。
        terrain: Vec<TileDeltaView>,
    },
    /// 廣播聊天訊息。
    Chat { from: String, text: String },
    /// 有玩家離線。
    PlayerLeft { id: Uuid },
    /// 某玩家成功購買第一塊領地（③ Slice D）。廣播給全部客戶端；
    /// 前端取 owner == myId 才顯示購買成功提示，其餘忽略即可。
    ClaimPlotOk { owner: Uuid, plot_index: usize },
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
    /// 目前持有的乙太（收成累積）。已登入玩家重連會帶回，跨重啟持久化。
    pub ether: u32,
    /// 已購買的農地擴張格數。前端擴地面板用它算下一格費用 + 顯示進度。
    pub expansions: u32,
    /// 背包內容（採集所得）。每位玩家都帶著,前端只取「自己那位」的來畫背包面板。
    /// 條目已依 `ItemKind` 排序（`Inventory` 用 `BTreeMap`),順序穩定。
    pub inventory: Vec<ItemStack>,
    /// 目前生命值與上限（戰鬥 1-F）。前端畫血條;0 = 被打趴休息中。
    pub hp: u32,
    pub max_hp: u32,
    /// 累積經驗值（ROADMAP 17）。前端由此推算等級（level = exp / 100）。
    pub exp: u32,
    /// 目前等級（由 exp 推算，server 算好送來省前端重算）。
    pub level: u32,
    /// 目前有效攻擊力（基礎武器力 + 等級加成，ROADMAP 18）。前端 HUD 顯示，讓玩家感受成長。
    pub attack: u32,
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

/// 快照裡的一筆市場掛單（玩家對玩家交易）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ListingView {
    pub id: Uuid,
    pub seller_id: Uuid,
    pub seller_name: String,
    pub item: ItemKind,
    pub qty: u32,
    /// 每單位乙太價格（0 = 贈送）。
    pub price_per: u32,
    /// 掛單時賣家的世界座標（AOI + 世界地圖渲染用）。
    pub x: f32,
    pub y: f32,
}

/// NPC 商店目錄的單筆條目（物品 + 每單位乙太價）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShopCatalogEntry {
    pub item: ItemKind,
    pub price_per: u32,
}

/// 快照裡的 NPC 可見狀態：位置 + 商品目錄（收購 / 販售），讓前端繪製並顯示商店面板。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NpcView {
    pub x: f32,
    pub y: f32,
    /// NPC 願意收購的物品（玩家賣給 NPC）。
    pub buy_list: Vec<ShopCatalogEntry>,
    /// NPC 願意販售的物品（玩家向 NPC 買）。
    pub sell_list: Vec<ShopCatalogEntry>,
}

/// 快照裡的日夜狀態：目前階段與環境亮度，讓前端疊出柔和的明暗流轉。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DayNightView {
    /// 目前階段（dawn/day/dusk/night）：前端可用來顯示或選色調。
    pub phase: Phase,
    /// 環境亮度，落在 `[daynight::MIN_LIGHT, 1.0]`：前端依此疊夜色，越暗疊越濃。
    pub light: f32,
    /// 夜間危機旗標（phase == Night 時為 true）：前端用來顯示危機暈輪 + 燐光族夜視效果。
    pub night_danger: bool,
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

    /// 前端送的 dig 訊息要能被解析成 `ClientMsg::Dig`（C-2 wire contract）。
    #[test]
    fn parses_dig_message() {
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"dig","wx":320.5,"wy":-64.0}"#).unwrap();
        match msg {
            ClientMsg::Dig { wx, wy } => {
                assert_eq!(wx, 320.5);
                assert_eq!(wy, -64.0);
            }
            other => panic!("解析成非預期變體：{other:?}"),
        }
    }

    /// 前端送的 attack 訊息要能被解析成 `ClientMsg::Attack`（主動攻擊 wire contract）。
    #[test]
    fn parses_attack_message() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"attack"}"#).unwrap();
        assert!(matches!(msg, ClientMsg::Attack));
    }

    /// 前端送的 place 訊息要能被解析成 `ClientMsg::Place`（C-4 wire contract）。
    #[test]
    fn parses_place_message() {
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"place","wx":128.0,"wy":64.0,"material":"dirt"}"#)
                .unwrap();
        match msg {
            ClientMsg::Place { wx, wy, material } => {
                assert_eq!(wx, 128.0);
                assert_eq!(wy, 64.0);
                assert_eq!(material, "dirt");
            }
            other => panic!("解析成非預期變體：{other:?}"),
        }
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
                expansions: 0,
                inventory: vec![ItemStack {
                    item: ItemKind::Wood,
                    qty: 3,
                }],
                hp: 18,
                max_hp: 20,
                exp: 0,
                level: 0,
                attack: 2,
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
                kind: NodeKind::Tree,
                x: 120.0,
                y: 240.0,
                remaining: 5,
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
                night_danger: false,
            },
            listings: vec![],
            npcs: vec![NpcView {
                x: 100.0,
                y: 200.0,
                buy_list: vec![ShopCatalogEntry { item: ItemKind::Wood, price_per: 1 }],
                sell_list: vec![ShopCatalogEntry { item: ItemKind::Pickaxe, price_per: 15 }],
            }],
            terrain: vec![],
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
        assert_eq!(v["nodes"][0]["kind"], "tree");
        assert_eq!(v["nodes"][0]["x"], 120.0);
        assert_eq!(v["enemies"][0]["kind"], "scrap_drone");
        assert_eq!(v["enemies"][0]["hp"], 5);
        assert_eq!(v["enemies"][0]["alive"], true);
        assert_eq!(v["nodes"][0]["harvestable"], true);
        assert_eq!(v["daynight"]["phase"], "day");
        assert_eq!(v["daynight"]["light"], 0.5);
        assert_eq!(v["daynight"]["night_danger"], false);
        // NPC 商人：確認序列化結構讓前端能讀 buy/sell 目錄。
        assert_eq!(v["npcs"][0]["x"], 100.0);
        assert_eq!(v["npcs"][0]["buy_list"][0]["item"], "wood");
        assert_eq!(v["npcs"][0]["buy_list"][0]["price_per"], 1);
        assert_eq!(v["npcs"][0]["sell_list"][0]["item"], "pickaxe");
        assert_eq!(v["npcs"][0]["sell_list"][0]["price_per"], 15);
    }
}
