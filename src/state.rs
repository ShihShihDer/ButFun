//! 伺服器的共享狀態：權威世界、玩家清單、廣播頻道。
//!
//! 目前狀態存在記憶體裡。持久化（Postgres）刻意藏在這層之後——之後把 `players`
//! 換成「啟動時從 DB 載入、變動時寫回」即可，不用動 WebSocket / 遊戲迴圈的程式。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::sync::atomic::AtomicU64;

use std::time::Instant;
use tokio::sync::{broadcast, Semaphore};
use uuid::Uuid;

use world_core::{biome_at, resolve_move, Biome};

use crate::achievement::AchievementSet;
use crate::auth::AuthConfig;
use crate::class::Masteries;
use crate::guild::GuildStore;
use crate::connections::ConnectionCounts;
use crate::daynight::DayNight;
use crate::daynight_store::DayNightStore;
use crate::enemy_field::EnemyField;
use crate::field::Field;
use crate::field_store::FieldStore;
use crate::dynamic_price::DynamicPriceMarket;
use crate::market::Market;
use crate::gather_field::NodeField;
use crate::daily_quest::PlayerDailyState;
use crate::quest::QuestState;
use crate::world_event::WorldEvent;
use crate::inventory::Inventory;
use crate::inventory_store::InventoryStore;
use crate::warehouse::Warehouse;
use crate::tiles::TileWorld;
use crate::tile_store::TileStore;
use crate::vitals::Vitals;
use crate::land_plot::LandPlotRegistry;
use crate::land_plot_store::LandPlotStore;
use crate::npc_memory_store::NpcMemoryStore;
use crate::friends::FriendStore;
use crate::plot_registry::PlotRegistry;
use crate::ranching::RanchRegistry;
use crate::farm_crops::FarmCropRegistry;
use crate::star_crystal::StarCrystalField;
use crate::positions::PositionStore;
use crate::protocol::{ItemStack, PlayerView, WorldInfo, ServerMsg};
use crate::suggestions::SuggestionStore;
use crate::users::UserStore;

/// 世界大小（像素）。放大成大世界,讓玩家散得開、各自有空間(回應「農地都擠中央、地圖太小」)。
pub const WORLD_WIDTH: f32 = 6000.0;
pub const WORLD_HEIGHT: f32 = 6000.0;
/// 玩家移動速度（像素 / 秒）。值定義在 world-core（前端 wasm 預測要用同一份）。
pub const PLAYER_SPEED: f32 = world_core::PLAYER_SPEED;

/// 玩家 tile 碰撞半徑（像素）。用四角檢查確保玩家不嵌入格子；
/// 明顯小於半格（16px），讓玩家在單格寬隧道裡有餘裕、不會稍微偏離中線就被牆角卡住。
/// 值定義在 world-core（前端 wasm 預測要用同一份）。
pub const PLAYER_TILE_RADIUS: f32 = world_core::PLAYER_TILE_RADIUS;

/// 公共農地（軟劫掠區）的世界座標。任何已登入玩家均可種植與收割——
/// 但種在這裡的作物隨時可能被路過的其他玩家搶收（「軟劫掠」設計）。
/// 落在個人地塊螺旋區西南，讓新玩家在探索途中自然遇到。
pub const PUB_FIELD_ORIGIN_X: f32 = 2200.0;
pub const PUB_FIELD_ORIGIN_Y: f32 = 2200.0;

/// 星球 ID 常數（ROADMAP 20 多星球旅程）。
pub const PLANET_HOME: &str = "home";
pub const PLANET_VERDANT: &str = "verdant";
/// 赤焰星（ROADMAP 22）星球 ID，位於主世界遠西方（X-18000）。
pub const PLANET_CRIMSON: &str = "crimson";
/// 虛空星（ROADMAP 23）星球 ID，位於主世界遠東方（X+42000，比翠幽星更遠）。
pub const PLANET_VOID: &str = "void";
/// 霧醚星（ROADMAP 24）星球 ID，位於主世界遠西方（X-32000，比赤焰星更深的遠西方）。
pub const PLANET_AETHER: &str = "aether";
/// 星源星（ROADMAP 25）星球 ID，位於主世界極西境（X-52000，比霧醚星更深的宇宙源頭）。
pub const PLANET_ORIGIN: &str = "origin";
/// 翠幽星出生點（對應公共農地在故鄉的相對位置，讓玩家一到就有地標可找）。
pub const VERDANT_SPAWN_X: f32 = 22_400.0;
pub const VERDANT_SPAWN_Y: f32 = 3_000.0;
/// 赤焰星出生點（故鄉遠西方，與翠幽星對稱方向）。
pub const CRIMSON_SPAWN_X: f32 = -18_000.0;
pub const CRIMSON_SPAWN_Y: f32 = 3_000.0;
/// 虛空星出生點（比翠幽星更遠的東方，宇宙邊界）。
pub const VOID_SPAWN_X: f32 = 42_000.0;
pub const VOID_SPAWN_Y: f32 = 3_000.0;
/// 霧醚星出生點（比赤焰星更深的遠西方，乙太迷霧宇宙邊際）。
pub const AETHER_SPAWN_X: f32 = -32_000.0;
pub const AETHER_SPAWN_Y: f32 = 3_000.0;
/// 星源星出生點（比霧醚星更深的極西境，乙太文明的宇宙源頭）。
pub const ORIGIN_SPAWN_X: f32 = -52_000.0;
pub const ORIGIN_SPAWN_Y: f32 = 3_000.0;
/// 故鄉 ↔ 翠幽星 / 赤焰星 / 虛空星 / 霧醚星 → 故鄉的乙太燃料費（單程 30）。
pub const TRAVEL_ETHER_COST: u32 = 30;
/// 故鄉 → 赤焰星的乙太燃料費（第二顆星球，需要更多乙太）。
pub const TRAVEL_ETHER_COST_CRIMSON: u32 = 50;
/// 故鄉 / 赤焰星 → 虛空星的乙太燃料費（第三顆星球，宇宙深淵，需要最多乙太）。
pub const TRAVEL_ETHER_COST_VOID: u32 = 80;
/// 虛空星 / 赤焰星 / 故鄉 → 霧醚星的乙太燃料費（第四顆星球，乙太迷霧邊際，需要最多乙太）。
pub const TRAVEL_ETHER_COST_AETHER: u32 = 120;
/// 任意星球 → 星源星的乙太燃料費（第五顆星球，宇宙源頭極西境，需要最多乙太）。
pub const TRAVEL_ETHER_COST_ORIGIN: u32 = 150;
/// 故鄉 → 翠幽星「乙太直購航票」費用（跳過五大生態武裝收集條件，純乙太直接出發）。
/// ROADMAP 39：肝的路便宜（30），錢的路直接（300），玩家自選。
pub const TRAVEL_ETHER_COST_VERDANT_DIRECT: u32 = 300;

/// 一名玩家在伺服器上的權威狀態。
#[derive(Debug, Clone)]
pub struct Player {
    pub id: Uuid,
    pub name: String,
    pub species: String,
    pub x: f32,
    pub y: f32,
    pub input: Input,
    /// 收成累積的乙太。已登入玩家重連帶回、跨伺服器重啟也撐得過（Phase 0-E 已落地：隨
    /// `positions` 存進 players 表的 ether 欄；見 `positions.rs`／game.rs flush／ws.rs recall）。
    pub ether: u32,
    /// 採集到的物品。已登入玩家重連帶回、跨伺服器重啟也撐得過（Phase 0-E 已落地：隨
    /// `inventories` 存進 inventories 表;見 `inventory_store.rs`／game.rs flush／ws.rs recall）。
    pub inventory: Inventory,
    /// 生命值（戰鬥 1-F）。敵人反擊扣血、離戰一陣子自動回復;歸零會「被打趴」短暫休息。記憶體前置。
    pub vitals: Vitals,
    /// 農地擴張狀態（已購格數）。跨重啟持久化於 players.wallet_expansions。
    pub wallet: crate::economy::PlotWallet,
    /// 主動攻擊冷卻剩餘秒數（0.0 = 可攻擊；> 0 = 冷卻中）。由 game.rs 每 tick 遞減。
    pub attack_cooldown: f32,
    /// 累積經驗值（ROADMAP 17 升級系統）。殺怪 / 採礦得 exp，等級由 exp 推算。
    pub exp: u32,
    /// 生態圖鑑已發現物種 bitmask（ROADMAP 333）。每個位元對應 `field_guide::CATALOG`
    /// 一種生物，走近即點亮、永久保留。照 `exp` 的模式持久化（跨重啟存活，蒐集才有意義）。
    pub codex: u64,
    /// 探索圖鑑已踏足地形 bitmask（ROADMAP 336）。每個位元對應 `terrain_atlas::CATALOG`
    /// 一種奇景地形，走近即點亮、永久保留。照 `codex` 的模式持久化（跨重啟存活）。
    pub atlas: u64,
    /// 天象圖鑑已目睹天象 bitmask（ROADMAP 337）。每個位元對應 `sky_codex::CATALOG`
    /// 一種天象奇觀，身處其下即點亮、永久保留。照 `atlas` 的模式持久化（跨重啟存活）。
    pub skylog: u64,
    /// 累積人氣（ROADMAP 341 喝采人氣）。其他玩家對你「👏 喝采」即 +1，到階在名牌上
    /// 亮起人氣徽記。照 `skylog` 的模式持久化（跨重啟存活，人氣身份才有意義）。
    pub cheers: u64,
    /// 玩家目前所在星球（ROADMAP 20/22/23/24/25 多星球旅程）。
    /// "home" = 故鄉，"verdant" = 翠幽星，"crimson" = 赤焰星，"void" = 虛空星，
    /// "aether" = 霧醚星，"origin" = 星源星。
    /// 執行期狀態，重連回 home 起算（不持久化，跨重啟無礙）。
    pub planet: String,
    /// 玩家五條熟練度 XP（ROADMAP 38 兼修熟練度）。重連從 DB 載回；每次活動自動累積。
    pub masteries: crate::class::Masteries,
    /// 五條熟練度「上次已見階級 tier」快照（ROADMAP 351 階梯榮銜）。順序對齊
    /// `class::JobClass::ALL`（warrior／farmer／artisan／explorer／merchant）。`game.rs` 每幀比對
    /// 當前 `masteries.tier_snapshot()`：某條 tier 升高即「晉階」（前端噴慶賀、跨到師匠以上世界同慶）。
    /// **記憶體前置、不持久化**——連線／重連時即以當前 masteries 種下，故已是高階的回鍋玩家
    /// 不會被回放歷史晉階；之後苦練跨階才觸發。
    pub seen_mastery_tiers: [u8; 5],
    /// 玩家所屬公會的標籤快取（ROADMAP 29）。None = 不在任何公會。
    /// 公會建立 / 加入 / 離開時由 ws.rs 同步更新，PlayerView 直接讀此欄位。
    pub guild_tag: Option<String>,
    /// 玩家目前所在隊伍 ID（ROADMAP 97 臨時隊伍，記憶體模式）。None = 無隊。
    /// 邀請加入 / 離開 / 解散時由 ws.rs 同步更新，PlayerView 讀此欄位推算 in_party。
    pub party_id: Option<uuid::Uuid>,
    /// 帽型選項（ROADMAP 98 捏臉）：0~4，0 = 頂帽（預設）。進場時從 UserStore 載入。
    pub hair_style: u8,
    /// 膚色選項（ROADMAP 98 捏臉）：0~4，0 = 古銅金（預設）。進場時從 UserStore 載入。
    pub skin_tone: u8,
    /// 護目鏡鏡片色（ROADMAP 98 捏臉）：0~4，0 = 藍（預設）。進場時從 UserStore 載入。
    pub goggle_color: u8,
    /// 服裝造型（ROADMAP 99 衣櫥）：0~5，0 = 探險家套裝（預設）。進場時從 UserStore 載入。
    pub costume: u8,
    /// 玩家已解鎖的成就（ROADMAP 31）。記憶體前置，重啟清空。
    pub achievements: AchievementSet,
    /// 累計擊殺敵人數（ROADMAP 31 成就觸發用）。記憶體前置，重啟清空。
    pub kill_count: u32,
    /// 本次對話採集成功次數（ROADMAP 503 英雄碑用）。記憶體前置，重啟清零。
    pub session_gather_count: u32,
    /// 本次對話農地收穫次數（ROADMAP 503 英雄碑用）。記憶體前置，重啟清零。
    pub session_harvest_count: u32,
    /// 玩家已解鎖的稱號集合（ROADMAP 389）。記憶體前置，重啟清空。
    pub title_set: crate::player_title::TitleSet,
    /// 今日活動鏈狀態（ROADMAP 390）。記憶體前置，重啟清空，零 migration。
    pub activity_chain: crate::activity_chain::ActivityChain,
    /// 進行中的打坐（ROADMAP 391）。None = 沒在打坐；Some = 打坐中（含開始時間與起始座標）。
    /// 記憶體前置、不持久化、零 migration；game.rs 每 tick 檢查移動中斷與完成。
    pub meditation: Option<crate::meditation::Meditation>,
    /// 上次打坐完成的時間點（冷卻追蹤，ROADMAP 391）。記憶體前置，重啟清空。
    pub last_meditate: Option<std::time::Instant>,
    /// 進行中的廣場獻奏（ROADMAP 399）。None = 沒在獻奏；Some = 獻奏中（含開始時間與起始座標）。
    /// 記憶體前置、不持久化、零 migration；game.rs 每 tick 檢查移動中斷與完成。
    pub busking: Option<crate::busking::Busking>,
    /// 上次獻奏完成的時間點（冷卻追蹤，ROADMAP 399）。記憶體前置，重啟清空。
    pub last_busk: Option<std::time::Instant>,
    /// 累積的獻奏資歷（ROADMAP 399）——完成一場 +1，純記憶體，重啟清零；完成時回給玩家看「第 N 場」。
    pub busk_count: u32,
    /// 街頭合奏·共鳴樂團人數（ROADMAP 472）。0 = 沒在合奏／單獨獻奏；≥2 = 與身旁其他獻奏者
    /// 湊成的樂團人數。記憶體前置、每 tick 由 game.rs 重算、不持久化、零 migration；放進快照
    /// 廣播讓前端對樂團畫漸強的和聲音符與暖光。
    pub ensemble_size: u8,
    /// 是否正在放風箏（ROADMAP 470）。true = 拿出風箏跟世界風（430）玩。
    /// 暫態 bool（鏡像 `guard_shield`／`dodging`／獻奏旗標）：記憶體前置、不持久化、零 migration，
    /// 斷線／重啟清零；倒地時 game.rs 自動收線。放進快照廣播，旁觀者看得見「有人在放風箏」。
    pub flying_kite: bool,
    /// 夜螢提燈裡的螢火數（ROADMAP 477）。0 = 提燈空著；>0 = 夜裡捕了幾隻螢火，越多身邊柔光越亮。
    /// 記憶體前置、不持久化、零 migration、零經濟；天亮（game.rs 偵測夜→晨）時全員清零，
    /// 一夜一更新。放進快照廣播，旁觀者夜裡看得見「誰提著一盞螢火燈」（自我表達）。
    pub lantern_fireflies: u8,
    /// 暖食飽足 buff（ROADMAP 395）。None = 沒在飽足；Some = 吃料理後的限時 HP 緩慢回復。
    /// 記憶體前置、不持久化、零 migration；game.rs 每 tick 推進回血、過期自動清除。
    pub meal_buff: Option<crate::meal_buff::MealBuff>,
    /// 拿手菜熟練（ROADMAP 407）：各道料理累積烹煮次數 → 熟練階位 → 放大暖食飽足。
    /// 記憶體前置、不持久化、零 migration（鏡像 `meal_buff` / `pet`）；煮成時 ws.rs 記一次，
    /// 進食時依階位放大 `meal_buff`。純療癒向、零平衡風險（只讓飽足更綿長）。
    pub dish_mastery: crate::dish_mastery::DishMastery,
    /// 新手引導（ROADMAP 396）。全新玩家連線時種成啟用、五步走完即畢業；老玩家／訪客種成
    /// 已畢業（永不顯示）。記憶體前置、不持久化、零 migration；由 ws.rs 連線時 `seed` 種下、
    /// 各核心動作鉤子推進。`Default` = 不顯示（占位／訪客的安全預設）。
    pub onboarding: crate::onboarding::Onboarding,
    /// 精煉嘗試計數（ROADMAP 37）：每次精煉操作（成功或失敗）都遞增，確保
    /// `refine_fails` 的確定性偽隨機在連續嘗試間能得到不同結果。記憶體前置，重啟清空。
    pub refine_attempt_count: u64,
    /// 三槽裝備（ROADMAP 36）：🗡️ 武器 / 🛡️ 防具 / 📿 飾品。
    /// 持久化於 inventories 表的 equipment 欄；首次登入由 `auto_equip_best` 遷移。
    pub equipment: crate::equipment::EquipmentSlots,

    // ── 主動技能（ROADMAP 45 / 151）─────────────────────────────────────────
    /// 五技能的冷卻倒數（記憶體前置，重啟清空）。
    pub skill_cooldowns: crate::active_skill::SkillCooldowns,
    /// 戰吼旗：true 時下次攻擊打中 ATTACK_REACH 內**所有**存活敵人（群攻）。
    pub pending_warcry: bool,
    /// 豐饒術旗：true 時下次採集額外得 +3 個物品。
    pub pending_bounty: bool,
    /// 精密合成旗：true 時下次合成額外產出 +1 個成品。
    pub pending_precision: bool,
    /// 議價術旗：true 時下次 NPC 賣出額外多得等額乙太（總收入 ×2）。
    pub pending_haggle: bool,
    /// 自動施放技能集合（ROADMAP 151）：玩家選擇哪些技能要自動施放。
    /// 冷卻好後在對應行動時自動觸發，不需手動開技能面板點按。記憶體前置，重啟清空。
    pub auto_skills: std::collections::HashSet<String>,

    // ── 屬性加點（ROADMAP 152）───────────────────────────────────────────────
    /// 玩家的屬性加點分配（未分配點 + 四條已分配點）。持久化於 players 表。
    pub stats: crate::stat_points::StatPoints,

    // ── 技能使用型熟練度（ROADMAP 153）───────────────────────────────────────
    /// 五條主動技能的使用次數，越用效果越強。持久化於 players 表。
    pub skill_masteries: crate::skill_mastery::SkillMasteries,

    // ── 寵物（ROADMAP 46）────────────────────────────────────────────────
    /// 目前的寵物種類（記憶體前置，重啟後從 None 開始；設計上不持久化）。
    pub pet: Option<crate::pet::PetKind>,
    /// 寵物的世界座標（ROADMAP 343）。記憶體前置、不持久化——寵物身分本就不持久化，
    /// 座標每 tick 由 `pet_follow::follow_step` 朝主人跟隨重算，重連 / 重啟時隨馴化瞬間
    /// 歸位到主人腳邊即可。沒寵物時這對值無意義（前端只在 `pet_kind` 存在時才畫）。
    pub pet_x: f32,
    pub pet_y: f32,
    /// 寵物此刻是否正在跟別的寵物玩耍（ROADMAP 344）。記憶體前置、不持久化——每 tick 由
    /// `game.rs` 依 `pet_play::detect` 重算：附近有寵物玩伴就 true（寵物跑去中間蹦跳玩耍），
    /// 否則 false（回復跟隨主人）。前端據此在玩耍的兩寵物間播放歡樂特效。
    pub pet_playing: bool,
    /// 進行中的一趟逗玩接物（ROADMAP 345）。`None` = 沒在接物（寵物照常跟隨／玩耍）。
    /// 記憶體前置、不持久化——玩家丟出玩具時由 `ws.rs` 設成 `Some(Chasing)`，`game.rs` 每 tick
    /// 用 `pet_fetch::chase_step` 推進（衝去叼→叼回主人），叼回即清回 `None`。
    /// 接物進行中優先於跟隨／玩耍。
    pub pet_fetch: Option<crate::pet_fetch::PetFetch>,
    /// 寵物此刻是否正在接物（ROADMAP 345，= `pet_fetch.is_some()` 的快照鏡像）。供前端據此
    /// 把寵物畫成「興奮衝刺」並畫出玩具；沒接物時 false、序列化略過。
    pub pet_fetching: bool,
    /// 累積完成的逗玩接物趟數（ROADMAP 484 寵物撈寶）：每完成一趟 `game.rs` +1，用來算
    /// 「羈絆等級」（`pet_forage::bond_level`）＋摻進撈寶 seed。記憶體前置暫態、不持久化、
    /// 零 migration、重連／重啟歸零（鏡像寵物本身記憶體模式與 `fish_attempt_count`）。
    pub pet_fetch_count: u64,

    // ── 釣魚（ROADMAP 47）────────────────────────────────────────────────
    /// 釣魚冷卻剩餘秒數（0.0 = 可釣；> 0 = 冷卻中）。由 game.rs 每 tick 遞減。
    pub fish_cooldown: f32,
    /// 釣魚嘗試計數（確保每次釣魚偽隨機結果不同；記憶體前置，重啟清空）。
    pub fish_attempt_count: u64,
    /// 進行中的一趟釣魚小遊戲（ROADMAP 346）：拋竿後等咬鉤／反應窗口。
    /// 記憶體前置、不持久化、重啟清空（沒在釣＝None）。由 game.rs 每 tick 推進。
    pub fishing: Option<crate::fishing_bite::FishingCast>,

    // ── 礦脈深掘（ROADMAP 348）────────────────────────────────────────────
    /// 採礦冷卻剩餘秒數（0.0 = 可開新礦脈；> 0 = 冷卻中）。一輪結束（收礦／崩塌）後起算，
    /// 由 game.rs 每 tick 遞減。注意：冷卻只擋「開新礦脈」，不擋已開礦脈的續敲／收礦。
    pub mine_cooldown: f32,
    /// 採礦嘗試計數（讓每條礦脈的隱藏崩塌深度偽隨機不同；記憶體前置，重啟清空）。
    pub mine_attempt_count: u64,
    /// 進行中的一條礦脈（ROADMAP 348）：press-your-luck 步步深掘／見好就收。
    /// 記憶體前置、不持久化、零 migration、重啟清空（沒在挖＝None）。
    pub mining: Option<crate::mining_vein::MiningVein>,

    /// 玩家目前所在「在地地名」locale 的 id（ROADMAP 398 天地有名）。
    /// 記憶體前置、不持久化、零 migration、重啟清空（None＝尚未定位）。
    /// game.rs 每 tick 比對：踏入新 locale 即廣播 `LocaleEntered`、更新此值。
    pub current_locale: Option<i64>,

    // ── 掌勺照譜烹調（ROADMAP 349）────────────────────────────────────────
    /// 開灶冷卻剩餘秒數（0.0 = 可開灶；> 0 = 冷卻中）。開灶起算，由 game.rs 每 tick 遞減。
    pub cook_cooldown: f32,
    /// 掌勺嘗試計數（讓每趟步序偽隨機不同；記憶體前置，重啟清空）。
    pub cook_attempt_count: u64,
    /// 進行中的一趟掌勺（ROADMAP 349）：照譜下料的順序記憶小遊戲。
    /// 記憶體前置、不入快照、不持久化、零 migration、重啟清空（沒在煮＝None）。
    pub cooking: Option<crate::cooking_steps::CookSession>,
    /// 累計煮出的「完美料理」道數（ROADMAP 349）：記憶體前置、不持久化、純成就感計數。
    pub perfect_dishes: u32,

    // ── 汲泉聚精（ROADMAP 350）────────────────────────────────────────────
    /// 進行中的一趟夜泉乙太汲取（擺盪準星甜蜜區小遊戲）：開始汲取後等準星掃到甜蜜區鎖定。
    /// 記憶體前置、不入快照（僅 elapsed 隨 PlayerView 廣播以渲染準星）、不持久化、零 migration、
    /// 重啟清空（沒在汲取＝None）。由 game.rs 每 tick 推進（逾時即中斷）。
    pub aether_draw: Option<crate::aether_draw::AetherDraw>,

    // ── 林間揮斧（ROADMAP 403）────────────────────────────────────────────
    /// 伐木冷卻剩餘秒數（0.0 = 可開新一趟連揮；> 0 = 冷卻中）。放倒樹後起算，
    /// 由 game.rs 每 tick 遞減；冷卻只擋「開新連揮」。
    pub chop_cooldown: f32,
    /// 進行中的一趟伐木連揮（ROADMAP 403）：踩節拍的節奏連擊小遊戲。
    /// 記憶體前置、不持久化、零 migration、重啟清空（沒在伐＝None）；
    /// 僅 elapsed 隨 PlayerView 廣播以渲染節拍環。由 game.rs 每 tick 推進（逾時即中斷）。
    pub chopping: Option<crate::woodcutting::ChopSwing>,

    // ── 打水漂（ROADMAP 475）──────────────────────────────────────────────
    /// 甩石冷卻剩餘秒數（0.0 = 可撿下一顆石頭開蓄；> 0 = 冷卻中）。甩出後起算，
    /// 由 game.rs 每 tick 遞減；冷卻只擋「開新一趟蓄力」。記憶體前置、不持久化、零 migration。
    pub skip_cooldown: f32,
    /// 進行中的一趟蓄力甩石（ROADMAP 475）：力道計擺盪、甜蜜點放手甩得最漂亮。
    /// 記憶體前置、不持久化、零 migration、重啟清空（沒在蓄＝None）；
    /// 僅 elapsed 隨 PlayerView 廣播以渲染力道條。由 game.rs 每 tick 推進（逾時即中斷）。
    pub skipping: Option<crate::skipstone::StoneSkip>,
    /// 打水漂撈寶嘗試計數（ROADMAP 483）：摻進撈寶擲骰 seed，確保每趟結果不同；
    /// 記憶體前置、不持久化、零 migration、重啟清空。
    pub skip_attempt_count: u64,

    /// 格擋結算後的冷卻（ROADMAP 408）：只擋開新一趟格擋，避免連續格擋達成永久無敵。
    /// 記憶體前置、不持久化、零 migration、重啟清零。由 game.rs 每 tick 遞減。
    pub guard_cooldown: f32,
    /// 進行中的一趟格擋備防（ROADMAP 408 臨陣格擋）：看準甜蜜點按下的反應格擋。
    /// 記憶體前置、不持久化、零 migration、重啟清空（沒在格擋＝None）；
    /// 僅 elapsed 隨 PlayerView 廣播以渲染格擋環。由 game.rs 每 tick 推進（逾時即解除）。
    pub guarding: Option<crate::guard::GuardBrace>,
    /// 一面凝起的乙太護盾（ROADMAP 408）：限時卸掉反擊傷害的一部分。
    /// 記憶體前置、不持久化、零 migration、重啟清空（沒上盾＝None）；
    /// 反擊迴圈讀它卸傷，game.rs 每 tick 遞減其剩餘秒數（消散即清空）。
    pub guard_shield: Option<crate::guard::GuardShield>,
    /// 翻滾結算後的冷卻（ROADMAP 410 翻滾閃避）：只擋開新一趟翻滾，避免連續翻滾達成永久免傷。
    /// 記憶體前置、不持久化、零 migration、重啟清零。由 game.rs 每 tick 遞減。
    pub dodge_cooldown: f32,
    /// 進行中的一趟翻滾（ROADMAP 410）：往移動方向翻身閃開，恩典窗內完全閃掉一次敵人反擊。
    /// 記憶體前置、不持久化、零 migration、重啟清空（沒在翻滾＝None）；
    /// 僅 elapsed 隨 PlayerView 廣播以渲染翻身位移。由 game.rs 每 tick 推進（恩典窗過即落幕）；
    /// 反擊迴圈讀它的 `in_grace()` 判斷此刻是否免傷。
    pub dodging: Option<crate::dodge::DodgeRoll>,
    /// 放開蓄力後的冷卻（ROADMAP 423 蓄力重擊）：只擋開新一趟蓄力，確保蓄力是換爆發、非疊 DPS。
    /// 記憶體前置、不持久化、零 migration、重啟清零。由 game.rs 每 tick 遞減。
    pub charge_cooldown: f32,
    /// 進行中的一趟蓄力（ROADMAP 423）：按住攻擊鈕凝聚乙太，蓄越久放開那一擊越重。
    /// 記憶體前置、不持久化、零 migration、重啟清空（沒在蓄力＝None）；
    /// 僅 elapsed 隨 PlayerView 廣播以渲染蓄力環。由 game.rs 每 tick 推進（夾在滿蓄上限、不自行結束）；
    /// 放開（ReleaseCharge）時結算檔位、清空。
    pub charging: Option<crate::charged_strike::ChargedStrike>,
    /// 蓄好待擊的一記重擊（ROADMAP 423）：放開後限時存活，被下一次攻擊消費或逾時消散。
    /// 記憶體前置、不持久化、零 migration、重啟清空（沒待擊＝None）；
    /// 攻擊管線讀它的傷害倍率疊乘進 power，game.rs 每 tick 倒數其存活窗（消散即清空）。
    pub charge_ready: Option<crate::charged_strike::ChargeReady>,
    /// 身上的中毒狀態（ROADMAP 469 敵人毒襲）：被乙太迷霧／孢子系敵人擊中時注入，之後即使走出
    /// 攻擊範圍，毒仍穿透護甲持續流失生命，直到自然代謝或回城加速解毒。記憶體前置、不持久化、
    /// 零 migration、重啟清零（比照 guard_shield／dodging：暫態戰鬥狀態不該存檔）。由 game.rs
    /// 反擊迴圈注入、每秒結算毒傷；`is_active()` 隨 PlayerView 廣播供前端畫毒泡。
    pub poison: crate::affliction::Poison,
    /// 本趟遠遊足跡（ROADMAP 411 遠遊見聞）：記得這趟連線踏足過哪些 locale，踏進沒去過的新地方
    /// ＝一次「初次踏足」（攢少量探索者熟練度、增足跡計數）。記憶體前置、不持久化、零 migration、
    /// 重啟清空。由 game.rs 的地名偵測（鏡像 398 `current_locale`）順手推進。
    pub wayfaring: crate::wayfaring::Wayfaring,

    /// 釣魚個人最大尾紀錄（ROADMAP 449 漁夫的驕傲）：這趟連線各魚種釣到的最大體長。
    /// 釣到更大的同種魚＝刷新紀錄、響獎盃慶賀，給釣魚迴圈第一個「破自己紀錄」的續釣動機。
    /// 記憶體前置、不持久化、零 migration（鏡像 wayfaring／traced_constellations）；重啟清空、
    /// 重新攢。尺寸不進戰鬥／經濟結算（純炫耀向），零平衡風險。
    pub fish_records: crate::fish_size::FishRecords,

    /// 觀星已連過的星座 bitmask（ROADMAP 347）：第 i 位對應 `constellation::CATALOG[i]`。
    /// 記憶體前置、不入快照、不持久化、零 migration（鏡像 fishing／pet 等記憶體切片）；
    /// 重啟清空＝星座錄歸零、可重新連、重新領那一小筆獎勵。用來判定「今夜星座是否已連過」
    /// 以避免同一座重複領獎（首次連對才給乙太＋熟練度）。
    pub traced_constellations: u64,

    /// 已解碼的古代秘文 bitmask（ROADMAP 384）：第 i 位對應 `ancient_inscription::CATALOG[i]`。
    /// 記憶體前置、不入快照、不持久化、零 migration（鏡像 `traced_constellations`）；
    /// 重啟清空＝秘文錄歸零、可重新解、重新領較小獎勵（ws.rs 判定首次 vs 重複）。
    pub inscriptions_mask: u8,

    /// 進行中的居民和解委託（ROADMAP 364）：`Some` 表玩家已接下一樁、正帶著信物
    /// 要送給 `to` NPC；送達後清回 `None`。記憶體前置、不入快照、不持久化、零 migration
    /// （鏡像 `traced_constellations` 等記憶體切片）；重啟＝未接委託，可重新接。
    pub reconcile_errand: Option<crate::reconcile::Errand>,

    // ── 席間舉杯（ROADMAP 329：玩家加入午餐社交）──────────────────────────
    /// 舉杯同席冷卻剩餘秒數（0.0 = 可舉杯；> 0 = 冷卻中）。由 game.rs 每 tick 遞減。
    /// 記憶體前置、不持久化（純社交互動，重啟清零無妨）。
    pub toast_cooldown: f32,
    /// 舉杯次數計數：讓每次 NPC 的回敬在其模板池內逐句推進、不老是同一句。
    /// 記憶體前置、不持久化、重啟清空。
    pub toast_count: u64,

    // ── 玩家擊掌（ROADMAP 339：玩家↔玩家雙向同步動作）────────────────────────
    /// 擊掌意願倒數（幀）。玩家比擊掌時由 ws.rs 設為 `high_five::OFFER_TICKS`；game.rs 每幀
    /// 把「還在比」的玩家兩兩配對、迸特效後清零，沒配上就遞減到 0。記憶體前置、不持久化、
    /// 不入快照（純社交一次性互動）。
    pub high_five_offer: u16,

    // ── 表情共鳴（ROADMAP 340：玩家↔玩家群體同步）──────────────────────────
    /// 「最近表情」：玩家比表情時由 ws.rs 設為 `(player_emote 索引, emote_resonance::RESONANCE_WINDOW)`，
    /// game.rs 每幀把「最近還在比同個表情、又靠得夠近」的玩家聚團偵測共鳴後清掉、否則遞減倒數到 0。
    /// 記憶體前置、不持久化、不入快照（純社交一次性互動）。None = 最近沒比表情。
    pub recent_emote: Option<(u8, u16)>,

    // ── 喝采人氣（ROADMAP 341：玩家↔玩家會留下印記的互動）──────────────────
    /// 喝采意願倒數（幀）。玩家按「👏 喝采」時由 ws.rs 設為 `player_cheer::OFFER_TICKS`；
    /// game.rs 每幀替「還在喝采」的玩家挑對象、替對方人氣 +1 後清零，沒挑到就遞減到 0。
    /// 記憶體前置、不持久化、不入快照（純承載 ws→game 的意圖）。
    pub cheer_offer: u16,
    /// 防洗榜冷卻表：Key = 已喝采過的對象 id，Value = 剩餘冷卻幀數。同一對象 60s 內不重複
    /// 計數（你可替全場每人各喝一次、但不能對同一人連按刷數）。記憶體前置、不持久化、重啟清空。
    pub cheer_cooldowns: std::collections::HashMap<uuid::Uuid, u16>,

    // ── 星際貿易（ROADMAP 51）────────────────────────────────────────────
    /// 目前攜帶的貿易包裹。None = 無任務；Some = 正在跑商途中。記憶體前置，重啟清空。
    pub trade_cargo: Option<crate::trade_route::TradeCargo>,
    /// 各條貿易路線的接取冷卻剩餘秒數（Key = route_id）。由 game.rs 每 tick 推進。
    pub trade_cooldowns: crate::trade_route::TradeCooldowns,

    // ── 工匠工坊訂單（ROADMAP 52）────────────────────────────────────────
    /// 目前接取的工坊訂單。None = 無任務；Some = 進行中（含剩餘秒）。記憶體前置，重啟清空。
    pub workshop_active: Option<crate::workshop::ActiveOrder>,
    /// 工坊完成冷卻剩餘秒數（0 = 可接新訂單）。由 game.rs 每 tick 推進。
    pub workshop_cooldown: f32,

    // ── 懸賞告示板（ROADMAP 53）──────────────────────────────────────────
    /// 目前接取的懸賞任務。None = 無任務；Some = 進行中（含擊殺進度+剩餘秒）。記憶體前置，重啟清空。
    pub bounty_active: Option<crate::bounty_board::ActiveBounty>,
    /// 懸賞完成冷卻剩餘秒數（0 = 可接新任務）。由 game.rs 每 tick 推進。
    pub bounty_cooldown: f32,

    // ── 古蹟探勘（ROADMAP 54）────────────────────────────────────────────
    /// 目前接取的探勘任務。None = 無任務；Some = 進行中（含剩餘秒）。記憶體前置，重啟清空。
    pub expedition_active: Option<crate::expedition::ActiveExpedition>,
    /// 探勘完成冷卻剩餘秒數（0 = 可接新任務）。由 game.rs 每 tick 推進。
    pub expedition_cooldown: f32,

    // ── 星際採購令（ROADMAP 55）──────────────────────────────────────────
    /// 目前接取的採購任務。None = 無任務；Some = 進行中（含剩餘秒）。記憶體前置，重啟清空。
    pub procurement_active: Option<crate::procurement::ActiveProcurement>,
    /// 採購完成冷卻剩餘秒數（0 = 可接新任務）。由 game.rs 每 tick 推進。
    pub procurement_cooldown: f32,

    // ── 農產品展覽會（ROADMAP 56）────────────────────────────────────────
    /// 目前接取的展覽委託。None = 無委託；Some = 進行中（含剩餘秒）。記憶體前置，重啟清空。
    pub farm_fair_active: Option<crate::farm_fair::ActiveFairOrder>,
    /// 展覽委託完成冷卻剩餘秒數（0 = 可接新委託）。由 game.rs 每 tick 推進。
    pub farm_fair_cooldown: f32,

    // ── 倉庫（ROADMAP 105）───────────────────────────────────────────────
    /// 玩家的個人倉庫：背包種類槽滿時自動溢出至此；花乙太購買容量擴充。
    /// 記憶體模式：重啟歸零（零 migration）。
    pub warehouse: Warehouse,

    // ── 易腐品腐壞（ROADMAP 106）──────────────────────────────────────────
    /// 易腐品倒數計時器（食物/作物）。
    /// 只在玩家連線時遞減；斷線暫停、重啟歸零，不懲罰不上線。
    pub decay_timers: crate::perishable::PerishableDecayState,

    // ── 住家內裝（ROADMAP 111）───────────────────────────────────────────
    /// 玩家目前所在的室內地塊 ID。None = 在室外；Some(plot_id) = 在該 FreeBuild 地塊的室內。
    /// 記憶體模式：重啟歸零（玩家重連後在室外，設計上可接受）。
    pub indoor_plot_id: Option<u32>,
    /// 室內 X 位置（像素，相對室內空間左上角）。indoor_plot_id 為 None 時無意義。
    pub indoor_x: f32,
    /// 室內 Y 位置（像素，相對室內空間左上角）。indoor_plot_id 為 None 時無意義。
    pub indoor_y: f32,

    // ── 家具加成（ROADMAP 155）───────────────────────────────────────────
    /// 乙太箱帶來的背包額外種類槽（0 = 無乙太箱；3 = 有一個乙太箱）。
    /// 記憶體模式：與 home_furnishings 同步，重啟歸零。
    pub inventory_extra_kinds: u32,

    // ── 連殺熱度（ROADMAP 381）────────────────────────────────────────────
    /// 目前連殺數（GAP_SECS 秒內連續擊殺怪物的累計次數）。
    /// 記憶體前置、不持久化、零 migration——重啟歸零；由 ws.rs 攻擊路徑推進。
    pub kill_streak: u8,
    /// 最後一次擊殺怪物的時間戳（供 decay_if_expired 判斷是否逾時歸零）。
    /// 記憶體前置、不持久化。
    pub streak_last_kill: Option<std::time::Instant>,

    /// 旅人到來徽記截止時間戳（ROADMAP 506）：首次登入領見面禮時設為 now + 10 分鐘，
    /// 期間快照帶 `is_newcomer=true` 讓全服看到名牌旁的「新」徽記；過期後自動 false。
    /// 記憶體前置、不持久化、零 migration（重啟清零無妨，徽記只是歡迎用、過期就消）。
    pub newcomer_until: Option<std::time::Instant>,

    // ── 蒸汽載具（Phase 1-E）─────────────────────────────────────────────────
    /// 玩家正乘騎的蒸汽腳踏車 id。None = 步行；Some(id) = 正騎著該台車（移動快 3 倍）。
    /// 記憶體前置、不持久化、零 migration——重啟回到步行、車回原位。
    pub riding: Option<u32>,
    /// 雙人共乘（ROADMAP 538）：`riding` 為 Some 時，此旗標區分身分——false=駕駛（自己操控、快 3 倍）、
    /// true=後座乘客（不操控移動，由迴圈每拍黏到駕駛座標）。記憶體前置、零持久化。
    pub riding_passenger: bool,
    /// 蒸汽衝刺（ROADMAP 539）：最近一次按下「💨 衝刺」的時刻（None＝從未衝過）。冷卻與加速窗
    /// 全由「此時刻 + 牆上時鐘」確定性推導（見 `vehicle::boost_is_active` / `boost_off_cooldown`）。
    /// 記憶體前置、零持久化、零 migration——重啟回到無衝刺、冷卻已退。
    pub boost_trigger: Option<std::time::Instant>,
    /// 蒸汽衝刺本拍是否正在加速窗內——遊戲迴圈每拍由 `refresh_boost` 從 `boost_trigger` 推算，
    /// `step` 與快照都讀它（避免在純 `step` 裡呼叫時鐘、保持可測）。
    pub boosting: bool,
    /// 共乘招呼鈴（ROADMAP 540）：最近一次按下「🔔 招呼鈴」的時刻（None＝從未搖過）。信標亮燈窗與
    /// 冷卻全由「此時刻 + 牆上時鐘」確定性推導（見 `vehicle::bell_is_active` / `bell_off_cooldown`）。
    /// 只有駕駛搖得響；記憶體前置、零持久化、零 migration——重啟回到無信標、冷卻已退。
    pub bell_trigger: Option<std::time::Instant>,
    /// 騎乘巡採（ROADMAP 544）：最近一次採集臂順手採到節點的時刻（None＝本次乘騎尚未巡採過）。
    /// 採集冷卻由「此時刻 + 牆上時鐘」確定性推導（見 `vehicle::mount_gather_ready`）。
    /// 只有駕駛巡採；記憶體前置、零持久化、零 migration——重啟回到可立即巡採。
    pub mount_gather_at: Option<std::time::Instant>,
}

impl Player {
    /// 由 exp 推算等級（每 100 exp 升一級，無上限）。
    pub fn level(&self) -> u32 {
        self.exp / 100
    }

    /// `traveler_xy`：目前在場旅人的座標（None = 旅人不在場）。
    /// `wandering_merchant_active`：旅行商人是否在城鎮（ROADMAP 135）。
    pub fn view(&self, sch: &crate::npc_schedule::NpcScheduleManager, traveler_xy: Option<(f32, f32)>, wandering_merchant_active: bool) -> PlayerView {
        PlayerView {
            id: self.id,
            name: self.name.clone(),
            species: self.species.clone(),
            x: self.x,
            y: self.y,
            ether: self.ether,
            expansions: self.wallet.expansions(),
            inventory: self
                .inventory
                .entries()
                .map(|(item, qty)| ItemStack { item, qty })
                .collect(),
            hp: self.vitals.hp(),
            max_hp: self.vitals.max_hp(),
            exp: self.exp,
            codex: self.codex,
            atlas: self.atlas,
            skylog: self.skylog,
            cheers: self.cheers,
            level: self.level(),
            attack: crate::equipment::equipped_weapon_power(&self.equipment)
                + crate::combat::level_attack_bonus(self.level())
                + crate::class::combat_bonus(&self.masteries)
                + self.pet.map(|p| p.bonus_attack()).unwrap_or(0)
                + self.stats.attack * crate::stat_points::ATTACK_PER_POINT,
            defense: crate::equipment::equipped_armor_defense(&self.equipment)
                + self.pet.map(|p| p.bonus_defense()).unwrap_or(0),
            planet: self.planet.clone(),
            job_class: self.masteries.title_class().map(|c| c.as_str().to_string()),
            masteries: self.masteries,
            guild_tag: self.guild_tag.clone(),
            achievement_count: self.achievements.count() as u32,
            achievements: self.achievements.as_wire_keys().into_iter().map(|s| s.to_string()).collect(),
            active_title: self.title_set.active_wire_key().map(|s| s.to_string()),
            unlocked_titles: self.title_set.unlocked_wire_keys().into_iter().map(|s| s.to_string()).collect(),
            equipped_weapon: self.equipment.weapon
                .map(crate::equipment::item_to_wire_key),
            equipped_armor: self.equipment.armor
                .map(crate::equipment::item_to_wire_key),
            equipped_accessory: self.equipment.accessory
                .map(crate::equipment::item_to_wire_key),
            weapon_refine: self.equipment.weapon_meta.refine,
            weapon_enchant: self.equipment.weapon_meta.enchant
                .map(|e| e.wire_key().to_string()),
            armor_refine: self.equipment.armor_meta.refine,
            skill_cooldowns: self.skill_cooldowns.as_wire_map(),
            active_skill_flags: {
                let mut flags = Vec::new();
                if self.pending_warcry    { flags.push("warcry".to_string()); }
                if self.pending_bounty    { flags.push("bounty".to_string()); }
                if self.pending_precision { flags.push("precision".to_string()); }
                if self.pending_haggle    { flags.push("haggle".to_string()); }
                flags
            },
            auto_skills: self.auto_skills.iter().cloned().collect(),
            pet_kind: self.pet.map(|p| p.as_str().to_string()),
            // ROADMAP 343：有寵物才送座標（沒寵物時為 0，序列化略過，省頻寬）。
            pet_x: if self.pet.is_some() { self.pet_x } else { 0.0 },
            pet_y: if self.pet.is_some() { self.pet_y } else { 0.0 },
            // ROADMAP 344：有寵物且正在玩耍才送 true（沒寵物時 false，序列化略過）。
            pet_playing: self.pet.is_some() && self.pet_playing,
            // ROADMAP 345：接物進行中才送玩具座標＋接物旗標（沒寵物 / 沒接物時為 0／false，略過）。
            pet_toy_x: if self.pet.is_some() { self.pet_fetch.map(|f| f.toy_x).unwrap_or(0.0) } else { 0.0 },
            pet_toy_y: if self.pet.is_some() { self.pet_fetch.map(|f| f.toy_y).unwrap_or(0.0) } else { 0.0 },
            pet_fetching: self.pet.is_some() && self.pet_fetching,
            // ROADMAP 358：有寵物才送性格 wire key（由主人帳號＋寵物種類確定性算出，沒寵物時 None、略過）。
            pet_personality: self.pet.map(|k| {
                crate::pet_personality::personality_for(self.id.as_bytes(), k)
                    .as_str()
                    .to_string()
            }),
            // ROADMAP 484：有寵物才送羈絆等級（由累積接物趟數即時算，沒寵物 / 羈絆 0 時為 0、略過序列化）。
            // 前端據此在寵物腳邊畫一排小愛心、寵物面板顯示「默契」——玩家一眼看得到默契越玩越深。
            pet_bond: if self.pet.is_some() {
                crate::pet_forage::bond_level(self.pet_fetch_count)
            } else {
                0
            },
            fish_cooldown: self.fish_cooldown,
            near_water: crate::fishing::is_near_water(self.x, self.y),
            // ROADMAP 346：進行中釣魚小遊戲的階段（沒在釣＝None，略過序列化）。
            fishing_phase: self.fishing.map(|c| c.phase().as_str()),
            // ROADMAP 384：是否在沙漠生態域附近（有遺跡可供啟靈）。
            near_ruin: crate::ancient_inscription::is_near_ruin(self.x, self.y),
            // ROADMAP 348：採礦冷卻＋是否站在岩地旁（前端採礦鈕依此啟用／顯示倒數）。
            mine_cooldown: self.mine_cooldown,
            near_rock: crate::mining_vein::is_near_rock(self.x, self.y),
            // ROADMAP 348：進行中礦脈的深度／累積袋量／震動等級（沒在挖＝None，略過序列化）。
            mining_depth: self.mining.map(|v| v.depth()),
            mining_haul: self.mining.map(|v| v.haul()),
            mining_tremor: self.mining.map(|v| v.tremor().as_str()),
            // ROADMAP 349：開灶冷卻，供前端料理「掌勺」鈕顯示冷卻倒數。
            cook_cooldown: self.cook_cooldown,
            // ROADMAP 350：進行中夜泉汲取的經過秒數（沒在汲取＝None，略過序列化）；
            // 前端據此用同一條三角波公式渲染擺盪準星位置。
            aether_draw_secs: self.aether_draw.map(|d| d.elapsed()),
            // ROADMAP 403：進行中伐木連揮的經過秒數（沒在伐＝None，略過序列化）；
            // 前端據此用同一條公式渲染脈動的節拍環。
            chop_secs: self.chopping.map(|c| c.elapsed()),
            // ROADMAP 475：進行中蓄力甩石的經過秒數（沒在蓄＝None，略過序列化）；
            // 前端據此用同一條公式渲染擺盪的力道條。
            skip_charge: self.skipping.map(|s| s.elapsed()),
            // ROADMAP 408：進行中格擋備防的經過秒數（沒在格擋＝None，略過序列化）；
            // 廣播給所有人，前端據此用同一條公式渲染收束的格擋環。
            guard_secs: self.guarding.map(|g| g.elapsed()),
            // ROADMAP 408：此刻乙太護盾的卸傷強度（沒上盾＝None，略過序列化）；
            // 廣播給所有人，前端畫頭頂護盾微光。
            guard_shield_pct: self.guard_shield.map(|s| s.pct()),
            // ROADMAP 410：進行中翻滾的經過秒數（沒在翻滾＝None，略過序列化）；
            // 廣播給所有人，前端據此演出翻身位移與翻滾環。
            dodge_secs: self.dodging.map(|d| d.elapsed()),
            // ROADMAP 541：復原喘息恩典剩餘秒數（沒在恩典中＝None，略過序列化）；
            // 廣播給所有人，前端據此在玩家身上畫一圈柔和的恩典護盾微光。
            recovery_grace_secs: self.vitals.recovery_grace_secs(),
            // ROADMAP 423：進行中蓄力的進度 [0,1]（沒在蓄力＝None，略過序列化）；
            // 廣播給所有人，前端據此渲染逐漸收束的蓄力環（滿蓄＝環滿）。
            charge_progress: self.charging.map(|c| c.progress()),
            // ROADMAP 411：本趟遠遊踏足過的不同地方數（0＝略過序列化）；前端 HUD 畫足跡計數。
            wayfare_count: self.wayfaring.tally(),
            // ROADMAP 329：舉杯同席冷卻，供前端在廣場餐桌旁的「舉杯」鈕顯示冷卻倒數。
            toast_cooldown: self.toast_cooldown,
            trade_cargo: self.trade_cargo.as_ref().map(|c| crate::protocol::TradeCargoBrief {
                route_id: c.route_id,
                cargo_name: c.cargo_name.clone(),
                dest: c.dest.clone(),
                reward: c.reward,
            }),
            near_trade_npc: {
                use crate::npc::*;
                use crate::trade_route::routes_for_planet;
                // 玩家是否靠近當前星球的商人（有可接取路線時才顯示接取 UI）。
                let merchant_xy = match self.planet.as_str() {
                    "verdant" => verdant_merchant_pos(),
                    "crimson" => crimson_merchant_pos(),
                    "void"    => void_merchant_pos(),
                    "aether"  => aether_merchant_pos(),
                    "origin"  => origin_merchant_pos(),
                    _         => sch.get_pos("merchant").unwrap_or(merchant_pos()),
                };
                let dx = self.x - merchant_xy.0;
                let dy = self.y - merchant_xy.1;
                let dist = (dx * dx + dy * dy).sqrt();
                dist <= SHOP_REACH && !routes_for_planet(&self.planet).is_empty()
            },
            // ── 工匠工坊訂單（ROADMAP 52）
            workshop_orders: {
                use crate::workshop::WORKSHOP_ORDERS;
                use crate::protocol::{WorkshopOrderBrief};
                WORKSHOP_ORDERS.iter().map(|o| WorkshopOrderBrief {
                    order_id: o.id,
                    name: o.name.to_string(),
                    item: o.required_item,
                    qty: o.required_qty,
                    reward: o.reward,
                    xp: o.xp,
                }).collect()
            },
            workshop_active: self.workshop_active.as_ref().and_then(|a| {
                use crate::workshop::find_order;
                use crate::protocol::WorkshopActiveView;
                find_order(a.order_id).map(|o| WorkshopActiveView {
                    order_id: a.order_id,
                    name: o.name.to_string(),
                    item: o.required_item,
                    qty: o.required_qty,
                    reward: o.reward,
                    remaining_secs: a.remaining_secs,
                })
            }),
            workshop_cooldown: self.workshop_cooldown,
            near_workshop: self.planet == PLANET_HOME
                && sch.get_pos("workshop_npc").map(|(nx, ny)| {
                    let dx = self.x - nx;
                    let dy = self.y - ny;
                    dx * dx + dy * dy <= crate::npc::SHOP_REACH * crate::npc::SHOP_REACH
                }).unwrap_or(false),
            bounty_cards: {
                use crate::bounty_board::BOUNTY_CARDS;
                use crate::protocol::BountyCardBrief;
                BOUNTY_CARDS.iter().map(|c| BountyCardBrief {
                    card_id: c.id,
                    name: c.name.to_string(),
                    target_name: c.target_kind.display_name().to_string(),
                    required_kills: c.required_kills,
                    reward: c.reward,
                    xp: c.xp,
                }).collect()
            },
            bounty_active: self.bounty_active.as_ref().and_then(|a| {
                use crate::bounty_board::find_card;
                use crate::protocol::BountyActiveView;
                find_card(a.card_id).map(|c| BountyActiveView {
                    card_id: a.card_id,
                    name: c.name.to_string(),
                    target_name: c.target_kind.display_name().to_string(),
                    required_kills: c.required_kills,
                    kills_done: a.kills_done,
                    reward: c.reward,
                    remaining_secs: a.remaining_secs,
                })
            }),
            bounty_cooldown: self.bounty_cooldown,
            near_bounty_board: self.planet == PLANET_HOME
                && sch.get_pos("bounty_npc").map(|(nx, ny)| {
                    let dx = self.x - nx;
                    let dy = self.y - ny;
                    dx * dx + dy * dy <= crate::npc::SHOP_REACH * crate::npc::SHOP_REACH
                }).unwrap_or(false),
            // ── 古蹟探勘（ROADMAP 54）
            expedition_orders: {
                use crate::expedition::EXPEDITION_ORDERS;
                use crate::protocol::ExpeditionOrderBrief;
                EXPEDITION_ORDERS.iter().map(|o| ExpeditionOrderBrief {
                    order_id: o.id,
                    name: o.name.to_string(),
                    biome_name: o.biome_name.to_string(),
                    min_dist: o.min_dist,
                    reward: o.reward,
                    xp: o.xp,
                }).collect()
            },
            expedition_active: self.expedition_active.as_ref().and_then(|a| {
                use crate::expedition::find_order;
                use crate::protocol::ExpeditionActiveView;
                find_order(a.order_id).map(|o| ExpeditionActiveView {
                    order_id: a.order_id,
                    name: o.name.to_string(),
                    biome_name: o.biome_name.to_string(),
                    min_dist: o.min_dist,
                    reward: o.reward,
                    remaining_secs: a.remaining_secs,
                })
            }),
            expedition_cooldown: self.expedition_cooldown,
            near_expedition_board: self.planet == PLANET_HOME
                && sch.get_pos("expedition_npc").map(|(nx, ny)| {
                    let dx = self.x - nx;
                    let dy = self.y - ny;
                    dx * dx + dy * dy <= crate::npc::SHOP_REACH * crate::npc::SHOP_REACH
                }).unwrap_or(false),
            // ── 星際採購令（ROADMAP 55）
            procurement_orders: {
                use crate::procurement::PROCUREMENT_ORDERS;
                use crate::protocol::ProcurementOrderBrief;
                PROCUREMENT_ORDERS.iter().map(|o| ProcurementOrderBrief {
                    order_id: o.id,
                    name: o.name.to_string(),
                    item: o.required_item,
                    item_name: o.item_name.to_string(),
                    required_qty: o.required_qty,
                    reward: o.reward,
                    xp: o.xp,
                }).collect()
            },
            procurement_active: self.procurement_active.as_ref().and_then(|a| {
                use crate::procurement::find_order;
                use crate::protocol::ProcurementActiveView;
                find_order(a.order_id).map(|o| ProcurementActiveView {
                    order_id: a.order_id,
                    name: o.name.to_string(),
                    item: o.required_item,
                    item_name: o.item_name.to_string(),
                    required_qty: o.required_qty,
                    reward: o.reward,
                    remaining_secs: a.remaining_secs,
                })
            }),
            procurement_cooldown: self.procurement_cooldown,
            near_procurement_agent: self.planet == PLANET_HOME
                && sch.get_pos("procurement_npc").map(|(nx, ny)| {
                    let dx = self.x - nx;
                    let dy = self.y - ny;
                    dx * dx + dy * dy <= crate::npc::SHOP_REACH * crate::npc::SHOP_REACH
                }).unwrap_or(false),
            farm_fair_orders: {
                use crate::farm_fair::FAIR_ORDERS;
                use crate::protocol::{FairOrderBrief, FairReqView};
                FAIR_ORDERS.iter().map(|o| FairOrderBrief {
                    order_id: o.id,
                    name: o.name.to_string(),
                    reqs: o.reqs.iter().map(|r| FairReqView {
                        item: r.item,
                        item_name: r.item_name.to_string(),
                        qty: r.qty,
                    }).collect(),
                    reward: o.reward,
                    xp: o.xp,
                }).collect()
            },
            farm_fair_active: self.farm_fair_active.as_ref().and_then(|a| {
                use crate::farm_fair::find_order;
                use crate::protocol::{FairActiveView, FairReqView};
                find_order(a.order_id).map(|o| FairActiveView {
                    order_id: a.order_id,
                    name: o.name.to_string(),
                    reqs: o.reqs.iter().map(|r| FairReqView {
                        item: r.item,
                        item_name: r.item_name.to_string(),
                        qty: r.qty,
                    }).collect(),
                    reward: o.reward,
                    remaining_secs: a.remaining_secs,
                })
            }),
            farm_fair_cooldown: self.farm_fair_cooldown,
            near_fair_judge: self.planet == PLANET_HOME
                && sch.get_pos("farm_fair_npc").map(|(nx, ny)| {
                    let dx = self.x - nx;
                    let dy = self.y - ny;
                    dx * dx + dy * dy <= crate::npc::SHOP_REACH * crate::npc::SHOP_REACH
                }).unwrap_or(false),
            near_village_chief: self.planet == PLANET_HOME
                && sch.get_pos("village_chief").map(|(nx, ny)| {
                    let dx = self.x - nx;
                    let dy = self.y - ny;
                    (dx * dx + dy * dy).sqrt() <= crate::village_chief::CHIEF_REACH
                }).unwrap_or(false),
            near_wandering_merchant: wandering_merchant_active
                && self.planet == PLANET_HOME
                && {
                    let dx = self.x - crate::wandering_merchant::WANDERER_X;
                    let dy = self.y - crate::wandering_merchant::WANDERER_Y;
                    dx * dx + dy * dy
                        <= crate::wandering_merchant::TRADE_REACH
                            * crate::wandering_merchant::TRADE_REACH
                },
            near_traveler: self.planet == PLANET_HOME
                && traveler_xy.map(|(nx, ny)| {
                    let dx = self.x - nx;
                    let dy = self.y - ny;
                    dx * dx + dy * dy <= crate::traveler_npc::TRAVELER_REACH * crate::traveler_npc::TRAVELER_REACH
                }).unwrap_or(false),
            near_memory_stone: self.planet == PLANET_HOME
                && crate::town_memory::is_near_stone(self.x, self.y),
            in_party: self.party_id.is_some(),
            hair_style: self.hair_style,
            skin_tone: self.skin_tone,
            goggle_color: self.goggle_color,
            costume: self.costume,
            // ── 易腐品腐壞（ROADMAP 106）
            decay_timers: self.decay_timers.all_timers()
                .map(|(item, secs)| {
                    // 序列化 key 為 snake_case（serde 約定）
                    let key = serde_json::to_value(item)
                        .ok()
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .unwrap_or_default();
                    (key, secs)
                })
                .filter(|(k, _)| !k.is_empty())
                .collect(),
            // ── 倉庫（ROADMAP 105）
            inventory_slot_count: self.inventory.kind_count() as u32,
            inventory_slot_max: crate::warehouse::MAX_INVENTORY_ITEM_KINDS as u32,
            warehouse_expansions: self.warehouse.expansions as u32,
            warehouse_slot_max: self.warehouse.capacity() as u32,
            warehouse: self.warehouse.entries()
                .map(|(item, qty)| crate::protocol::ItemStack { item, qty })
                .collect(),
            // ── 住家內裝（ROADMAP 111）
            indoor_plot_id: self.indoor_plot_id,
            indoor_x: self.indoor_plot_id.map(|_| self.indoor_x),
            indoor_y: self.indoor_plot_id.map(|_| self.indoor_y),
            // ── 擊殺計數（ROADMAP 147）
            kill_count: self.kill_count,
            // ── 屬性加點（ROADMAP 152）
            stat_points_unspent: self.stats.unspent,
            stat_hp: self.stats.hp,
            stat_attack: self.stats.attack,
            stat_speed: self.stats.speed,
            stat_atk_speed: self.stats.atk_speed,
            // ── 技能使用型熟練度（ROADMAP 153）
            skill_mastery_warcry:    self.skill_masteries.warcry,
            skill_mastery_bounty:    self.skill_masteries.bounty,
            skill_mastery_precision: self.skill_masteries.precision,
            skill_mastery_gale:      self.skill_masteries.gale,
            skill_mastery_haggle:    self.skill_masteries.haggle,
            // ROADMAP 381：連殺熱度，只有本人快照才有意義；
            // 衰退判斷由 ws.rs 在出招前呼叫 decay_if_expired，此處只取當下值。
            kill_streak: self.kill_streak,
            // ROADMAP 390：今日活動鏈環數（0~5）。只在自己的快照裡有意義，他人省略（is_zero_u8 略過）。
            chain_links: self.activity_chain.link_count(),
            // ROADMAP 391：是否正在打坐（廣播給所有人，前端畫呼吸光圈）。
            meditating: self.meditation.is_some(),
            // ROADMAP 399：是否正在廣場獻奏（廣播給所有人，前端畫頭頂飄動音符）。
            busking: self.busking.is_some(),
            // ROADMAP 535：曲目身段（由累計獻奏場次推導，前端據此挑頭頂音符調色盤的華麗度）。
            busk_tier: crate::busking_repertoire::tier_for_count(self.busk_count).wire(),
            // ROADMAP 472：合奏人數（廣播給所有人，前端對樂團畫漸強和聲音符、圍聽者見療癒）。
            ensemble: self.ensemble_size,
            // ROADMAP 470：是否正在放風箏（廣播給所有人，前端畫順風飄揚的風箏）。
            flying_kite: self.flying_kite,
            // ROADMAP 477：夜螢提燈螢火數（夜間廣播給所有人，前端畫身邊柔光暈）。
            lantern_fireflies: self.lantern_fireflies,
            // ROADMAP 395：暖食飽足進度（廣播給所有人，前端畫頭頂暖食光暈）。
            well_fed: self.meal_buff.as_ref().map(|b| b.progress()),
            // ROADMAP 407：此刻飽足來自的料理熟練階位（順手／拿手才標記；生手與沒飽足＝None，省流量）。
            well_fed_tier: self.meal_buff.as_ref()
                .and_then(|b| self.dish_mastery.tier_of(b.kind).badge())
                .map(|s| s.to_string()),
            // ROADMAP 396：新手引導進度（只對引導啟用中的全新玩家有值；老玩家／已畢業＝None）。
            onboarding: self.onboarding.is_active().then(|| crate::protocol::OnboardView {
                done: self.onboarding.mask(),
                count: self.onboarding.done_count(),
                // ROADMAP 413：當前該做的下一步 index（教學順序中第一個未完成步），前端據此引路。
                next: self.onboarding.next_step().map(|s| s as u8),
            }),
            // ROADMAP 418：歸家羅盤——這裡 view() 拿不到 plots 登記表，故先留 None，
            // 由快照層（game.rs，握得到 app.plots）對「有地玩家」補上回家方位／距離／八方位。
            home_bearing: None,
            home_dist: None,
            home_dir: None,
            // ROADMAP 426：情境下一步提示——同樣由快照層（game.rs，握得到 is_night 等情境）
            // 對已畢業玩家補上，這裡先留 None。
            idle_nudge: None,
            // ROADMAP 469：中毒狀態隨快照廣播，前端據此畫毒泡。
            poisoned: self.poison.is_active(),
            // ROADMAP 506：旅人到來——首次登入 10 分鐘內名牌旁出現「新」徽記，廣播給全服。
            is_newcomer: self.newcomer_until
                .map(|t| t > std::time::Instant::now())
                .unwrap_or(false),
            // Phase 1-E 蒸汽載具：是否正乘騎，廣播給全服（前端把人畫在車座上）。
            riding: self.riding.is_some(),
            // ROADMAP 539 蒸汽衝刺：駕駛此刻是否正在衝刺加速窗內，廣播給全服（前端噴一團蒸汽爆發）。
            boosting: self.boosting,
            // ROADMAP 533：守護者元素祝福——由 game.rs 快照迴圈補填，view() 預設 None。
            guardian_blessing: None,
        }
    }

    /// 向背包加物品；若背包種類槽已滿則自動轉存倉庫（ROADMAP 105）。
    /// 回傳 `(added_to_inv, added_to_warehouse, dropped)`。
    /// - `added_to_inv`：實際加入背包的量。
    /// - `added_to_warehouse`：轉存倉庫的量。
    /// - `dropped`：兩者皆滿而丟棄的量（實務上幾乎不會發生，因為最大容量 > 全遊戲 58 種）。
    pub fn add_item_overflow(&mut self, item: crate::inventory::ItemKind, qty: u32) -> (u32, u32, u32) {
        use crate::warehouse::MAX_INVENTORY_ITEM_KINDS;
        if qty == 0 {
            return (0, 0, 0);
        }
        let max_kinds = MAX_INVENTORY_ITEM_KINDS + self.inventory_extra_kinds as usize;
        // 背包仍可接受（已有此種類，或種類槽未滿）
        if !self.inventory.is_full_for_new_kind(item, max_kinds) {
            let added = self.inventory.add(item, qty);
            let remaining = qty - added;
            if remaining == 0 {
                return (added, 0, 0);
            }
            // MAX_STACK 夾住了，剩餘嘗試倉庫
            let wh = self.warehouse.add(item, remaining);
            return (added, wh, remaining - wh);
        }
        // 背包種類槽已滿 → 全部轉倉庫
        let wh = self.warehouse.add(item, qty);
        (0, wh, qty - wh)
    }

    /// 星際旅行可行性驗證（純函式，供 ws.rs 呼叫與測試）。
    /// `discount` 為探索者職業折扣（`crate::class::travel_cost_reduction`），無職業傳 0。
    /// 回傳 `Ok(())` = 可以旅行；`Err(msg)` = 失敗原因字串。
    pub fn can_travel_to(&self, dest: &str, discount: u32) -> Result<(), String> {
        use crate::inventory::ItemKind;
        if self.vitals.is_downed() {
            return Err("倒地中無法旅行".into());
        }
        if dest == PLANET_VERDANT && self.planet == PLANET_HOME {
            // 故鄉 → 翠幽星：兩條路徑（ROADMAP 39）。
            // 武裝路：集齊五大生態武裝 + 30 乙太（享探索者折扣）。
            // 直購路：300 乙太直接購票（跳過武裝條件，享探索者折扣）。
            let biome_weapons = [
                ItemKind::MeadowAmulet,
                ItemKind::MushroomStaff,
                ItemKind::CrystalBlade,
                ItemKind::RuneBlade,
                ItemKind::CoralLance,
            ];
            let has_all_weapons = biome_weapons.iter().all(|w| self.inventory.count(*w) > 0);
            if has_all_weapons {
                let cost = TRAVEL_ETHER_COST.saturating_sub(discount).max(10);
                if self.ether < cost {
                    return Err(format!("乙太不足（五大武裝路前往翠幽星需要 {} 乙太）", cost));
                }
            } else {
                let cost = TRAVEL_ETHER_COST_VERDANT_DIRECT.saturating_sub(discount).max(30);
                if self.ether < cost {
                    return Err(format!(
                        "乙太不足（直購航票需 {} 乙太，或集齊五大生態武裝僅需 {} 乙太）",
                        cost, TRAVEL_ETHER_COST
                    ));
                }
            }
            Ok(())
        } else if dest == PLANET_HOME
            && (self.planet == PLANET_VERDANT
                || self.planet == PLANET_CRIMSON
                || self.planet == PLANET_VOID
                || self.planet == PLANET_AETHER
                || self.planet == PLANET_ORIGIN)
        {
            // 翠幽星 / 赤焰星 / 虛空星 / 霧醚星 / 星源星 → 故鄉：只需 30 乙太。
            let cost = TRAVEL_ETHER_COST.saturating_sub(discount).max(10);
            if self.ether < cost {
                return Err(format!("乙太不足（返回故鄉需要 {} 乙太）", cost));
            }
            Ok(())
        } else if dest == PLANET_CRIMSON && self.planet == PLANET_HOME {
            // 故鄉 → 赤焰星：需持有翠幽碎片（證明踏上過翠幽星）+ 50 乙太。
            let cost = TRAVEL_ETHER_COST_CRIMSON.saturating_sub(discount).max(10);
            if self.ether < cost {
                return Err(format!("乙太不足（前往赤焰星需要 {} 乙太）", cost));
            }
            if self.inventory.count(ItemKind::JadeShard) == 0 {
                return Err("需要持有翠幽碎片才能找到赤焰星的星際航道（先探索翠幽星）".into());
            }
            Ok(())
        } else if dest == PLANET_VOID && (self.planet == PLANET_HOME || self.planet == PLANET_CRIMSON) {
            // 故鄉 / 赤焰星 → 虛空星：需持有熔晶碎片（證明踏上過赤焰星）+ 80 乙太。
            let cost = TRAVEL_ETHER_COST_VOID.saturating_sub(discount).max(10);
            if self.ether < cost {
                return Err(format!("乙太不足（前往虛空星需要 {} 乙太）", cost));
            }
            if self.inventory.count(ItemKind::LavaCrystal) == 0 {
                return Err("需要持有熔晶碎片才能找到虛空星的星際航道（先探索赤焰星）".into());
            }
            Ok(())
        } else if dest == PLANET_AETHER
            && (self.planet == PLANET_HOME
                || self.planet == PLANET_CRIMSON
                || self.planet == PLANET_VOID)
        {
            // 故鄉 / 赤焰星 / 虛空星 → 霧醚星：需持有虛空碎片（證明踏上過虛空星）+ 120 乙太。
            let cost = TRAVEL_ETHER_COST_AETHER.saturating_sub(discount).max(10);
            if self.ether < cost {
                return Err(format!("乙太不足（前往霧醚星需要 {} 乙太）", cost));
            }
            if self.inventory.count(ItemKind::VoidShard) == 0 {
                return Err("需要持有虛空碎片才能找到霧醚星的星際航道（先探索虛空星）".into());
            }
            Ok(())
        } else if dest == PLANET_ORIGIN
            && (self.planet == PLANET_HOME
                || self.planet == PLANET_CRIMSON
                || self.planet == PLANET_VOID
                || self.planet == PLANET_AETHER)
        {
            // 故鄉 / 赤焰星 / 虛空星 / 霧醚星 → 星源星：需持有霧醚碎片（證明踏上過霧醚星）+ 150 乙太。
            let cost = TRAVEL_ETHER_COST_ORIGIN.saturating_sub(discount).max(10);
            if self.ether < cost {
                return Err(format!("乙太不足（前往星源星需要 {} 乙太）", cost));
            }
            if self.inventory.count(ItemKind::AetherShard) == 0 {
                return Err("需要持有霧醚碎片才能找到星源星的星際航道（先探索霧醚星）".into());
            }
            Ok(())
        } else {
            Err("未知星球或已在該星球".into())
        }
    }

    /// 依目前輸入意圖，把位置往前推進 `dt` 秒（權威整合，含碰撞解算）。
    /// 抽成純函式以便自動測試。
    ///
    /// `tile_solid(x, y)` 回傳該世界像素座標是否為實心地形格（C-3 碰撞）。
    /// 傳 `|_, _| false` 可關閉 tile 碰撞（向下相容）。
    pub fn step<F: Fn(f32, f32) -> bool>(&mut self, dt: f32, tile_solid: F) {
        // 被打趴（倒地）期間定身：等待復原計時器跑完，不接受任何移動輸入。
        if self.vitals.is_downed() {
            return;
        }
        // 室內模式：不走世界地形，改走室內邊界夾緊移動（ROADMAP 111）。
        if self.indoor_plot_id.is_some() {
            (self.indoor_x, self.indoor_y) = crate::home_interior::indoor_step(
                self.indoor_x,
                self.indoor_y,
                self.input.up,
                self.input.down,
                self.input.left,
                self.input.right,
                dt,
            );
            return;
        }
        // 移動數學整段在 world_core::step_with_keys（對角線正規化、水域阻擋、
        // 實心格四角碰撞、受困逃脫）——前端 wasm 預測呼叫同一份，預測==權威。
        // 速度加點＋跑步皆透過縮放 dt 實現，保持碰撞邏輯不變。
        // 蒸汽載具（Phase 1-E）：騎乘時把步進再放大 `VEHICLE_SPEED_MULT` 倍——移動變快，
        // 碰撞／對角正規化／水域阻擋全部仍走 world_core::step_with_keys 那唯一一份移動數學
        // （不另寫車輛物理）；車與人共用同一套碰撞，過不了牆、下不了水。
        let run_mult = if self.input.run { world_core::RUN_MULT } else { 1.0 };
        // 共乘乘客（ROADMAP 538）不自行推進——其座標由迴圈每拍黏到駕駛；只有「駕駛」才享 3 倍速。
        // 蒸汽衝刺（ROADMAP 539）：駕駛衝刺窗內把巡航 ×3 拉到 ×5（boosting 僅對駕駛有效）。
        let is_driver = self.riding.is_some() && !self.riding_passenger;
        let ride_dt = crate::vehicle::ride_effective_dt(dt, is_driver, is_driver && self.boosting);
        let effective_dt = ride_dt * self.stats.speed_mult() * run_mult;
        (self.x, self.y) = world_core::step_with_keys(
            self.x,
            self.y,
            self.input.up,
            self.input.down,
            self.input.left,
            self.input.right,
            effective_dt,
            tile_solid,
        );
    }

    /// 蒸汽衝刺（ROADMAP 539）：冷卻是否已退、此刻可再次衝刺。
    /// 從未衝過（`boost_trigger` 為 None）一律可衝；否則看距上次觸發是否已過冷卻。
    pub fn boost_ready(&self) -> bool {
        self.boost_trigger
            .map_or(true, |t| crate::vehicle::boost_off_cooldown(t.elapsed().as_secs_f32()))
    }

    /// 蒸汽衝刺：嘗試觸發一次衝刺。冷卻已退才成功（記下觸發時刻、本拍即進加速窗）並回 `true`；
    /// 仍在冷卻回 `false`、不改狀態。接線端（`ws.rs`）負責先確認「此人是駕駛」才呼叫。
    pub fn trigger_boost(&mut self) -> bool {
        if self.boost_ready() {
            self.boost_trigger = Some(std::time::Instant::now());
            self.boosting = true;
            true
        } else {
            false
        }
    }

    /// 蒸汽衝刺：遊戲迴圈每拍呼叫，依 `boost_trigger` 與牆上時鐘刷新本拍 `boosting`
    /// （加速窗內為 true、過了回 false）。把時鐘呼叫收束在此，`step` 維持純粹可測。
    pub fn refresh_boost(&mut self) {
        self.boosting = self
            .boost_trigger
            .map_or(false, |t| crate::vehicle::boost_is_active(t.elapsed().as_secs_f32()));
    }

    /// 蒸汽衝刺：下車／離線時清除衝刺狀態（不讓殘留的衝刺旗標帶到下一次乘騎）。
    pub fn clear_boost(&mut self) {
        self.boost_trigger = None;
        self.boosting = false;
    }

    /// 共乘招呼鈴（ROADMAP 540）：冷卻是否已退、此刻可再次搖鈴。
    /// 從未搖過（`bell_trigger` 為 None）一律可搖；否則看距上次觸發是否已過冷卻。
    pub fn bell_ready(&self) -> bool {
        self.bell_trigger
            .map_or(true, |t| crate::vehicle::bell_off_cooldown(t.elapsed().as_secs_f32()))
    }

    /// 共乘招呼鈴：嘗試搖一次鈴。冷卻已退才成功（記下觸發時刻、本拍即亮信標）並回 `true`；
    /// 仍在冷卻回 `false`、不改狀態。接線端（`ws.rs`）負責先確認「此人是駕駛」才呼叫。
    pub fn ring_bell(&mut self) -> bool {
        if self.bell_ready() {
            self.bell_trigger = Some(std::time::Instant::now());
            true
        } else {
            false
        }
    }

    /// 共乘招呼鈴：本拍信標是否還亮著（由遊戲迴圈讀去同步到所屬車輛的顯示旗標）。
    /// 把時鐘呼叫收束在此，保持 `step` 與快照建構純粹。
    pub fn bell_active(&self) -> bool {
        self.bell_trigger
            .map_or(false, |t| crate::vehicle::bell_is_active(t.elapsed().as_secs_f32()))
    }

    /// 共乘招呼鈴：下車／離線時清掉搖鈴狀態（不讓殘留信標帶到下一次乘騎）。
    pub fn clear_bell(&mut self) {
        self.bell_trigger = None;
    }

    /// 騎乘巡採（ROADMAP 544）：採集臂冷卻是否已退、此刻可順手採一次。
    /// 本次乘騎尚未巡採過（`mount_gather_at` 為 None）一律可採；否則看距上次是否已過冷卻。
    pub fn mount_gather_ready(&self) -> bool {
        self.mount_gather_at
            .map_or(true, |t| crate::vehicle::mount_gather_ready(t.elapsed().as_secs_f32()))
    }

    /// 騎乘巡採：記下「本拍剛順手採到節點」的時刻，重置採集臂冷卻。
    pub fn note_mount_gather(&mut self) {
        self.mount_gather_at = Some(std::time::Instant::now());
    }

    /// 騎乘巡採：下車／離線時清掉巡採冷卻（不讓殘留冷卻帶到下一次乘騎）。
    pub fn clear_mount_gather(&mut self) {
        self.mount_gather_at = None;
    }
}

/// 玩家目前按住的方向鍵（移動意圖）。
#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    /// 跑步旗標：true 時移動速度 ×`world_core::RUN_MULT`（手機推到底／電腦 Shift）。
    pub run: bool,
}

/// 整個應用程式共享的狀態。用 `Arc` 包起來在 handler 間共用。
#[derive(Clone)]
pub struct AppState {
    /// 權威玩家清單。
    pub players: Arc<RwLock<HashMap<Uuid, Player>>>,
    /// 每個玩家自己的農地（Phase 0-G-O1：per-player 擁有）。鍵是擁有者 `user_id`，
    /// 值是那塊地（origin 由其地塊序號決定，互不重疊）。玩家（已登入者）進場時建立、
    /// 之後整個伺服器生命週期都留著——人離線了作物仍在自己的地裡繼續長，重連看得到
    /// 自己離開時的進度（記憶體量級＝歷來已登入玩家數，比照 `plots`／`positions` 有界）。
    /// Phase 0-E：啟動時由 `field_store` 把上次存的地灌回來，重啟後種田進度（翻土/播種/澆水/
    /// 成長）還在。
    pub fields: Arc<RwLock<HashMap<Uuid, Field>>>,
    /// 地塊歸屬登記：哪個玩家擁有第幾塊地（決定其農地 origin、往外排不重疊）。
    /// Phase 0-E：啟動時由 `field_store` 存的序號重建（`from_saved`），returning 玩家拿回原地塊、
    /// 續發序號不撞既有地塊。
    pub plots: PlotRegistry,
    /// 伺服器權威的日夜時鐘（Phase 0-G 療癒核心）。遊戲迴圈每 tick 推進、隨快照廣播。
    /// Phase 0-E：啟動時由 `daynight_store` 把上次存的時刻種回（見 `with_stores`），遊戲迴圈
    /// 再定期把當下時刻 flush 回去，重啟後從同一個時刻接續、不再跳回破曉。
    pub daynight: Arc<RwLock<DayNight>>,
    /// 世界裡共享的採集節點（樹／石／乙太礦,Phase 1-A）。所有玩家從同一組節點採集,
    /// 採空後各自重生。遊戲迴圈每 tick 推進重生、隨快照廣播位置與狀態。目前存記憶體,
    /// 持久化待後續（重啟回到全滿一組）。
    pub nodes: Arc<RwLock<NodeField>>,
    /// 世界裡共享的敵人（戰鬥 1-F：銹蝕巡邏機 / 迷途乙太靈）。遊戲迴圈每 tick 推進重生、
    /// 每秒結算戰鬥(玩家自動打最近的、敵人反擊),隨快照廣播。目前存記憶體,重啟回到全滿一組。
    pub enemies: Arc<RwLock<EnemyField>>,
    /// 廣播頻道：高頻 tick 快照與 `PlayerLeft` 走這裡。
    /// ③ 無限世界（切片 C）：改傳 `Arc<ServerMsg>` 而非已序列化的 JSON，讓連線層
    /// （ws.rs）能依玩家位置做 AOI 剔除後才序列化，避免無限世界爆廣播頻寬。
    pub tx: broadcast::Sender<Arc<ServerMsg>>,
    /// 聊天專用廣播頻道，刻意與高頻快照分開。聊天是「一次性事件」：客戶端漏掉就永久
    /// 看不到那行。先前聊天和快照共用一條，手機 Lagged（網路抖／分頁背景）追快照時
    /// 會把同段時間捲過的聊天一起丟掉——延續「Lagged 不踢人」修復後浮現的缺口。
    /// 分開後聊天量極低、幾乎不可能 Lagged，廣播得以可靠送達。
    pub tx_chat: broadcast::Sender<String>,
    /// 遊戲內建議箱（玩家回饋迴圈的伺服器端）。
    pub suggestions: SuggestionStore,
    /// 使用者帳號(provider 無關)。
    pub users: UserStore,
    /// 玩家最後位置記憶(Phase 0-E,已接 PG):已登入玩家重連回到離線前位置、重啟也撐得過。
    pub positions: PositionStore,
    /// 玩家背包記憶(Phase 0-E):已登入玩家重連帶回採集/打怪/收成囤積的素材。
    pub inventories: InventoryStore,
    /// 玩家農地記憶(Phase 0-E):啟動載回、定期/離線落地整塊地與其序號。權威的 `fields`／`plots`
    /// 由它在 `with_stores` 種回;遊戲迴圈與離線清理再把當下進度寫回它(見 game.rs／ws.rs)。
    pub field_store: FieldStore,
    /// 日夜時刻記憶(Phase 0-E):啟動把上次時刻種給權威 `daynight`、遊戲迴圈定期 flush 回去,
    /// 讓世界時刻撐得過換版重啟(見 game.rs 的 10s flush 區塊)。
    pub daynight_store: DayNightStore,
    /// 每個玩家 id 當前的在線連線數。同帳號多分頁/多裝置共用同一玩家 id,靠這個計數
    /// 讓「先離線的那條連線」不會把另一條還在線的 session 一起從世界移除。
    pub connections: ConnectionCounts,
    /// OAuth 設定;沒設環境變數時為 None,登入相關 API 會回 503。
    pub auth: Option<AuthConfig>,
    /// 公共農地：任何已登入玩家均可種植與收割。owner = Uuid::nil() 廣播給前端，
    /// 與個人地塊（owner = user_id）視覺上明顯區分，並允許多人互動（軟劫掠）。
    pub pub_field: Arc<RwLock<Field>>,
    /// 玩家對玩家的世界市場掛單（記憶體，v1；重啟後清空）。
    pub market: Arc<RwLock<Market>>,
    /// 地形格世界（delta-save）：記憶體前置、非同步落地到 `TileStore`。
    /// C-1 只有確定性生成（deltas 為空）；C-2 起 Dig 時寫入 deltas。
    pub tile_world: Arc<RwLock<TileWorld>>,
    /// 地形差異的持久化 store：啟動時載回、C-2 挖掘時非同步落地。
    pub tile_store: TileStore,
    /// 宇宙裂縫世界事件狀態（ROADMAP 26）。遊戲迴圈每 tick 推進；
    /// 觸發時注入裂縫守護者並廣播聊天公告，前端據此渲染裂縫視覺效果。
    pub world_event: Arc<RwLock<WorldEvent>>,
    /// 全服社群探索任務（ROADMAP 27）。遊戲迴圈每 tick 推進計時，
    /// kill/gather/travel 事件推進進度；完成時全員分潤乙太並廣播公告。
    pub quests: Arc<RwLock<QuestState>>,
    /// 公會管理器（ROADMAP 29 + 113）：Postgres 模式下跨重啟保留；記憶體模式重啟清空。
    /// 玩家建立 / 加入 / 離開 / 捐贈公會由 ws.rs 操作此 store。
    pub guilds: GuildStore,
    /// 每位玩家的每日任務狀態（ROADMAP 32）：記憶體前置，重啟從當天重新生成。
    /// key = 玩家 Uuid（已登入玩家才有每日任務）。
    pub daily_quests: Arc<RwLock<HashMap<Uuid, PlayerDailyState>>>,
    /// 城外產權地塊（ROADMAP 34）：20 塊預定義地塊的地主歸屬。
    /// 購買後鎖住該地塊的 Dig/Place 只讓地主操作。
    pub land_plots: Arc<RwLock<LandPlotRegistry>>,
    /// 農田地塊牧場狀態（ROADMAP 48）：記憶體模式，重啟歸零。
    pub ranch: Arc<RwLock<RanchRegistry>>,
    /// 養蜂釀蜜（ROADMAP 412）：按 owner 鍵的蜂巢狀態，蜜蜂採自家田裡作物花蜜釀蜜。
    /// 記憶體模式，重啟歸零；蜂蜜本身入持久化背包，不丟。
    pub apiary: Arc<RwLock<crate::apiary::ApiaryRegistry>>,
    /// 農田地塊作物狀態（ROADMAP 49）：記憶體模式，重啟歸零。
    pub farm_crops: Arc<RwLock<FarmCropRegistry>>,
    /// 夜採星晶礦脈（ROADMAP 50）：夜間生成、白天清除；記憶體模式。
    pub star_crystals: Arc<RwLock<StarCrystalField>>,
    /// 觀星夜數（ROADMAP 347）：`game.rs` 每進入一次夜晚就 +1，決定「今夜星座」逐夜輪替。
    /// 記憶體模式、重啟歸零（從第 0 座重新開始輪替）；lock-free 原子，伺服器各處可無鎖讀取。
    pub night_index: Arc<AtomicU64>,
    /// 地塊產權持久化 store：啟動時載回、購買時 fire-and-forget upsert。
    pub land_plot_store: LandPlotStore,
    /// NPC 浮動收購價市場（ROADMAP 40）：記憶體前置，重啟後商人回基準價。
    /// 賣越多收購價越低（地板 30%）；每小時自動回升 5%。
    pub dynamic_prices: Arc<RwLock<DynamicPriceMarket>>,
    /// AI 導演層：獸潮攻城計時與狀態機（ROADMAP 44）。
    /// 遊戲迴圈每 tick 推進；觸發時注入怪波並廣播公告。
    pub director: Arc<RwLock<crate::director::DirectorState>>,
    /// 會動腦的 NPC 對每位玩家的「印象」（個人記憶，記憶體 v1）：(玩家 id, NPC id) → 一句話印象。
    /// 隔離設計：A 對某 NPC 的印象只影響 NPC 對 A 的口吻（見 npc_chat.rs / 願景文件）。
    pub npc_memory: Arc<RwLock<HashMap<(Uuid, String), crate::npc_chat::NpcRel>>>,
    /// 每個 NPC「自己有限的餘裕」（npc id → 還能送出的小禮份數）。約束＝真實稀缺：
    /// NPC 的好意來自他實際擁有的東西，送完就沒了（不能無中生有）。見 npc_chat.rs。
    pub npc_gift_stock: Arc<RwLock<HashMap<String, u32>>>,
    /// LLM 並發信號量（全域上限 MAX_CONCURRENT_LLM）。超限等待 2 秒 → 逾時回罐頭句。
    pub npc_llm_sem: Arc<Semaphore>,
    /// 自主 agent 決策匯流排（P0 live 接線）：少數居民每 ~15 秒在主迴圈短鎖快照 → spawn 思考 →
    /// 思考 task 把決策投進這裡 → 主迴圈下一 tick 同步套用。兩把內部鎖都絕不跨 await 持有。
    pub agent_bus: Arc<crate::npc_agent_wire::AgentBus>,
    /// 玩家對各 NPC 的上次對話時間（(player_id, npc_id) → Instant）。
    /// 同一玩家對同一 NPC 需間隔 PER_PLAYER_NPC_COOLDOWN_SECS 秒。
    pub npc_last_chat: Arc<RwLock<HashMap<(Uuid, String), Instant>>>,
    /// NPC 個人記憶 + 送禮餘裕持久化 store（ROADMAP 60）。
    /// Postgres 模式下，對話後 fire-and-forget upsert；記憶體模式重啟歸零。
    pub npc_memory_store: NpcMemoryStore,
    /// 商人熟客折扣待用票（ROADMAP 63）：玩家 id → (折扣百分比, 到期時刻)。
    /// 商人自主送出後存入；下次 ShopBuy 套用一次後清除；到期自動失效。
    /// 記憶體模式，重啟清空（折扣本就限時，重啟等同過期，行為正確）。
    pub npc_pending_discount: Arc<RwLock<HashMap<Uuid, (u32, Instant)>>>,
    /// 村落金庫餘額（乙太，ROADMAP 64）。玩家捐獻後增加；里長辦活動後扣減。
    /// 記憶體模式（重啟從 INITIAL_TREASURY 重設，行為正確——金庫代表「當前活躍信任」）。
    pub village_treasury: Arc<RwLock<u32>>,
    /// 村落節慶加成的到期時刻（ROADMAP 64）。`None` = 無活躍加成；`Some(t)` = t 前全服 EXP +30%。
    /// 記憶體模式（重啟清空，加成重來，行為正確）。
    pub village_buff_until: Arc<RwLock<Option<Instant>>>,
    /// 引擎世界事件公共記憶（ROADMAP 65）：NPC 對話可自然提及近期大事。
    /// **只有引擎能寫**（玩家文字永遠進不來）；記憶體模式，重啟清空（近況快訊不需持久化）。
    pub world_log: Arc<RwLock<crate::world_log::WorldLog>>,
    /// 玩家個人事跡日誌（ROADMAP 67）：每位玩家完成任務/訂單/探勘等，引擎記一條。
    /// NPC 對話時讀取當前玩家的日誌，讓 NPC 可自然提及「你在整個村子的事跡」。
    /// 記憶體模式，重啟清空（近況日誌，不需持久化）。
    pub player_logs: Arc<RwLock<HashMap<Uuid, crate::player_log::PlayerLog>>>,
    /// NPC 生命週期管理器（ROADMAP 66）：每個 NPC 有壽命計時器，老年期注入語境，
    /// 壽命到時觸發「回歸乙太」→ 繼承人接手。記憶體模式，重啟歸零（「世界換季」）。
    pub npc_lifecycle: Arc<RwLock<crate::npc_lifecycle::NpcLifecycle>>,
    /// NPC 主動評論冷卻追蹤器（ROADMAP 68）：世界事件發生時 NPC 主動在聊天頻道表達看法；
    /// 每個 NPC 每 10 分鐘最多主動說一次。記憶體模式，重啟清空。
    pub npc_proactive: Arc<RwLock<crate::npc_proactive::NpcProactiveCooldowns>>,
    /// NPC 需求驅力狀態（ROADMAP 69）：每個 NPC 有安全感/歸屬感/繁榮感三個需求值（0~100）；
    /// 世界事件影響數值，對話時注入 system prompt 讓 NPC 語氣自然反映情緒狀態。
    /// 記憶體模式，重啟清零（世界換季，NPC 重新出發）。
    pub npc_needs: Arc<RwLock<crate::npc_needs::NpcNeedsState>>,
    /// NPC 人際關係網（ROADMAP 70）：每對 NPC 之間有好惡值（0~100），
    /// 世界事件影響關係（共患難加深信任），對話時注入 system prompt 讓 NPC 談到彼此時語氣自然。
    /// 記憶體模式，重啟清零。
    pub npc_relations: Arc<RwLock<crate::npc_relations::NpcRelationsState>>,
    /// NPC 派系自主湧現（ROADMAP 71）：追蹤已公開的結盟/競爭對，偵測派系事件並廣播到聊天頻道；
    /// 對話時注入 system prompt 讓 NPC 自然流露對盟友/對手的口吻。記憶體模式，重啟清零。
    pub npc_factions: Arc<RwLock<crate::npc_factions::NpcFactionState>>,
    /// NPC 社交平衡漣漪（ROADMAP 365）：關係網依社會平衡理論自我演化（朋友的朋友更親、
    /// 朋友的敵人漸疏），並對「人情漸染」的回暖漣漪管理每對廣播冷卻。記憶體模式，重啟清零。
    pub social_dynamics: Arc<RwLock<crate::social_dynamics::SocialDynamicsState>>,
    /// 鎮民派系成形（ROADMAP 366）：把當前關係網上連通成盟的三人以上居民認作「陣營」，
    /// 並對新成形的陣營管理廣播冷卻。記憶體模式，重啟清零（陣營從當下關係值重新湧現）。
    pub town_blocs: Arc<RwLock<crate::town_blocs::TownBlocState>>,
    /// 鎮民互助分享（ROADMAP 369）：寬裕的居民依「繁榮感」需求自發勻一份心意給拮据的居民；
    /// 持有防反覆刷頻的 last_pair 與進行中的送禮手勢計時。記憶體模式，重啟清零。
    pub town_share: Arc<RwLock<crate::town_share::TownShareState>>,
    /// 玩家親手種下、隨真實時間長大的世界樹群（ROADMAP 370）。全服共享，記憶體模式、重啟清零。
    pub world_grove: Arc<RwLock<crate::world_grove::WorldGrove>>,
    /// 世界冒險日報（ROADMAP 385）：追蹤今日精彩、黎明時廣播回顧。純記憶體、重啟清零。
    pub daily_recap: Arc<RwLock<crate::daily_recap::DailyHighlights>>,
    /// 合成儀式世界首次追蹤（ROADMAP 388）：記憶體模式，重啟清零。
    pub craft_ceremony: Arc<RwLock<crate::craft_ceremony::CraftCeremonyState>>,
    /// 稱號系統世界首次追蹤（ROADMAP 389）：wire key → 首位解鎖者名稱。記憶體模式，重啟清零。
    pub world_title_first: Arc<RwLock<std::collections::HashMap<String, String>>>,
    /// NPC 作息與移動管理器（ROADMAP 73）。
    pub npc_schedule: Arc<RwLock<crate::npc_schedule::NpcScheduleManager>>,
    /// 城外旅人 NPC（ROADMAP 74）：每 15 分鐘到訪一次，純記憶體模式，重啟清零。
    pub traveler: Arc<RwLock<crate::traveler_npc::TravelerNpc>>,
    /// 路人 / 居民 NPC 群體（ROADMAP 115）：純模板驅動（零 LLM），人口隨繁榮感自然增減。
    /// 記憶體模式，重啟清零（從最小人口重啟）。
    pub residents: Arc<RwLock<crate::resident_npc::ResidentManager>>,
    /// 居民廣場聚會狀態（ROADMAP 124）：每 30 分鐘白天觸發一次聚會，持續 10 分鐘，全服 EXP +20%。
    /// 純邏輯、記憶體模式，重啟清零。
    pub community_gathering: Arc<RwLock<crate::community_gathering::CommunityGatheringState>>,
    /// 人氣聚會狀態（ROADMAP 342）：追蹤各高人氣玩家身邊湧現的聚會生命週期（含散場緩衝），
    /// 純記憶體模式，重啟清零。與 124 廣場聚會（定時、全城）不同——這是繞著「受歡迎的玩家」湧現的社交節點。
    pub popularity_gathering: Arc<RwLock<crate::popularity_gathering::GatheringState>>,
    /// 怪物王咆哮狀態（ROADMAP 75）：追蹤各菁英精英的咆哮冷卻，純記憶體模式，重啟清零。
    pub boss_roar: Arc<RwLock<crate::boss_roar::BossRoarState>>,
    /// 怪物王咆哮專屬 Semaphore（容量 1）：同時最多一個 AI 咆哮呼叫，不佔用 NPC LLM 配額。
    pub boss_roar_sem: Arc<Semaphore>,
    /// 怪物王戰術指揮狀態（ROADMAP 117）：追蹤各菁英精英的戰術決策冷卻，純記憶體模式，重啟清零。
    pub boss_ai: Arc<RwLock<crate::boss_ai::BossAiState>>,
    /// 怪物王戰術指揮專屬 Semaphore（容量 1）：同時最多一個 AI 台詞呼叫。
    pub boss_ai_sem: Arc<Semaphore>,
    /// 廣場夜談狀態（ROADMAP 76）：夜間 NPC 閒聊冷卻，純記憶體模式，重啟清零。
    pub plaza_talk: Arc<RwLock<crate::plaza_talk::PlazaTalkState>>,
    /// 廣場夜談專屬 Semaphore（容量 1）：同時最多一個 AI 閒聊呼叫，不佔用 NPC LLM 配額。
    pub plaza_talk_sem: Arc<Semaphore>,
    /// 晨喚狀態（ROADMAP 77）：偵測黎明轉換並觸發凱爾長老致辭，純記憶體模式，重啟清零。
    pub dawn_call: Arc<RwLock<crate::npc_dawn_call::DawnCallState>>,
    /// 晨喚專屬 Semaphore（容量 1）：同時最多一個 AI 晨喚呼叫，不佔用 NPC LLM 配額。
    pub dawn_call_sem: Arc<Semaphore>,
    /// 暮告狀態（ROADMAP 78）：偵測白天→黃昏轉換並觸發商人薇拉傍晚感言，純記憶體模式，重啟清零。
    pub dusk_call: Arc<RwLock<crate::npc_dusk_call::DuskCallState>>,
    /// 暮告專屬 Semaphore（容量 1）：同時最多一個 AI 暮告呼叫，不佔用 NPC LLM 配額。
    pub dusk_call_sem: Arc<Semaphore>,
    /// 午鐘廣播狀態（ROADMAP 79）：偵測黎明→白天轉換並觸發工匠老胡開工令，純記憶體模式，重啟清零。
    pub noon_bell: Arc<RwLock<crate::npc_noon_bell::NoonBellState>>,
    /// 午鐘廣播專屬 Semaphore（容量 1）：同時最多一個 AI 午鐘呼叫，不佔用 NPC LLM 配額。
    pub noon_bell_sem: Arc<Semaphore>,
    /// 入夜守衛令狀態（ROADMAP 80）：偵測黃昏→夜晚轉換並觸發獵手蘭卡守衛令，純記憶體模式，重啟清零。
    pub night_watch: Arc<RwLock<crate::npc_night_watch::NightWatchState>>,
    /// 入夜守衛令專屬 Semaphore（容量 1）：同時最多一個 AI 守衛令呼叫，不佔用 NPC LLM 配額。
    pub night_watch_sem: Arc<Semaphore>,
    /// 白日工位對話狀態（ROADMAP 81）：白天 NPC 在各自崗位輪流閒聊，純記憶體模式，重啟清零。
    pub daytime_talk: Arc<RwLock<crate::daytime_talk::DaytimeTalkState>>,
    /// 白日工位對話專屬 Semaphore（容量 1）：同時最多一個 AI 對話呼叫，不佔用 NPC LLM 配額。
    pub daytime_talk_sem: Arc<Semaphore>,
    /// 午休席間閒話狀態（ROADMAP 328）：正午圍桌共食時 NPC 輪流冒出家常閒話泡泡，
    /// 純模板零 LLM、純記憶體模式，重啟清零。
    pub lunch_chatter: Arc<RwLock<crate::lunch_chatter::LunchChatterState>>,
    /// 餐桌熟客帳本（ROADMAP 330）：玩家↔NPC 相熟度（席間舉杯累積），
    /// 純記憶體模式重啟清零、零 LLM、碰不到遊戲狀態，與 npc_relations〔NPC↔NPC〕獨立。
    pub lunch_regulars: Arc<RwLock<crate::lunch_regular::RegularBook>>,
    /// 街坊相認冷卻帳本（ROADMAP 331）：白天崗位上，七大 NPC 認出走近的熟客玩家（相熟度 ≥ 點頭之交）
    /// 點名招呼一句；per-(玩家,NPC) 冷卻避免連珠炮。純記憶體模式重啟清零、零 LLM、碰不到遊戲狀態。
    pub npc_recognition: Arc<RwLock<crate::npc_recognition::RecognitionBook>>,
    /// 鎮民認得你的夥伴冷卻帳本（ROADMAP 359）：帶寵物走近在崗 NPC 時，NPC 順著寵物個性（358）
    /// 搭一句就地評論；per-(玩家,NPC) 冷卻避免連珠炮。純記憶體模式重啟清零、零 LLM、碰不到遊戲狀態。
    pub pet_greeting: Arc<RwLock<crate::pet_greeting::GreetBook>>,
    /// NPC 自主懸賞系統（ROADMAP 82）：蘭卡根據安全感發布通緝令，討伐者得獎。
    pub npc_bounty: Arc<RwLock<crate::npc_bounty::NpcBountyState>>,
    /// NPC 落敗反應系統（ROADMAP 83）：玩家倒地時 NPC 廣播慰問 / 警示，純記憶體模式，重啟清零。
    pub npc_defeat_reaction: Arc<RwLock<crate::npc_defeat_reaction::NpcDefeatReactionState>>,
    /// 落敗反應專屬 Semaphore（容量 1）：同時最多一個 AI 反應呼叫。
    pub npc_defeat_reaction_sem: Arc<Semaphore>,
    /// NPC 升等賀詞系統（ROADMAP 84）：玩家升等時凱爾長老私信賀詞、里程碑全服廣播，純記憶體模式。
    pub npc_level_greet: Arc<RwLock<crate::npc_level_greet::NpcLevelGreetState>>,
    /// NPC 主動資材委託系統（ROADMAP 85）：繁榮感低時商人薇拉發急收令、玩家賣出加價，純記憶體模式。
    pub npc_commission: Arc<RwLock<crate::npc_commission::NpcCommissionState>>,
    /// NPC 探勘加碼令系統（ROADMAP 86）：安全感高時芙利亞宣告加碼、探勘採樣額外得獎，純記憶體模式。
    pub npc_expedition_boost: Arc<RwLock<crate::npc_expedition_boost::NpcExpeditionBoostState>>,
    /// NPC 工坊加成令系統（ROADMAP 87）：歸屬感高時老胡宣告急修加成、工坊訂單完成額外得獎，純記憶體模式。
    pub npc_workshop_boost: Arc<RwLock<crate::npc_workshop_boost::NpcWorkshopBoostState>>,
    /// 天氣系統（ROADMAP 93）：目前天氣類型與粒子強度，每 8 分鐘輪換一次，切換時廣播聊天公告。
    /// 對應生態域採集時給 +1 加成；記憶體模式，重啟從晴天開始（天氣不需持久化）。
    pub weather: Arc<RwLock<crate::weather::WeatherState>>,
    /// 雨後彩虹（ROADMAP 361）：草原細雨停下、天還亮著的瞬間架起的全服共享天象。
    /// 彩虹高掛期間，全服存活玩家每隔數秒同享一次溫和的「🌈 彩虹祝福」緩回血（療癒向，
    /// 非採集節點）。伺服器權威、全服同步，前端據此畫彩虹弧並顯示祝福 HUD pill。
    /// 純記憶體模式，重啟清零（天象不需持久化）。
    pub rainbow: Arc<RwLock<crate::rainbow::RainbowState>>,
    /// 每條線上連線的直達單播通道（ROADMAP 95 私聊密語）：player_id → tx_direct。
    /// 連線建立時插入、離線時移除；讓密語可直達目標而不廣播全服。
    pub whisper_senders: Arc<RwLock<HashMap<Uuid, tokio::sync::mpsc::Sender<String>>>>,
    /// 好友關係持久化 store（ROADMAP 96）：單向 follow；Postgres 模式下跨重啟保留。
    pub friends: FriendStore,
    /// 臨時隊伍管理器（ROADMAP 97）：純記憶體、重啟清空（臨時隊伍不需持久化）。
    pub parties: crate::party::PartyStore,
    /// 灑水器記憶體快照（ROADMAP 112）：放置時加入、tick 時澆水。
    pub sprinklers: Arc<RwLock<crate::sprinkler::SprinklerStore>>,
    /// 灑水器持久化 store（ROADMAP 112）：INSERT 到 DB + 啟動 SELECT 全載。
    pub sprinkler_persist: crate::sprinkler::SprinklerPersist,
    /// 商人有限金庫（ROADMAP 100）：收購從金庫付，商隊定期回補，終結無限印鈔。
    /// 記憶體模式，重啟從初始值開始（金庫代表「當前商隊現金」，重啟等同換班補貨）。
    pub npc_treasury: Arc<RwLock<crate::npc_treasury::NpcTreasuryState>>,
    /// AI 議價待確認交易（ROADMAP 101）：玩家 id → PendingDeal（商人提議，玩家確認後才執行）。
    /// 每人只能有一筆待確認議價（新的覆蓋舊的）；超過 DEAL_EXPIRE_SECS 秒後自動失效。
    /// 記憶體模式，重啟清空（議價本就限時，重啟等同過期，行為正確）。
    pub npc_pending_deal: Arc<RwLock<HashMap<Uuid, crate::npc_deal::PendingDeal>>>,
    /// NPC 販售庫存（ROADMAP 104）：賣商有庫存上限、賣完缺貨、定期補貨。
    /// 記憶體模式（重啟從初始滿庫開始——相當於商隊補完貨才開門）。
    pub npc_stock: Arc<RwLock<crate::npc_stock::NpcStockState>>,
    /// 城鎮大工程狀態（ROADMAP 131）。
    pub town_project: Arc<RwLock<crate::town_project::TownProjectState>>,
    /// 城鎮大工程持久化 store。
    pub town_project_store: crate::town_project_store::TownProjectStore,
    /// 連日歸鄉·歸鄉印記持久化 store（ROADMAP 397）：keyed by user_id，跨重啟保留留存印記。
    pub visit_streaks: crate::visit_streak_store::VisitStreakStore,
    /// 新手見面禮持久化 store（ROADMAP 444）：keyed by user_id，記某帳號有沒有領過一次性起手禮。
    pub welcome_kits: crate::welcome_kit_store::WelcomeKitStore,
    /// 天文台星象預報狀態（ROADMAP 132）：天文台竣工後每個黎明廣播星象、啟用全服加成。
    /// 純記憶體模式，重啟清零；不破壞玩家資料。
    pub observatory: Arc<RwLock<crate::observatory::ObservatoryState>>,
    /// 星象預報專屬 Semaphore（容量 1）：同時最多一個 AI 星象生成呼叫。
    pub observatory_sem: Arc<Semaphore>,
    /// 流星雨狀態（ROADMAP 133）：天文台竣工後每 30 分鐘觸發流星雨，地面出現星塵採集點。
    /// 純記憶體模式，重啟清零；不破壞玩家資料。
    pub meteor_shower: Arc<RwLock<crate::meteor_shower::MeteorShowerState>>,
    /// 野營篝火（ROADMAP 474）：玩家在荒野升起的篝火，火光暖意把附近野獸逼退。
    /// 純記憶體模式，重啟清零；不碰玩家資料與經濟。
    pub campfires: Arc<RwLock<crate::campfire::CampfireField>>,
    /// 協力共建·邊境瞭望塔（ROADMAP 546）：邊境工地由多人合力建造，落成後永久壓制周圍野獸。
    /// 純記憶體模式，重啟回到未完工的工地；不碰玩家資料與經濟。
    pub watchtowers: Arc<RwLock<crate::coop_build::CoopBuildField>>,
    /// 蒸汽載具場（Phase 1-E 北極星「載具」MVP）：故鄉草原上可乘騎的蒸汽腳踏車。
    /// 純記憶體模式，重啟回到初始車況（車回原位、無人乘騎）；不碰玩家資料與經濟。
    pub vehicles: Arc<RwLock<crate::vehicle::VehicleField>>,
    /// 廢棄蒸汽星艦共修（ROADMAP 492）：多位旅人共同貢獻木材修繕世界東北方的墜落星艦。
    /// 純記憶體模式，重啟清零；不碰玩家資料與持久化。
    pub ship_repair: Arc<RwLock<crate::ship_repair::ShipRepairState>>,
    /// 雪季堆雪人（ROADMAP 478）：隆冬時玩家堆起的署名雪人，全服可見、天回暖即融化。
    /// 純記憶體模式，重啟清零；不碰玩家資料與經濟。
    pub snowmen: Arc<RwLock<crate::snowman::SnowmanField>>,
    /// 夜間乙太泉（ROADMAP 162）：每當日夜轉入夜晚時在城外生成 5 個乙太泉採集點。
    /// 玩家靠近採集得 +8 乙太；天亮自動清除。純記憶體模式，重啟清零。
    pub night_springs: Arc<RwLock<crate::night_aether_springs::NightAetherSprings>>,
    /// 夜螢提燈（ROADMAP 477）：每當日夜轉入夜晚時在城外生成 6 群螢火蟲，玩家走近輕捕入提燈，
    /// 提燈柔光全服可見；天亮自動清除螢群並清空各玩家提燈。純記憶體模式，重啟清零、零經濟。
    pub firefly_lantern: Arc<RwLock<crate::firefly_lantern::FireflyLantern>>,
    /// 乙太暴走事件（ROADMAP 504）：每約 20 分鐘，地圖某個採集點爆發高濃度乙太，
    /// 玩家在有效半徑內採集可額外獲得獎勵；持續 90 秒後消退。純記憶體，重啟清零。
    pub ether_surge: Arc<RwLock<crate::ether_surge::EtherSurge>>,
    /// 旅行商人狀態（ROADMAP 135）：每 2 小時來訪，停留 10 分鐘，限時出售稀有物品。
    /// 純記憶體模式，重啟清零；不破壞玩家資料。
    pub wandering_merchant: Arc<RwLock<crate::wandering_merchant::WanderingMerchantState>>,
    /// 季節循環（ROADMAP 137）：春夏秋冬各 20 分鐘輪替，影響作物成長速度。
    /// 記憶體模式，重啟從春天開始（世界換季，行為合理）。
    pub season: Arc<RwLock<crate::season::SeasonState>>,
    /// 季節性野外採集節點（ROADMAP 154）：每季節在城外 3 個固定節點，各 3 次共用採集次數。
    /// 記憶體模式，季節切換自動重置；重啟從當前季節重新生成。
    pub seasonal_nodes: Arc<RwLock<crate::seasonal_nodes::SeasonalNodesState>>,
    /// 本季全服「旺收」累計次數（ROADMAP 493 季節豐收獎）。
    /// 當玩家在品種旺季收穫時遞增；季節切換時歸零，讓每季里程碑獨立計算。
    /// 記憶體模式；零 migration、不持久化（重啟清零，行為正確）。
    pub season_peak_harvest_count: Arc<RwLock<u32>>,
    /// 探索者路標（ROADMAP 353）：玩家在世界裡立的留言路標，非同步的玩家↔玩家痕跡。
    /// 記憶體模式、有界、會過期；遊戲迴圈每 tick 推進過期、立牌/過期時廣播 `ServerMsg::Wayposts`。
    pub wayposts: Arc<RwLock<crate::wayposts::WaypostBoard>>,
    /// 星海寄語 / 漂流瓶（ROADMAP 354）：玩家拋向星海、漂到陌生旅人腳邊的留言瓶，
    /// 第二條非同步玩家↔玩家互動（點對點、可回贈，與路標的定點廣播換骨架）。
    /// 記憶體模式、有界、會過期；遊戲迴圈每 tick 推進過期、海上數量變動時廣播 `ServerMsg::BottleSeaCount`。
    pub bottles: Arc<RwLock<crate::bottle_drift::BottleSea>>,
    /// 中立野生動物（ROADMAP 140）：野鳥/野鹿/小動物。
    /// 記憶體模式，重啟重新在固定座標生成。
    pub wildlife_manager: Arc<RwLock<crate::wildlife::WildlifeManager>>,
    /// 人類↔物種關係（ROADMAP 144）：各物種對人類的態度值。
    /// 記憶體模式，重啟清零（世界換季重生）。
    pub species_relations: Arc<RwLock<crate::species_relations::SpeciesRelations>>,
    /// 怪物=物種關係（ROADMAP 163）：各怪物種類對人類的集體態度值。
    /// 玩家殺怪→態度+（怪物學會敬畏）；怪物擊倒玩家→態度-（怪物更囂張）。
    /// 態度層級影響 aggro 半徑；記憶體模式，重啟清零。
    pub monster_species: Arc<RwLock<crate::species_relations::MonsterSpeciesRelations>>,
    /// 怪物巢穴=聚落（ROADMAP 164）：5 個固定巢穴，怪物從此出生/回巢；
    /// 族群可清剿衰退或放著壯大。記憶體模式，重啟全重置。
    pub monster_colonies: Arc<RwLock<crate::monster_colony::MonsterColonyManager>>,
    /// 玩家住家家具（ROADMAP 155）：每位玩家（UUID）所放置的家具列表。
    /// 記憶體模式，重啟清空（玩家需重新進室內放置）；家具材料在背包持久化。
    pub home_furnishings: Arc<RwLock<std::collections::HashMap<uuid::Uuid, crate::home_furniture::HomeFurnishings>>>,
    /// 公民投票（ROADMAP 156）：居民代言人定期提案，玩家投票決定城鎮短期效果。
    /// 記憶體模式，重啟清零；不破壞玩家資料。
    pub civic_vote: Arc<RwLock<crate::civic_vote::CivicVoteState>>,
    /// 城鎮記憶石（ROADMAP 157）：記錄世界大事（守城勝敗、提案、季節、大工程等），
    /// 玩家靠近記憶石可讀取。純記憶體，重啟清零，不破壞玩家資料。
    pub town_memory: Arc<RwLock<crate::town_memory::TownMemory>>,
    /// 城鎮入侵警報（ROADMAP 158）：每 90 分鐘怪物大舉入侵城鎮外圍，玩家攜手抵禦。
    /// 純記憶體，重啟清零，不破壞玩家資料。
    pub invasion: Arc<RwLock<crate::invasion::InvasionState>>,
    /// 生態清剿委託（ROADMAP 172）：生態壓力超標時自動發布全服清剿委託，完成後在線玩家分潤乙太。
    /// 純記憶體，重啟清零，不破壞玩家資料。
    pub eco_bounty: Arc<RwLock<crate::eco_bounty::EcoBountyState>>,
    /// 生態豐收節（ROADMAP 178）：壓力曾衝上危機後被壓回安寧時自動開節、發獎、亮慶典橫幅。
    /// 純記憶體，重啟清零，不破壞玩家資料。
    pub eco_festival: Arc<RwLock<crate::eco_festival::EcoFestivalState>>,
    /// 今日世界戰報（ROADMAP 495）：全伺服器自啟動起的採集/收穫/擊殺/登入累計。
    /// 純記憶體，重啟清零，零 migration，零持久化。
    pub world_tally: Arc<RwLock<crate::world_tally::WorldTally>>,
    /// 戰鬥記跡（ROADMAP 499）：最近 20 筆、5 分鐘內的擊殺地點記號，全服廣播顯示。
    /// 純記憶體，重啟清零，零 migration，零持久化。
    pub combat_marks: Arc<RwLock<crate::combat_mark::CombatMarkState>>,
    /// 黃金礦脈爭奪戰（ROADMAP 521）：每 30 分鐘週期性競技採礦事件（首次 10 分鐘後）。
    /// 純記憶體，重啟清零，零 migration，零持久化。
    pub gold_rush: Arc<RwLock<crate::gold_rush::GoldRushState>>,
    /// 星際拍賣行（ROADMAP 522）：每 2 小時全服競標一件傳說遺物（首次 5 分鐘後）。
    /// 純記憶體，重啟清零，零 migration，零持久化。
    pub auction: Arc<RwLock<crate::auction::AuctionState>>,
    /// 萬尾釣魚大賽（ROADMAP 523）：每 45 分鐘全服釣魚競速，前三名得乙太。
    /// 純記憶體，重啟清零，零 migration，零持久化。
    pub fishing_contest: Arc<RwLock<crate::fishing_contest::FishingContest>>,
    /// 世界奇觀首探（ROADMAP 524）：五處隱藏秘境，首位踏入者留名並得乙太。
    /// 純記憶體，重啟後玩家可重新探索（探索是遊戲體驗，不需持久化）。
    pub wonders: Arc<RwLock<crate::world_wonder::WorldWonderState>>,
    /// 世界守護者（ROADMAP 525）：周期現身的超強 BOSS，玩家協力擊敗並獲得乙太獎勵。
    pub world_boss: Arc<RwLock<crate::world_boss::WorldBossState>>,
    /// 旅人紀念碑（ROADMAP 526）：銘記守護者首殺、奇觀首探、釣魚冠軍、礦脈冠軍等首批成就。
    /// 純記憶體，重啟清零，零 migration，零持久化。
    pub monument: Arc<RwLock<crate::monument::Monument>>,
    /// 守護者元素祝福（ROADMAP 533）：擊敗守護者的參戰玩家獲得元素光環，持續 2 小時。
    /// 純記憶體，重啟清零，零 migration，零持久化。
    pub guardian_blessings: Arc<RwLock<crate::guardian_blessing::GuardianBlessingStore>>,
}

impl AppState {
    /// 無 DB 模式（測試、本機 `cargo run`）：位置/背包/農地/日夜/地形走記憶體退回層。
    pub fn new() -> Self {
        Self::with_stores(
            PositionStore::new(),
            InventoryStore::new(),
            FieldStore::new(),
            DayNightStore::new(),
            UserStore::new(),
            SuggestionStore::new(),
            TileStore::new(),
            LandPlotStore::new(),
            NpcMemoryStore::new(),
            FriendStore::new(),
            GuildStore::new(),
            crate::sprinkler::SprinklerPersist::new(),
            vec![],
            crate::town_project_store::TownProjectStore::new(),
            crate::visit_streak_store::VisitStreakStore::new(),
            crate::welcome_kit_store::WelcomeKitStore::new(),
        )
    }

    /// 用已備好的位置 / 背包 / 農地 / 日夜 / 帳號 / 建議 / 地形 store 建狀態。`main` 連好 Postgres 後
    /// 會傳入 DB-backed 的 store（見各自的 `from_pool`），其餘狀態不變。農地 store 同時種回兩份
    /// 權威狀態：`fields`（每塊地）與 `plots`（序號歸屬）；日夜 store 種回上次的世界時刻；
    /// tile_store 把上次存的地形差異種回 `tile_world`。
    pub fn with_stores(
        positions: PositionStore,
        inventories: InventoryStore,
        field_store: FieldStore,
        daynight_store: DayNightStore,
        users: UserStore,
        suggestions: SuggestionStore,
        tile_store: TileStore,
        land_plot_store: LandPlotStore,
        npc_memory_store: NpcMemoryStore,
        friends: FriendStore,
        guilds: GuildStore,
        sprinkler_persist: crate::sprinkler::SprinklerPersist,
        sprinkler_preload: Vec<(uuid::Uuid, crate::sprinkler::SprinklerData)>,
        town_project_store: crate::town_project_store::TownProjectStore,
        visit_streaks: crate::visit_streak_store::VisitStreakStore,
        welcome_kits: crate::welcome_kit_store::WelcomeKitStore,
    ) -> Self {
        let (tx, _rx) = broadcast::channel(256);
        // 聊天頻道：量極低、給足緩衝，正常使用幾乎不會 Lagged。
        let (tx_chat, _rx_chat) = broadcast::channel(256);
        // 啟動時把上次存的農地與地塊歸屬種回權威狀態（無存檔時等同全新的空 map / next=0）。
        let plots = PlotRegistry::from_saved(field_store.saved_plots());
        // 城外產權地塊：從持久化載入歸屬（無 DB 時重啟後清空，行為正確）。
        let land_plot_registry = LandPlotRegistry::from_saved(land_plot_store.saved_ownerships());
        let fields = field_store.loaded_fields();
        // 把上次存的世界時刻種回權威時鐘（無存檔時等同破曉 `DayNight::new()`）。
        let daynight = daynight_store.loaded();
        // 把 DB 載回的地形差異種進 tile_world（C-1 通常為空，C-2+ 才有真實 delta）。
        let tile_world = TileWorld::with_deltas(tile_store.loaded_deltas());
        Self {
            players: Arc::new(RwLock::new(HashMap::new())),
            fields: Arc::new(RwLock::new(fields)),
            plots,
            daynight: Arc::new(RwLock::new(daynight)),
            nodes: Arc::new(RwLock::new(NodeField::new())),
            enemies: Arc::new(RwLock::new(EnemyField::new())),
            tx,
            tx_chat,
            suggestions,
            users,
            positions,
            inventories,
            field_store,
            daynight_store,
            connections: ConnectionCounts::new(),
            auth: AuthConfig::from_env(),
            pub_field: Arc::new(RwLock::new(Field::at(PUB_FIELD_ORIGIN_X, PUB_FIELD_ORIGIN_Y))),
            market: Arc::new(RwLock::new(Market::new())),
            tile_world: Arc::new(RwLock::new(tile_world)),
            tile_store,
            world_event: Arc::new(RwLock::new(WorldEvent::new())),
            quests: Arc::new(RwLock::new(QuestState::new())),
            guilds,
            daily_quests: Arc::new(RwLock::new(HashMap::new())),
            land_plots: Arc::new(RwLock::new(land_plot_registry)),
            land_plot_store,
            ranch: Arc::new(RwLock::new(RanchRegistry::new())),
            apiary: Arc::new(RwLock::new(crate::apiary::ApiaryRegistry::new())),
            farm_crops: Arc::new(RwLock::new(FarmCropRegistry::new())),
            star_crystals: Arc::new(RwLock::new(StarCrystalField::new())),
            night_index: Arc::new(AtomicU64::new(0)),
            dynamic_prices: Arc::new(RwLock::new(DynamicPriceMarket::new())),
            director: Arc::new(RwLock::new(crate::director::DirectorState::new())),
            npc_memory: Arc::new(RwLock::new({
                // 從持久化載回個人記憶；無 DB 或首次啟動時為空，等同全新狀態。
                let mut m = HashMap::new();
                for (player_id, npc_id, rel) in npc_memory_store.saved_memory() {
                    m.insert((player_id, npc_id), rel);
                }
                m
            })),
            npc_gift_stock: Arc::new(RwLock::new({
                // 從持久化載回 NPC 餘裕；未出現在 DB 的 NPC 用初始值補齊（首次啟動）。
                let mut stock = crate::npc_chat::initial_gift_stock();
                for (npc_id, s) in npc_memory_store.saved_gift_stock() {
                    stock.insert(npc_id, s);
                }
                stock
            })),
            npc_llm_sem: Arc::new(Semaphore::new(crate::npc_chat::MAX_CONCURRENT_LLM)),
            agent_bus: Arc::new(crate::npc_agent_wire::AgentBus::new()),
            npc_last_chat: Arc::new(RwLock::new(HashMap::new())),
            npc_memory_store,
            npc_pending_discount: Arc::new(RwLock::new(HashMap::new())),
            village_treasury: Arc::new(RwLock::new(crate::village_chief::INITIAL_TREASURY)),
            village_buff_until: Arc::new(RwLock::new(None)),
            world_log: Arc::new(RwLock::new(crate::world_log::WorldLog::new())),
            npc_lifecycle: Arc::new(RwLock::new(crate::npc_lifecycle::NpcLifecycle::new())),
            player_logs: Arc::new(RwLock::new(HashMap::new())),
            npc_proactive: Arc::new(RwLock::new(crate::npc_proactive::NpcProactiveCooldowns::new())),
            npc_needs: Arc::new(RwLock::new(crate::npc_needs::NpcNeedsState::new())),
            npc_relations: Arc::new(RwLock::new(crate::npc_relations::NpcRelationsState::new())),
            npc_factions: Arc::new(RwLock::new(crate::npc_factions::NpcFactionState::new())),
            social_dynamics: Arc::new(RwLock::new(crate::social_dynamics::SocialDynamicsState::new())),
            town_blocs: Arc::new(RwLock::new(crate::town_blocs::TownBlocState::new())),
            town_share: Arc::new(RwLock::new(crate::town_share::TownShareState::new())),
            world_grove: Arc::new(RwLock::new(crate::world_grove::WorldGrove::new())),
            daily_recap: Arc::new(RwLock::new(crate::daily_recap::DailyHighlights::new())),
            craft_ceremony: Arc::new(RwLock::new(crate::craft_ceremony::CraftCeremonyState::new())),
            world_title_first: Arc::new(RwLock::new(std::collections::HashMap::new())),
            npc_schedule: Arc::new(RwLock::new(crate::npc_schedule::NpcScheduleManager::new())),
            traveler: Arc::new(RwLock::new(crate::traveler_npc::TravelerNpc::new())),
            residents: Arc::new(RwLock::new(crate::resident_npc::ResidentManager::new())),
            community_gathering: Arc::new(RwLock::new(crate::community_gathering::CommunityGatheringState::new())),
            popularity_gathering: Arc::new(RwLock::new(crate::popularity_gathering::GatheringState::new())),
            boss_roar: Arc::new(RwLock::new(crate::boss_roar::BossRoarState::new())),
            boss_roar_sem: Arc::new(Semaphore::new(crate::boss_roar::MAX_CONCURRENT_ROARS)),
            boss_ai: Arc::new(RwLock::new(crate::boss_ai::BossAiState::new())),
            boss_ai_sem: Arc::new(Semaphore::new(crate::boss_ai::MAX_CONCURRENT_DECISIONS)),
            plaza_talk: Arc::new(RwLock::new(crate::plaza_talk::PlazaTalkState::new())),
            plaza_talk_sem: Arc::new(Semaphore::new(crate::plaza_talk::MAX_CONCURRENT_TALKS)),
            dawn_call: Arc::new(RwLock::new(crate::npc_dawn_call::DawnCallState::new())),
            dawn_call_sem: Arc::new(Semaphore::new(crate::npc_dawn_call::MAX_CONCURRENT_CALLS)),
            dusk_call: Arc::new(RwLock::new(crate::npc_dusk_call::DuskCallState::new())),
            dusk_call_sem: Arc::new(Semaphore::new(crate::npc_dusk_call::MAX_CONCURRENT_CALLS)),
            noon_bell: Arc::new(RwLock::new(crate::npc_noon_bell::NoonBellState::new())),
            noon_bell_sem: Arc::new(Semaphore::new(crate::npc_noon_bell::MAX_CONCURRENT_CALLS)),
            night_watch: Arc::new(RwLock::new(crate::npc_night_watch::NightWatchState::new())),
            night_watch_sem: Arc::new(Semaphore::new(crate::npc_night_watch::MAX_CONCURRENT_CALLS)),
            daytime_talk: Arc::new(RwLock::new(crate::daytime_talk::DaytimeTalkState::new())),
            daytime_talk_sem: Arc::new(Semaphore::new(crate::daytime_talk::MAX_CONCURRENT_TALKS)),
            lunch_chatter: Arc::new(RwLock::new(crate::lunch_chatter::LunchChatterState::new())),
            lunch_regulars: Arc::new(RwLock::new(crate::lunch_regular::RegularBook::new())),
            npc_recognition: Arc::new(RwLock::new(crate::npc_recognition::RecognitionBook::new())),
            pet_greeting: Arc::new(RwLock::new(crate::pet_greeting::GreetBook::new())),
            npc_bounty: Arc::new(RwLock::new(crate::npc_bounty::NpcBountyState::new())),
            npc_defeat_reaction: Arc::new(RwLock::new(crate::npc_defeat_reaction::NpcDefeatReactionState::new())),
            npc_defeat_reaction_sem: Arc::new(Semaphore::new(crate::npc_defeat_reaction::MAX_CONCURRENT_REACTIONS)),
            npc_level_greet: Arc::new(RwLock::new(crate::npc_level_greet::NpcLevelGreetState::new())),
            npc_commission: Arc::new(RwLock::new(crate::npc_commission::NpcCommissionState::new())),
            npc_expedition_boost: Arc::new(RwLock::new(crate::npc_expedition_boost::NpcExpeditionBoostState::new())),
            npc_workshop_boost: Arc::new(RwLock::new(crate::npc_workshop_boost::NpcWorkshopBoostState::new())),
            weather: Arc::new(RwLock::new(crate::weather::WeatherState::new())),
            rainbow: Arc::new(RwLock::new(crate::rainbow::RainbowState::new())),
            whisper_senders: Arc::new(RwLock::new(HashMap::new())),
            friends,
            parties: crate::party::PartyStore::default(),
            sprinklers: {
                let mut store = crate::sprinkler::SprinklerStore::new();
                store.load(sprinkler_preload);
                Arc::new(RwLock::new(store))
            },
            sprinkler_persist,
            npc_treasury: Arc::new(RwLock::new(crate::npc_treasury::NpcTreasuryState::new())),
            npc_pending_deal: Arc::new(RwLock::new(HashMap::new())),
            npc_stock: Arc::new(RwLock::new(crate::npc_stock::NpcStockState::new())),
            town_project: Arc::new(RwLock::new(
                town_project_store.saved_project().unwrap_or_else(crate::town_project::TownProjectState::new_observatory)
            )),
            town_project_store,
            visit_streaks,
            welcome_kits,
            observatory: Arc::new(RwLock::new(crate::observatory::ObservatoryState::new())),
            observatory_sem: Arc::new(Semaphore::new(crate::observatory::MAX_CONCURRENT_CALLS)),
            meteor_shower: Arc::new(RwLock::new(crate::meteor_shower::MeteorShowerState::new())),
            campfires: Arc::new(RwLock::new(crate::campfire::CampfireField::new())),
            watchtowers: Arc::new(RwLock::new(crate::coop_build::CoopBuildField::new())),
            vehicles: Arc::new(RwLock::new(crate::vehicle::VehicleField::with_default())),
            ship_repair: Arc::new(RwLock::new(crate::ship_repair::ShipRepairState::new())),
            snowmen: Arc::new(RwLock::new(crate::snowman::SnowmanField::new())),
            night_springs: Arc::new(RwLock::new(crate::night_aether_springs::NightAetherSprings::new())),
            firefly_lantern: Arc::new(RwLock::new(crate::firefly_lantern::FireflyLantern::new())),
            ether_surge: Arc::new(RwLock::new(crate::ether_surge::EtherSurge::new())),
            wandering_merchant: Arc::new(RwLock::new(crate::wandering_merchant::WanderingMerchantState::new())),
            season: Arc::new(RwLock::new(crate::season::SeasonState::new())),
            seasonal_nodes: Arc::new(RwLock::new(crate::seasonal_nodes::SeasonalNodesState::new())),
            season_peak_harvest_count: Arc::new(RwLock::new(0u32)),
            wayposts: Arc::new(RwLock::new(crate::wayposts::WaypostBoard::new())),
            bottles: Arc::new(RwLock::new(crate::bottle_drift::BottleSea::new())),
            wildlife_manager: Arc::new(RwLock::new(crate::wildlife::WildlifeManager::new())),
            species_relations: Arc::new(RwLock::new(crate::species_relations::SpeciesRelations::new())),
            monster_species: Arc::new(RwLock::new(crate::species_relations::MonsterSpeciesRelations::new())),
            monster_colonies: Arc::new(RwLock::new(crate::monster_colony::MonsterColonyManager::new())),
            home_furnishings: Arc::new(RwLock::new(std::collections::HashMap::new())),
            civic_vote: Arc::new(RwLock::new(crate::civic_vote::CivicVoteState::new())),
            town_memory: Arc::new(RwLock::new(crate::town_memory::TownMemory::new())),
            invasion: Arc::new(RwLock::new(crate::invasion::InvasionState::new())),
            eco_bounty: Arc::new(RwLock::new(crate::eco_bounty::EcoBountyState::new())),
            eco_festival: Arc::new(RwLock::new(crate::eco_festival::EcoFestivalState::new())),
            world_tally: Arc::new(RwLock::new(crate::world_tally::WorldTally::new())),
            combat_marks: Arc::new(RwLock::new(crate::combat_mark::CombatMarkState::new())),
            gold_rush: Arc::new(RwLock::new(crate::gold_rush::GoldRushState::new())),
            auction: Arc::new(RwLock::new(crate::auction::AuctionState::new())),
            fishing_contest: Arc::new(RwLock::new(crate::fishing_contest::FishingContest::new())),
            wonders: Arc::new(RwLock::new(crate::world_wonder::WorldWonderState::new())),
            world_boss: Arc::new(RwLock::new(crate::world_boss::WorldBossState::new())),
            monument: Arc::new(RwLock::new(crate::monument::Monument::new())),
            guardian_blessings: Arc::new(RwLock::new(crate::guardian_blessing::GuardianBlessingStore::new())),
        }
    }

    pub fn world_info(&self) -> WorldInfo {
        WorldInfo {
            width: WORLD_WIDTH,
            height: WORLD_HEIGHT,
        }
    }

    /// 把線上玩家的權威顯示名即時改掉(若該玩家此刻在線),讓下一張快照就帶新名——不必重連
    /// (HUD／世界名牌／聊天 from 都讀權威 `Player.name`)。改名 API 持久化成功後呼叫;
    /// 回傳是否命中線上玩家(離線者回 `false`,其下次進場時 `ws.rs` 自會從 `UserStore` 讀到新名)。
    /// 抽成具名方法以便迴歸測試鎖住「改名即時反映線上世界」這條契約。
    pub fn apply_live_rename(&self, uid: Uuid, new_name: &str) -> bool {
        match self.players.write().unwrap().get_mut(&uid) {
            Some(p) => {
                p.name = new_name.to_string();
                true
            }
            None => false,
        }
    }

    /// 檢查玩家是否在特定 NPC 的互動範圍內（ROADMAP 73：支援動態位置）。
    pub fn is_near_npc(&self, px: f32, py: f32, npc_id: &str) -> bool {
        let sch = self.npc_schedule.read().unwrap();
        if let Some((nx, ny)) = sch.get_pos(npc_id) {
            let dx = px - nx;
            let dy = py - ny;
            let reach = if npc_id == "village_chief" {
                crate::village_chief::CHIEF_REACH
            } else {
                crate::npc::SHOP_REACH
            };
            dx * dx + dy * dy <= reach * reach
        } else {
            false
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player_at(x: f32, y: f32, input: Input) -> Player {
        Player {
            id: Uuid::new_v4(),
            name: "測試".into(),
            species: "terran".into(),
            x,
            y,
            input,
            ether: 0,
            inventory: Inventory::new(),
            vitals: Vitals::new(),
            wallet: crate::economy::PlotWallet::new(),
            attack_cooldown: 0.0,
            exp: 0,
            codex: 0,
            atlas: 0,
            skylog: 0,
            cheers: 0,
            planet: PLANET_HOME.to_string(),
            masteries: crate::class::Masteries::new(),
            seen_mastery_tiers: [0; 5],
            guild_tag: None,
            party_id: None,
            hair_style: 0,
            skin_tone: 0,
            goggle_color: 0,
            costume: 0,
            achievements: AchievementSet::new(),
            kill_count: 0,
            session_gather_count: 0,
            session_harvest_count: 0,
            title_set: crate::player_title::TitleSet::new(),
            activity_chain: crate::activity_chain::ActivityChain::new(0),
            meditation: None,
            last_meditate: None,
            busking: None,
            last_busk: None,
            busk_count: 0,
            ensemble_size: 0,
            flying_kite: false,
            lantern_fireflies: 0,
            meal_buff: None,
            dish_mastery: crate::dish_mastery::DishMastery::default(),
            onboarding: crate::onboarding::Onboarding::default(),
            refine_attempt_count: 0,
            equipment: crate::equipment::EquipmentSlots::default(),
            skill_cooldowns: crate::active_skill::SkillCooldowns::default(),
            pending_warcry: false,
            pending_bounty: false,
            pending_precision: false,
            pending_haggle: false,
            auto_skills: std::collections::HashSet::new(),
            stats: crate::stat_points::StatPoints::default(),
            skill_masteries: crate::skill_mastery::SkillMasteries::default(),
            pet: None,
            pet_x: x,
            pet_y: y,
            pet_playing: false,
            pet_fetch: None,
            pet_fetching: false,
            pet_fetch_count: 0,
            fish_cooldown: 0.0,
            fish_attempt_count: 0,
            fishing: None,
            mine_cooldown: 0.0,
            mine_attempt_count: 0,
            mining: None,
            current_locale: None,
            cook_cooldown: 0.0,
            cook_attempt_count: 0,
            cooking: None,
            perfect_dishes: 0,
            aether_draw: None,
            chop_cooldown: 0.0,
            chopping: None,
            skip_cooldown: 0.0,
            skipping: None,
            skip_attempt_count: 0,
            guard_cooldown: 0.0,
            guarding: None,
            guard_shield: None,
            dodge_cooldown: 0.0,
            dodging: None,
            charge_cooldown: 0.0,
            charging: None,
            charge_ready: None,
            poison: crate::affliction::Poison::new(),
            wayfaring: crate::wayfaring::Wayfaring::default(),
            fish_records: crate::fish_size::FishRecords::default(),
            traced_constellations: 0,
            inscriptions_mask: 0,
            reconcile_errand: None,
            toast_cooldown: 0.0,
            toast_count: 0,
            high_five_offer: 0,
            recent_emote: None,
            cheer_offer: 0,
            cheer_cooldowns: std::collections::HashMap::new(),
            trade_cargo: None,
            trade_cooldowns: crate::trade_route::TradeCooldowns::new(),
            workshop_active: None,
            workshop_cooldown: 0.0,
            bounty_active: None,
            bounty_cooldown: 0.0,
            expedition_active: None,
            expedition_cooldown: 0.0,
            procurement_active: None,
            procurement_cooldown: 0.0,
            farm_fair_active: None,
            farm_fair_cooldown: 0.0,
            warehouse: Warehouse::default(),
            decay_timers: crate::perishable::PerishableDecayState::new(),
            indoor_plot_id: None,
            indoor_x: 0.0,
            indoor_y: 0.0,
            inventory_extra_kinds: 0,
            kill_streak: 0,
            streak_last_kill: None,
            newcomer_until: None,
            riding: None,
            riding_passenger: false,
            boost_trigger: None,
            boosting: false,
            bell_trigger: None,
            mount_gather_at: None,
        }
    }

    #[test]
    fn moves_right_at_expected_speed() {
        // (1360,200)：方圓 700px 內無水的陸地，純測速度（掃掠碰撞不再穿水，故須避開水域路徑）。
        let mut p = player_at(
            1360.0,
            200.0,
            Input {
                right: true,
                ..Default::default()
            },
        );
        p.step(1.0, |_, _| false); // 一秒
        assert!((p.x - (1360.0 + PLAYER_SPEED)).abs() < 0.001);
        assert!((p.y - 200.0).abs() < 0.001);
    }

    #[test]
    fn running_is_run_mult_times_walking() {
        // 相同輸入、相同時間：run=true 的位移應 ≈ run=false 的 RUN_MULT 倍。
        let walk_start = 1360.0_f32;
        let mut walker = player_at(
            walk_start,
            200.0,
            Input {
                right: true,
                ..Default::default()
            },
        );
        let mut runner = player_at(
            walk_start,
            200.0,
            Input {
                right: true,
                run: true,
                ..Default::default()
            },
        );
        walker.step(0.5, |_, _| false);
        runner.step(0.5, |_, _| false);
        let walk_dist = walker.x - walk_start;
        let run_dist = runner.x - walk_start;
        assert!(walk_dist > 0.0, "走路應有位移");
        // 容差放寬到 0.05px：碰撞掃掠解算在不同距離有極微的次像素差，非倍率邏輯問題。
        assert!(
            (run_dist - walk_dist * world_core::RUN_MULT).abs() < 0.05,
            "跑步位移 {run_dist} 應為走路 {walk_dist} 的 {} 倍",
            world_core::RUN_MULT
        );
    }

    #[test]
    fn diagonal_is_not_faster() {
        // (1360,200)：方圓 700px 無水陸地（掃掠碰撞不穿水，純測對角線速度需避水）。
        let mut p = player_at(
            1360.0,
            200.0,
            Input {
                right: true,
                down: true,
                ..Default::default()
            },
        );
        p.step(1.0, |_, _| false);
        let dist = (((p.x - 1360.0).powi(2)) + ((p.y - 200.0).powi(2))).sqrt();
        // 對角線位移量應約等於單軸速度，而非 sqrt(2) 倍。
        assert!((dist - PLAYER_SPEED).abs() < 0.01, "dist={dist}");
    }

    #[test]
    fn walks_past_world_edge_into_negative() {
        // ③ 無限世界（切片 A）：邊界 clamp 已拿掉，往上走應能跨過 y=0 進入負座標、不被夾在 0。
        // 從 (1360,200) 陸地往左上跑步約 260px → y 跨進負值（避開原點附近的水，掃掠碰撞不穿水）。
        // 走路基速放慢成 230 後單秒位移不足以越過 y=0，改用 run 跑步補回距離、同時順帶驗證跑步可越界。
        let mut p = player_at(
            1360.0,
            200.0,
            Input {
                up: true,
                left: true,
                run: true,
                ..Default::default()
            },
        );
        p.step(1.0, |_, _| false);
        assert!(p.y < 0.0, "應跨過 y=0 邊界進入負座標: ({}, {})", p.x, p.y);
    }

    #[test]
    fn idle_player_stays_put() {
        let mut p = player_at(300.0, 300.0, Input::default());
        p.step(1.0, |_, _| false);
        assert_eq!(p.x, 300.0);
        assert_eq!(p.y, 300.0);
    }

    #[test]
    fn downed_player_cannot_move() {
        // 被打趴後按方向鍵也不能移動（倒地定身）。
        let mut p = player_at(300.0, 300.0, Input {
            right: true,
            ..Default::default()
        });
        p.vitals.take_damage(crate::vitals::MAX_HP);
        assert!(p.vitals.is_downed());
        p.step(1.0, |_, _| false);
        assert_eq!(p.x, 300.0, "被打趴後不應移動");
        assert_eq!(p.y, 300.0, "被打趴後不應移動");
    }

    #[test]
    fn player_cannot_walk_into_water() {
        // 掃描找一個水域座標（biome_at 確定性，多次呼叫結果相同）
        let mut water: Option<(f32, f32)> = None;
        'outer: for gy in 0..30i32 {
            for gx in 0..30i32 {
                let x = gx as f32 * 200.0;
                let y = gy as f32 * 200.0;
                if biome_at(x as f64, y as f64) == Biome::Water {
                    water = Some((x, y));
                    break 'outer;
                }
            }
        }
        let (wx, wy) = match water {
            Some(p) => p,
            None => return, // 掃描範圍內沒有水域，跳過（世界生成之後極罕見）
        };
        // 從一個肯定不是水的位置出發，朝水域方向持續走幾步
        // 找非水域起點（水域座標附近、往外偏移一點）
        let offsets = [(-400.0, 0.0), (400.0, 0.0), (0.0, -400.0), (0.0, 400.0)];
        for (ox, oy) in offsets {
            let sx = wx + ox;
            let sy = wy + oy;
            if biome_at(sx as f64, sy as f64) != Biome::Water {
                // 找到陸地起點，朝水域方向走
                let dir_x = (wx - sx).signum();
                let dir_y = (wy - sy).signum();
                let input = Input {
                    right: dir_x > 0.0,
                    left: dir_x < 0.0,
                    down: dir_y > 0.0,
                    up: dir_y < 0.0,
                    run: false,
                };
                let mut p = player_at(sx, sy, input);
                for _ in 0..10 {
                    p.step(0.1, |_, _| false); // 共一秒
                }
                // 最終位置不應在水裡（水域擋路）
                assert!(
                    biome_at(p.x as f64, p.y as f64) != Biome::Water,
                    "玩家走進了水域 ({}, {})，水域碰撞應阻擋",
                    p.x, p.y
                );
                return;
            }
        }
        // 所有方向都是水域，跳過
    }

    #[test]
    fn chat_and_snapshot_channels_are_independent() {
        // 聊天與快照走不同廣播頻道：高頻快照灌滿 tx 造成 Lagged 時，不會把聊天一起丟。
        // 這裡驗證兩條頻道彼此隔離——各自的訂閱者只收到自己頻道的訊息，不會串流。
        let app = AppState::new();
        let mut rx_snap = app.tx.subscribe();
        let mut rx_chat = app.tx_chat.subscribe();
        app.tx_chat.send("聊天".to_string()).unwrap();
        app.tx.send(Arc::new(ServerMsg::PlayerLeft { id: Uuid::nil() })).unwrap();
        // 聊天訂閱者只拿到聊天，沒有快照混進來。
        assert_eq!(rx_chat.try_recv().unwrap(), "聊天");
        assert!(rx_chat.try_recv().is_err());
        // 快照訂閱者只拿到快照（Arc 包著的列舉），沒有聊天混進來。
        match &*rx_snap.try_recv().unwrap() {
            ServerMsg::PlayerLeft { .. } => {}
            _ => panic!("應拿到 PlayerLeft"),
        }
        assert!(rx_snap.try_recv().is_err());
    }

    #[test]
    fn live_rename_updates_online_player_snapshot_name() {
        // 改名 API 持久化成功後呼叫 apply_live_rename:線上玩家的權威 Player.name 立刻換,
        // 下一張快照(view)就帶新名,不必重連。鎖住「改名即時反映線上世界」這條契約。
        let app = AppState::new();
        let mut p = player_at(100.0, 100.0, Input::default());
        let uid = p.id;
        p.name = "舊名".into();
        app.players.write().unwrap().insert(uid, p);

        let hit = app.apply_live_rename(uid, "新名");

        assert!(hit, "玩家在線應命中");
        let sch = app.npc_schedule.read().unwrap();
        assert_eq!(
            app.players.read().unwrap().get(&uid).unwrap().view(&sch, None, false).name,
            "新名",
            "下一張快照應帶新名"
        );
    }

    #[test]
    fn live_rename_for_offline_player_is_noop() {
        // 改名者此刻不在線(查無此 id):apply_live_rename 回 false、不 panic、不誤改別人。
        // (離線者改名仍持久化在 UserStore,下次進場時 ws.rs 自會讀到新名。)
        let app = AppState::new();
        let online = player_at(0.0, 0.0, Input::default());
        let online_id = online.id;
        app.players.write().unwrap().insert(online_id, online);

        let hit = app.apply_live_rename(Uuid::new_v4(), "幽靈");

        assert!(!hit, "查無此線上玩家應回 false");
        assert_eq!(
            app.players.read().unwrap().get(&online_id).unwrap().name,
            "測試",
            "不該誤改到其他線上玩家"
        );
    }

    #[test]
    fn pub_field_initializes_at_configured_origin() {
        // 公共農地啟動時以正確座標建立，且不是 nil（有格可互動）。
        let app = AppState::new();
        let pf = app.pub_field.read().unwrap();
        let (ox, oy) = pf.origin();
        assert_eq!(ox, PUB_FIELD_ORIGIN_X, "公共農地 X 座標應對齊常數");
        assert_eq!(oy, PUB_FIELD_ORIGIN_Y, "公共農地 Y 座標應對齊常數");
        // 初始格數 > 0 且任一格均為自然地（尚未被耕種）。
        assert!(pf.rows() > 0, "公共農地應有至少一列可耕格");
        // view() 的 owner 由 game.rs 廣播層戳上 Uuid::nil()；
        // 此處只驗農地本身初始化正確（不需驗 owner，那是廣播層的責任）。
    }

    #[test]
    fn pub_field_accepts_farming_action() {
        // 任意一格自然地翻土可成功（模擬玩家在公共農地操作）——驗農地本身能被互動。
        let app = AppState::new();
        let mut pf = app.pub_field.write().unwrap();
        let (ox, oy) = pf.origin();
        // cell_at 在 origin 應回 (col=0, row=0)。
        let cell = pf.cell_at(ox, oy);
        assert!(cell.is_some(), "公共農地 origin 座標應能對應到一格");
        let (col, row) = cell.unwrap();
        let tilled = pf.till(col, row);
        assert!(tilled, "公共農地自然地翻土應成功");
    }

    // ── C-3 tile 碰撞測試 ──
    // 注意：座標要在陸地（非水域）；否則 step 的水域 blocked 優先觸發，干擾 tile 碰撞邏輯。
    // 使用距公共農地中心不遠的安全陸地區（biome 確定性，與前端一致），省去動態掃描。
    const LAND_X: f32 = 2200.0;
    const LAND_Y: f32 = 2200.0;

    #[test]
    fn tile_collision_blocks_direct_movement() {
        // 玩家往右走，右側有牆；應被擋在牆外（含碰撞半徑）。
        let wall_x: f32 = LAND_X + 50.0;
        let mut p = player_at(LAND_X, LAND_Y, Input { right: true, ..Default::default() });
        for _ in 0..20 {
            p.step(0.1, |x, _y| x > wall_x);
        }
        // 玩家中心 + 碰撞半徑不得超過牆。
        assert!(
            p.x + PLAYER_TILE_RADIUS <= wall_x + 1.0,
            "應被牆擋住, x={} r={} wall_x={}",
            p.x,
            PLAYER_TILE_RADIUS,
            wall_x
        );
    }

    #[test]
    fn tile_collision_slides_along_wall() {
        // 玩家往右下走，右側有牆；應沿 Y 方向滑動（X 被擋住、Y 自由）。
        let wall_x: f32 = LAND_X + 30.0;
        let mut p = player_at(LAND_X, LAND_Y, Input { right: true, down: true, ..Default::default() });
        for _ in 0..10 {
            p.step(0.1, |x, _y| x > wall_x);
        }
        assert!(p.x + PLAYER_TILE_RADIUS <= wall_x + 1.0, "X 應被牆擋住");
        assert!(p.y > LAND_Y, "Y 方向應能沿牆滑行");
    }

    #[test]
    fn tile_collision_escape_from_inside_solid() {
        // 玩家生成在實心格內（如傳送後落地），應能逃出（受困時以中心判定、允許逃脫）。
        let start_x = LAND_X;
        let start_y = LAND_Y;
        let mut p = player_at(start_x, start_y, Input { right: true, ..Default::default() });
        // 以玩家中心為圓心的 40px 範圍全為實心格（模擬傳送落地在格內）。
        let tile_solid = |x: f32, y: f32| {
            (start_x - 40.0..=start_x + 40.0).contains(&x)
                && (start_y - 40.0..=start_y + 40.0).contains(&y)
        };
        p.step(0.1, tile_solid);
        assert!(p.x > start_x, "受困玩家應能逃出實心格，x={}", p.x);
    }

    #[test]
    fn level_is_zero_at_start() {
        let p = player_at(0.0, 0.0, Input::default());
        assert_eq!(p.level(), 0);
        assert_eq!(p.exp, 0);
    }

    #[test]
    fn level_increments_every_100_exp() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.exp = 99;
        assert_eq!(p.level(), 0, "99 exp 仍是 0 級");
        p.exp = 100;
        assert_eq!(p.level(), 1, "100 exp 升至 1 級");
        p.exp = 399;
        assert_eq!(p.level(), 3, "399 exp 升至 3 級");
        p.exp = 500;
        assert_eq!(p.level(), 5, "500 exp 升至 5 級");
    }

    // ROADMAP 20 — 星際旅行條件測試。
    fn all_biome_weapons() -> [crate::inventory::ItemKind; 5] {
        use crate::inventory::ItemKind;
        [
            ItemKind::MeadowAmulet,
            ItemKind::MushroomStaff,
            ItemKind::CrystalBlade,
            ItemKind::RuneBlade,
            ItemKind::CoralLance,
        ]
    }

    #[test]
    fn travel_home_to_verdant_requires_ether() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST - 1;
        for w in all_biome_weapons() { p.inventory.add(w, 1); }
        assert!(p.can_travel_to(PLANET_VERDANT, 0).is_err(), "乙太不足應拒絕旅行");
    }

    #[test]
    fn travel_home_to_verdant_requires_all_biome_weapons() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST + 100;
        // 只放 4 件，少 1 件。
        let weapons = all_biome_weapons();
        for w in &weapons[..4] { p.inventory.add(*w, 1); }
        assert!(p.can_travel_to(PLANET_VERDANT, 0).is_err(), "武裝未齊應拒絕旅行");
    }

    #[test]
    fn travel_home_to_verdant_succeeds_with_weapons_and_ether() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST;
        for w in all_biome_weapons() { p.inventory.add(w, 1); }
        assert!(p.can_travel_to(PLANET_VERDANT, 0).is_ok(), "武裝齊 + 乙太足應允許旅行");
    }

    #[test]
    fn travel_verdant_to_home_only_requires_ether() {
        let mut p = player_at(VERDANT_SPAWN_X, VERDANT_SPAWN_Y, Input::default());
        p.planet = PLANET_VERDANT.to_string();
        p.ether = TRAVEL_ETHER_COST;
        // 不需要武器即可返回。
        assert!(p.can_travel_to(PLANET_HOME, 0).is_ok(), "翠幽星→故鄉只需乙太");
    }

    #[test]
    fn travel_already_on_same_planet_is_error() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST + 100;
        for w in all_biome_weapons() { p.inventory.add(w, 1); }
        // 已在故鄉，嘗試去故鄉。
        assert!(p.can_travel_to(PLANET_HOME, 0).is_err(), "已在故鄉不能再去故鄉");
    }

    // ROADMAP 39 — 翠幽星直購路測試。
    #[test]
    fn travel_verdant_direct_purchase_succeeds_with_300_ether() {
        // 無武裝但有 300 乙太 → 直購路應通過。
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_VERDANT_DIRECT;
        assert!(p.can_travel_to(PLANET_VERDANT, 0).is_ok(), "300 乙太直購路應允許旅行");
    }

    #[test]
    fn travel_verdant_direct_purchase_fails_with_insufficient_ether() {
        // 無武裝且乙太不足 300 → 兩條路都不通，應拒絕。
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_VERDANT_DIRECT - 1;
        assert!(p.can_travel_to(PLANET_VERDANT, 0).is_err(), "299 乙太直購路應拒絕旅行");
    }

    #[test]
    fn travel_verdant_weapons_path_cheaper_when_armed() {
        use crate::inventory::ItemKind;
        // 武裝齊 + 30 乙太（少於 300）→ 武裝路應通過。
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST;
        for w in all_biome_weapons() { p.inventory.add(w, 1); }
        assert!(p.can_travel_to(PLANET_VERDANT, 0).is_ok(), "武裝路 30 乙太應通過");
    }

    // ROADMAP 22 — 赤焰星旅行條件測試。
    #[test]
    fn travel_home_to_crimson_requires_ether() {
        use crate::inventory::ItemKind;
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_CRIMSON - 1;
        p.inventory.add(ItemKind::JadeShard, 1);
        assert!(p.can_travel_to(PLANET_CRIMSON, 0).is_err(), "赤焰星乙太不足應拒絕旅行");
    }

    #[test]
    fn travel_home_to_crimson_requires_jade_shard() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_CRIMSON + 10;
        // 沒有翠幽碎片（未探索過翠幽星）。
        assert!(p.can_travel_to(PLANET_CRIMSON, 0).is_err(), "無翠幽碎片應拒絕赤焰星旅行");
    }

    #[test]
    fn travel_home_to_crimson_succeeds_with_jade_shard_and_ether() {
        use crate::inventory::ItemKind;
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_CRIMSON;
        p.inventory.add(ItemKind::JadeShard, 1);
        assert!(p.can_travel_to(PLANET_CRIMSON, 0).is_ok(), "翠幽碎片 + 乙太足應允許赤焰星旅行");
    }

    #[test]
    fn travel_crimson_to_home_only_requires_ether() {
        let mut p = player_at(CRIMSON_SPAWN_X, CRIMSON_SPAWN_Y, Input::default());
        p.planet = PLANET_CRIMSON.to_string();
        p.ether = TRAVEL_ETHER_COST;
        assert!(p.can_travel_to(PLANET_HOME, 0).is_ok(), "赤焰星→故鄉只需 30 乙太");
    }

    #[test]
    fn travel_home_to_aether_fails_with_insufficient_ether() {
        use crate::inventory::ItemKind;
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_AETHER - 1;
        p.inventory.add(ItemKind::VoidShard, 1);
        assert!(p.can_travel_to(PLANET_AETHER, 0).is_err(), "乙太不足應拒絕霧醚星旅行");
    }

    #[test]
    fn travel_home_to_aether_fails_without_void_shard() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_AETHER;
        assert!(p.can_travel_to(PLANET_AETHER, 0).is_err(), "無虛空碎片應拒絕霧醚星旅行");
    }

    #[test]
    fn travel_void_to_aether_succeeds_with_void_shard_and_ether() {
        use crate::inventory::ItemKind;
        let mut p = player_at(0.0, 0.0, Input::default());
        p.planet = PLANET_VOID.to_string();
        p.ether = TRAVEL_ETHER_COST_AETHER;
        p.inventory.add(ItemKind::VoidShard, 1);
        assert!(p.can_travel_to(PLANET_AETHER, 0).is_ok(), "虛空碎片 + 乙太足應允許霧醚星旅行");
    }

    #[test]
    fn travel_aether_to_home_only_requires_ether() {
        let mut p = player_at(AETHER_SPAWN_X, AETHER_SPAWN_Y, Input::default());
        p.planet = PLANET_AETHER.to_string();
        p.ether = TRAVEL_ETHER_COST;
        assert!(p.can_travel_to(PLANET_HOME, 0).is_ok(), "霧醚星→故鄉只需 30 乙太");
    }

    #[test]
    fn travel_to_origin_fails_with_insufficient_ether() {
        use crate::inventory::ItemKind;
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_ORIGIN - 1;
        p.inventory.add(ItemKind::AetherShard, 1);
        assert!(p.can_travel_to(PLANET_ORIGIN, 0).is_err(), "乙太不足應拒絕星源星旅行");
    }

    #[test]
    fn travel_to_origin_fails_without_aether_shard() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_ORIGIN;
        assert!(p.can_travel_to(PLANET_ORIGIN, 0).is_err(), "無霧醚碎片應拒絕星源星旅行");
    }

    #[test]
    fn travel_aether_to_origin_succeeds_with_aether_shard_and_ether() {
        use crate::inventory::ItemKind;
        let mut p = player_at(AETHER_SPAWN_X, AETHER_SPAWN_Y, Input::default());
        p.planet = PLANET_AETHER.to_string();
        p.ether = TRAVEL_ETHER_COST_ORIGIN;
        p.inventory.add(ItemKind::AetherShard, 1);
        assert!(p.can_travel_to(PLANET_ORIGIN, 0).is_ok(), "霧醚碎片 + 乙太足應允許星源星旅行");
    }

    #[test]
    fn travel_origin_to_home_only_requires_ether() {
        let mut p = player_at(ORIGIN_SPAWN_X, ORIGIN_SPAWN_Y, Input::default());
        p.planet = PLANET_ORIGIN.to_string();
        p.ether = TRAVEL_ETHER_COST;
        assert!(p.can_travel_to(PLANET_HOME, 0).is_ok(), "星源星→故鄉只需 30 乙太");
    }
}
