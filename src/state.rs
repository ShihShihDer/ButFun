//! 伺服器的共享狀態：權威世界、玩家清單、廣播頻道。
//!
//! 目前狀態存在記憶體裡。持久化（Postgres）刻意藏在這層之後——之後把 `players`
//! 換成「啟動時從 DB 載入、變動時寫回」即可，不用動 WebSocket / 遊戲迴圈的程式。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

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
    /// 玩家目前所在星球（ROADMAP 20/22/23/24/25 多星球旅程）。
    /// "home" = 故鄉，"verdant" = 翠幽星，"crimson" = 赤焰星，"void" = 虛空星，
    /// "aether" = 霧醚星，"origin" = 星源星。
    /// 執行期狀態，重連回 home 起算（不持久化，跨重啟無礙）。
    pub planet: String,
    /// 玩家五條熟練度 XP（ROADMAP 38 兼修熟練度）。重連從 DB 載回；每次活動自動累積。
    pub masteries: crate::class::Masteries,
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

    // ── 寵物（ROADMAP 46）────────────────────────────────────────────────
    /// 目前的寵物種類（記憶體前置，重啟後從 None 開始；設計上不持久化）。
    pub pet: Option<crate::pet::PetKind>,

    // ── 釣魚（ROADMAP 47）────────────────────────────────────────────────
    /// 釣魚冷卻剩餘秒數（0.0 = 可釣；> 0 = 冷卻中）。由 game.rs 每 tick 遞減。
    pub fish_cooldown: f32,
    /// 釣魚嘗試計數（確保每次釣魚偽隨機結果不同；記憶體前置，重啟清空）。
    pub fish_attempt_count: u64,

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
            fish_cooldown: self.fish_cooldown,
            near_water: crate::fishing::is_near_water(self.x, self.y),
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
        // 背包仍可接受（已有此種類，或種類槽未滿）
        if !self.inventory.is_full_for_new_kind(item, MAX_INVENTORY_ITEM_KINDS) {
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
        // 速度加點透過縮放 dt 實現，保持碰撞邏輯不變。
        let effective_dt = dt * self.stats.speed_mult();
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
    /// 農田地塊作物狀態（ROADMAP 49）：記憶體模式，重啟歸零。
    pub farm_crops: Arc<RwLock<FarmCropRegistry>>,
    /// 夜採星晶礦脈（ROADMAP 50）：夜間生成、白天清除；記憶體模式。
    pub star_crystals: Arc<RwLock<StarCrystalField>>,
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
    /// 天文台星象預報狀態（ROADMAP 132）：天文台竣工後每個黎明廣播星象、啟用全服加成。
    /// 純記憶體模式，重啟清零；不破壞玩家資料。
    pub observatory: Arc<RwLock<crate::observatory::ObservatoryState>>,
    /// 星象預報專屬 Semaphore（容量 1）：同時最多一個 AI 星象生成呼叫。
    pub observatory_sem: Arc<Semaphore>,
    /// 流星雨狀態（ROADMAP 133）：天文台竣工後每 30 分鐘觸發流星雨，地面出現星塵採集點。
    /// 純記憶體模式，重啟清零；不破壞玩家資料。
    pub meteor_shower: Arc<RwLock<crate::meteor_shower::MeteorShowerState>>,
    /// 旅行商人狀態（ROADMAP 135）：每 2 小時來訪，停留 10 分鐘，限時出售稀有物品。
    /// 純記憶體模式，重啟清零；不破壞玩家資料。
    pub wandering_merchant: Arc<RwLock<crate::wandering_merchant::WanderingMerchantState>>,
    /// 季節循環（ROADMAP 137）：春夏秋冬各 20 分鐘輪替，影響作物成長速度。
    /// 記憶體模式，重啟從春天開始（世界換季，行為合理）。
    pub season: Arc<RwLock<crate::season::SeasonState>>,
    /// 中立野生動物（ROADMAP 140）：野鳥/野鹿/小動物。
    /// 記憶體模式，重啟重新在固定座標生成。
    pub wildlife_manager: Arc<RwLock<crate::wildlife::WildlifeManager>>,
    /// 人類↔物種關係（ROADMAP 144）：各物種對人類的態度值。
    /// 記憶體模式，重啟清零（世界換季重生）。
    pub species_relations: Arc<RwLock<crate::species_relations::SpeciesRelations>>,
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
            farm_crops: Arc::new(RwLock::new(FarmCropRegistry::new())),
            star_crystals: Arc::new(RwLock::new(StarCrystalField::new())),
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
            npc_schedule: Arc::new(RwLock::new(crate::npc_schedule::NpcScheduleManager::new())),
            traveler: Arc::new(RwLock::new(crate::traveler_npc::TravelerNpc::new())),
            residents: Arc::new(RwLock::new(crate::resident_npc::ResidentManager::new())),
            community_gathering: Arc::new(RwLock::new(crate::community_gathering::CommunityGatheringState::new())),
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
            npc_bounty: Arc::new(RwLock::new(crate::npc_bounty::NpcBountyState::new())),
            npc_defeat_reaction: Arc::new(RwLock::new(crate::npc_defeat_reaction::NpcDefeatReactionState::new())),
            npc_defeat_reaction_sem: Arc::new(Semaphore::new(crate::npc_defeat_reaction::MAX_CONCURRENT_REACTIONS)),
            npc_level_greet: Arc::new(RwLock::new(crate::npc_level_greet::NpcLevelGreetState::new())),
            npc_commission: Arc::new(RwLock::new(crate::npc_commission::NpcCommissionState::new())),
            npc_expedition_boost: Arc::new(RwLock::new(crate::npc_expedition_boost::NpcExpeditionBoostState::new())),
            npc_workshop_boost: Arc::new(RwLock::new(crate::npc_workshop_boost::NpcWorkshopBoostState::new())),
            weather: Arc::new(RwLock::new(crate::weather::WeatherState::new())),
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
            observatory: Arc::new(RwLock::new(crate::observatory::ObservatoryState::new())),
            observatory_sem: Arc::new(Semaphore::new(crate::observatory::MAX_CONCURRENT_CALLS)),
            meteor_shower: Arc::new(RwLock::new(crate::meteor_shower::MeteorShowerState::new())),
            wandering_merchant: Arc::new(RwLock::new(crate::wandering_merchant::WanderingMerchantState::new())),
            season: Arc::new(RwLock::new(crate::season::SeasonState::new())),
            wildlife_manager: Arc::new(RwLock::new(crate::wildlife::WildlifeManager::new())),
            species_relations: Arc::new(RwLock::new(crate::species_relations::SpeciesRelations::new())),
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
            planet: PLANET_HOME.to_string(),
            masteries: crate::class::Masteries::new(),
            guild_tag: None,
            party_id: None,
            hair_style: 0,
            skin_tone: 0,
            goggle_color: 0,
            costume: 0,
            achievements: AchievementSet::new(),
            kill_count: 0,
            refine_attempt_count: 0,
            equipment: crate::equipment::EquipmentSlots::default(),
            skill_cooldowns: crate::active_skill::SkillCooldowns::default(),
            pending_warcry: false,
            pending_bounty: false,
            pending_precision: false,
            pending_haggle: false,
            auto_skills: std::collections::HashSet::new(),
            stats: crate::stat_points::StatPoints::default(),
            pet: None,
            fish_cooldown: 0.0,
            fish_attempt_count: 0,
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
        // 從 (1360,200) 陸地往左上走 226px → y 跨進負值（避開原點附近的水，掃掠碰撞不穿水）。
        let mut p = player_at(
            1360.0,
            200.0,
            Input {
                up: true,
                left: true,
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
