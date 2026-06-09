//! 伺服器的共享狀態：權威世界、玩家清單、廣播頻道。
//!
//! 目前狀態存在記憶體裡。持久化（Postgres）刻意藏在這層之後——之後把 `players`
//! 換成「啟動時從 DB 載入、變動時寫回」即可，不用動 WebSocket / 遊戲迴圈的程式。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;
use uuid::Uuid;

use world_core::{biome_at, resolve_move, Biome};

use crate::auth::AuthConfig;
use crate::connections::ConnectionCounts;
use crate::daynight::DayNight;
use crate::daynight_store::DayNightStore;
use crate::enemy_field::EnemyField;
use crate::field::Field;
use crate::field_store::FieldStore;
use crate::market::Market;
use crate::gather_field::NodeField;
use crate::inventory::Inventory;
use crate::inventory_store::InventoryStore;
use crate::tiles::TileWorld;
use crate::tile_store::TileStore;
use crate::vitals::Vitals;
use crate::plot_registry::PlotRegistry;
use crate::positions::PositionStore;
use crate::protocol::{ItemStack, PlayerView, WorldInfo, ServerMsg};
use crate::suggestions::SuggestionStore;
use crate::users::UserStore;

/// 世界大小（像素）。放大成大世界,讓玩家散得開、各自有空間(回應「農地都擠中央、地圖太小」)。
pub const WORLD_WIDTH: f32 = 6000.0;
pub const WORLD_HEIGHT: f32 = 6000.0;
/// 玩家移動速度（像素 / 秒）。大世界一併調快,跨圖不至於太久。
pub const PLAYER_SPEED: f32 = 320.0;

/// 玩家 tile 碰撞半徑（像素）。用四角檢查確保玩家不嵌入格子；
/// 明顯小於半格（16px），讓玩家在單格寬隧道裡有餘裕、不會稍微偏離中線就被牆角卡住。
pub const PLAYER_TILE_RADIUS: f32 = 8.0;

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
/// 故鄉 ↔ 翠幽星 / 赤焰星 / 虛空星 / 霧醚星 → 故鄉的乙太燃料費（單程 30）。
pub const TRAVEL_ETHER_COST: u32 = 30;
/// 故鄉 → 赤焰星的乙太燃料費（第二顆星球，需要更多乙太）。
pub const TRAVEL_ETHER_COST_CRIMSON: u32 = 50;
/// 故鄉 / 赤焰星 → 虛空星的乙太燃料費（第三顆星球，宇宙深淵，需要最多乙太）。
pub const TRAVEL_ETHER_COST_VOID: u32 = 80;
/// 虛空星 / 赤焰星 / 故鄉 → 霧醚星的乙太燃料費（第四顆星球，乙太迷霧邊際，需要最多乙太）。
pub const TRAVEL_ETHER_COST_AETHER: u32 = 120;

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
    /// 玩家目前所在星球（ROADMAP 20/22/23/24 多星球旅程）。
    /// "home" = 故鄉，"verdant" = 翠幽星，"crimson" = 赤焰星，"void" = 虛空星，"aether" = 霧醚星。
    /// 執行期狀態，重連回 home 起算（不持久化，跨重啟無礙）。
    pub planet: String,
}

impl Player {
    /// 由 exp 推算等級（每 100 exp 升一級，無上限）。
    pub fn level(&self) -> u32 {
        self.exp / 100
    }

    pub fn view(&self) -> PlayerView {
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
            attack: crate::combat::weapon_power(&self.inventory)
                + crate::combat::level_attack_bonus(self.level()),
            defense: crate::combat::armor_defense(&self.inventory),
            planet: self.planet.clone(),
        }
    }

    /// 星際旅行可行性驗證（純函式，供 ws.rs 呼叫與測試）。
    /// 回傳 `Ok(())` = 可以旅行；`Err(msg)` = 失敗原因字串。
    pub fn can_travel_to(&self, dest: &str) -> Result<(), String> {
        use crate::inventory::ItemKind;
        if self.vitals.is_downed() {
            return Err("倒地中無法旅行".into());
        }
        if dest == PLANET_VERDANT && self.planet == PLANET_HOME {
            // 故鄉 → 翠幽星：需五大生態武裝全套 + 30 乙太。
            if self.ether < TRAVEL_ETHER_COST {
                return Err(format!("乙太不足（前往翠幽星需要 {} 乙太）", TRAVEL_ETHER_COST));
            }
            let biome_weapons = [
                ItemKind::MeadowAmulet,
                ItemKind::MushroomStaff,
                ItemKind::CrystalBlade,
                ItemKind::RuneBlade,
                ItemKind::CoralLance,
            ];
            if !biome_weapons.iter().all(|w| self.inventory.count(*w) > 0) {
                return Err("需要五大生態武裝全套才能啟動星際旅行".into());
            }
            Ok(())
        } else if dest == PLANET_HOME
            && (self.planet == PLANET_VERDANT
                || self.planet == PLANET_CRIMSON
                || self.planet == PLANET_VOID
                || self.planet == PLANET_AETHER)
        {
            // 翠幽星 / 赤焰星 / 虛空星 / 霧醚星 → 故鄉：只需 30 乙太。
            if self.ether < TRAVEL_ETHER_COST {
                return Err(format!("乙太不足（返回故鄉需要 {} 乙太）", TRAVEL_ETHER_COST));
            }
            Ok(())
        } else if dest == PLANET_CRIMSON && self.planet == PLANET_HOME {
            // 故鄉 → 赤焰星：需持有翠幽碎片（證明踏上過翠幽星）+ 50 乙太。
            if self.ether < TRAVEL_ETHER_COST_CRIMSON {
                return Err(format!("乙太不足（前往赤焰星需要 {} 乙太）", TRAVEL_ETHER_COST_CRIMSON));
            }
            if self.inventory.count(ItemKind::JadeShard) == 0 {
                return Err("需要持有翠幽碎片才能找到赤焰星的星際航道（先探索翠幽星）".into());
            }
            Ok(())
        } else if dest == PLANET_VOID && (self.planet == PLANET_HOME || self.planet == PLANET_CRIMSON) {
            // 故鄉 / 赤焰星 → 虛空星：需持有熔晶碎片（證明踏上過赤焰星）+ 80 乙太。
            if self.ether < TRAVEL_ETHER_COST_VOID {
                return Err(format!("乙太不足（前往虛空星需要 {} 乙太）", TRAVEL_ETHER_COST_VOID));
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
            if self.ether < TRAVEL_ETHER_COST_AETHER {
                return Err(format!("乙太不足（前往霧醚星需要 {} 乙太）", TRAVEL_ETHER_COST_AETHER));
            }
            if self.inventory.count(ItemKind::VoidShard) == 0 {
                return Err("需要持有虛空碎片才能找到霧醚星的星際航道（先探索虛空星）".into());
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
        let mut dx = 0.0;
        let mut dy = 0.0;
        if self.input.up {
            dy -= 1.0;
        }
        if self.input.down {
            dy += 1.0;
        }
        if self.input.left {
            dx -= 1.0;
        }
        if self.input.right {
            dx += 1.0;
        }
        // 對角線正規化，避免斜走變快。
        if dx != 0.0 && dy != 0.0 {
            let inv = 1.0 / (2.0_f32).sqrt();
            dx *= inv;
            dy *= inv;
        }
        // ③ 無限世界：不再 clamp 到世界邊界，但水域擋路——用 resolve_move 做滑動碰撞，
        // 玩家撞到水邊能沿岸滑行；已在水裡（如舊存檔）仍可逃出（resolve_move 的「受困放行」保證）。
        // C-3：實心格也擋路。策略：
        //   - 「中心落在實心格」→ 受困（傳送/生成落地等罕見情況），以中心點判斷逃脫、允許自由移動。
        //   - 一般走路時用碰撞盒四角（半徑 PLAYER_TILE_RADIUS）判斷是否碰牆，精準阻擋、可沿牆滑行。
        // 重點：不能讓四角落到牆上就觸發「受困逃脫」，否則玩家靠近牆即可穿牆。
        let new_x = self.x + dx * PLAYER_SPEED * dt;
        let new_y = self.y + dy * PLAYER_SPEED * dt;
        let r = PLAYER_TILE_RADIUS;
        let is_center_stuck = tile_solid(self.x, self.y);
        // 是否已身陷水中（中心在水）——舊存檔/被推入等罕見情況，保留逃脫通道。
        let is_on_water = biome_at(self.x as f64, self.y as f64) == Biome::Water;
        let corners = [(r, r), (-r, r), (r, -r), (-r, -r)];
        let any_corner = |cx: f32, cy: f32| {
            corners.iter().any(|&(ox, oy)| tile_solid(cx + ox, cy + oy))
        };
        // 水域也用四角判定（與地形碰撞一致）：身體邊緣碰到水就擋，不再讓玩家中心壓到水邊、
        // 露出半個身體站在水上（先前水域只判中心 → 身體會凸進水裡，看起來像走在水上）。
        let water_corner = |cx: f32, cy: f32| {
            corners
                .iter()
                .any(|&(ox, oy)| biome_at((cx + ox) as f64, (cy + oy) as f64) == Biome::Water)
        };
        (self.x, self.y) = resolve_move(self.x, self.y, new_x, new_y, |x, y| {
            // 一般時水域用四角；已陷在水裡時改用中心，留逃脫通道、不卡死。
            let water_blocked = if is_on_water {
                biome_at(x as f64, y as f64) == Biome::Water
            } else {
                water_corner(x, y)
            };
            if water_blocked {
                return true;
            }
            // 受困時以中心點判定（保留逃脫通道）；一般時以四角判定（精準碰牆）。
            if is_center_stuck { tile_solid(x, y) } else { any_corner(x, y) }
        });
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
    ) -> Self {
        let (tx, _rx) = broadcast::channel(256);
        // 聊天頻道：量極低、給足緩衝，正常使用幾乎不會 Lagged。
        let (tx_chat, _rx_chat) = broadcast::channel(256);
        // 啟動時把上次存的農地與地塊歸屬種回權威狀態（無存檔時等同全新的空 map / next=0）。
        let plots = PlotRegistry::from_saved(field_store.saved_plots());
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
        assert_eq!(
            app.players.read().unwrap().get(&uid).unwrap().view().name,
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
        assert!(p.can_travel_to(PLANET_VERDANT).is_err(), "乙太不足應拒絕旅行");
    }

    #[test]
    fn travel_home_to_verdant_requires_all_biome_weapons() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST + 100;
        // 只放 4 件，少 1 件。
        let weapons = all_biome_weapons();
        for w in &weapons[..4] { p.inventory.add(*w, 1); }
        assert!(p.can_travel_to(PLANET_VERDANT).is_err(), "武裝未齊應拒絕旅行");
    }

    #[test]
    fn travel_home_to_verdant_succeeds_with_weapons_and_ether() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST;
        for w in all_biome_weapons() { p.inventory.add(w, 1); }
        assert!(p.can_travel_to(PLANET_VERDANT).is_ok(), "武裝齊 + 乙太足應允許旅行");
    }

    #[test]
    fn travel_verdant_to_home_only_requires_ether() {
        let mut p = player_at(VERDANT_SPAWN_X, VERDANT_SPAWN_Y, Input::default());
        p.planet = PLANET_VERDANT.to_string();
        p.ether = TRAVEL_ETHER_COST;
        // 不需要武器即可返回。
        assert!(p.can_travel_to(PLANET_HOME).is_ok(), "翠幽星→故鄉只需乙太");
    }

    #[test]
    fn travel_already_on_same_planet_is_error() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST + 100;
        for w in all_biome_weapons() { p.inventory.add(w, 1); }
        // 已在故鄉，嘗試去故鄉。
        assert!(p.can_travel_to(PLANET_HOME).is_err(), "已在故鄉不能再去故鄉");
    }

    // ROADMAP 22 — 赤焰星旅行條件測試。
    #[test]
    fn travel_home_to_crimson_requires_ether() {
        use crate::inventory::ItemKind;
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_CRIMSON - 1;
        p.inventory.add(ItemKind::JadeShard, 1);
        assert!(p.can_travel_to(PLANET_CRIMSON).is_err(), "赤焰星乙太不足應拒絕旅行");
    }

    #[test]
    fn travel_home_to_crimson_requires_jade_shard() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_CRIMSON + 10;
        // 沒有翠幽碎片（未探索過翠幽星）。
        assert!(p.can_travel_to(PLANET_CRIMSON).is_err(), "無翠幽碎片應拒絕赤焰星旅行");
    }

    #[test]
    fn travel_home_to_crimson_succeeds_with_jade_shard_and_ether() {
        use crate::inventory::ItemKind;
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_CRIMSON;
        p.inventory.add(ItemKind::JadeShard, 1);
        assert!(p.can_travel_to(PLANET_CRIMSON).is_ok(), "翠幽碎片 + 乙太足應允許赤焰星旅行");
    }

    #[test]
    fn travel_crimson_to_home_only_requires_ether() {
        let mut p = player_at(CRIMSON_SPAWN_X, CRIMSON_SPAWN_Y, Input::default());
        p.planet = PLANET_CRIMSON.to_string();
        p.ether = TRAVEL_ETHER_COST;
        assert!(p.can_travel_to(PLANET_HOME).is_ok(), "赤焰星→故鄉只需 30 乙太");
    }

    #[test]
    fn travel_home_to_aether_fails_with_insufficient_ether() {
        use crate::inventory::ItemKind;
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_AETHER - 1;
        p.inventory.add(ItemKind::VoidShard, 1);
        assert!(p.can_travel_to(PLANET_AETHER).is_err(), "乙太不足應拒絕霧醚星旅行");
    }

    #[test]
    fn travel_home_to_aether_fails_without_void_shard() {
        let mut p = player_at(0.0, 0.0, Input::default());
        p.ether = TRAVEL_ETHER_COST_AETHER;
        assert!(p.can_travel_to(PLANET_AETHER).is_err(), "無虛空碎片應拒絕霧醚星旅行");
    }

    #[test]
    fn travel_void_to_aether_succeeds_with_void_shard_and_ether() {
        use crate::inventory::ItemKind;
        let mut p = player_at(0.0, 0.0, Input::default());
        p.planet = PLANET_VOID.to_string();
        p.ether = TRAVEL_ETHER_COST_AETHER;
        p.inventory.add(ItemKind::VoidShard, 1);
        assert!(p.can_travel_to(PLANET_AETHER).is_ok(), "虛空碎片 + 乙太足應允許霧醚星旅行");
    }

    #[test]
    fn travel_aether_to_home_only_requires_ether() {
        let mut p = player_at(AETHER_SPAWN_X, AETHER_SPAWN_Y, Input::default());
        p.planet = PLANET_AETHER.to_string();
        p.ether = TRAVEL_ETHER_COST;
        assert!(p.can_travel_to(PLANET_HOME).is_ok(), "霧醚星→故鄉只需 30 乙太");
    }
}
