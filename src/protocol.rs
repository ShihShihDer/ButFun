//! 客戶端 <-> 伺服器的 WebSocket 訊息協定（JSON，以 "type" 標籤區分）。
//!
//! 權威伺服器模型：客戶端只送「意圖（輸入）」，伺服器模擬並廣播權威狀態快照。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::combat::EnemyKind;
use crate::world_event::WorldEventView;
use crate::director::HordeView;
use crate::daynight::Phase;
use crate::gather::NodeKind;
use crate::inventory::ItemKind;
use crate::quest::QuestState;
use world_core::TileKind;

/// 玩家正在攜帶的貿易包裹摘要（送進快照，前端 HUD 顯示用）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TradeCargoBrief {
    pub route_id: u8,
    pub cargo_name: String,
    pub dest: String,
    pub reward: u32,
}

/// 工坊訂單摘要（送進快照，前端面板顯示用）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkshopOrderBrief {
    pub order_id: u8,
    pub name: String,
    pub item: ItemKind,
    pub qty: u32,
    pub reward: u32,
    pub xp: u32,
}

/// 玩家目前接取的工坊訂單狀態（含剩餘秒數）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorkshopActiveView {
    pub order_id: u8,
    pub name: String,
    pub item: ItemKind,
    pub qty: u32,
    pub reward: u32,
    pub remaining_secs: f32,
}

/// 懸賞令摘要（送進快照，前端面板列表用）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BountyCardBrief {
    pub card_id: u8,
    pub name: String,
    pub target_name: String,
    pub required_kills: u32,
    pub reward: u32,
    pub xp: u32,
}

/// 玩家目前接取的懸賞任務狀態（含擊殺進度+剩餘秒數）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BountyActiveView {
    pub card_id: u8,
    pub name: String,
    pub target_name: String,
    pub required_kills: u32,
    pub kills_done: u32,
    pub reward: u32,
    pub remaining_secs: f32,
}

/// 星際採購令摘要（靜態，前端列表用）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProcurementOrderBrief {
    pub order_id: u8,
    pub name: String,
    pub item: ItemKind,
    pub item_name: String,
    pub required_qty: u32,
    pub reward: u32,
    pub xp: u32,
}

/// 玩家目前接取的採購任務狀態（含剩餘秒數）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProcurementActiveView {
    pub order_id: u8,
    pub name: String,
    pub item: ItemKind,
    pub item_name: String,
    pub required_qty: u32,
    pub reward: u32,
    pub remaining_secs: f32,
}

/// 古蹟探勘令摘要（靜態，前端列表用）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExpeditionOrderBrief {
    pub order_id: u8,
    pub name: String,
    pub biome_name: String,
    pub min_dist: u32,
    pub reward: u32,
    pub xp: u32,
}

/// 玩家目前接取的探勘任務狀態（含剩餘秒數）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExpeditionActiveView {
    pub order_id: u8,
    pub name: String,
    pub biome_name: String,
    pub min_dist: u32,
    pub reward: u32,
    pub remaining_secs: f32,
}

/// 農產品展覽委託中單一物品需求的視圖（前端顯示用）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FairReqView {
    pub item: ItemKind,
    pub item_name: String,
    pub qty: u32,
}

/// 農產品展覽委託摘要（靜態，前端列表用）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FairOrderBrief {
    pub order_id: u8,
    pub name: String,
    pub reqs: Vec<FairReqView>,
    pub reward: u32,
    pub xp: u32,
}

/// 玩家目前接取的展覽委託狀態（含剩餘秒數）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FairActiveView {
    pub order_id: u8,
    pub name: String,
    pub reqs: Vec<FairReqView>,
    pub reward: u32,
    pub remaining_secs: f32,
}

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
    JadeVine,
    LavaRock,
    VoidCrystal,
    AetherMist,
    OriginCrystal,
    TownWall,
}

impl From<TileKind> for TileKindView {
    fn from(k: TileKind) -> Self {
        match k {
            TileKind::Empty         => TileKindView::Empty,
            TileKind::Dirt          => TileKindView::Dirt,
            TileKind::Stone         => TileKindView::Stone,
            TileKind::Ore           => TileKindView::Ore,
            TileKind::Crystal       => TileKindView::Crystal,
            TileKind::Mushroom      => TileKindView::Mushroom,
            TileKind::AncientRuin   => TileKindView::AncientRuin,
            TileKind::CoralReef     => TileKindView::CoralReef,
            TileKind::WildFlower    => TileKindView::WildFlower,
            TileKind::JadeVine      => TileKindView::JadeVine,
            TileKind::LavaRock      => TileKindView::LavaRock,
            TileKind::VoidCrystal   => TileKindView::VoidCrystal,
            TileKind::AetherMist    => TileKindView::AetherMist,
            TileKind::OriginCrystal => TileKindView::OriginCrystal,
            TileKind::TownWall      => TileKindView::TownWall,
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
    /// 確認 / 拒絕商人議價（ROADMAP 101）：前端確認對話框的回應。
    /// `accept = true` → 引擎執行交易（背包扣物品、加乙太、金庫扣）。
    /// `accept = false` → 靜默清除 PendingDeal，不執行任何交易。
    ConfirmDeal { accept: bool },
    /// 挖掘地形格（C-2）：玩家點擊世界座標 (wx, wy)。
    /// 伺服器換算成 cell 座標，驗可及距離（DIG_REACH）、目標為實心格後：
    /// delta 設 Empty、對應材料入背包、廣播差異（随下一次快照帶出）。
    Dig { wx: f32, wy: f32 },
    /// 建造——放置地形格（C-4）：玩家右鍵點擊世界座標 (wx, wy)。
    /// `material` 為 ItemKind 的 snake_case 名（目前可放：`"dirt"` / `"stone"`）。
    /// 伺服器驗可及距離、目標為 Empty、背包有該材料後：
    /// 背包扣 1、delta 設對應 TileKind、廣播差異。
    Place { wx: f32, wy: f32, material: String },
    /// 放置灑水器（ROADMAP 112）：玩家背包有灑水器後，點擊農地旁的世界座標放置。
    /// 伺服器驗：背包有 Sprinkler、放置點在自己農地的 FARM_REACH 內、未倒地。
    PlaceSprinkler { wx: f32, wy: f32 },
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
    /// 星際旅行（ROADMAP 20/22）：玩家在星圖彈窗點「出發」，請求傳送到指定星球。
    /// `planet` 支援 "verdant"（翠幽星）、"crimson"（赤焰星）和 "home"（返回故鄉）。
    /// 故鄉 → 翠幽星 須持五大生態武裝全套 + 30 乙太；
    /// 故鄉 → 赤焰星 須持翠幽碎片（有探索翠幽星的證明）+ 50 乙太；
    /// 各星球 → 故鄉 只需 30 乙太。
    TravelToPlanet { planet: String },
    /// 設定職業（ROADMAP 28）：玩家在職業選擇視窗點選職業。
    /// `class` 為職業 wire key（"warrior" / "farmer" / "artisan" / "explorer" / "merchant"）。
    /// 未登入 / 無效職業靜默忽略；可隨時更換職業（職業加成立即生效）。
    SetClass { class: String },
    /// 建立公會（ROADMAP 29）：花 50 乙太建立新公會。
    /// `name` 最多 20 字；`tag` 最多 3 字元（英文自動轉大寫）。
    /// 未登入 / 已有公會 / 乙太不足靜默忽略。
    CreateGuild { name: String, tag: String },
    /// 加入公會（ROADMAP 29）：依 guild_id 加入已存在的公會。
    /// 公會不存在 / 已滿員 / 自己已有公會靜默忽略。
    JoinGuild { guild_id: Uuid },
    /// 離開目前公會（ROADMAP 29）：自願退出；若是最後成員公會自動解散。
    /// 不在任何公會靜默忽略。
    LeaveGuild,
    /// 向公會金庫捐贈乙太（ROADMAP 29）。
    /// 不在公會 / 乙太不足 / 金額為 0 靜默忽略。
    DonateToGuild { amount: u32 },
    /// 請求公會列表（ROADMAP 29）：前端開啟「加入公會」面板時送出，
    /// 伺服器回 `GuildList`。
    RequestGuildList,
    /// 請求每日任務（ROADMAP 32）：前端開啟每日任務面板時送出，
    /// 伺服器回 `DailyQuestsUpdate`。未登入靜默忽略。
    RequestDailyQuests,
    /// 請求排行榜（ROADMAP 33）：前端開啟排行榜面板時送出，
    /// 伺服器回 `Leaderboard`（等級/乙太/殺怪三榜前 20 名）。
    RequestLeaderboard,
    /// 購買城外地塊（ROADMAP 35）：玩家在「購地」面板選用途並確認。
    /// `purpose`：`"farm"` = 農田地塊、`"free_build"` = 自由建地（未知字串預設 free_build）。
    /// 伺服器驗：已登入、乙太足夠（LAND_PLOT_COST=60）、地塊未被他人購走、自己尚無地塊。
    /// 結果隨快照廣播（land_plots 欄位更新）；失敗靜默忽略（前端依乙太/狀態已灰掉按鈕）。
    BuyLandPlot { plot_id: u32, purpose: Option<String> },
    /// 裝備道具（ROADMAP 36）：把背包裡的武器或護甲裝進對應槽。
    /// 背包無此物品 / 物品不可裝備靜默忽略；換裝時舊裝備退回背包。
    EquipItem { item: ItemKind },
    /// 卸下裝備（ROADMAP 36）：把指定槽的裝備退回背包。
    /// 槽名：`"weapon"` / `"armor"` / `"accessory"`；空槽或無效槽名靜默忽略。
    UnequipItem { slot: String },
    /// 精煉裝備（ROADMAP 37）：消耗同系材料，提升指定槽裝備的精煉等級。
    /// `slot`：`"weapon"` / `"armor"`；空槽 / 已達上限 / 材料不足靜默忽略。
    /// 成功：精煉等級 +1；失敗（+4 起有機率）：精煉等級降 1（材料仍消耗，不碎裝）。
    RefineEquip { slot: String },
    /// 附魔（ROADMAP 37）：消耗 1 個星球碎片，賦予武器槽特效（僅武器有意義）。
    /// `shard`：翠幽碎片/熔晶碎片/虛空碎片/霧醚碎片/源晶碎片——各自對應一種附魔。
    /// 武器槽空 / 碎片不是附魔碎片 / 背包無此碎片靜默忽略；可覆蓋舊附魔。
    EnchantEquip { shard: ItemKind },
    /// 主動技能（ROADMAP 45）：玩家點擊技能面板或按快捷鍵觸發。
    /// `kind`：`"warcry"` / `"bounty"` / `"precision"` / `"gale"` / `"haggle"`。
    /// 熟練度未達 Lv.5 / 冷卻中 / 未登入靜默忽略。
    UseSkill { kind: String },
    /// 設定技能自動施放（ROADMAP 151）：玩家在技能面板勾選/取消某技能的自動施放。
    /// `enabled = true` 時加入自動集合；`false` 時移除。
    /// 風之步（gale）不支援自動（方向依賴）；其餘四種皆可。
    SetAutoSkill { kind: String, enabled: bool },
    /// 屬性加點（ROADMAP 152）：玩家將 `points` 個屬性點分配到指定屬性。
    /// `stat` 可為 "hp" / "attack" / "speed" / "atk_speed"。
    /// 點數不足 / 未知屬性種類靜默忽略。
    AllocateStat { stat: String, points: u32 },
    /// 馴化寵物（ROADMAP 46）：嘗試馴化 ATTACK_REACH 內 HP < 25% 的最近怪物。
    /// 不帶座標參數——伺服器以玩家自己的位置判定（防隔空馴化）。
    /// 乙太不足 / 無符合條件怪物 / 不可馴化種類 / 倒地中靜默忽略。
    /// 成功後舊寵物自動放生；一次只能帶一隻。
    TamePet,
    /// 放生寵物（ROADMAP 46）：解除目前的寵物（加成一併取消）。無寵物靜默忽略。
    ReleasePet,
    /// 釣魚（ROADMAP 47）：站在水域邊緣（80px 內有 Water biome）垂釣。
    /// 伺服器驗距離、冷卻是否到期；成功後依機率（小魚 70%/星星魚 25%/深海魚 5%）
    /// 加一條魚進背包，並給 10 點農夫熟練度 XP。
    /// 不在水邊 / 冷卻中 / 倒地中靜默忽略。
    Fish,
    /// 購入雞（ROADMAP 48）：在自己的農田地塊花乙太購入一隻雞。
    /// `plot_id`：要購雞的農田地塊編號（必須是本人擁有的 Farm 類型地塊）。
    /// 乙太不足（BUY_CHICKEN_COST=15）/ 非農田 / 非本人地塊 / 達 MAX_CHICKENS 靜默忽略。
    BuyChicken { plot_id: u32 },
    /// 收雞蛋（ROADMAP 48）：收取農田地塊堆積的雞蛋進背包，並給農夫熟練度 XP。
    /// `plot_id`：目標農田地塊（必須是本人擁有的地塊且有雞蛋）。
    /// 無蛋 / 非本人地塊 / 倒地中靜默忽略。
    CollectEggs { plot_id: u32 },
    /// 種植作物（ROADMAP 49）：在自己的農田地塊種植作物，花乙太購入種苗。
    /// `plot_id`：目標農田地塊（必須是本人擁有的 Farm 類型地塊）。
    /// `crop_type`：作物種類 wire key（"wheat"/"carrot"/"potato"）。
    /// 乙太不足 / 非農田 / 非本人地塊 / 已達上限 / 倒地中靜默忽略。
    PlantCrop { plot_id: u32, crop_type: String },
    /// 收割作物（ROADMAP 49）：收取農田地塊所有成熟作物進背包，並給農夫熟練度 XP。
    /// `plot_id`：目標農田地塊（必須是本人擁有的地塊且有成熟作物）。
    /// 無成熟作物 / 非本人地塊 / 倒地中靜默忽略。
    HarvestCrops { plot_id: u32 },
    /// 採集星晶礦脈（ROADMAP 50）：在夜間採集附近的星晶礦脈。
    /// 伺服器驗：目前是夜間、玩家在礦脈 80px 內、礦脈未被採集、未倒地。
    /// 成功：背包加 1 個星晶碎片、給 15 點探索者熟練度 XP。
    /// 白天 / 太遠 / 礦脈已採 / 倒地中靜默忽略。
    GatherStarCrystal,
    /// 採集乙太微粒（ROADMAP 142）：玩家靠近野生動物死亡位置，採集乙太微粒得乙太。
    /// `orb_id`：要採集的微粒 ID（前端從快照取得）。
    /// 倒地中 / 太遠 / ID 不存在 / 已被他人採集 → 靜默忽略。
    CollectCarrionOrb { orb_id: u32 },
    /// 採集季節性野外節點（ROADMAP 154）：玩家靠近節點 80px 內可採集，得到對應季節素材。
    /// 節點 ID 由快照 seasonal_nodes 提供；超距離 / 已耗盡 / 倒地中靜默忽略。
    #[serde(rename = "gather_seasonal_node")]
    GatherSeasonalNode { node_id: u32 },
    /// 攻擊野生動物（ROADMAP 144）：在攻擊距離內擊殺指定 ID 的野生動物。
    /// 未倒地、距離 ≤ ATTACK_WILDLIFE_REACH、ID 存在且存活 → 成功。
    /// 獵物物種：該物種對人類態度降低；掠食者物種：被獵獵物物種態度升高。
    #[serde(rename = "attack_wildlife")]
    AttackWildlife { wildlife_id: u32 },
    /// 餵食野生動物（ROADMAP 144）：消耗一個野花種子餵食指定野生動物。
    /// 需未倒地、距離 ≤ FEED_REACH、背包有野花種子 → 成功，該物種態度升高。
    #[serde(rename = "feed_wildlife")]
    FeedWildlife { wildlife_id: u32 },
    /// 接取貿易任務（ROADMAP 51）：在當前星球商人處接取一個貿易包裹。
    /// `route_id`：要接取的路線編號（需在本星球且未在冷卻中且未持有其他包裹）。
    /// 一次只能攜帶一個包裹；同路線有 5 分鐘冷卻。倒地中靜默忽略。
    PickupTrade { route_id: u8 },
    /// 交付貿易包裹（ROADMAP 51）：在目標星球商人處交付包裹，換取乙太 + 商人熟練度 XP。
    /// 伺服器驗：持有包裹、當前星球 == 包裹目標星球、靠近目標商人（SHOP_REACH）。
    /// 不符條件靜默忽略。
    DeliverTrade,
    /// 取消貿易任務（ROADMAP 51）：丟棄目前攜帶的包裹，無懲罰。
    /// 無包裹時靜默忽略。
    CancelTrade,
    /// 接取工坊訂單（ROADMAP 52）：在主城工坊 NPC 接取一張加急訂單。
    /// `order_id`：要接取的訂單編號（1-5）。需故鄉、未倒地、無進行中訂單、不在冷卻中。
    TakeWorkshopOrder { order_id: u8 },
    /// 交付工坊訂單（ROADMAP 52）：在工坊 NPC 交付所需物品，換取乙太 + 工匠熟練度 XP。
    /// 伺服器驗：有進行中訂單 + 背包有足夠物品 + 靠近工坊 NPC。不符靜默忽略。
    FulfillWorkshopOrder,
    /// 放棄工坊訂單（ROADMAP 52）：取消目前進行中的訂單，無懲罰（不啟動冷卻）。
    AbandonWorkshopOrder,
    /// 接取懸賞任務（ROADMAP 53）：在主城懸賞告示板接取一張狩獵令。
    /// `card_id`：要接取的懸賞令編號（1-5）。需故鄉、未倒地、無進行中任務、不在冷卻中。
    AcceptBounty { card_id: u8 },
    /// 放棄懸賞任務（ROADMAP 53）：取消目前進行中的任務，無懲罰（不啟動冷卻）。
    AbandonBounty,
    /// 接取探勘任務（ROADMAP 54）：在主城探勘公告欄接取一張探勘令。
    /// `order_id`：要接取的令編號（1-5）。需故鄉、未倒地、無進行中任務、不在冷卻中。
    AcceptExpedition { order_id: u8 },
    /// 採樣（ROADMAP 54）：在目標生態域且距主城達 min_dist 的地點送出採樣請求。
    /// 伺服器驗：有進行中任務、生態域正確、距離足夠。成功立即發獎並進入冷卻。
    SurveyExpedition,
    /// 放棄探勘任務（ROADMAP 54）：取消目前進行中的任務，無懲罰（不啟動冷卻）。
    AbandonExpedition,

    // ── 星際採購令（ROADMAP 55）
    /// 接取採購令：在主城採購代理人接取一張採購令。
    /// `order_id`：要接取的令編號（1-5）。需故鄉、未倒地、無進行中任務、不在冷卻中。
    #[serde(rename = "accept_procurement")]
    AcceptProcurement { order_id: u8 },
    /// 交付採購令：靠近代理人且背包碎片足夠時送出交付請求。
    /// 伺服器驗：有進行中任務、靠近 NPC、背包碎片 >= required_qty。成功消耗碎片並發獎。
    #[serde(rename = "deliver_procurement")]
    DeliverProcurement,
    /// 放棄採購任務（ROADMAP 55）：取消目前進行中的任務，無懲罰（不啟動冷卻）。
    #[serde(rename = "abandon_procurement")]
    AbandonProcurement,
    /// 接取農展委託（ROADMAP 56）：接取指定 id 的展覽委託，開始備料計時。
    #[serde(rename = "accept_fair_order")]
    AcceptFairOrder { order_id: u8 },
    /// 提交農展委託（ROADMAP 56）：備齊物品後靠近評審 NPC 提交，發放獎勵。
    #[serde(rename = "submit_fair_order")]
    SubmitFairOrder,
    /// 放棄農展委託（ROADMAP 56）：取消目前進行中的委託，無懲罰（不啟動冷卻）。
    #[serde(rename = "abandon_fair_order")]
    AbandonFairOrder,

    // ── 會動腦的 NPC（ROADMAP 57）
    /// 跟會動腦的 NPC 對話（見 npc_chat.rs）。`npc` 是 NPC 穩定 id（如 "merchant"）。
    /// 伺服器非同步呼叫地端 LLM，回 `NpcReply`（不卡遊戲迴圈）。
    TalkToNpc { npc: String, text: String },

    // ── 居民搭話（ROADMAP 118）
    /// 跟路人居民搭話。`resident_id` 為 "resident_N" 格式；玩家必須在互動範圍內。
    /// 伺服器回傳 `NpcReply` 給本人、廣播 `NpcSpeech` 泡泡給周圍玩家。
    TalkToResident { resident_id: String },
    /// 協助正在求助的居民（ROADMAP 125）。玩家必須在互動範圍內且居民有活躍請求。
    /// 成功後：清除請求、玩家獲得 HELP_REWARD_ETHER 乙太、廣播居民感謝語。
    #[serde(rename = "help_resident")]
    HelpResident { resident_id: String },
    /// 公民投票（ROADMAP 156）：對當前活躍提案投下讚成（true）或反對（false）票。
    /// 每位玩家每次提案限投一票；無活躍投票或已投過則靜默忽略。
    #[serde(rename = "civic_vote")]
    CivicVote { yes: bool },
    /// 採集流星雨星塵節點（ROADMAP 133）。玩家必須在 COLLECT_REACH 範圍內。
    /// 成功後：節點標為已採集、玩家獲得 StarDust×1。
    #[serde(rename = "collect_dust_node")]
    CollectDustNode { node_id: u32 },
    /// 向旅行商人購買（ROADMAP 135）。玩家需在 TRADE_REACH 範圍內、登入、持有足夠乙太。
    /// 成功後：扣乙太、玩家背包增加 item×qty。
    #[serde(rename = "buy_from_wanderer")]
    BuyFromWanderer { item: ItemKind, qty: u32 },
    /// 接取旅行商人限時委託（ROADMAP 136）。玩家需在 TRADE_REACH 範圍內且已登入。
    /// 商人在場期間完成委託條件後，引擎自動發放獎勵（無需手動交付）。
    #[serde(rename = "accept_merchant_quest")]
    AcceptMerchantQuest { quest_id: u8 },
    /// 向村落金庫捐獻一筆乙太（固定金額 `village_chief::DONATE_AMOUNT`）。
    /// 需登入 + 在里長互動範圍內 + 持有足夠乙太；成功廣播聊天公告。
    DonateToVillage,

    // ── 好友系統（ROADMAP 96）
    /// 加好友：依顯示名稱加對方帳號為好友。需登入；自己加自己 / 對象不存在靜默忽略。
    /// 成功後伺服器回 `FriendList` 更新。
    #[serde(rename = "add_friend")]
    AddFriend { name: String },
    /// 刪好友：依顯示名稱移除好友關係。需登入；不在好友清單靜默忽略。
    /// 成功後伺服器回 `FriendList` 更新。
    #[serde(rename = "remove_friend")]
    RemoveFriend { name: String },
    /// 請求好友清單（ROADMAP 96）：前端開啟好友面板時送出，伺服器回 `FriendList`。
    /// 未登入靜默忽略。
    #[serde(rename = "request_friend_list")]
    RequestFriendList,

    // ── 隊伍系統（ROADMAP 97）───────────────────────────────────────────
    /// 邀請玩家加入隊伍（以顯示名搜尋）。未登入 / 找不到 / 目標已在隊 → 系統通知。
    #[serde(rename = "invite_to_party")]
    InviteToParty { name: String },
    /// 接受待定的隊伍邀請。無邀請時靜默忽略。
    #[serde(rename = "join_party")]
    JoinParty,
    /// 離開目前所在隊伍（隊長離開 → 整隊解散）。不在任何隊時靜默忽略。
    #[serde(rename = "leave_party")]
    LeaveParty,
    /// 拒絕待定的隊伍邀請。
    #[serde(rename = "decline_party")]
    DeclineParty,

    // ── 倉庫（ROADMAP 105）───────────────────────────────────────────────────
    /// 購買一次倉庫擴充（WAREHOUSE_EXPANSION_COST 乙太 → 增加 WAREHOUSE_SLOTS_PER_EXPANSION 種槽）。
    /// 乙太不足 / 已達 MAX_WAREHOUSE_EXPANSIONS 上限靜默忽略；需登入。
    #[serde(rename = "buy_warehouse_expansion")]
    BuyWarehouseExpansion,
    /// 從倉庫取出物品放回背包（背包種類槽有空間才成功）。
    /// 背包種類槽滿 / 物品不在倉庫 / 數量不足靜默忽略；需登入。
    #[serde(rename = "withdraw_from_warehouse")]
    WithdrawFromWarehouse { item: ItemKind, qty: u32 },
    /// 進入自家室內（ROADMAP 111）：玩家需登入、擁有 FreeBuild 地塊、且站在地塊中心附近。
    /// 未符合條件靜默忽略；成功後玩家 snapshot 帶 indoor_plot_id。
    EnterHome,
    /// 進入室內回到室外（ROADMAP 111）：玩家需在室內。
        /// 不在室內靜默忽略；離開後玩家回到進入前的世界坐標。
        ExitHome,
        /// 向城鎮大工程捐獻（ROADMAP 131）。
        /// `item = None` 表示捐乙太；`Some(ItemKind)` 表示捐材料。
        #[serde(rename = "donate_to_project")]
        DonateToProject { item: Option<ItemKind>, qty: u32 },
        /// 在住家室內放置家具（ROADMAP 155）：玩家需在室內、背包有對應家具物品、未超過上限。
        /// `kind` 為 snake_case 家具種類（如 "steam_bed"）。
        #[serde(rename = "place_furniture")]
        PlaceFurniture { kind: String },
        /// 從住家室內移除家具（ROADMAP 155）：玩家需在室內、idx 有效。
        /// 移除後家具退還到背包；`idx` 為 home_furniture 陣列索引。
        #[serde(rename = "remove_furniture")]
        RemoveFurniture { idx: usize },
        /// 讀取城鎮記憶石（ROADMAP 157）：玩家需在 INTERACT_REACH 範圍內。
        /// 後端回傳 TownMemoryList。
        #[serde(rename = "read_town_memory")]
        ReadTownMemory,
    }

    /// 快照裡的城鎮大工程狀態（ROADMAP 131）。
    #[derive(Debug, Clone, Serialize, PartialEq)]
    pub struct TownProjectView {
        pub project_id: String,
        pub name: String,
        pub status: String, // "planning", "building", "completed"
        pub progress_pct: f32,
        pub current_ether: u32,
        pub target_ether: u32,
        pub current_wood: u32,
        pub target_wood: u32,
        pub current_stone: u32,
        pub target_stone: u32,
        pub current_crystal: u32,
        pub target_crystal: u32,
        pub top_contributors: Vec<ContributorView>,
    }

    #[derive(Debug, Clone, Serialize, PartialEq)]
    pub struct ContributorView {
        pub name: String,
        pub score: u32,
    }

    /// 快照裡的星塵採集點（ROADMAP 133 流星雨）。
    #[derive(Debug, Clone, Serialize, PartialEq)]
    pub struct DustNodeView {
        /// 節點唯一 ID（用於 CollectDustNode ClientMsg）。
        pub id: u32,
        /// 世界座標 X。
        pub wx: f32,
        /// 世界座標 Y。
        pub wy: f32,
        /// 是否為彩虹節點——每場恰好 1 個，採集得到 RainbowStarDust（ROADMAP 134）。
        pub is_rainbow: bool,
    }

/// 快照裡的季節性野外採集節點（ROADMAP 154）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SeasonalNodeView {
    /// 節點唯一 ID（跨季節不重複）。
    pub id: u32,
    /// 世界座標 X。
    pub wx: f32,
    /// 世界座標 Y。
    pub wy: f32,
    /// 所屬季節（"spring"/"summer"/"autumn"/"winter"）。
    pub season: String,
    /// 剩餘採集次數。
    pub charges: u8,
}

/// 城鎮入侵警報快照（ROADMAP 158/161）——供前端 HUD 顯示入侵狀態與倒數。
#[derive(Debug, Clone, Serialize)]
pub struct InvasionView {
    /// 入侵是否進行中。
    pub active: bool,
    /// 入侵剩餘秒數（active = false 時為 0）。
    pub remaining_secs: f32,
    /// 累計已完成的入侵波次（供前端顯示「第 N 波」）。
    pub wave_count: u32,
    /// 入侵首領「乙太霸主」是否仍存活（active = true 時有意義）。
    pub boss_alive: bool,
    /// 當前入侵等級（1/2/3），依連勝次數決定（ROADMAP 161）。
    pub wave_level: u32,
    /// 連續成功守城次數（供前端顯示連勝指示）。
    pub consecutive_successes: u32,
}

/// 旅行商人商品目錄一個條目（ROADMAP 135）。
#[derive(Debug, Clone, Serialize)]
pub struct WanderingCatalogEntry {
    /// 商品種類（snake_case 字串，前端用於顯示名稱與圖示）。
    pub item: crate::inventory::ItemKind,
    /// 乙太單價。
    pub price_ether: u32,
    /// 剩餘可售數量。
    pub remaining: u32,
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
        /// 目前開啟中的宇宙裂縫事件（ROADMAP 26）；`None` 表示無事件。
        /// 前端用來在小地圖顯示裂縫標記 + 畫面上渲染裂縫光效。
        world_event: Option<WorldEventView>,
        /// 獸潮攻城事件（ROADMAP 44）；`None` 表示目前無獸潮。
        /// 前端用來顯示倒數橫幅 + 小地圖紅色標記。
        horde_event: Option<HordeView>,
        /// 全服社群探索任務（ROADMAP 27）：三條任務的說明、進度、完成狀態。
        quests: Vec<QuestView>,
        /// 城外產權地塊（ROADMAP 34）：20 塊預定義地塊的幾何 + 地主資訊。
        /// 全部送（量小，20 × ~80 bytes）；前端繪製邊界樁與地主名牌。
        land_plots: Vec<crate::land_plot::LandPlotSnapshot>,
        /// 農田地塊牧場狀態（ROADMAP 48）：只送有雞或有蛋的地塊（稀疏）。
        /// 前端在農田地塊上繪製雞 emoji 🐔 與蛋計數。
        ranch_plots: Vec<crate::ranching::RanchPlotView>,
        /// 農田地塊作物狀態（ROADMAP 49）：只送有種植作物的地塊（稀疏）。
        /// 前端在農田地塊上繪製作物 emoji 與成熟狀態。
        farm_crop_plots: Vec<crate::farm_crops::FarmCropPlotView>,
        /// 夜間星晶礦脈（ROADMAP 50）：只有夜晚才有節點，白天空陣列。
        /// 前端在夜間地圖上渲染閃爍的星晶礦脈 ✨，靠近可採集。
        star_crystals: Vec<crate::star_crystal::StarCrystalView>,
        /// 村落節慶加成剩餘秒數（ROADMAP 64）。0 表示無活躍加成；>0 表示全服 EXP +30%。
        /// 前端依此顯示 HUD 計時器。
        village_buff_remaining_secs: u32,
        /// 村庫乙太現值（ROADMAP 64）。前端里長面板顯示給靠近玩家。
        village_treasury: u32,
        /// 天氣狀態（ROADMAP 93）：目前天氣類型與粒子強度。前端據此畫粒子特效。
        weather: WeatherView,
        /// 世界上所有灑水器（ROADMAP 112）：位置與歸屬。前端在農地上畫小圖示。
        sprinklers: Vec<crate::sprinkler::SprinklerView>,
        /// 居民廣場聚會剩餘秒數（ROADMAP 124）。0 表示無活躍聚會；>0 時全服 EXP +20%。
        /// 前端依此顯示 HUD 倒數 pill。
        gathering_secs: u32,
        /// 目前有活躍互助請求的居民 id 清單（ROADMAP 125）。
        /// 前端依此在靠近的求助居民旁顯示「🤝 幫忙」按鈕。
        active_help_requests: Vec<String>,
        /// 居民心情：(resident_id, happiness: 0-100)（ROADMAP 126）。
        /// happiness >= 70 視為快樂；前端在快樂居民頭上顯示 💛。
        resident_moods: Vec<(String, u8)>,
        /// 城鎮繁榮等級（ROADMAP 128）：0=凋零 1=平靜 2=生機 3=繁盛。
        town_prosperity_level: u8,
        /// 城鎮大工程狀態（ROADMAP 131）。
        town_project: TownProjectView,
        /// 天文台星象預報剩餘秒數（ROADMAP 132）。0 表示無活躍預報；>0 時對應 star_forecast_bonus 生效。
        star_forecast_secs: u32,
        /// 天文台星象預報加成類型（ROADMAP 132）。空字串=無加成；"exp_boost"/"travel_discount"/"gather_extra"/"npc_bonus"。
        star_forecast_bonus: String,
        /// 流星雨剩餘秒數（ROADMAP 133）。0=無流星雨；>0 時前端顯示流星粒子特效。
        meteor_shower_secs: u32,
        /// 活躍星塵採集點清單（ROADMAP 133）。流星雨期間前端在各節點位置顯示採集點。
        dust_nodes: Vec<DustNodeView>,
        /// 旅行商人剩餘秒數（ROADMAP 135）。0=不在城鎮；>0 時前端顯示商人 NPC。
        wandering_merchant_secs: u32,
        /// 旅行商人當前商品目錄（ROADMAP 135）；商人不在城鎮時為空陣列。
        wandering_catalog: Vec<WanderingCatalogEntry>,
        /// 旅行商人限時委託清單（ROADMAP 136）；商人不在城鎮時為空陣列。
        merchant_quests: Vec<crate::wandering_merchant::MerchantQuestView>,
        /// 目前季節（ROADMAP 137）：spring/summer/autumn/winter。
        /// 前端依此顯示 HUD pill；影響作物成長速度。
        current_season: String,
        /// 目前季節剩餘秒數（ROADMAP 137）：前端顯示倒計時。
        season_remaining_secs: u32,
        /// 季節性野外採集節點（ROADMAP 154）：當季活躍節點清單（耗盡者已過濾）。
        seasonal_nodes: Vec<SeasonalNodeView>,
        /// 玩家自己住家的家具清單（ROADMAP 155）：只在室內時送出（節省流量）。
        /// 前端用於繪製室內場景的家具 + 顯示家具管理面板。
        home_furniture: Vec<crate::home_furniture::FurnitureView>,
        /// 中立野生動物（ROADMAP 140）：野鳥/野鹿/小動物。
        /// 全部送出（18 隻量小；前端依 AOI 過濾）。
        wildlife: Vec<WildlifeView>,
        /// 乙太微粒（ROADMAP 142）：獵物死亡後釋出的乙太節點。
        /// 玩家靠近採集可得乙太；TTL 90 秒後自動消失。
        carion_orbs: Vec<CarrionOrbView>,
        /// 物種聚落（ROADMAP 143）：各物種的巢穴/棲地，有領地守衛行為。
        /// 靜態資料（位置不變），前端用於渲染聚落邊界圓圈與小地圖標記。
        colonies: Vec<crate::wildlife::ColonyView>,
        /// 物種態度（ROADMAP 144）：各物種對人類的態度值（0-100）與層級。
        /// 前端用於「生態」面板顯示關係狀態，並根據態度調整視覺提示。
        species_attitudes: Vec<crate::species_relations::SpeciesAttitudeView>,
        /// 公民投票（ROADMAP 156）：當前活躍投票視圖，None = 無進行中的投票。
        /// 前端顯示代言人、提案文字、投票按鈕與倒數計時。
        civic_vote: Option<crate::civic_vote::CivicVoteView>,
        /// 公民投票效果剩餘秒數（ROADMAP 156）：0 = 無效果；> 0 時依 civic_effect_kind 顯示 HUD pill。
        civic_effect_secs: u32,
        /// 公民投票效果種類（ROADMAP 156）：空字串=無；farming_festival/night_market/defense_drill。
        /// 前端依此顯示效果標籤。
        civic_effect_kind: String,
        /// 城鎮入侵警報（ROADMAP 158）：入侵是否進行中、剩餘秒數、累計波次。
        /// 前端依此顯示 HUD 警報橫幅與倒數計時。
        invasion: InvasionView,
    },
    /// 廣播聊天訊息。
    Chat { from: String, text: String },
    /// 有玩家離線。
    PlayerLeft { id: Uuid },
    /// 某玩家成功購買第一塊領地（③ Slice D）。廣播給全部客戶端；
    /// 前端取 owner == myId 才顯示購買成功提示，其餘忽略即可。
    ClaimPlotOk { owner: Uuid, plot_index: usize },
    /// 星際旅行結果（ROADMAP 20）：僅送給發起旅行的玩家。
    /// `ok=true`：旅行成功，前端播放傳送動畫並更新 HUD 行星指示。
    /// `ok=false`：旅行失敗（乙太不足 / 武裝未齊），`message` 給前端顯示原因。
    TravelResult { ok: bool, planet: String, message: String },
    /// 玩家自己的公會狀態更新（ROADMAP 29）：建立 / 加入 / 離開後送給本人。
    /// `guild = None` 表示目前不在任何公會。
    GuildUpdate { guild: Option<GuildView> },
    /// 公會列表（ROADMAP 29）：回應 `RequestGuildList`，供前端顯示瀏覽介面。
    GuildList { guilds: Vec<GuildBrief> },
    /// 公會頻道聊天訊息（ROADMAP 29）：只送給同公會成員。
    GuildChat { guild_tag: String, from: String, text: String },
    /// 玩家自己的每日任務狀態（ROADMAP 32）：回應 `RequestDailyQuests` 或任務完成後送出。
    /// `tasks` 為 3 條任務的詳細資訊；`done_count` = 目前完成數（0-3）。
    DailyQuestsUpdate { tasks: Vec<crate::daily_quest::DailyTaskView>, done_count: u32 },
    /// 排行榜（ROADMAP 33）：回應 `RequestLeaderboard`。
    /// 三榜各含前 20 名（Postgres 模式含離線玩家；記憶體模式線上玩家補底）。
    Leaderboard {
        level_top: Vec<LeaderboardEntry>,
        ether_top: Vec<LeaderboardEntry>,
        kills_top: Vec<LeaderboardEntry>,
    },
    /// 主動技能觸發廣播（ROADMAP 45）：廣播給所有連線客戶端，供前端播放技能特效動畫。
    /// `player_id`：施法玩家 id；`kind`：技能 snake_case 名稱。
    SkillActivated { player_id: Uuid, kind: String },
    /// 遠程攻擊事件廣播（ROADMAP 146）：玩家使用遠程武器攻擊時廣播，供前端播放彈道特效。
    /// `from_x/from_y`：攻擊者世界座標；`hit`：是否命中敵人。
    RangedHit { from_x: f32, from_y: f32, hit: bool },
    /// 會動腦的 NPC 對玩家說的話（單播，非同步生成後才送）。
    NpcReply { npc: String, display: String, text: String },
    /// 里長自主決定辦「村落節慶」——廣播給所有連線玩家。
    /// 收到後前端顯示公告橫幅 + 金色光暈；`duration_secs` 秒內 EXP 加成 +30%。
    VillageEvent { message: String, duration_secs: u64, new_treasury: u32 },
    /// NPC 對話泡泡（ROADMAP 92）：NPC 互聊時廣播，前端在說話者頭頂畫對話泡泡。
    /// 同時保留既有 Chat 廣播（聊天頻道記錄），兩者互補。
    /// `npc_id`：說話者穩定 id；`npc_name`：顯示名；`text`：對話內容；
    /// `display_secs`：泡泡顯示秒數；`wx`/`wy`：說話者世界座標（讓前端定位到頭頂）。
    NpcSpeech {
        npc_id: String,
        npc_name: String,
        text: String,
        display_secs: u32,
        wx: f32,
        wy: f32,
    },
    /// 一對一密語（ROADMAP 95）：只送給寄件人（回顯）和收件人。
    /// `from` = 寄件人顯示名；`to` = 收件人顯示名；`text` = 訊息內容。
    /// 後端保證：非本人相關的密語不會送達（零廣播，純單播）。
    Whisper { from: String, to: String, text: String },
    /// 好友清單（ROADMAP 96）：回應 `RequestFriendList`、`AddFriend`、`RemoveFriend`。
    /// 僅送給請求者本人；`friends` 含顯示名與即時在線狀態。
    FriendList { friends: Vec<FriendEntry> },

    // ── 隊伍系統（ROADMAP 97）───────────────────────────────────────────
    /// 收到隊伍邀請（ROADMAP 97）：發給被邀請者，前端彈出邀請通知。
    PartyInvite { from_name: String },
    /// 隊伍成員清單更新（ROADMAP 97）：邀請成功/有人離隊/人員進出後發給所有成員。
    PartyUpdate { members: Vec<String>, is_leader: bool },
    /// 隊伍已解散 / 已退出（ROADMAP 97）：發給所有前成員。
    PartyDisbanded,
    /// 隊伍頻道聊天（ROADMAP 97）：`/p 訊息` → 僅發給隊伍成員。
    PartyChat { from: String, text: String },
    /// 商人 AI 議價提案（ROADMAP 101）：商人 LLM 自主夾暗號後，引擎驗證通過時單播給玩家。
    /// 玩家看到確認對話框：顯示物品名/數量/每個價/總計；可接受或拒絕。
    /// `item_display`：中文顯示名（如「木材」）供前端直接顯示，無需前端做對映。
    DealOffer {
        npc: String,
        display: String,
        item_display: String,
        qty: u32,
        price_per: u32,
        total: u32,
    },
    /// 擊殺通知（ROADMAP 147）：擊殺怪物後單播給本人。讓玩家知道打倒了什麼、得到什麼。
    /// `enemy_name`：怪物中文顯示名；`item_display`：戰利品名×數量；`kill_total`：累計擊殺數。
    KillNotify {
        enemy_name: String,
        item_display: String,
        kill_total: u32,
    },
    /// 城鎮記憶石列表（ROADMAP 157）：回應 ReadTownMemory，單播給請求的玩家。
    TownMemoryList {
        entries: Vec<crate::town_memory::MemoryEntry>,
    },
}

/// 好友清單單筆條目（ROADMAP 96）。
#[derive(Debug, Clone, Serialize)]
pub struct FriendEntry {
    pub id: Uuid,
    pub name: String,
    pub online: bool,
}

/// 排行榜單筆條目（ROADMAP 33）。
#[derive(Debug, Clone, Serialize)]
pub struct LeaderboardEntry {
    pub rank: u32,
    pub name: String,
    pub value: u32,
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
    /// 目前護甲減傷值（持有護甲時，每次受傷减去此值，ROADMAP 19）。
    pub defense: u32,
    /// 玩家目前所在星球（ROADMAP 20/22）。"home" = 故鄉，"verdant" = 翠幽星，"crimson" = 赤焰星。
    pub planet: String,
    /// 玩家頭銜職業（ROADMAP 38 兼修熟練度；最高熟練度那條，None = 全部 0 尚未解鎖）。
    pub job_class: Option<String>,
    /// 五條熟練度累積 XP（ROADMAP 38）。前端由此計算等級（= xp / 10）與進度條。
    pub masteries: crate::class::Masteries,
    /// 玩家公會標籤（ROADMAP 29）。None = 不在任何公會。如 "STA"、"龍"。
    pub guild_tag: Option<String>,
    /// 已解鎖成就數（ROADMAP 31）。前端 HUD 顯示 「🏆 N/12」。
    pub achievement_count: u32,
    /// 已解鎖成就的 wire key 清單（ROADMAP 31）。前端成就面板顯示解鎖狀態。
    pub achievements: Vec<String>,
    /// 🗡️ 武器槽（ROADMAP 36）：已裝備武器的 snake_case ItemKind，`None` = 空槽。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equipped_weapon: Option<String>,
    /// 🛡️ 防具槽（ROADMAP 36）：已裝備護甲的 snake_case ItemKind，`None` = 空槽。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equipped_armor: Option<String>,
    /// 📿 飾品槽（ROADMAP 36）：MVP 保留，目前恆為 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equipped_accessory: Option<String>,
    /// 武器精煉等級（ROADMAP 37）。0 = 未精煉。
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub weapon_refine: u8,
    /// 武器附魔 wire key（ROADMAP 37）。`None` = 無附魔。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weapon_enchant: Option<String>,
    /// 護甲精煉等級（ROADMAP 37）。0 = 未精煉。
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub armor_refine: u8,
    /// 五技能的冷卻剩餘秒數（ROADMAP 45）。key = skill kind，value = 秒（0 = 可用）。
    /// 前端 HUD 技能面板顯示倒數。
    pub skill_cooldowns: std::collections::HashMap<String, u32>,
    /// 目前掛起的一次性技能旗標（ROADMAP 45）。前端顯示待發光效（"warcry", "bounty"...）。
    pub active_skill_flags: Vec<String>,
    /// 設定為自動施放的技能（ROADMAP 151）。前端顯示 ⚡ 自動圖示。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto_skills: Vec<String>,
    /// 目前的寵物種類 wire key（ROADMAP 46）。None = 沒有寵物。
    /// 前端用來在玩家旁邊顯示寵物 emoji，以及在 HUD 顯示寵物加成。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pet_kind: Option<String>,

    // ── 釣魚（ROADMAP 47）────────────────────────────────────────────────────
    /// 釣魚冷卻剩餘秒數（0.0 = 可立即垂釣）。前端釣魚面板顯示倒數。
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub fish_cooldown: f32,
    /// 玩家是否站在水域邊緣（80px 內有 Water biome）。前端釣魚按鈕依此啟用/禁用。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_water: bool,

    // ── 星際貿易（ROADMAP 51）────────────────────────────────────────────────
    /// 目前攜帶的貿易包裹摘要（None = 無任務）。前端 HUD 顯示「📦 攜帶中 → 目標星球」。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trade_cargo: Option<TradeCargoBrief>,
    /// 玩家是否靠近本星球商人（且本星球有可接取路線）。前端據此顯示接取 UI。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_trade_npc: bool,

    // ── 工匠工坊訂單（ROADMAP 52）────────────────────────────────────────────
    /// 工坊目前提供的 5 張訂單（靜態，前端面板列表用）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workshop_orders: Vec<WorkshopOrderBrief>,
    /// 玩家目前接取的工坊訂單（None = 無任務）。含剩餘秒數。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workshop_active: Option<WorkshopActiveView>,
    /// 工坊訂單完成後的冷卻秒數（0 = 可接取）。
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub workshop_cooldown: f32,
    /// 玩家是否靠近主城工坊 NPC（故鄉限定）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_workshop: bool,

    // ── 懸賞告示板（ROADMAP 53）──────────────────────────────────────────
    /// 告示板目前提供的 5 張懸賞令（靜態，前端面板列表用）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bounty_cards: Vec<BountyCardBrief>,
    /// 玩家目前接取的懸賞任務（None = 無任務）。含擊殺進度+剩餘秒數。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounty_active: Option<BountyActiveView>,
    /// 懸賞完成後的冷卻秒數（0 = 可接取）。
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub bounty_cooldown: f32,
    /// 玩家是否靠近主城懸賞告示板 NPC（故鄉限定）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_bounty_board: bool,

    // ── 古蹟探勘（ROADMAP 54）──────────────────────────────────────────
    /// 公告欄目前提供的 5 張探勘令（靜態，前端面板列表用）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expedition_orders: Vec<ExpeditionOrderBrief>,
    /// 玩家目前接取的探勘任務（None = 無任務）。含剩餘秒數。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expedition_active: Option<ExpeditionActiveView>,
    /// 探勘完成後的冷卻秒數（0 = 可接取）。
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub expedition_cooldown: f32,
    /// 玩家是否靠近主城探勘公告欄 NPC（故鄉限定）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_expedition_board: bool,

    // ── 星際採購令（ROADMAP 55）──────────────────────────────────────────
    /// 代理人目前提供的 5 張採購令（靜態，前端面板列表用）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procurement_orders: Vec<ProcurementOrderBrief>,
    /// 玩家目前接取的採購任務（None = 無任務）。含剩餘秒數。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procurement_active: Option<ProcurementActiveView>,
    /// 採購完成後的冷卻秒數（0 = 可接取）。
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub procurement_cooldown: f32,
    /// 玩家是否靠近主城採購代理人 NPC（故鄉限定）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_procurement_agent: bool,
    /// 農展評審同時提供的 5 張展覽委託（靜態定義，每幀廣播）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub farm_fair_orders: Vec<FairOrderBrief>,
    /// 玩家目前進行中的展覽委託（無任務時省略）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub farm_fair_active: Option<FairActiveView>,
    /// 展覽委託完成後的冷卻剩餘秒數（0 時省略）。
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub farm_fair_cooldown: f32,
    /// 玩家是否靠近農展評審 NPC（false 時省略節省流量）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_fair_judge: bool,
    /// 玩家是否靠近里長 NPC（ROADMAP 64）（false 時省略節省流量）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_village_chief: bool,
    /// 玩家是否靠近目前在場的旅人 NPC（ROADMAP 74）（false 時省略節省流量）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_traveler: bool,
    /// 玩家是否在旅行商人交易範圍內（ROADMAP 135）（false 時省略節省流量）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_wandering_merchant: bool,
    /// 玩家是否在城鎮記憶石互動範圍內（ROADMAP 157）（false 時省略節省流量）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub near_memory_stone: bool,

    /// 玩家是否在隊伍中（ROADMAP 97）。前端在名牌顯示 [隊] 標記；false 時省略節省流量。
    #[serde(default, skip_serializing_if = "is_false")]
    pub in_party: bool,

    // ── 外觀自訂（ROADMAP 98 捏臉）────────────────────────────────────────
    /// 帽型選項：0~4，0 = 頂帽（預設）。0 時省略節省流量。
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub hair_style: u8,
    /// 膚色選項：0~4，0 = 古銅金（預設）。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub skin_tone: u8,
    /// 護目鏡鏡片色：0~4，0 = 藍（預設）。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub goggle_color: u8,
    /// 服裝造型（ROADMAP 99 衣櫥）：0~5，0 = 探險家套裝（預設）。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub costume: u8,

    // ── 易腐品腐壞（ROADMAP 106）──────────────────────────────────────────────
    /// 正在倒計時的易腐品剩餘秒數（key = ItemKind snake_case，value = 秒）。
    /// 只包含背包/倉庫中實際存在的易腐品；空時省略流量。
    /// 前端背包面板用此在物品旁顯示腐壞倒計時。
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub decay_timers: std::collections::HashMap<String, u32>,

    // ── 倉庫（ROADMAP 105）───────────────────────────────────────────────────
    /// 背包目前使用的種類槽數（0~inventory_slot_max）。前端背包面板顯示 X/20。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub inventory_slot_count: u32,
    /// 背包最大種類槽數（目前固定 20）。前端背包面板顯示 X/20。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub inventory_slot_max: u32,
    /// 已購倉庫擴充次數（0 = 未解鎖，倉庫容量 0）。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub warehouse_expansions: u32,
    /// 倉庫最大種類槽數（= expansions × 20）。0 = 未購，無法存入任何物品。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub warehouse_slot_max: u32,
    /// 倉庫目前存放的物品清單（格式同 inventory）。空時省略流量。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warehouse: Vec<ItemStack>,
    // ── 住家內裝（ROADMAP 111）
    /// 玩家目前在室內的地塊 ID。None（省略）= 在室外；Some = 在該地塊的室內。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indoor_plot_id: Option<u32>,
    /// 室內 X 位置（像素）。indoor_plot_id 有值時才送。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indoor_x: Option<f32>,
    /// 室內 Y 位置（像素）。indoor_plot_id 有值時才送。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indoor_y: Option<f32>,
    /// 累計擊殺怪物數（ROADMAP 147）。HUD 顯示；重啟清空。0 時省略流量。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub kill_count: u32,

    // ── 屬性加點（ROADMAP 152）────────────────────────────────────────────────
    /// 未分配的屬性點。0 時省略流量。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub stat_points_unspent: u32,
    /// 已分配到 HP 的點數（每點 +5 max HP）。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub stat_hp: u32,
    /// 已分配到攻擊的點數（每點 +2 攻擊）。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub stat_attack: u32,
    /// 已分配到移動速度的點數（每點 +8% 速度）。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub stat_speed: u32,
    /// 已分配到攻擊速度的點數（每點 -5% 攻擊冷卻）。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub stat_atk_speed: u32,

    // ── 技能使用型熟練度（ROADMAP 153）────────────────────────────────────────
    /// 戰吼使用次數（熟練度）。0 時省略流量。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub skill_mastery_warcry: u32,
    /// 豐饒術使用次數。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub skill_mastery_bounty: u32,
    /// 精密合成使用次數。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub skill_mastery_precision: u32,
    /// 風之步使用次數。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub skill_mastery_gale: u32,
    /// 議價術使用次數。0 時省略。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub skill_mastery_haggle: u32,
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn is_zero_f32(v: &f32) -> bool {
    *v <= 0.0
}

fn is_false(v: &bool) -> bool {
    !v
}

/// 快照裡一個世界敵人的可見狀態。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EnemyView {
    /// 敵人種類（scrap_drone / ether_wisp）：前端據此選圖示與血色。
    pub kind: EnemyKind,
    pub x: f32,
    pub y: f32,
    /// 怪物等級（ROADMAP 41）：前端據此顯示 Lv.N 名牌並以顏色相對玩家等級標危險度。
    pub level: u32,
    /// 剩餘血量 / 上限（畫血條）。`alive=false` 表示被打倒、重生中(畫淡或不畫)。
    pub hp: u32,
    pub max_hp: u32,
    pub alive: bool,
    /// 兇名精英（ROADMAP 42）：level >= base_level+3，名牌加「兇名」前綴、體型微大、全服通告過。
    #[serde(default, skip_serializing_if = "is_false")]
    pub notorious: bool,
    /// 夜間休息中（ROADMAP 148）：此怪夜間回巢靜止，前端畫成半透明 + 💤 符號。
    #[serde(default, skip_serializing_if = "is_false")]
    pub resting: bool,
}

/// 背包裡的一疊物品（種類 + 數量），給快照序列化用。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ItemStack {
    pub item: ItemKind,
    pub qty: u32,
}

/// 快照裡一隻野生動物的可見狀態（ROADMAP 140）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WildlifeView {
    pub id: u32,
    /// 種類：wild_bird / wild_deer / small_critter。
    pub kind: String,
    /// 顯示名稱（中文）。
    pub name: String,
    pub x: f32,
    pub y: f32,
    /// 行為狀態：wandering / resting / fleeing / returning。
    pub state: String,
}

/// 快照裡一顆乙太微粒的位置（ROADMAP 142）。
/// 獵物死亡後在原地生成，玩家靠近可採集乙太。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CarrionOrbView {
    pub id: u32,
    pub x: f32,
    pub y: f32,
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

/// NPC 商店目錄的單筆條目（物品 + 每單位乙太價 + 收購趨勢）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShopCatalogEntry {
    pub item: ItemKind,
    /// 當前有效收購價（已套用浮動倍率，ROADMAP 40）；販售時為有效售價（含稀缺溢價，ROADMAP 104）。
    pub price_per: u32,
    /// 收購趨勢："stable"（基準價）或 "down"（被大量賣出壓低中）。
    /// 前端顯示 ↘ 指示讓玩家知道市場供給過剩。
    pub trend: String,
    /// 剩餘庫存（ROADMAP 104）：NPC 販售用；收購條目填 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stock: Option<u32>,
    /// 最大庫存（ROADMAP 104）：NPC 販售用；收購條目填 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_stock: Option<u32>,
}

/// 快照裡的 NPC 可見狀態：位置 + 商品目錄（收購 / 販售），讓前端繪製並顯示商店面板。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NpcView {
    pub id: String,
    pub name: String,
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

/// 天氣快照（ROADMAP 93）：目前天氣類型與粒子強度，前端據此畫粒子特效。
#[derive(Debug, Clone, Serialize)]
pub struct WeatherView {
    /// snake_case 天氣類型字串（clear / grassland_rain / desert_sandstorm / rocky_crystal_dust / water_sea_mist）。
    pub weather_type: String,
    /// 粒子強度 [0.0, 1.0]。晴天為 0，其餘淡入淡出。
    pub intensity: f32,
}

/// 一條全服社群任務的可見狀態（ROADMAP 27）。
#[derive(Debug, Clone, Serialize)]
pub struct QuestView {
    /// 前端顯示用說明（繁中）。
    pub description: String,
    /// 任務目標數量。
    pub goal: u32,
    /// 目前進度（全員累積）。
    pub progress: u32,
    /// 是否已完成。
    pub completed: bool,
}

/// 把 QuestState 轉成前端快照用的 Vec<QuestView>。
pub fn quests_view(qs: &QuestState) -> Vec<QuestView> {
    qs.quests.iter().map(|q| QuestView {
        description: q.description.clone(),
        goal: q.goal(),
        progress: q.progress,
        completed: q.completed,
    }).collect()
}

/// 公會詳細資訊（ROADMAP 29）：送給公會成員本人。
#[derive(Debug, Clone, Serialize)]
pub struct GuildView {
    pub id: Uuid,
    pub name: String,
    pub tag: String,
    /// 是否為創始人（會長）。
    pub is_founder: bool,
    /// 成員人數。
    pub member_count: usize,
    /// 公會金庫乙太。
    pub treasury: u32,
}

/// 公會簡介（ROADMAP 29）：供瀏覽清單使用。
#[derive(Debug, Clone, Serialize)]
pub struct GuildBrief {
    pub id: Uuid,
    pub name: String,
    pub tag: String,
    pub member_count: usize,
    pub treasury: u32,
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

    /// 前端送的 gather_seasonal_node 訊息要能被解析成 `ClientMsg::GatherSeasonalNode`（ROADMAP 154 wire contract）。
    #[test]
    fn parses_gather_seasonal_node_message() {
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"gather_seasonal_node","node_id":2}"#).unwrap();
        match msg {
            ClientMsg::GatherSeasonalNode { node_id } => assert_eq!(node_id, 2),
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
                defense: 0,
                planet: "home".into(),
                job_class: None,
                masteries: crate::class::Masteries::default(),
                guild_tag: None,
                achievement_count: 0,
                achievements: vec![],
                equipped_weapon: None,
                equipped_armor: None,
                equipped_accessory: None,
                weapon_refine: 0,
                weapon_enchant: None,
                armor_refine: 0,
                skill_cooldowns: std::collections::HashMap::new(),
                active_skill_flags: vec![],
                auto_skills: vec![],
                pet_kind: None,
                fish_cooldown: 0.0,
                near_water: false,
                trade_cargo: None,
                near_trade_npc: false,
                workshop_orders: vec![],
                workshop_active: None,
                workshop_cooldown: 0.0,
                near_workshop: false,
                bounty_cards: vec![],
                bounty_active: None,
                bounty_cooldown: 0.0,
                near_bounty_board: false,
                expedition_orders: vec![],
                expedition_active: None,
                expedition_cooldown: 0.0,
                near_expedition_board: false,
                procurement_orders: vec![],
                procurement_active: None,
                procurement_cooldown: 0.0,
                near_procurement_agent: false,
                farm_fair_orders: vec![],
                farm_fair_active: None,
                farm_fair_cooldown: 0.0,
                near_fair_judge: false,
                near_village_chief: false,
                near_traveler: false, near_wandering_merchant: false,
                near_memory_stone: false,
                in_party: false,
                hair_style: 0,
                skin_tone: 0,
                goggle_color: 0,
                costume: 0,
                decay_timers: std::collections::HashMap::new(),
                inventory_slot_count: 0,
                inventory_slot_max: 20,
                warehouse_expansions: 0,
                warehouse_slot_max: 0,
                warehouse: vec![],
                indoor_plot_id: None,
                indoor_x: None,
                indoor_y: None,
                kill_count: 0,
                stat_points_unspent: 0,
                stat_hp: 0,
                stat_attack: 0,
                stat_speed: 0,
                stat_atk_speed: 0,
                skill_mastery_warcry: 0,
                skill_mastery_bounty: 0,
                skill_mastery_precision: 0,
                skill_mastery_gale: 0,
                skill_mastery_haggle: 0,
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
                level: 1,
                hp: 5,
                max_hp: 6,
                alive: true,
                notorious: false,
                resting: false,
            }],
            daynight: DayNightView {
                phase: Phase::Day,
                light: 0.5, // 0.5 在 f32 可精確表示，避免序列化後比對浮點誤差
                night_danger: false,
            },
            listings: vec![],
            npcs: vec![NpcView {
                id: "merchant".to_string(),
                name: "商人薇拉".to_string(),
                x: 100.0,
                y: 200.0,
                buy_list: vec![ShopCatalogEntry { item: ItemKind::Wood, price_per: 1, trend: "stable".to_string(), stock: None, max_stock: None }],
                sell_list: vec![ShopCatalogEntry { item: ItemKind::Pickaxe, price_per: 15, trend: "stable".to_string(), stock: Some(8), max_stock: Some(8) }],
            }],
            terrain: vec![],
            world_event: None,
            horde_event: None,
            quests: vec![],
            land_plots: vec![],
            ranch_plots: vec![],
            farm_crop_plots: vec![],
            star_crystals: vec![],
            village_buff_remaining_secs: 0,
            village_treasury: 0,
            weather: WeatherView { weather_type: "clear".to_string(), intensity: 0.0 },
            sprinklers: vec![],
            gathering_secs: 0,
            active_help_requests: vec![],
            resident_moods: vec![],
            town_prosperity_level: 1,
            town_project: TownProjectView {
                project_id: "test".into(),
                name: "測試".into(),
                status: "building".into(),
                progress_pct: 0.5,
                current_ether: 50,
                target_ether: 100,
                current_wood: 10,
                target_wood: 20,
                current_stone: 10,
                target_stone: 20,
                current_crystal: 5,
                target_crystal: 10,
                top_contributors: vec![],
            },
            star_forecast_secs: 0,
            star_forecast_bonus: String::new(),
            meteor_shower_secs: 0,
            dust_nodes: vec![],
            wandering_merchant_secs: 0,
            wandering_catalog: vec![],
            merchant_quests: vec![],
            current_season: "spring".to_string(),
            season_remaining_secs: 1200,
            wildlife: vec![],
            carion_orbs: vec![],
            colonies: vec![],
            species_attitudes: vec![],
            seasonal_nodes: vec![],
            home_furniture: vec![],
            civic_vote: None,
            civic_effect_secs: 0,
            civic_effect_kind: String::new(),
            invasion: InvasionView { active: false, remaining_secs: 0.0, wave_count: 0, boss_alive: false, wave_level: 1, consecutive_successes: 0 },
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
        assert_eq!(v["enemies"][0]["level"], 1);
        assert_eq!(v["enemies"][0]["hp"], 5);
        assert_eq!(v["enemies"][0]["alive"], true);
        assert_eq!(v["nodes"][0]["harvestable"], true);
        assert_eq!(v["daynight"]["phase"], "day");
        assert_eq!(v["daynight"]["light"], 0.5);
        assert_eq!(v["daynight"]["night_danger"], false);
        // NPC 商人：確認序列化結構讓前端能讀 buy/sell 目錄。
        assert_eq!(v["npcs"][0]["id"], "merchant");
        assert_eq!(v["npcs"][0]["x"], 100.0);
        assert_eq!(v["npcs"][0]["buy_list"][0]["item"], "wood");
        assert_eq!(v["npcs"][0]["buy_list"][0]["price_per"], 1);
        assert_eq!(v["npcs"][0]["sell_list"][0]["item"], "pickaxe");
        assert_eq!(v["npcs"][0]["sell_list"][0]["price_per"], 15);
    }

    /// 前端送的 travel_to_planet 訊息要能被解析成 `ClientMsg::TravelToPlanet`（ROADMAP 20 wire contract）。
    #[test]
    fn parses_travel_to_planet_message() {
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"travel_to_planet","planet":"verdant"}"#).unwrap();
        match msg {
            ClientMsg::TravelToPlanet { planet } => assert_eq!(planet, "verdant"),
            other => panic!("解析成非預期變體：{other:?}"),
        }
    }

    /// 伺服器 TravelResult 序列化後帶前端依賴的欄位。
    #[test]
    fn travel_result_serializes_correctly() {
        let msg = ServerMsg::TravelResult {
            ok: true,
            planet: "verdant".into(),
            message: "歡迎來到翠幽星！".into(),
        };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(v["type"], "travel_result");
        assert_eq!(v["ok"], true);
        assert_eq!(v["planet"], "verdant");
        assert!(v["message"].as_str().unwrap().contains("翠幽星"));
    }

    /// PlayerView 快照含 planet 欄位（前端需要以判斷 HUD 星球指示器）。
    #[test]
    fn player_view_includes_planet_field() {
        use crate::inventory::ItemKind;
        let pv = PlayerView {
            id: Uuid::nil(),
            name: "測試".into(),
            species: "terran".into(),
            x: 0.0, y: 0.0,
            ether: 0, expansions: 0,
            inventory: vec![ItemStack { item: ItemKind::Wood, qty: 1 }],
            hp: 20, max_hp: 20, exp: 0, level: 0, attack: 2, defense: 0,
            planet: "verdant".into(),
            job_class: None,
            masteries: crate::class::Masteries::default(),
            guild_tag: None,
            achievement_count: 0,
            achievements: vec![],
            equipped_weapon: None,
            equipped_armor: None,
            equipped_accessory: None,
            weapon_refine: 0,
            weapon_enchant: None,
            armor_refine: 0,
            skill_cooldowns: std::collections::HashMap::new(),
            active_skill_flags: vec![],
            auto_skills: vec![],
            pet_kind: None,
            fish_cooldown: 0.0,
            near_water: false,
            trade_cargo: None,
            near_trade_npc: false,
            workshop_orders: vec![],
            workshop_active: None,
            workshop_cooldown: 0.0,
            near_workshop: false,
            bounty_cards: vec![],
            bounty_active: None,
            bounty_cooldown: 0.0,
            near_bounty_board: false,
            expedition_orders: vec![],
            expedition_active: None,
            expedition_cooldown: 0.0,
            near_expedition_board: false,
            procurement_orders: vec![],
            procurement_active: None,
            procurement_cooldown: 0.0,
            near_procurement_agent: false,
            farm_fair_orders: vec![],
            farm_fair_active: None,
            farm_fair_cooldown: 0.0,
            near_fair_judge: false,
            near_village_chief: false,
            near_traveler: false, near_wandering_merchant: false,
            near_memory_stone: false,
            in_party: false,
            hair_style: 0,
            skin_tone: 0,
            goggle_color: 0,
            costume: 0,
            decay_timers: std::collections::HashMap::new(),
            inventory_slot_count: 0,
            inventory_slot_max: 20,
            warehouse_expansions: 0,
            warehouse_slot_max: 0,
            warehouse: vec![],
            indoor_plot_id: None,
            indoor_x: None,
            indoor_y: None,
            kill_count: 0,
            stat_points_unspent: 0,
            stat_hp: 0,
            stat_attack: 0,
            stat_speed: 0,
            stat_atk_speed: 0,
            skill_mastery_warcry: 0,
            skill_mastery_bounty: 0,
            skill_mastery_precision: 0,
            skill_mastery_gale: 0,
            skill_mastery_haggle: 0,
        };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&pv).unwrap()).unwrap();
        assert_eq!(v["planet"], "verdant");
    }

    /// 赤焰星旅行 wire contract：travel_to_planet crimson 可被解析。
    #[test]
    fn parses_travel_to_crimson_message() {
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"travel_to_planet","planet":"crimson"}"#).unwrap();
        match msg {
            ClientMsg::TravelToPlanet { planet } => assert_eq!(planet, "crimson"),
            other => panic!("解析成非預期變體：{other:?}"),
        }
    }

    /// 赤焰星旅行結果序列化含正確欄位。
    #[test]
    fn travel_result_crimson_serializes_correctly() {
        let msg = ServerMsg::TravelResult {
            ok: true,
            planet: "crimson".into(),
            message: "歡迎來到赤焰星！".into(),
        };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(v["type"], "travel_result");
        assert_eq!(v["ok"], true);
        assert_eq!(v["planet"], "crimson");
        assert!(v["message"].as_str().unwrap().contains("赤焰星"));
    }

    /// ROADMAP 33 排行榜 wire contract：request_leaderboard 可被解析。
    #[test]
    fn parses_request_leaderboard_message() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"request_leaderboard"}"#).unwrap();
        assert!(matches!(msg, ClientMsg::RequestLeaderboard));
    }

    /// ROADMAP 33 排行榜回應序列化含正確欄位。
    #[test]
    fn leaderboard_response_serializes_correctly() {
        let msg = ServerMsg::Leaderboard {
            level_top: vec![LeaderboardEntry { rank: 1, name: "Alice".into(), value: 10 }],
            ether_top: vec![LeaderboardEntry { rank: 1, name: "Bob".into(), value: 500 }],
            kills_top: vec![],
        };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(v["type"], "leaderboard");
        assert_eq!(v["level_top"][0]["rank"], 1);
        assert_eq!(v["level_top"][0]["name"], "Alice");
        assert_eq!(v["level_top"][0]["value"], 10);
        assert_eq!(v["ether_top"][0]["name"], "Bob");
        assert_eq!(v["kills_top"].as_array().unwrap().len(), 0);
    }

    /// ROADMAP 95 密語回應序列化含正確欄位（wire contract）。
    #[test]
    fn whisper_response_serializes_correctly() {
        let msg = ServerMsg::Whisper {
            from: "Alice".into(),
            to: "Bob".into(),
            text: "哈囉 Bob！".into(),
        };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(v["type"], "whisper");
        assert_eq!(v["from"], "Alice");
        assert_eq!(v["to"], "Bob");
        assert_eq!(v["text"], "哈囉 Bob！");
    }

    /// ROADMAP 96 好友系統 ClientMsg wire contract。
    #[test]
    fn add_friend_message_parses_correctly() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"add_friend","name":"Alice"}"#).unwrap();
        assert!(matches!(msg, ClientMsg::AddFriend { name } if name == "Alice"));
    }

    #[test]
    fn remove_friend_message_parses_correctly() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"remove_friend","name":"Bob"}"#).unwrap();
        assert!(matches!(msg, ClientMsg::RemoveFriend { name } if name == "Bob"));
    }

    #[test]
    fn request_friend_list_message_parses_correctly() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"request_friend_list"}"#).unwrap();
        assert!(matches!(msg, ClientMsg::RequestFriendList));
    }

    #[test]
    fn friend_list_response_serializes_correctly() {
        let id = uuid::Uuid::new_v4();
        let msg = ServerMsg::FriendList {
            friends: vec![super::FriendEntry { id, name: "Charlie".into(), online: true }],
        };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(v["type"], "friend_list");
        assert_eq!(v["friends"][0]["name"], "Charlie");
        assert_eq!(v["friends"][0]["online"], true);
    }

    /// ROADMAP 97 隊伍系統 ClientMsg wire contract。
    #[test]
    fn invite_to_party_message_parses_correctly() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"invite_to_party","name":"Alice"}"#).unwrap();
        assert!(matches!(msg, ClientMsg::InviteToParty { name } if name == "Alice"));
    }

    #[test]
    fn join_party_message_parses_correctly() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"join_party"}"#).unwrap();
        assert!(matches!(msg, ClientMsg::JoinParty));
    }

    #[test]
    fn leave_party_message_parses_correctly() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"leave_party"}"#).unwrap();
        assert!(matches!(msg, ClientMsg::LeaveParty));
    }

    #[test]
    fn decline_party_message_parses_correctly() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"decline_party"}"#).unwrap();
        assert!(matches!(msg, ClientMsg::DeclineParty));
    }

    #[test]
    fn party_invite_serializes_correctly() {
        let msg = ServerMsg::PartyInvite { from_name: "Bob".into() };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(v["type"], "party_invite");
        assert_eq!(v["from_name"], "Bob");
    }

    #[test]
    fn party_update_serializes_correctly() {
        let msg = ServerMsg::PartyUpdate { members: vec!["Alice".into(), "Bob".into()], is_leader: true };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(v["type"], "party_update");
        assert_eq!(v["members"][0], "Alice");
        assert_eq!(v["is_leader"], true);
    }

    #[test]
    fn party_chat_serializes_correctly() {
        let msg = ServerMsg::PartyChat { from: "Alice".into(), text: "衝啦！".into() };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(v["type"], "party_chat");
        assert_eq!(v["from"], "Alice");
        assert_eq!(v["text"], "衝啦！");
    }

    #[test]
    fn party_disbanded_serializes_correctly() {
        let msg = ServerMsg::PartyDisbanded;
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(v["type"], "party_disbanded");
    }

    #[test]
    fn in_party_field_absent_when_false() {
        use crate::inventory::ItemKind;
        let pv = super::PlayerView {
            id: uuid::Uuid::nil(),
            name: "測試".into(),
            species: "terran".into(),
            x: 0.0, y: 0.0, ether: 0, expansions: 0,
            inventory: vec![],
            hp: 20, max_hp: 20, exp: 0, level: 0, attack: 2, defense: 0,
            planet: "home".into(),
            job_class: None,
            masteries: crate::class::Masteries::default(),
            guild_tag: None,
            achievement_count: 0, achievements: vec![],
            equipped_weapon: None, equipped_armor: None, equipped_accessory: None,
            weapon_refine: 0, weapon_enchant: None, armor_refine: 0,
            skill_cooldowns: std::collections::HashMap::new(),
            active_skill_flags: vec![],
            auto_skills: vec![],
            pet_kind: None, fish_cooldown: 0.0, near_water: false,
            trade_cargo: None, near_trade_npc: false,
            workshop_orders: vec![], workshop_active: None, workshop_cooldown: 0.0, near_workshop: false,
            bounty_cards: vec![], bounty_active: None, bounty_cooldown: 0.0, near_bounty_board: false,
            expedition_orders: vec![], expedition_active: None, expedition_cooldown: 0.0, near_expedition_board: false,
            procurement_orders: vec![], procurement_active: None, procurement_cooldown: 0.0, near_procurement_agent: false,
            farm_fair_orders: vec![], farm_fair_active: None, farm_fair_cooldown: 0.0, near_fair_judge: false,
            near_village_chief: false, near_traveler: false, near_wandering_merchant: false,
            near_memory_stone: false,
            in_party: false,
            hair_style: 0,
            skin_tone: 0,
            goggle_color: 0,
            costume: 0,
            decay_timers: std::collections::HashMap::new(),
            inventory_slot_count: 0,
            inventory_slot_max: 20,
            warehouse_expansions: 0,
            warehouse_slot_max: 0,
            warehouse: vec![],
            indoor_plot_id: None,
            indoor_x: None,
            indoor_y: None,
            kill_count: 0,
            stat_points_unspent: 0,
            stat_hp: 0,
            stat_attack: 0,
            stat_speed: 0,
            stat_atk_speed: 0,
            skill_mastery_warcry: 0,
            skill_mastery_bounty: 0,
            skill_mastery_precision: 0,
            skill_mastery_gale: 0,
            skill_mastery_haggle: 0,
        };
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&pv).unwrap()).unwrap();
        // in_party=false 時應被 skip_serializing_if 省略，節省流量
        assert!(v.get("in_party").is_none(), "in_party=false 時不應出現在 JSON");
    }

    fn make_base_player_view() -> super::PlayerView {
        super::PlayerView {
            id: uuid::Uuid::nil(),
            name: "測試".into(),
            species: "terran".into(),
            x: 0.0, y: 0.0, ether: 0, expansions: 0,
            inventory: vec![],
            hp: 20, max_hp: 20, exp: 0, level: 0, attack: 2, defense: 0,
            planet: "home".into(),
            job_class: None,
            masteries: crate::class::Masteries::default(),
            guild_tag: None,
            achievement_count: 0, achievements: vec![],
            equipped_weapon: None, equipped_armor: None, equipped_accessory: None,
            weapon_refine: 0, weapon_enchant: None, armor_refine: 0,
            skill_cooldowns: std::collections::HashMap::new(),
            active_skill_flags: vec![],
            auto_skills: vec![],
            pet_kind: None, fish_cooldown: 0.0, near_water: false,
            trade_cargo: None, near_trade_npc: false,
            workshop_orders: vec![], workshop_active: None, workshop_cooldown: 0.0, near_workshop: false,
            bounty_cards: vec![], bounty_active: None, bounty_cooldown: 0.0, near_bounty_board: false,
            expedition_orders: vec![], expedition_active: None, expedition_cooldown: 0.0, near_expedition_board: false,
            procurement_orders: vec![], procurement_active: None, procurement_cooldown: 0.0, near_procurement_agent: false,
            farm_fair_orders: vec![], farm_fair_active: None, farm_fair_cooldown: 0.0, near_fair_judge: false,
            near_village_chief: false, near_traveler: false, near_wandering_merchant: false,
            near_memory_stone: false,
            in_party: false,
            hair_style: 0,
            skin_tone: 0,
            goggle_color: 0,
            costume: 0,
            decay_timers: std::collections::HashMap::new(),
            inventory_slot_count: 0,
            inventory_slot_max: 20,
            warehouse_expansions: 0,
            warehouse_slot_max: 0,
            warehouse: vec![],
            indoor_plot_id: None,
            indoor_x: None,
            indoor_y: None,
            kill_count: 0,
            stat_points_unspent: 0,
            stat_hp: 0,
            stat_attack: 0,
            stat_speed: 0,
            stat_atk_speed: 0,
            skill_mastery_warcry: 0,
            skill_mastery_bounty: 0,
            skill_mastery_precision: 0,
            skill_mastery_gale: 0,
            skill_mastery_haggle: 0,
        }
    }

    #[test]
    fn appearance_zero_values_are_omitted_from_json() {
        // 外觀三欄位都是 0（預設值）時，不應出現在 JSON 以節省流量。
        // 非零時應出現。
        let mut pv = make_base_player_view();
        pv.hair_style = 0;
        pv.skin_tone = 0;
        pv.goggle_color = 0;
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&pv).unwrap()).unwrap();
        assert!(v.get("hair_style").is_none(), "hair_style=0 時不應出現");
        assert!(v.get("skin_tone").is_none(), "skin_tone=0 時不應出現");
        assert!(v.get("goggle_color").is_none(), "goggle_color=0 時不應出現");

        // 非零值應出現。
        pv.hair_style = 2;
        pv.skin_tone = 3;
        pv.goggle_color = 4;
        let v2: serde_json::Value = serde_json::from_str(&serde_json::to_string(&pv).unwrap()).unwrap();
        assert_eq!(v2["hair_style"], 2);
        assert_eq!(v2["skin_tone"], 3);
        assert_eq!(v2["goggle_color"], 4);
    }

    #[test]
    fn costume_zero_omitted_nonzero_present() {
        // costume=0（預設探險家套裝）時省略節省流量，非零才出現（ROADMAP 99 衣櫥）。
        let mut pv = make_base_player_view();
        pv.costume = 0;
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&pv).unwrap()).unwrap();
        assert!(v.get("costume").is_none(), "costume=0 時不應出現在 JSON");

        pv.costume = 3;
        let v2: serde_json::Value = serde_json::from_str(&serde_json::to_string(&pv).unwrap()).unwrap();
        assert_eq!(v2["costume"], 3, "costume=3 應出現在 JSON");
    }

    /// 前端送的 allocate_stat 訊息要能被解析成 `ClientMsg::AllocateStat`（ROADMAP 152 JSON 契約）。
    #[test]
    fn parses_allocate_stat_message() {
        let msg: ClientMsg =
            serde_json::from_str(r#"{"type":"allocate_stat","stat":"hp","points":2}"#).unwrap();
        match msg {
            ClientMsg::AllocateStat { stat, points } => {
                assert_eq!(stat, "hp");
                assert_eq!(points, 2);
            }
            other => panic!("解析成非預期變體：{other:?}"),
        }
    }
}
