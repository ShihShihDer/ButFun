//! 伺服器的共享狀態：權威世界、玩家清單、廣播頻道。
//!
//! 目前狀態存在記憶體裡。持久化（Postgres）刻意藏在這層之後——之後把 `players`
//! 換成「啟動時從 DB 載入、變動時寫回」即可，不用動 WebSocket / 遊戲迴圈的程式。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::auth::AuthConfig;
use crate::connections::ConnectionCounts;
use crate::daynight::DayNight;
use crate::daynight_store::DayNightStore;
use crate::enemy_field::EnemyField;
use crate::field::Field;
use crate::field_store::FieldStore;
use crate::gather_field::NodeField;
use crate::inventory::Inventory;
use crate::inventory_store::InventoryStore;
use crate::vitals::Vitals;
use crate::plot_registry::PlotRegistry;
use crate::positions::PositionStore;
use crate::protocol::{ItemStack, PlayerView, WorldInfo};
use crate::suggestions::SuggestionStore;
use crate::users::UserStore;

/// 世界大小（像素）。放大成大世界,讓玩家散得開、各自有空間(回應「農地都擠中央、地圖太小」)。
pub const WORLD_WIDTH: f32 = 6000.0;
pub const WORLD_HEIGHT: f32 = 6000.0;
/// 玩家移動速度（像素 / 秒）。大世界一併調快,跨圖不至於太久。
pub const PLAYER_SPEED: f32 = 320.0;

/// 一名玩家在伺服器上的權威狀態。
#[derive(Debug, Clone)]
pub struct Player {
    pub id: Uuid,
    pub name: String,
    pub species: String,
    pub x: f32,
    pub y: f32,
    pub input: Input,
    /// 收成累積的乙太。已登入玩家重連會帶回（記憶體前置），跨伺服器重啟才歸零（待 Phase 0-E 持久化）。
    pub ether: u32,
    /// 採集到的物品。記憶體前置（重連不帶回、重啟歸零,待持久化）——薄切片先讓「採到 → 進背包 → 看得見」成立。
    pub inventory: Inventory,
    /// 生命值（戰鬥 1-F）。敵人反擊扣血、離戰一陣子自動回復;歸零會「被打趴」短暫休息。記憶體前置。
    pub vitals: Vitals,
}

impl Player {
    pub fn view(&self) -> PlayerView {
        PlayerView {
            id: self.id,
            name: self.name.clone(),
            species: self.species.clone(),
            x: self.x,
            y: self.y,
            ether: self.ether,
            inventory: self
                .inventory
                .entries()
                .map(|(item, qty)| ItemStack { item, qty })
                .collect(),
            hp: self.vitals.hp(),
            max_hp: self.vitals.max_hp(),
        }
    }

    /// 依目前輸入意圖，把位置往前推進 `dt` 秒（權威整合，含邊界夾制）。
    /// 抽成純函式以便自動測試。
    pub fn step(&mut self, dt: f32) {
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
        self.x = (self.x + dx * PLAYER_SPEED * dt).clamp(0.0, WORLD_WIDTH);
        self.y = (self.y + dy * PLAYER_SPEED * dt).clamp(0.0, WORLD_HEIGHT);
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
    /// 廣播頻道：高頻 tick 快照與 `PlayerLeft` 走這裡，內容是已序列化的 JSON 字串
    /// （只序列化一次，再扇出給所有連線）。這條會被 15Hz 快照灌滿，跟不上的客戶端
    /// 收到 `Lagged` 時丟掉舊快照繼續追即可——快照本身自我修正（含「移除缺席玩家」），
    /// 漏幾張無害。
    pub tx: broadcast::Sender<String>,
    /// 聊天專用廣播頻道，刻意與高頻快照分開。聊天是「一次性事件」：客戶端漏掉就永久
    /// 看不到那行。先前聊天和快照共用一條，手機 Lagged（網路抖／分頁背景）追快照時
    /// 會把同段時間捲過的聊天一起丟掉——延續「Lagged 不踢人」修復後浮現的缺口。
    /// 分開後聊天量極低、幾乎不可能 Lagged，廣播得以可靠送達。
    pub tx_chat: broadcast::Sender<String>,
    /// 遊戲內建議箱（玩家回饋迴圈的伺服器端）。
    pub suggestions: SuggestionStore,
    /// 使用者帳號(provider 無關)。
    pub users: UserStore,
    /// 玩家最後位置記憶(Phase 0-E 記憶體前置):已登入玩家重連回到離線前位置。
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
}

impl AppState {
    /// 無 DB 模式（測試、本機 `cargo run`）：位置/背包/農地/日夜走 JSONL 退回層（見各自的 `new`）。
    pub fn new() -> Self {
        Self::with_stores(
            PositionStore::new(),
            InventoryStore::new(),
            FieldStore::new(),
            DayNightStore::new(),
        )
    }

    /// 用已備好的位置 / 背包 / 農地 / 日夜 store 建狀態。`main` 連好 Postgres 後會傳入 DB-backed 的
    /// store（見各自的 `from_pool`）,其餘狀態不變。農地 store 同時種回兩份權威狀態:`fields`
    /// （每塊地、origin 已 reseat 好）與 `plots`（序號歸屬），讓重啟後農地與地塊歸屬都還在；
    /// 日夜 store 種回上次的世界時刻（沒存檔時為破曉）。
    pub fn with_stores(
        positions: PositionStore,
        inventories: InventoryStore,
        field_store: FieldStore,
        daynight_store: DayNightStore,
    ) -> Self {
        let (tx, _rx) = broadcast::channel(256);
        // 聊天頻道：量極低、給足緩衝，正常使用幾乎不會 Lagged。
        let (tx_chat, _rx_chat) = broadcast::channel(256);
        // 啟動時把上次存的農地與地塊歸屬種回權威狀態（無存檔時等同全新的空 map / next=0）。
        let plots = PlotRegistry::from_saved(field_store.saved_plots());
        let fields = field_store.loaded_fields();
        // 把上次存的世界時刻種回權威時鐘（無存檔時等同破曉 `DayNight::new()`）。
        let daynight = daynight_store.loaded();
        Self {
            players: Arc::new(RwLock::new(HashMap::new())),
            fields: Arc::new(RwLock::new(fields)),
            plots,
            daynight: Arc::new(RwLock::new(daynight)),
            nodes: Arc::new(RwLock::new(NodeField::new())),
            enemies: Arc::new(RwLock::new(EnemyField::new())),
            tx,
            tx_chat,
            suggestions: SuggestionStore::new(),
            users: UserStore::new(),
            positions,
            inventories,
            field_store,
            daynight_store,
            connections: ConnectionCounts::new(),
            auth: AuthConfig::from_env(),
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
        }
    }

    #[test]
    fn moves_right_at_expected_speed() {
        let mut p = player_at(
            100.0,
            100.0,
            Input {
                right: true,
                ..Default::default()
            },
        );
        p.step(1.0); // 一秒
        assert!((p.x - (100.0 + PLAYER_SPEED)).abs() < 0.001);
        assert!((p.y - 100.0).abs() < 0.001);
    }

    #[test]
    fn diagonal_is_not_faster() {
        let mut p = player_at(
            500.0,
            500.0,
            Input {
                right: true,
                down: true,
                ..Default::default()
            },
        );
        p.step(1.0);
        let dist = (((p.x - 500.0).powi(2)) + ((p.y - 500.0).powi(2))).sqrt();
        // 對角線位移量應約等於單軸速度，而非 sqrt(2) 倍。
        assert!((dist - PLAYER_SPEED).abs() < 0.01, "dist={dist}");
    }

    #[test]
    fn clamped_to_world_bounds() {
        let mut p = player_at(
            5.0,
            5.0,
            Input {
                up: true,
                left: true,
                ..Default::default()
            },
        );
        p.step(1.0);
        assert!(p.x >= 0.0 && p.y >= 0.0);
    }

    #[test]
    fn idle_player_stays_put() {
        let mut p = player_at(300.0, 300.0, Input::default());
        p.step(1.0);
        assert_eq!(p.x, 300.0);
        assert_eq!(p.y, 300.0);
    }

    #[test]
    fn chat_and_snapshot_channels_are_independent() {
        // 聊天與快照走不同廣播頻道：高頻快照灌滿 tx 造成 Lagged 時，不會把聊天一起丟。
        // 這裡驗證兩條頻道彼此隔離——各自的訂閱者只收到自己頻道的訊息，不會串流。
        let app = AppState::new();
        let mut rx_snap = app.tx.subscribe();
        let mut rx_chat = app.tx_chat.subscribe();
        app.tx_chat.send("聊天".to_string()).unwrap();
        app.tx.send("快照".to_string()).unwrap();
        // 聊天訂閱者只拿到聊天，沒有快照混進來。
        assert_eq!(rx_chat.try_recv().unwrap(), "聊天");
        assert!(rx_chat.try_recv().is_err());
        // 快照訂閱者只拿到快照，沒有聊天混進來。
        assert_eq!(rx_snap.try_recv().unwrap(), "快照");
        assert!(rx_snap.try_recv().is_err());
    }
}
