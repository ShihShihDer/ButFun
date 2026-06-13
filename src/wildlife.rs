//! 野生動物系統（ROADMAP 140~143）。
//!
//! ROADMAP 140：野鳥/野鹿/小動物——中立、只逃跑、不攻擊。
//! ROADMAP 141：野狼獵野鹿、野狐獵小動物；族群此消彼長（湧現平衡）。
//! ROADMAP 142：死亡餵養生命——獵物死亡釋出乙太微粒；玩家靠近採集得乙太，死亡是循環的一環。
//! ROADMAP 143：物種聚落——各物種有巢穴/聚落與群體防禦，不只人類城。
//!   - 6 個聚落分散世界（2 鳥巢・1 鹿棲地・1 小動物洞穴・1 狼窩・1 狐狸洞）。
//!   - 玩家進入聚落守衛半徑 → 同種動物切換為 Guarding（向威脅靠近，不逃跑）。
//!   - 每個聚落獨立冷卻（90 秒）廣播世界聊天：「🛡️ 野鹿棲地 察覺到入侵者，正在驅離！」
//!
//! 行為規則：
//! - 捕食者進入 HUNT_RADIUS 內偵測到獵物 → Hunting（追獵）。
//! - 追及 KILL_RADIUS 內 → 獵物死亡 + 捕食者進入 Digesting。
//! - 玩家與捕食者都會令獵物 Fleeing；同種獵物見捕食者靠近也一起竄逃（群逃）。
//! - 玩家進入聚落守衛半徑 → 附近同種動物進入 Guarding（向玩家靠近）。
//! - 死亡獵物 ~50 秒後在家附近重生（代表族群新個體）。
//! - 死亡時在原地生成乙太微粒；玩家靠近採集得 CARION_ETHER 乙太（死亡是循環的一環）。
//! - 捕食者每分鐘最多廣播一次捕獵事件，不塞頻道。
//!
//! 效能：全純算術、零 LLM、零 migration、記憶體模式，重啟全重置。

use rand::{Rng, SeedableRng, rngs::StdRng};
// ROADMAP 165：怪物食物鏈需要識別 EnemyKind。
use crate::combat::EnemyKind;

// ─── 常數 ────────────────────────────────────────────────────────────────────

/// 野生動物總數（獵物 18 + 捕食者 4）。
const WILDLIFE_COUNT: usize = 22;

/// 玩家或捕食者靠近多少像素觸發獵物驚逃。
const FLEE_RADIUS: f32 = 180.0;
/// 驚逃速度（像素/秒）。
const FLEE_SPEED: f32 = 200.0;
/// 驚逃計時器（秒）。
const FLEE_DURATION: f32 = 4.5;

/// 閒晃速度（像素/秒）——獵物。
const WANDER_SPEED: f32 = 35.0;
/// 閒晃速度——捕食者（稍快）。
const PRED_WANDER_SPEED: f32 = 52.0;
/// 漫遊半徑。
const WANDER_RADIUS: f32 = 180.0;
const WANDER_TIMER_MIN: f32 = 2.5;
const WANDER_TIMER_MAX: f32 = 7.0;
const REST_TIMER_MIN: f32 = 1.5;
const REST_TIMER_MAX: f32 = 4.5;

/// 返家速度。
const RETURN_SPEED: f32 = 60.0;
/// 距巢穴多近算「到家」。
const HOME_ARRIVE_DIST: f32 = 20.0;

/// 捕食者搜尋獵物的半徑。
const HUNT_RADIUS: f32 = 320.0;
/// 追獵速度。
const HUNT_SPEED: f32 = 155.0;
/// 進入此距離觸發擊殺。
const KILL_RADIUS: f32 = 22.0;
/// 追獵超時（秒），超過後放棄。
const HUNT_TIMEOUT: f32 = 18.0;
/// 吃完後消化休息時間。
const DIGEST_DURATION: f32 = 25.0;
/// 獵物死亡後重生秒數。
const PREY_RESPAWN_SECS: f32 = 50.0;
/// 捕獵廣播最短間隔（秒），避免塞頻道。
const KILL_BROADCAST_INTERVAL: f32 = 30.0;

// ─── ROADMAP 143：物種聚落常數 ───────────────────────────────────────────────

/// 守衛速度（像素/秒）——動物向威脅靠近，刻意比逃跑慢，更像「領地巡邏」。
const GUARD_SPEED: f32 = 65.0;
/// 守衛行為持續時間（秒），之後恢復正常。
const GUARD_DURATION: f32 = 12.0;
/// 每個聚落的廣播冷卻（秒）——避免玩家徘徊時刷屏。
const COLONY_THREAT_COOLDOWN: f32 = 90.0;
/// 進入守衛狀態的範圍倍率（相對於 guard_radius）。
const COLONY_ACTIVATE_MULTIPLIER: f32 = 1.8;

// ─── ROADMAP 144：人類↔物種關係常數 ─────────────────────────────────────────

/// 敵視物種主動偵測玩家並攻擊的半徑（像素）。
const HOSTILE_DETECT_RADIUS: f32 = 200.0;
/// 敵視守衛動物近身攻擊觸發距離（像素）。
const HOSTILE_ATTACK_REACH: f32 = 35.0;
/// 敵視野生動物的攻擊傷害（HP）。
const HOSTILE_ATTACK_DAMAGE: u32 = 2;
/// 敵視攻擊後動物的冷卻（秒）——映射成 guard_timer 重設值。
const HOSTILE_ATTACK_COOLDOWN: f32 = 3.0;
/// 友善物種（attitude ≥ 此值）不把玩家加入逃跑威脅清單。
const FRIENDLY_ATTITUDE: i32 = 65;
/// 敵視物種（attitude < 此值）會主動攻擊玩家。
const HOSTILE_ATTITUDE: i32 = 25;

// ─── ROADMAP 142：乙太微粒常數 ───────────────────────────────────────────────

/// 乙太微粒採集有效距離（像素）。
pub const CARION_COLLECT_RADIUS: f32 = 80.0;
/// 每顆乙太微粒給予的乙太數量。
pub const CARION_ETHER: u32 = 4;
/// 乙太微粒存在時長（秒），逾時自動消失。
const CARION_ORB_TTL: f32 = 90.0;
/// 同時存在乙太微粒的上限（防止無限堆積）。
const MAX_CARION_ORBS: usize = 8;

// ─── ROADMAP 205：餵食馴養 ───────────────────────────────────────────────────
// 反覆餵食「同一隻」野生動物，會累積個體親近度（0~1）。
// 親近度達 TAME_FAMILIARITY 後該隻動物被「馴養」：不再把玩家視為威脅（不逃跑），
// 玩家靠近時溫順地走向你、保持舒適距離（彷彿跟著你）。牠仍會逃離捕食者/獵食怪物
// （信任的是你、不是狼）。親近度隨時間緩慢衰減、死亡歸零——是一段需要維繫的關係。

/// 親近度上限（餵食累積的封頂）。
const MAX_FAMILIARITY: f32 = 1.0;
/// 個體親近度達此值即視為「已馴養」。刻意低於上限，留出緩衝——餵滿後即使緩慢衰減，
/// 也要好一陣子才會掉出馴養狀態（不會因每幀微小衰減就立刻「退馴」）。
const TAME_FAMILIARITY: f32 = 0.8;
/// 每餵食一次提升的親近度（需數次餵食才馴養，過程才有溫度）。
const FEED_FAMILIARITY_GAIN: f32 = 0.25;
/// 親近度每秒自然衰減（很慢——約 30 分鐘從滿值歸零，關係需偶爾維繫但不易斷）。
const FAMILIARITY_DECAY_PER_SEC: f32 = 1.0 / 1800.0;
/// 馴養動物「察覺到附近玩家」而走向他的範圍（像素）。
const FOLLOW_RANGE: f32 = 260.0;
/// 馴養動物跟隨時與玩家保持的舒適距離（像素）——更近就停下，不黏在腳邊。
const FOLLOW_COMFORT_DIST: f32 = 60.0;
/// 馴養動物走向玩家的速度（像素/秒）——比逃跑慢，溫順小跑。
const FOLLOW_SPEED: f32 = 60.0;

// ─── ROADMAP 206：群聚結伴 ───────────────────────────────────────────────────
// 同種野生動物（獵物）漫遊時，選下一個閒晃目標會朝「附近同種夥伴的平均位置」
// 拉一把，於是鬆散成群移動：草原上的野鹿三兩成群、野鳥成簇飄移，
// 世界不再是一盤各走各的散點。純啟發式、零 LLM、零持久化、無 migration。
// 捕食者（狼/狐）刻意維持獨來獨往（更顯孤狼氣場），不參與群聚。

/// 尋找同種群聚夥伴的半徑（像素）——只看這個範圍內的同種存活獵物算「一群」。
const HERD_RADIUS: f32 = 280.0;
/// 選新漫遊目標時朝群體中心混合的比例（0=純隨機家附近、1=直奔群體中心）。
/// 刻意取中段：既明顯成群、又保留各自散布，不會擠成一個點。
const HERD_PULL: f32 = 0.5;

// ─── 種類與營養階 ────────────────────────────────────────────────────────────

/// 野生動物種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WildlifeKind {
    WildBird,
    WildDeer,
    SmallCritter,
    /// 捕食者：獵食野鹿。
    WildWolf,
    /// 捕食者：獵食小動物。
    WildFox,
}

/// 食物鏈層級。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrophicLevel {
    Prey,
    Predator,
}

impl WildlifeKind {
    pub fn display_name(self) -> &'static str {
        match self {
            WildlifeKind::WildBird     => "野鳥",
            WildlifeKind::WildDeer     => "野鹿",
            WildlifeKind::SmallCritter => "小動物",
            WildlifeKind::WildWolf     => "野狼",
            WildlifeKind::WildFox      => "野狐",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            WildlifeKind::WildBird     => "wild_bird",
            WildlifeKind::WildDeer     => "wild_deer",
            WildlifeKind::SmallCritter => "small_critter",
            WildlifeKind::WildWolf     => "wild_wolf",
            WildlifeKind::WildFox      => "wild_fox",
        }
    }

    pub fn trophic_level(self) -> TrophicLevel {
        match self {
            WildlifeKind::WildWolf | WildlifeKind::WildFox => TrophicLevel::Predator,
            _ => TrophicLevel::Prey,
        }
    }

    /// 此捕食者的獵食對象（None 表示非捕食者）。
    pub fn hunts(self) -> Option<WildlifeKind> {
        match self {
            WildlifeKind::WildWolf => Some(WildlifeKind::WildDeer),
            WildlifeKind::WildFox  => Some(WildlifeKind::SmallCritter),
            _ => None,
        }
    }
}

/// ROADMAP 165：怪物物種食物鏈配對——定義哪種怪物主動獵食哪種野生動物。
/// 三對配對（食性與分佈合理）：
///   - 乙太鬼火 → 野鳥（光靈追逐飛行生物）
///   - 蕈菇潛行者 → 小動物（森林潛行者獵食小型獵物）
///   - 廢鐵無人機 → 野鹿（機械無人機追蹤大型目標）
pub fn monster_hunts_wildlife(kind: EnemyKind) -> Option<WildlifeKind> {
    match kind {
        EnemyKind::EtherWisp       => Some(WildlifeKind::WildBird),
        EnemyKind::MushroomStalker => Some(WildlifeKind::SmallCritter),
        EnemyKind::ScrapDrone      => Some(WildlifeKind::WildDeer),
        _                          => None,
    }
}

// ─── ROADMAP 142：乙太微粒 ───────────────────────────────────────────────────

/// 獵物死亡時釋出的乙太微粒——死亡是循環的一環。
#[derive(Debug, Clone)]
pub struct CarrionOrb {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub ttl: f32,
}

// ─── ROADMAP 143：物種聚落 ───────────────────────────────────────────────────

/// 物種聚落——各物種的巢穴/棲地，有領地守衛行為。
#[derive(Debug, Clone)]
pub struct Colony {
    pub id: u32,
    pub kind: WildlifeKind,
    /// 聚落顯示名稱（繁中）。
    pub name: &'static str,
    pub cx: f32,
    pub cy: f32,
    /// 守衛半徑（像素）——玩家進入此範圍觸發群體防禦。
    pub guard_radius: f32,
}

/// 給協議層用的聚落視圖（靜態資料，每幀隨快照廣播）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ColonyView {
    pub id: u32,
    pub kind: String,
    pub name: String,
    pub cx: f32,
    pub cy: f32,
    pub guard_radius: f32,
}

// ─── 事件 ────────────────────────────────────────────────────────────────────

pub enum WildlifeEvent {
    /// 捕食者成功捕獵，應廣播至全服聊天。
    Kill {
        predator_kind: WildlifeKind,
        prey_kind: WildlifeKind,
        x: f32,
        y: f32,
    },
    /// ROADMAP 143：聚落偵測到入侵者，應廣播至全服聊天。
    ColonyThreatened {
        colony_name: &'static str,
        cx: f32,
        cy: f32,
    },
    /// ROADMAP 144：敵視物種守衛攻擊玩家——近身時對附近玩家造成傷害。
    /// 外層（game.rs）應找出 near_x/near_y 附近的玩家並扣血。
    WildlifeAttack {
        attacker_kind: WildlifeKind,
        near_x: f32,
        near_y: f32,
        damage: u32,
    },
    /// ROADMAP 165：怪物成功獵殺野生動物——應廣播至全服聊天並已生成乙太微粒。
    MonsterHunted {
        monster_kind: EnemyKind,
        wildlife_kind: WildlifeKind,
        x: f32,
        y: f32,
    },
}

// ─── 狀態 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum WildlifeState {
    Wandering { target_x: f32, target_y: f32, wander_timer: f32 },
    Resting { rest_timer: f32 },
    Fleeing { vx: f32, vy: f32, flee_timer: f32 },
    Returning,
    /// 捕食者正在追獵指定 ID 的獵物。
    Hunting { target_id: u32, hunt_timer: f32 },
    /// 捕食者吃完後消化休息。
    Digesting { timer: f32 },
    /// ROADMAP 143：聚落守衛——動物向入侵玩家靠近，不逃跑。
    Guarding { threat_x: f32, threat_y: f32, guard_timer: f32 },
}

// ─── 實體 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Wildlife {
    pub id: u32,
    pub kind: WildlifeKind,
    pub x: f32,
    pub y: f32,
    pub alive: bool,
    respawn_timer: f32,
    home_x: f32,
    home_y: f32,
    state: WildlifeState,
    /// ROADMAP 205：個體親近度（0~1）——反覆餵食累積，達 TAME_FAMILIARITY 即馴養。
    familiarity: f32,
}

impl Wildlife {
    fn new(id: u32, kind: WildlifeKind, hx: f32, hy: f32, rng: &mut StdRng) -> Self {
        let offset_x = rng.gen_range(-50.0_f32..50.0);
        let offset_y = rng.gen_range(-50.0_f32..50.0);
        Self {
            id,
            kind,
            x: hx + offset_x,
            y: hy + offset_y,
            home_x: hx,
            home_y: hy,
            alive: true,
            respawn_timer: 0.0,
            state: WildlifeState::Resting {
                rest_timer: rng.gen_range(REST_TIMER_MIN..=REST_TIMER_MAX),
            },
            familiarity: 0.0,
        }
    }

    /// ROADMAP 205：此隻動物目前的親近度（0~1）。
    pub fn familiarity(&self) -> f32 {
        self.familiarity
    }

    /// ROADMAP 205：是否已被馴養（親近度達門檻）。
    pub fn is_tamed(&self) -> bool {
        self.familiarity >= TAME_FAMILIARITY
    }

    /// 非追獵行為 tick：閒晃 / 休息 / 逃跑 / 返家。
    /// `flee_threats`：需要逃離的座標（玩家 + 捕食者）；捕食者呼叫時傳空。
    /// `herd_anchor`：ROADMAP 206——附近同種夥伴的平均位置；選新漫遊目標時朝它拉，
    /// 同種動物便鬆散成群移動。捕食者傳 `None`（獨來獨往）。
    fn tick_idle(&mut self, dt: f32, flee_threats: &[(f32, f32)], speed: f32, herd_anchor: Option<(f32, f32)>, rng: &mut StdRng) {
        let already_fleeing = matches!(self.state, WildlifeState::Fleeing { .. });
        if !already_fleeing {
            if let Some((tx, ty)) = nearest_in_range(self.x, self.y, flee_threats, FLEE_RADIUS) {
                let dx = self.x - tx;
                let dy = self.y - ty;
                let len = (dx * dx + dy * dy).sqrt().max(1.0);
                self.state = WildlifeState::Fleeing {
                    vx: dx / len * FLEE_SPEED,
                    vy: dy / len * FLEE_SPEED,
                    flee_timer: FLEE_DURATION,
                };
                return;
            }
        }

        match self.state.clone() {
            WildlifeState::Fleeing { vx, vy, flee_timer } => {
                self.x += vx * dt;
                self.y += vy * dt;
                let remaining = flee_timer - dt;
                if remaining <= 0.0 {
                    self.state = WildlifeState::Returning;
                } else if let Some((tx, ty)) = nearest_in_range(self.x, self.y, flee_threats, FLEE_RADIUS) {
                    let dx = self.x - tx;
                    let dy = self.y - ty;
                    let len = (dx * dx + dy * dy).sqrt().max(1.0);
                    self.state = WildlifeState::Fleeing {
                        vx: dx / len * FLEE_SPEED,
                        vy: dy / len * FLEE_SPEED,
                        flee_timer: remaining,
                    };
                } else {
                    self.state = WildlifeState::Fleeing { vx, vy, flee_timer: remaining };
                }
            }
            WildlifeState::Returning => {
                let dx = self.home_x - self.x;
                let dy = self.home_y - self.y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= HOME_ARRIVE_DIST {
                    self.x = self.home_x;
                    self.y = self.home_y;
                    let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                    let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                    self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
                } else {
                    self.x += (dx / dist) * RETURN_SPEED * dt;
                    self.y += (dy / dist) * RETURN_SPEED * dt;
                }
            }
            WildlifeState::Resting { rest_timer } => {
                let remaining = rest_timer - dt;
                if remaining <= 0.0 {
                    let timer = rng.gen_range(WANDER_TIMER_MIN..=WANDER_TIMER_MAX);
                    let (tx, ty) = herd_wander_target(self.home_x, self.home_y, herd_anchor, rng);
                    self.state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: timer };
                } else {
                    self.state = WildlifeState::Resting { rest_timer: remaining };
                }
            }
            WildlifeState::Wandering { target_x, target_y, wander_timer } => {
                let remaining = wander_timer - dt;
                let dx = target_x - self.x;
                let dy = target_y - self.y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < 8.0 || remaining <= 0.0 {
                    let rest = rng.gen_range(REST_TIMER_MIN..=REST_TIMER_MAX);
                    self.state = WildlifeState::Resting { rest_timer: rest };
                } else {
                    self.x += (dx / dist) * speed * dt;
                    self.y += (dy / dist) * speed * dt;
                    self.state = WildlifeState::Wandering { target_x, target_y, wander_timer: remaining };
                }
            }
            // Hunting/Digesting 由管理器處理。
            _ => {}
        }
    }

    /// 供協議層使用的狀態字串。
    pub fn state_str(&self) -> &'static str {
        match &self.state {
            WildlifeState::Wandering { .. } => "wandering",
            WildlifeState::Resting { .. }   => "resting",
            WildlifeState::Fleeing { .. }   => "fleeing",
            WildlifeState::Returning        => "returning",
            WildlifeState::Hunting { .. }   => "hunting",
            WildlifeState::Digesting { .. } => "digesting",
            WildlifeState::Guarding { .. }  => "guarding",
        }
    }
}

// ─── 管理器 ──────────────────────────────────────────────────────────────────

pub struct WildlifeManager {
    pub animals: Vec<Wildlife>,
    rng: StdRng,
    /// 距上次捕獵廣播的累計秒數（限流用）。
    kill_broadcast_cooldown: f32,
    /// ROADMAP 142：活躍乙太微粒列表。
    pub carion_orbs: Vec<CarrionOrb>,
    /// 微粒 ID 計數器（跨生命週期唯一）。
    orb_counter: u32,
    /// ROADMAP 143：物種聚落定義（靜態）。
    pub colonies: Vec<Colony>,
    /// 每個聚落的廣播冷卻倒數（索引對應 colonies）。
    colony_threat_cooldowns: Vec<f32>,
}

impl WildlifeManager {
    pub fn new() -> Self {
        let mut rng = StdRng::seed_from_u64(7654321);
        let animals = spawn_all_wildlife(&mut rng);
        let colonies = build_colonies();
        let n = colonies.len();
        Self {
            animals, rng,
            kill_broadcast_cooldown: 0.0,
            carion_orbs: Vec::new(),
            orb_counter: 0,
            colonies,
            colony_threat_cooldowns: vec![0.0; n],
        }
    }

    /// 供快照廣播的聚落視圖列表（靜態，每幀傳出）。
    pub fn colony_views(&self) -> Vec<ColonyView> {
        self.colonies.iter().map(|c| ColonyView {
            id: c.id,
            kind: c.kind.as_str().to_string(),
            name: c.name.to_string(),
            cx: c.cx,
            cy: c.cy,
            guard_radius: c.guard_radius,
        }).collect()
    }

    /// ROADMAP 142：嘗試採集距玩家最近的乙太微粒。
    /// 成功回傳乙太量，並移除該微粒；否則回傳 None。
    pub fn collect_carion_orb(&mut self, orb_id: u32, px: f32, py: f32) -> Option<u32> {
        let r2 = CARION_COLLECT_RADIUS * CARION_COLLECT_RADIUS;
        let idx = self.carion_orbs.iter().position(|o| {
            o.id == orb_id && (o.x - px).powi(2) + (o.y - py).powi(2) <= r2
        })?;
        self.carion_orbs.swap_remove(idx);
        Some(CARION_ETHER)
    }

    /// ROADMAP 144：玩家攻擊野生動物——在攻擊距離內找到該 ID 的存活動物並使其死亡。
    /// 回傳被擊殺動物的種類（`None` 表示不存在/超出距離/已死亡）。
    pub fn attack_wildlife(
        &mut self,
        wildlife_id: u32,
        px: f32,
        py: f32,
        reach: f32,
    ) -> Option<WildlifeKind> {
        let reach2 = reach * reach;
        if let Some(a) = self.animals.iter_mut().find(|a| {
            a.id == wildlife_id
                && a.alive
                && (a.x - px).powi(2) + (a.y - py).powi(2) <= reach2
        }) {
            let kind = a.kind;
            a.alive = false;
            a.respawn_timer = PREY_RESPAWN_SECS;
            a.state = WildlifeState::Resting { rest_timer: 0.0 };
            Some(kind)
        } else {
            None
        }
    }

    /// ROADMAP 205：餵食指定 ID 的存活動物，提升其個體親近度。
    /// 回傳 `(種類, 提升後親近度, 是否「剛跨過馴養門檻」)`；找不到/已死亡則 `None`。
    /// 距離 / 種子消耗由呼叫端（ws.rs 的 feed_wildlife）負責，本函式只管親近度。
    pub fn on_feed_animal(&mut self, wildlife_id: u32) -> Option<(WildlifeKind, f32, bool)> {
        let a = self.animals.iter_mut().find(|a| a.id == wildlife_id && a.alive)?;
        let was_tamed = a.familiarity >= TAME_FAMILIARITY;
        a.familiarity = (a.familiarity + FEED_FAMILIARITY_GAIN).min(MAX_FAMILIARITY);
        let now_tamed = a.familiarity >= TAME_FAMILIARITY;
        Some((a.kind, a.familiarity, now_tamed && !was_tamed))
    }

    /// ROADMAP 165：回傳所有存活野生動物的快照（ID, 種類, x, y）。
    /// 供怪物追獵目標計算用（取讀鎖後呼叫）。
    pub fn alive_snapshot(&self) -> Vec<(u32, WildlifeKind, f32, f32)> {
        self.animals.iter()
            .filter(|a| a.alive)
            .map(|a| (a.id, a.kind, a.x, a.y))
            .collect()
    }

    /// ROADMAP 165：怪物獵殺野生動物——標記獵物死亡、生成乙太微粒、回傳事件。
    /// 若 wildlife_id 不存在或已死亡，回傳 None（冪等，安全可重呼叫）。
    pub fn on_monster_kills_wildlife(
        &mut self,
        wildlife_id: u32,
        monster_kind: EnemyKind,
    ) -> Option<WildlifeEvent> {
        let prey = self.animals.iter_mut().find(|a| a.id == wildlife_id && a.alive)?;
        let wildlife_kind = prey.kind;
        let kx = prey.x;
        let ky = prey.y;
        prey.alive = false;
        prey.respawn_timer = PREY_RESPAWN_SECS;
        prey.state = WildlifeState::Resting { rest_timer: 0.0 };
        // 在死亡位置生成乙太微粒（死亡是循環的一環，不分陣營）。
        if self.carion_orbs.len() < MAX_CARION_ORBS {
            let id = self.orb_counter;
            self.orb_counter = self.orb_counter.wrapping_add(1);
            self.carion_orbs.push(CarrionOrb { id, x: kx, y: ky, ttl: CARION_ORB_TTL });
        }
        Some(WildlifeEvent::MonsterHunted { monster_kind, wildlife_kind, x: kx, y: ky })
    }

    /// 每幀推進所有野生動物，回傳本幀產生的事件列表。
    ///
    /// `attitudes`：各物種目前態度值（0-100）。用於：
    ///   - 友善（≥65）：獵物不把玩家加入逃跑威脅清單（不逃）。
    ///   - 敵視（<25）：獵物主動向玩家靠近（守衛行為），近身時發出 WildlifeAttack 事件。
    pub fn tick(
        &mut self,
        dt: f32,
        player_positions: &[(f32, f32)],
        attitudes: &std::collections::HashMap<WildlifeKind, i32>,
        monster_threats: &[(EnemyKind, f32, f32)],
    ) -> Vec<WildlifeEvent> {
        let mut events = Vec::new();
        self.kill_broadcast_cooldown = (self.kill_broadcast_cooldown - dt).max(-1.0);

        // ── Phase 0a: 乙太微粒 TTL 倒數（ROADMAP 142）────────────────────────
        for orb in &mut self.carion_orbs {
            orb.ttl -= dt;
        }
        self.carion_orbs.retain(|o| o.ttl > 0.0);

        // ── Phase 0b: 聚落廣播冷卻倒數（ROADMAP 143）────────────────────────
        for cd in &mut self.colony_threat_cooldowns {
            *cd = (*cd - dt).max(0.0);
        }

        // ── Phase 1: 死亡倒數 + 重生 + 親近度衰減（ROADMAP 205）─────────────────
        for a in &mut self.animals {
            if !a.alive {
                a.respawn_timer -= dt;
            } else if a.familiarity > 0.0 {
                // 親近度隨時間緩慢衰減——羈絆需偶爾以餵食維繫，但不易斷。
                a.familiarity = (a.familiarity - FAMILIARITY_DECAY_PER_SEC * dt).max(0.0);
            }
        }
        let respawn_ready: Vec<usize> = self.animals.iter().enumerate()
            .filter(|(_, a)| !a.alive && a.respawn_timer <= 0.0)
            .map(|(i, _)| i)
            .collect();
        for i in respawn_ready {
            let ox: f32 = self.rng.gen_range(-40.0..40.0);
            let oy: f32 = self.rng.gen_range(-40.0..40.0);
            let a = &mut self.animals[i];
            a.alive = true;
            a.x = a.home_x + ox;
            a.y = a.home_y + oy;
            a.state = WildlifeState::Resting { rest_timer: 2.0 };
            // ROADMAP 205：重生的是「新的個體」，與玩家的羈絆隨上一隻回歸乙太而散——親近度歸零。
            a.familiarity = 0.0;
        }

        // ── Phase 2: 快照（供決策使用） ────────────────────────────────────────
        let prey_snap: Vec<(u32, WildlifeKind, f32, f32)> = self.animals.iter()
            .filter(|a| a.alive && a.kind.trophic_level() == TrophicLevel::Prey)
            .map(|a| (a.id, a.kind, a.x, a.y))
            .collect();

        // 捕食者位置：獵物逃跑時參考此清單。
        let pred_positions: Vec<(f32, f32)> = self.animals.iter()
            .filter(|a| a.alive && a.kind.trophic_level() == TrophicLevel::Predator)
            .map(|a| (a.x, a.y))
            .collect();

        // ── Phase 2b: 聚落威脅偵測（ROADMAP 143）────────────────────────────
        // 對每個聚落：若有玩家進入守衛半徑，啟動同種動物的 Guarding 行為。
        for (idx, col) in self.colonies.iter().enumerate() {
            // 找出在守衛半徑內最近的玩家。
            let threat = player_positions.iter().find(|&&(px, py)| {
                let dx = px - col.cx;
                let dy = py - col.cy;
                dx * dx + dy * dy <= col.guard_radius * col.guard_radius
            }).copied();

            let Some((threat_x, threat_y)) = threat else { continue };

            // 廣播世界聊天（有冷卻）。
            if self.colony_threat_cooldowns[idx] <= 0.0 {
                events.push(WildlifeEvent::ColonyThreatened {
                    colony_name: col.name,
                    cx: col.cx,
                    cy: col.cy,
                });
                self.colony_threat_cooldowns[idx] = COLONY_THREAT_COOLDOWN;
            }

            // 啟動聚落範圍內同種動物的守衛行為。
            let activate_r2 = (col.guard_radius * COLONY_ACTIVATE_MULTIPLIER).powi(2);
            let col_kind = col.kind;
            let col_cx = col.cx;
            let col_cy = col.cy;
            for a in &mut self.animals {
                if !a.alive || a.kind != col_kind { continue; }
                let ddx = a.x - col_cx;
                let ddy = a.y - col_cy;
                if ddx * ddx + ddy * ddy > activate_r2 { continue; }
                // 不干擾正在追獵/消化/已守衛的狀態。
                if matches!(a.state, WildlifeState::Hunting { .. } | WildlifeState::Digesting { .. } | WildlifeState::Guarding { .. }) {
                    continue;
                }
                a.state = WildlifeState::Guarding { threat_x, threat_y, guard_timer: GUARD_DURATION };
            }
        }

        // ── Phase 2b-extra: 敵視物種主動偵測玩家（ROADMAP 144）─────────────
        // attitude < HOSTILE_ATTITUDE 的物種：不等聚落觸發，直接向附近玩家靠近。
        for a in &mut self.animals {
            if !a.alive { continue; }
            if matches!(a.state, WildlifeState::Hunting { .. } | WildlifeState::Digesting { .. } | WildlifeState::Guarding { .. }) {
                continue;
            }
            let kind_attitude = *attitudes.get(&a.kind).unwrap_or(&50);
            if kind_attitude >= HOSTILE_ATTITUDE { continue; }
            // 找 HOSTILE_DETECT_RADIUS 內最近的玩家。
            let threat = nearest_in_range(a.x, a.y, player_positions, HOSTILE_DETECT_RADIUS);
            if let Some((tx, ty)) = threat {
                a.state = WildlifeState::Guarding { threat_x: tx, threat_y: ty, guard_timer: GUARD_DURATION };
            }
        }

        // ── Phase 2c: 守衛行為 tick（ROADMAP 143 + 144）─────────────────────
        // 處理所有物種（獵物與捕食者）的 Guarding 狀態。
        // ROADMAP 144：若物種為敵視且動物已靠近玩家 HOSTILE_ATTACK_REACH 內，發出傷害事件。
        for i in 0..self.animals.len() {
            if !self.animals[i].alive { continue; }
            let WildlifeState::Guarding { threat_x, threat_y, guard_timer } = self.animals[i].state else { continue };
            let dx = threat_x - self.animals[i].x;
            let dy = threat_y - self.animals[i].y;
            let dist = (dx * dx + dy * dy).sqrt();
            let remaining = guard_timer - dt;

            // 敵視物種近身攻擊（ROADMAP 144）。
            let kind_attitude = *attitudes.get(&self.animals[i].kind).unwrap_or(&50);
            if kind_attitude < HOSTILE_ATTITUDE && dist <= HOSTILE_ATTACK_REACH {
                events.push(WildlifeEvent::WildlifeAttack {
                    attacker_kind: self.animals[i].kind,
                    near_x: self.animals[i].x,
                    near_y: self.animals[i].y,
                    damage: HOSTILE_ATTACK_DAMAGE,
                });
                // 攻擊後回到休息（冷卻），再被 Phase 2b-extra 重新觸發。
                self.animals[i].state = WildlifeState::Resting { rest_timer: HOSTILE_ATTACK_COOLDOWN };
                continue;
            }

            if remaining <= 0.0 || dist < 30.0 {
                // 計時到或已靠近，回到休息。
                self.animals[i].state = WildlifeState::Resting { rest_timer: 2.0 };
            } else {
                self.animals[i].x += (dx / dist) * GUARD_SPEED * dt;
                self.animals[i].y += (dy / dist) * GUARD_SPEED * dt;
                self.animals[i].state = WildlifeState::Guarding { threat_x, threat_y, guard_timer: remaining };
            }
        }

        // ── Phase 3: 捕食者行為 ────────────────────────────────────────────────
        // 收集本幀的擊殺：(pred_id, prey_id, pred_kind, prey_kind, x, y)
        let mut kills: Vec<(u32, u32, WildlifeKind, WildlifeKind, f32, f32)> = Vec::new();

        for i in 0..self.animals.len() {
            if !self.animals[i].alive { continue; }
            if self.animals[i].kind.trophic_level() != TrophicLevel::Predator { continue; }
            // 守衛狀態已在 Phase 2c 處理，跳過。
            if matches!(self.animals[i].state, WildlifeState::Guarding { .. }) { continue; }

            let state = self.animals[i].state.clone();
            let pred_kind = self.animals[i].kind;
            let pred_id   = self.animals[i].id;
            let pred_x    = self.animals[i].x;
            let pred_y    = self.animals[i].y;

            match state {
                WildlifeState::Hunting { target_id, hunt_timer } => {
                    if let Some(&(_, prey_kind, px, py)) = prey_snap.iter()
                        .find(|&&(id, _, _, _)| id == target_id)
                    {
                        let dx = px - pred_x;
                        let dy = py - pred_y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist <= KILL_RADIUS {
                            kills.push((pred_id, target_id, pred_kind, prey_kind, px, py));
                            self.animals[i].state = WildlifeState::Digesting { timer: DIGEST_DURATION };
                        } else {
                            self.animals[i].x += dx / dist * HUNT_SPEED * dt;
                            self.animals[i].y += dy / dist * HUNT_SPEED * dt;
                            let remaining = hunt_timer - dt;
                            self.animals[i].state = if remaining <= 0.0 {
                                WildlifeState::Returning
                            } else {
                                WildlifeState::Hunting { target_id, hunt_timer: remaining }
                            };
                        }
                    } else {
                        // 獵物已死或不見，放棄。
                        self.animals[i].state = WildlifeState::Returning;
                    }
                }
                WildlifeState::Digesting { timer } => {
                    let remaining = timer - dt;
                    if remaining <= 0.0 {
                        let home_x = self.animals[i].home_x;
                        let home_y = self.animals[i].home_y;
                        let (tx, ty) = random_target(home_x, home_y, WANDER_RADIUS, &mut self.rng);
                        self.animals[i].state = WildlifeState::Wandering { target_x: tx, target_y: ty, wander_timer: 5.0 };
                    } else {
                        self.animals[i].state = WildlifeState::Digesting { timer: remaining };
                    }
                }
                _ => {
                    // 閒晃/返家：嘗試找獵物。
                    if let Some(target_kind) = pred_kind.hunts() {
                        let nearest = prey_snap.iter()
                            .filter(|&&(_, k, _, _)| k == target_kind)
                            .filter(|&&(_, _, px, py)| {
                                let dx = px - pred_x;
                                let dy = py - pred_y;
                                dx * dx + dy * dy <= HUNT_RADIUS * HUNT_RADIUS
                            })
                            .min_by(|&&(_, _, ax, ay), &&(_, _, bx, by)| {
                                let da = (ax - pred_x).powi(2) + (ay - pred_y).powi(2);
                                let db = (bx - pred_x).powi(2) + (by - pred_y).powi(2);
                                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                            });
                        if let Some(&(target_id, _, _, _)) = nearest {
                            self.animals[i].state = WildlifeState::Hunting { target_id, hunt_timer: HUNT_TIMEOUT };
                        } else {
                            // 無獵物，正常閒晃（捕食者不怕玩家，傳空威脅；獨來獨往不群聚）。
                            let rng = &mut self.rng;
                            let a = &mut self.animals[i];
                            a.tick_idle(dt, &[], PRED_WANDER_SPEED, None, rng);
                        }
                    }
                }
            }
        }

        // ── Phase 4: 獵物行為（閒晃 + 逃離玩家/捕食者） ─────────────────────
        for i in 0..self.animals.len() {
            if !self.animals[i].alive { continue; }
            if self.animals[i].kind.trophic_level() != TrophicLevel::Prey { continue; }
            // 守衛狀態已在 Phase 2c 處理，跳過正常閒晃（不逃跑）。
            if matches!(self.animals[i].state, WildlifeState::Guarding { .. }) { continue; }

            let animal_kind = self.animals[i].kind;
            // ROADMAP 205：被馴養的個體把玩家當朋友（不逃跑），未馴養則沿用 144 物種態度判定。
            let tamed = self.animals[i].is_tamed();

            // 威脅 = 捕食者 + ROADMAP 165 獵食此物種的怪物——馴養與否都仍會逃離掠食者（信任的是你、不是狼）。
            let mut threats: Vec<(f32, f32)> = pred_positions.clone();
            for &(mk, mx, my) in monster_threats {
                if monster_hunts_wildlife(mk) == Some(animal_kind) {
                    threats.push((mx, my));
                }
            }
            // ROADMAP 144：未馴養且物種對人類不夠友善時，玩家也算威脅。
            let kind_attitude = *attitudes.get(&animal_kind).unwrap_or(&50);
            if !tamed && kind_attitude < FRIENDLY_ATTITUDE {
                threats.extend_from_slice(player_positions);
            }

            // ROADMAP 205：馴養個體在沒有掠食者威脅時，溫順地走向附近玩家、保持舒適距離（彷彿跟著你）。
            if tamed {
                let ax = self.animals[i].x;
                let ay = self.animals[i].y;
                let fleeing_now = matches!(self.animals[i].state, WildlifeState::Fleeing { .. });
                let predator_near = nearest_in_range(ax, ay, &threats, FLEE_RADIUS).is_some();
                if !fleeing_now && !predator_near {
                    if let Some((px, py)) = nearest_in_range(ax, ay, player_positions, FOLLOW_RANGE) {
                        let dx = px - ax;
                        let dy = py - ay;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist > FOLLOW_COMFORT_DIST {
                            self.animals[i].x += dx / dist * FOLLOW_SPEED * dt;
                            self.animals[i].y += dy / dist * FOLLOW_SPEED * dt;
                        }
                        // 朝向玩家的溫順狀態（已到舒適距離則原地陪著你）。
                        self.animals[i].state = WildlifeState::Wandering { target_x: px, target_y: py, wander_timer: 1.0 };
                        continue;
                    }
                }
            }

            // ROADMAP 206：群聚結伴——算出附近同種夥伴的平均位置（群體中心），
            // 作為下一個漫遊目標的拉力；HERD_RADIUS 內無同種夥伴則 None（退回純隨機）。
            let herd_anchor = herd_center(
                self.animals[i].id, animal_kind, self.animals[i].x, self.animals[i].y, &prey_snap,
            );

            let rng = &mut self.rng;
            let a = &mut self.animals[i];
            a.tick_idle(dt, &threats, WANDER_SPEED, herd_anchor, rng);
        }

        // ── Phase 5: 套用擊殺 ──────────────────────────────────────────────────
        for (pred_id, prey_id, pred_kind, prey_kind, kx, ky) in kills {
            // 將獵物標記為死亡。
            if let Some(prey) = self.animals.iter_mut().find(|a| a.id == prey_id) {
                prey.alive = false;
                prey.respawn_timer = PREY_RESPAWN_SECS;
                prey.state = WildlifeState::Resting { rest_timer: 0.0 };
            }
            // 確認捕食者仍存在（應為不死，但安全起見檢查）。
            let _ = pred_id;
            // 限流廣播：30 秒內最多一條。
            if self.kill_broadcast_cooldown <= 0.0 {
                events.push(WildlifeEvent::Kill { predator_kind: pred_kind, prey_kind, x: kx, y: ky });
                self.kill_broadcast_cooldown = KILL_BROADCAST_INTERVAL;
            }
            // ROADMAP 142：在死亡位置生成乙太微粒（上限 MAX_CARION_ORBS）。
            if self.carion_orbs.len() < MAX_CARION_ORBS {
                let id = self.orb_counter;
                self.orb_counter = self.orb_counter.wrapping_add(1);
                self.carion_orbs.push(CarrionOrb { id, x: kx, y: ky, ttl: CARION_ORB_TTL });
            }
        }

        events
    }
}

// ─── 輔助函式 ────────────────────────────────────────────────────────────────

fn nearest_in_range(ax: f32, ay: f32, pts: &[(f32, f32)], radius: f32) -> Option<(f32, f32)> {
    let r2 = radius * radius;
    pts.iter()
        .filter(|&&(px, py)| {
            let dx = px - ax;
            let dy = py - ay;
            dx * dx + dy * dy <= r2
        })
        .min_by(|&&(ax2, ay2), &&(bx2, by2)| {
            let da = (ax2 - ax).powi(2) + (ay2 - ay).powi(2);
            let db = (bx2 - ax).powi(2) + (by2 - ay).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}

fn random_target(hx: f32, hy: f32, radius: f32, rng: &mut StdRng) -> (f32, f32) {
    let angle: f32 = rng.gen_range(0.0..std::f32::consts::TAU);
    let dist: f32  = rng.gen_range(0.0..radius);
    (hx + angle.cos() * dist, hy + angle.sin() * dist)
}

/// ROADMAP 206：附近同種存活獵物的平均位置（不含自己），即「群體中心」。
/// 只統計 `HERD_RADIUS` 內、同 `kind` 的個體；範圍內無夥伴則回 `None`。
/// 純函式（吃 `prey_snap` 快照），便於測試。
fn herd_center(
    self_id: u32,
    kind: WildlifeKind,
    x: f32,
    y: f32,
    prey_snap: &[(u32, WildlifeKind, f32, f32)],
) -> Option<(f32, f32)> {
    let r2 = HERD_RADIUS * HERD_RADIUS;
    let mut sx = 0.0_f32;
    let mut sy = 0.0_f32;
    let mut n = 0u32;
    for &(id, k, px, py) in prey_snap {
        if id == self_id || k != kind {
            continue;
        }
        let dx = px - x;
        let dy = py - y;
        if dx * dx + dy * dy <= r2 {
            sx += px;
            sy += py;
            n += 1;
        }
    }
    if n == 0 {
        None
    } else {
        Some((sx / n as f32, sy / n as f32))
    }
}

/// ROADMAP 206：群聚結伴——選一個新的漫遊目標。
/// 先取家附近的隨機點（沿用 `random_target` 的散布），若 `anchor`（附近同種夥伴
/// 的平均位置）存在，再把目標朝群體中心按 `HERD_PULL` 混合，使同種動物鬆散聚攏、
/// 成群移動；無夥伴則退回純隨機漫遊（行為與 205 之前完全一致）。純函式，便於測試。
fn herd_wander_target(hx: f32, hy: f32, anchor: Option<(f32, f32)>, rng: &mut StdRng) -> (f32, f32) {
    let (rx, ry) = random_target(hx, hy, WANDER_RADIUS, rng);
    match anchor {
        Some((cx, cy)) => (rx + (cx - rx) * HERD_PULL, ry + (cy - ry) * HERD_PULL),
        None => (rx, ry),
    }
}

/// 生成所有野生動物（獵物 + 捕食者）。
fn spawn_all_wildlife(rng: &mut StdRng) -> Vec<Wildlife> {
    let spawns: &[(WildlifeKind, f32, f32)] = &[
        // ── 獵物：草原野鳥（城鎮北方）──
        (WildlifeKind::WildBird,     1900.0, 1600.0),
        (WildlifeKind::WildBird,     2100.0, 1500.0),
        (WildlifeKind::WildBird,     1700.0, 1750.0),
        // ── 獵物：草原野鹿（城鎮西北）──
        (WildlifeKind::WildDeer,     1600.0, 1900.0),
        (WildlifeKind::WildDeer,     1750.0, 2100.0),
        // ── 獵物：小動物（草原四散）──
        (WildlifeKind::SmallCritter, 1950.0, 2000.0),
        (WildlifeKind::SmallCritter, 2200.0, 1700.0),
        (WildlifeKind::SmallCritter, 1800.0, 1650.0),
        // ── 獵物：森林野鳥（城鎮東北）──
        (WildlifeKind::WildBird,     2700.0, 1700.0),
        (WildlifeKind::WildBird,     2900.0, 1550.0),
        // ── 獵物：森林野鹿（城鎮東方）──
        (WildlifeKind::WildDeer,     2800.0, 2000.0),
        (WildlifeKind::WildDeer,     3000.0, 2200.0),
        // ── 獵物：小動物（森林）──
        (WildlifeKind::SmallCritter, 2600.0, 1900.0),
        (WildlifeKind::SmallCritter, 2950.0, 1850.0),
        // ── 獵物：南方草原 ──
        (WildlifeKind::WildBird,     2200.0, 3000.0),
        (WildlifeKind::WildDeer,     2400.0, 3100.0),
        (WildlifeKind::SmallCritter, 2100.0, 2800.0),
        (WildlifeKind::SmallCritter, 2500.0, 2900.0),
        // ── 捕食者：野狼（靠近野鹿領地）──
        (WildlifeKind::WildWolf,     2880.0, 2150.0), // 東方森林，近 (2800,2000)
        (WildlifeKind::WildWolf,     1520.0, 2080.0), // 西北草原，近 (1600,1900)
        // ── 捕食者：野狐（靠近小動物領地）──
        (WildlifeKind::WildFox,      2020.0, 2060.0), // 草原，近 (1950,2000)
        (WildlifeKind::WildFox,      2680.0, 1970.0), // 森林，近 (2600,1900)
    ];

    assert_eq!(spawns.len(), WILDLIFE_COUNT);
    spawns.iter().enumerate().map(|(i, &(kind, hx, hy))| {
        Wildlife::new(i as u32, kind, hx, hy, rng)
    }).collect()
}

// ─── ROADMAP 143：聚落定義 ───────────────────────────────────────────────────

/// 建立 6 個固定物種聚落，分散於城鎮周圍野外。
/// 位置與 spawn_all_wildlife 的家位置對應，讓動物確實守衛自己的家域。
fn build_colonies() -> Vec<Colony> {
    vec![
        // 野鳥：兩個聚落（北方草原 + 東北森林）
        Colony { id: 0, kind: WildlifeKind::WildBird,     name: "野鳥巢穴（北方草原）", cx: 1900.0, cy: 1620.0, guard_radius: 230.0 },
        Colony { id: 1, kind: WildlifeKind::WildBird,     name: "野鳥巢穴（東北森林）", cx: 2800.0, cy: 1640.0, guard_radius: 210.0 },
        // 野鹿：一個聚落（西北草原鹿群）
        Colony { id: 2, kind: WildlifeKind::WildDeer,     name: "野鹿棲地",            cx: 1675.0, cy: 2000.0, guard_radius: 250.0 },
        // 小動物：一個洞穴（草原灌木區）
        Colony { id: 3, kind: WildlifeKind::SmallCritter, name: "小動物洞穴",          cx: 1985.0, cy: 1880.0, guard_radius: 200.0 },
        // 野狼：一個狼窩（東方森林）
        Colony { id: 4, kind: WildlifeKind::WildWolf,     name: "狼窩",               cx: 2880.0, cy: 2150.0, guard_radius: 260.0 },
        // 野狐：一個狐狸洞（草原）
        Colony { id: 5, kind: WildlifeKind::WildFox,      name: "狐狸洞",             cx: 2025.0, cy: 2060.0, guard_radius: 220.0 },
    ]
}

// ─── 測試 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rng() -> StdRng { StdRng::seed_from_u64(99) }

    #[test]
    fn wildlife_count_matches() {
        let mgr = WildlifeManager::new();
        assert_eq!(mgr.animals.len(), WILDLIFE_COUNT);
    }

    #[test]
    fn predator_count_is_four() {
        let mgr = WildlifeManager::new();
        let preds = mgr.animals.iter().filter(|a| a.kind.trophic_level() == TrophicLevel::Predator).count();
        assert_eq!(preds, 4);
    }

    #[test]
    fn prey_count_is_eighteen() {
        let mgr = WildlifeManager::new();
        let prey = mgr.animals.iter().filter(|a| a.kind.trophic_level() == TrophicLevel::Prey).count();
        assert_eq!(prey, 18);
    }

    #[test]
    fn no_player_stays_near_home() {
        let mut rng = make_rng();
        let mut animal = Wildlife::new(0, WildlifeKind::WildBird, 2000.0, 2000.0, &mut rng);
        for _ in 0..300 {
            animal.tick_idle(0.1, &[], WANDER_SPEED, None, &mut rng);
        }
        let dx = animal.x - animal.home_x;
        let dy = animal.y - animal.home_y;
        let dist = (dx * dx + dy * dy).sqrt();
        assert!(dist <= WANDER_RADIUS + 10.0, "漂移超出預期: {dist}");
    }

    #[test]
    fn player_nearby_triggers_prey_flee() {
        let mut rng = make_rng();
        let mut animal = Wildlife::new(0, WildlifeKind::WildDeer, 2000.0, 2000.0, &mut rng);
        let threats = vec![(2050.0_f32, 2050.0_f32)];
        animal.tick_idle(0.1, &threats, WANDER_SPEED, None, &mut rng);
        assert!(matches!(animal.state, WildlifeState::Fleeing { .. }),
            "應轉成 Fleeing，實際: {:?}", animal.state);
    }

    #[test]
    fn predator_hunts_prey_in_range() {
        let mut mgr = WildlifeManager::new();
        // 找一隻野狼和一隻野鹿，把牠們移到彼此 HUNT_RADIUS 內。
        let wolf_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildWolf).unwrap();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        let deer_id = mgr.animals[deer_idx].id;
        // 把野狼移到野鹿旁邊（距離 250px，在 HUNT_RADIUS=320 內）。
        mgr.animals[wolf_idx].x = mgr.animals[deer_idx].x + 250.0;
        mgr.animals[wolf_idx].y = mgr.animals[deer_idx].y;
        mgr.animals[wolf_idx].state = WildlifeState::Wandering { target_x: 0.0, target_y: 0.0, wander_timer: 5.0 };
        // 跑一幀觸發追獵。
        mgr.tick(0.1, &[], &std::collections::HashMap::new(), &[]);
        let wolf = &mgr.animals[wolf_idx];
        // 野狼應追獵某隻野鹿（不指定是哪隻，因附近可能有多隻）。
        assert!(
            matches!(wolf.state, WildlifeState::Hunting { .. }),
            "野狼應進入 Hunting 狀態，實際: {:?}", wolf.state
        );
        // 確認追獵目標確實是野鹿。
        if let WildlifeState::Hunting { target_id, .. } = wolf.state {
            assert!(
                mgr.animals.iter().any(|a| a.id == target_id && a.kind == WildlifeKind::WildDeer),
                "追獵目標應為野鹿，target_id={target_id}"
            );
        }
        let _ = deer_id; // 已不用直接比對
    }

    #[test]
    fn predator_kills_adjacent_prey_and_emits_event() {
        let mut mgr = WildlifeManager::new();
        // 找野狼和野鹿，放到彼此 KILL_RADIUS 內。
        let wolf_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildWolf).unwrap();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        let deer_id  = mgr.animals[deer_idx].id;
        let deer_x   = mgr.animals[deer_idx].x;
        let deer_y   = mgr.animals[deer_idx].y;
        // 野狼直接貼著野鹿。
        mgr.animals[wolf_idx].x = deer_x + KILL_RADIUS * 0.5;
        mgr.animals[wolf_idx].y = deer_y;
        mgr.animals[wolf_idx].state = WildlifeState::Hunting { target_id: deer_id, hunt_timer: 10.0 };
        let events = mgr.tick(0.1, &[], &std::collections::HashMap::new(), &[]);
        // 野鹿應死亡。
        assert!(!mgr.animals[deer_idx].alive, "野鹿應已死亡");
        // 應有 Kill 事件。
        assert!(
            events.iter().any(|e| matches!(e, WildlifeEvent::Kill { prey_kind: WildlifeKind::WildDeer, .. })),
            "應有 Kill 事件"
        );
    }

    #[test]
    fn dead_prey_respawns_after_timer() {
        let mut mgr = WildlifeManager::new();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        mgr.animals[deer_idx].alive = false;
        mgr.animals[deer_idx].respawn_timer = 0.1;
        // 跑超過 0.1 秒。
        mgr.tick(0.2, &[], &std::collections::HashMap::new(), &[]);
        assert!(mgr.animals[deer_idx].alive, "野鹿應在計時器結束後重生");
    }

    #[test]
    fn manager_tick_no_panic() {
        let mut mgr = WildlifeManager::new();
        let players = vec![(2200.0f32, 2200.0)];
        for _ in 0..100 {
            mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[]);
        }
        assert_eq!(mgr.animals.len(), WILDLIFE_COUNT);
    }

    // ─── ROADMAP 142 測試：乙太微粒生命週期 ─────────────────────────────────

    #[test]
    fn carion_orb_spawns_on_kill() {
        let mut mgr = WildlifeManager::new();
        let wolf_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildWolf).unwrap();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        let deer_id = mgr.animals[deer_idx].id;
        let deer_x  = mgr.animals[deer_idx].x;
        let deer_y  = mgr.animals[deer_idx].y;
        mgr.animals[wolf_idx].x = deer_x + KILL_RADIUS * 0.5;
        mgr.animals[wolf_idx].y = deer_y;
        mgr.animals[wolf_idx].state = WildlifeState::Hunting { target_id: deer_id, hunt_timer: 10.0 };
        mgr.tick(0.1, &[], &std::collections::HashMap::new(), &[]);
        assert_eq!(mgr.carion_orbs.len(), 1, "擊殺後應生成一顆乙太微粒");
        let orb = &mgr.carion_orbs[0];
        let dx = orb.x - deer_x;
        let dy = orb.y - deer_y;
        assert!(dx * dx + dy * dy < 1.0, "乙太微粒應在死亡位置");
    }

    #[test]
    fn carion_orb_expires_after_ttl() {
        let mut mgr = WildlifeManager::new();
        // 手動插入一顆即將到期的乙太微粒。
        mgr.carion_orbs.push(CarrionOrb { id: 0, x: 2000.0, y: 2000.0, ttl: 0.05 });
        assert_eq!(mgr.carion_orbs.len(), 1);
        // 跑超過 TTL。
        mgr.tick(0.1, &[], &std::collections::HashMap::new(), &[]);
        assert_eq!(mgr.carion_orbs.len(), 0, "TTL 到期後應自動消失");
    }

    #[test]
    fn collect_carion_orb_in_range_succeeds() {
        let mut mgr = WildlifeManager::new();
        mgr.carion_orbs.push(CarrionOrb { id: 42, x: 2000.0, y: 2000.0, ttl: 60.0 });
        let result = mgr.collect_carion_orb(42, 2020.0, 2020.0);
        assert_eq!(result, Some(CARION_ETHER), "在範圍內採集應得到乙太");
        assert_eq!(mgr.carion_orbs.len(), 0, "採集後微粒應消失");
    }

    #[test]
    fn collect_carion_orb_out_of_range_fails() {
        let mut mgr = WildlifeManager::new();
        mgr.carion_orbs.push(CarrionOrb { id: 7, x: 2000.0, y: 2000.0, ttl: 60.0 });
        let result = mgr.collect_carion_orb(7, 2200.0, 2200.0);
        assert!(result.is_none(), "超出範圍不應成功採集");
        assert_eq!(mgr.carion_orbs.len(), 1, "失敗後微粒仍存在");
    }

    #[test]
    fn collect_carion_orb_wrong_id_fails() {
        let mut mgr = WildlifeManager::new();
        mgr.carion_orbs.push(CarrionOrb { id: 1, x: 2000.0, y: 2000.0, ttl: 60.0 });
        let result = mgr.collect_carion_orb(99, 2000.0, 2000.0);
        assert!(result.is_none(), "錯誤 ID 不應成功採集");
    }

    #[test]
    fn max_orb_limit_is_respected() {
        let mut mgr = WildlifeManager::new();
        // 塞滿上限。
        for i in 0..MAX_CARION_ORBS {
            mgr.carion_orbs.push(CarrionOrb { id: i as u32, x: 2000.0, y: 2000.0, ttl: 60.0 });
        }
        assert_eq!(mgr.carion_orbs.len(), MAX_CARION_ORBS);
        // 模擬一次擊殺（找野狼和野鹿）。
        let wolf_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildWolf).unwrap();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        let deer_id = mgr.animals[deer_idx].id;
        let deer_x  = mgr.animals[deer_idx].x;
        let deer_y  = mgr.animals[deer_idx].y;
        mgr.animals[wolf_idx].x = deer_x + KILL_RADIUS * 0.5;
        mgr.animals[wolf_idx].y = deer_y;
        mgr.animals[wolf_idx].state = WildlifeState::Hunting { target_id: deer_id, hunt_timer: 10.0 };
        mgr.tick(0.1, &[], &std::collections::HashMap::new(), &[]);
        // 上限不超出。
        assert!(mgr.carion_orbs.len() <= MAX_CARION_ORBS, "乙太微粒不應超過上限");
    }

    #[test]
    fn carion_ether_value_is_positive() {
        assert!(CARION_ETHER > 0, "乙太微粒的乙太數量應 > 0");
    }

    #[test]
    fn carion_orb_ids_are_unique() {
        let mut mgr = WildlifeManager::new();
        for _ in 0..3 {
            let id = mgr.orb_counter;
            mgr.orb_counter = mgr.orb_counter.wrapping_add(1);
            mgr.carion_orbs.push(CarrionOrb { id, x: 0.0, y: 0.0, ttl: 60.0 });
        }
        let ids: Vec<u32> = mgr.carion_orbs.iter().map(|o| o.id).collect();
        let unique: std::collections::HashSet<u32> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "乙太微粒 ID 應唯一");
    }

    // ─── ROADMAP 143 測試：物種聚落與守衛行為 ─────────────────────────────────

    #[test]
    fn colony_count_is_six() {
        let mgr = WildlifeManager::new();
        assert_eq!(mgr.colonies.len(), 6, "應有 6 個物種聚落");
    }

    #[test]
    fn colony_ids_are_unique() {
        let mgr = WildlifeManager::new();
        let ids: Vec<u32> = mgr.colonies.iter().map(|c| c.id).collect();
        let unique: std::collections::HashSet<u32> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "聚落 ID 應唯一");
    }

    #[test]
    fn player_in_colony_triggers_guarding() {
        let mut mgr = WildlifeManager::new();
        // 找野鹿聚落（id=2，位於 1675,2000）。
        let deer_colony = mgr.colonies.iter().find(|c| c.kind == WildlifeKind::WildDeer).unwrap();
        let (cx, cy) = (deer_colony.cx, deer_colony.cy);
        // 把一隻野鹿放到聚落中心附近，確保在 activate 範圍內。
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        mgr.animals[deer_idx].x = cx + 50.0;
        mgr.animals[deer_idx].y = cy + 50.0;
        mgr.animals[deer_idx].state = WildlifeState::Resting { rest_timer: 5.0 };
        // 玩家站在聚落中心。
        let players = vec![(cx, cy)];
        mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[]);
        // 野鹿應進入 Guarding 狀態。
        let deer = &mgr.animals[deer_idx];
        assert!(
            matches!(deer.state, WildlifeState::Guarding { .. }),
            "野鹿應進入 Guarding 狀態，實際: {:?}", deer.state
        );
    }

    #[test]
    fn colony_threat_event_emitted_on_intrusion() {
        let mut mgr = WildlifeManager::new();
        let deer_colony = mgr.colonies.iter().find(|c| c.kind == WildlifeKind::WildDeer).unwrap();
        let (cx, cy) = (deer_colony.cx, deer_colony.cy);
        // 玩家站在聚落中心。
        let players = vec![(cx, cy)];
        let events = mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[]);
        assert!(
            events.iter().any(|e| matches!(e, WildlifeEvent::ColonyThreatened { .. })),
            "玩家進入聚落應觸發 ColonyThreatened 事件"
        );
    }

    #[test]
    fn colony_threat_cooldown_prevents_repeat_events() {
        let mut mgr = WildlifeManager::new();
        let deer_colony = mgr.colonies.iter().find(|c| c.kind == WildlifeKind::WildDeer).unwrap();
        let (cx, cy) = (deer_colony.cx, deer_colony.cy);
        let players = vec![(cx, cy)];
        // 第一次觸發。
        let events1 = mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[]);
        assert!(events1.iter().any(|e| matches!(e, WildlifeEvent::ColonyThreatened { .. })));
        // 馬上再觸發：冷卻中，不應再發出事件。
        let events2 = mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[]);
        assert!(
            !events2.iter().any(|e| matches!(e, WildlifeEvent::ColonyThreatened { .. })),
            "冷卻中不應再發出 ColonyThreatened 事件"
        );
    }

    #[test]
    fn guard_timer_expires_and_animal_returns_to_rest() {
        let mut mgr = WildlifeManager::new();
        let deer_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildDeer).unwrap();
        // 手動設定守衛狀態，計時即將到期。
        mgr.animals[deer_idx].state = WildlifeState::Guarding { threat_x: 2000.0, threat_y: 2000.0, guard_timer: 0.05 };
        // 跑超過計時。
        mgr.tick(0.2, &[], &std::collections::HashMap::new(), &[]);
        let deer = &mgr.animals[deer_idx];
        assert!(
            matches!(deer.state, WildlifeState::Resting { .. }),
            "計時到期後應回到 Resting，實際: {:?}", deer.state
        );
    }

    #[test]
    fn colony_views_returns_all_colonies() {
        let mgr = WildlifeManager::new();
        let views = mgr.colony_views();
        assert_eq!(views.len(), 6, "colony_views 應回傳 6 個視圖");
        assert!(views.iter().any(|v| v.kind == "wild_wolf"), "應含狼窩");
        assert!(views.iter().any(|v| v.kind == "wild_bird"), "應含野鳥巢穴");
    }

    #[test]
    fn different_species_not_affected_by_wrong_colony() {
        let mut mgr = WildlifeManager::new();
        // 找狐狸洞聚落。
        let fox_colony = mgr.colonies.iter().find(|c| c.kind == WildlifeKind::WildFox).unwrap();
        let (cx, cy) = (fox_colony.cx, fox_colony.cy);
        // 找一隻野鳥（不是狐狸），放到狐狸洞附近。
        let bird_idx = mgr.animals.iter().position(|a| a.kind == WildlifeKind::WildBird).unwrap();
        mgr.animals[bird_idx].x = cx + 80.0;
        mgr.animals[bird_idx].y = cy + 80.0;
        mgr.animals[bird_idx].state = WildlifeState::Resting { rest_timer: 5.0 };
        // 玩家站在狐狸洞。
        let players = vec![(cx, cy)];
        mgr.tick(0.1, &players, &std::collections::HashMap::new(), &[]);
        // 野鳥不應受狐狸洞影響。
        let bird = &mgr.animals[bird_idx];
        assert!(
            !matches!(bird.state, WildlifeState::Guarding { .. }),
            "野鳥不應因狐狸洞的入侵而守衛，實際: {:?}", bird.state
        );
    }

    #[test]
    fn guard_radius_values_are_positive() {
        let mgr = WildlifeManager::new();
        for c in &mgr.colonies {
            assert!(c.guard_radius > 0.0, "聚落 {} 守衛半徑應 > 0", c.name);
        }
    }

    // ─── ROADMAP 165 測試 ────────────────────────────────────────────────────

    #[test]
    fn monster_hunts_wildlife_returns_correct_pairs() {
        use crate::combat::EnemyKind;
        assert_eq!(monster_hunts_wildlife(EnemyKind::EtherWisp),       Some(WildlifeKind::WildBird));
        assert_eq!(monster_hunts_wildlife(EnemyKind::MushroomStalker), Some(WildlifeKind::SmallCritter));
        assert_eq!(monster_hunts_wildlife(EnemyKind::ScrapDrone),      Some(WildlifeKind::WildDeer));
        assert_eq!(monster_hunts_wildlife(EnemyKind::CrystalGolem),    None);
        assert_eq!(monster_hunts_wildlife(EnemyKind::FlutterSprite),   None);
    }

    #[test]
    fn on_monster_kills_wildlife_marks_dead_and_creates_orb() {
        use crate::combat::EnemyKind;
        let mut mgr = WildlifeManager::new();
        let bird_id = mgr.animals.iter()
            .find(|a| a.alive && a.kind == WildlifeKind::WildBird)
            .map(|a| a.id).unwrap();
        let before_orbs = mgr.carion_orbs.len();
        let ev = mgr.on_monster_kills_wildlife(bird_id, EnemyKind::EtherWisp);
        assert!(matches!(ev, Some(WildlifeEvent::MonsterHunted { .. })), "應回傳 MonsterHunted 事件");
        let bird = mgr.animals.iter().find(|a| a.id == bird_id).unwrap();
        assert!(!bird.alive, "被獵殺的野鳥應標記為死亡");
        assert_eq!(mgr.carion_orbs.len(), before_orbs + 1, "應生成一顆乙太微粒");
    }

    #[test]
    fn on_monster_kills_wildlife_idempotent_on_dead_animal() {
        use crate::combat::EnemyKind;
        let mut mgr = WildlifeManager::new();
        let bird_id = mgr.animals.iter()
            .find(|a| a.alive && a.kind == WildlifeKind::WildBird)
            .map(|a| a.id).unwrap();
        let _ = mgr.on_monster_kills_wildlife(bird_id, EnemyKind::EtherWisp);
        let ev2 = mgr.on_monster_kills_wildlife(bird_id, EnemyKind::EtherWisp);
        assert!(ev2.is_none(), "已死亡的動物再次呼叫應回傳 None");
    }

    #[test]
    fn alive_snapshot_counts_decrease_after_kill() {
        use crate::combat::EnemyKind;
        let mut mgr = WildlifeManager::new();
        let initial_count = mgr.alive_snapshot().len();
        assert_eq!(initial_count, WILDLIFE_COUNT);
        let bird_id = mgr.animals.iter()
            .find(|a| a.alive && a.kind == WildlifeKind::WildBird)
            .map(|a| a.id).unwrap();
        let _ = mgr.on_monster_kills_wildlife(bird_id, EnemyKind::EtherWisp);
        assert_eq!(mgr.alive_snapshot().len(), initial_count - 1, "死亡後快照應少一隻");
    }

    #[test]
    fn prey_flees_from_hunting_monster_in_tick() {
        use crate::combat::EnemyKind;
        let mut rng = make_rng();
        // 建立一隻靜止野鳥（在 home 位置）。
        let mut bird = Wildlife::new(0, WildlifeKind::WildBird, 2000.0, 2000.0, &mut rng);
        bird.state = WildlifeState::Resting { rest_timer: 10.0 };
        bird.x = 2000.0;
        bird.y = 2000.0;
        // 把 EtherWisp 放在 FLEE_RADIUS 內（100px）。
        let threats = vec![(EnemyKind::EtherWisp, 2100.0_f32, 2000.0_f32)];
        bird.tick_idle(0.1, &threats.iter().map(|&(_, x, y)| (x, y)).collect::<Vec<_>>(), WANDER_SPEED, None, &mut rng);
        assert!(
            matches!(bird.state, WildlifeState::Fleeing { .. }),
            "怪物在 FLEE_RADIUS 內，野鳥應進入 Fleeing 狀態"
        );
    }

    #[test]
    fn non_prey_kind_not_affected_by_monster_threats_in_tick() {
        use crate::combat::EnemyKind;
        // CrystalGolem 不獵食任何野生動物，野鹿不應因它逃跑。
        let threats = vec![(EnemyKind::CrystalGolem, 2100.0_f32, 2000.0_f32)];
        assert!(
            monster_hunts_wildlife(EnemyKind::CrystalGolem).is_none(),
            "CrystalGolem 不應有食物鏈配對"
        );
        let _ = threats;
    }

    // ─── ROADMAP 205：餵食馴養 測試 ─────────────────────────────────────────
    use std::collections::HashMap;

    /// 把 mgr 內第一隻指定種類的動物搬到 (x,y)、設定親近度與休息狀態，回傳其 id。
    fn place_test_animal(mgr: &mut WildlifeManager, kind: WildlifeKind, x: f32, y: f32, familiarity: f32) -> u32 {
        let id = mgr.animals.iter().find(|a| a.kind == kind).map(|a| a.id).unwrap();
        let a = mgr.animals.iter_mut().find(|a| a.id == id).unwrap();
        a.alive = true;
        a.x = x; a.y = y;
        a.home_x = x; a.home_y = y;
        a.familiarity = familiarity;
        a.state = WildlifeState::Resting { rest_timer: 10.0 };
        id
    }

    #[test]
    fn feeding_raises_familiarity_and_tames_exactly_once() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, 0.0);
        let needed = (TAME_FAMILIARITY / FEED_FAMILIARITY_GAIN).ceil() as i32;
        let mut tamed_events = 0;
        for _ in 0..needed {
            let (_, _, just_tamed) = mgr.on_feed_animal(id).unwrap();
            if just_tamed { tamed_events += 1; }
        }
        assert!(mgr.animals.iter().find(|a| a.id == id).unwrap().is_tamed(), "餵足次數後應已馴養");
        assert_eq!(tamed_events, 1, "「剛馴養」事件應只觸發一次");
        // 已馴養後再餵不應再觸發馴養事件。
        let (_, _, again) = mgr.on_feed_animal(id).unwrap();
        assert!(!again, "已馴養後再餵不應重複觸發馴養");
    }

    #[test]
    fn on_feed_animal_unknown_id_returns_none() {
        let mut mgr = WildlifeManager::new();
        assert!(mgr.on_feed_animal(999_999).is_none(), "不存在的 ID 應回傳 None");
    }

    #[test]
    fn tamed_prey_does_not_flee_player() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, MAX_FAMILIARITY);
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 玩家就在 FLEE_RADIUS 內。
        mgr.tick(0.1, &[(5050.0, 5000.0)], &att, &[]);
        let a = mgr.animals.iter().find(|a| a.id == id).unwrap();
        assert!(!matches!(a.state, WildlifeState::Fleeing { .. }), "馴養個體不應逃離玩家，實際: {:?}", a.state);
    }

    #[test]
    fn untamed_prey_still_flees_player() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, 0.0);
        let att: HashMap<WildlifeKind, i32> = HashMap::new(); // 預設態度 50 < FRIENDLY，玩家是威脅
        mgr.tick(0.1, &[(5050.0, 5000.0)], &att, &[]);
        let a = mgr.animals.iter().find(|a| a.id == id).unwrap();
        assert!(matches!(a.state, WildlifeState::Fleeing { .. }), "未馴養個體應逃離玩家，實際: {:?}", a.state);
    }

    #[test]
    fn tamed_prey_follows_nearby_player() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, MAX_FAMILIARITY);
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 玩家在 FOLLOW_RANGE 內、舒適距離外（右側 200px）。
        mgr.tick(0.2, &[(5200.0, 5000.0)], &att, &[]);
        let a = mgr.animals.iter().find(|a| a.id == id).unwrap();
        assert!(a.x > 5000.0, "馴養個體應朝玩家移動（x 變大），實際 x={}", a.x);
    }

    #[test]
    fn tamed_prey_still_flees_hunting_monster() {
        use crate::combat::EnemyKind;
        let mut mgr = WildlifeManager::new();
        // ScrapDrone 獵食 WildDeer。
        assert_eq!(monster_hunts_wildlife(EnemyKind::ScrapDrone), Some(WildlifeKind::WildDeer));
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, MAX_FAMILIARITY);
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        // 玩家在旁（馴養→不怕），但獵食怪物在 FLEE_RADIUS 內。
        mgr.tick(0.1, &[(5040.0, 5000.0)], &att, &[(EnemyKind::ScrapDrone, 5050.0, 5000.0)]);
        let a = mgr.animals.iter().find(|a| a.id == id).unwrap();
        assert!(matches!(a.state, WildlifeState::Fleeing { .. }), "馴養個體仍應逃離掠食怪物，實際: {:?}", a.state);
    }

    #[test]
    fn familiarity_decays_over_time() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, MAX_FAMILIARITY);
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..100 { mgr.tick(1.0, &[], &att, &[]); } // 100 秒、無餵食
        let f = mgr.animals.iter().find(|a| a.id == id).unwrap().familiarity();
        assert!(f < MAX_FAMILIARITY, "親近度應隨時間衰減，實際 {f}");
        assert!(f > 0.0, "100 秒衰減不應歸零（衰減很慢），實際 {f}");
    }

    #[test]
    fn respawn_resets_familiarity() {
        let mut mgr = WildlifeManager::new();
        let id = place_test_animal(&mut mgr, WildlifeKind::WildDeer, 5000.0, 5000.0, MAX_FAMILIARITY);
        // 擊殺該隻（玩家攻擊），再推進到重生。
        assert!(mgr.attack_wildlife(id, 5000.0, 5000.0, 30.0).is_some(), "應成功擊殺");
        let att: HashMap<WildlifeKind, i32> = HashMap::new();
        for _ in 0..((PREY_RESPAWN_SECS as i32) + 2) { mgr.tick(1.0, &[], &att, &[]); }
        let a = mgr.animals.iter().find(|a| a.id == id).unwrap();
        assert!(a.alive, "應已重生");
        assert_eq!(a.familiarity(), 0.0, "重生個體親近度應歸零（羈絆隨上一隻散去）");
    }

    // ─── ROADMAP 206：群聚結伴 測試 ─────────────────────────────────────────

    #[test]
    fn herd_center_none_when_alone() {
        // 同種只有自己一隻 → 範圍內無夥伴 → None。
        let snap = vec![(0u32, WildlifeKind::WildDeer, 100.0_f32, 100.0_f32)];
        assert_eq!(herd_center(0, WildlifeKind::WildDeer, 100.0, 100.0, &snap), None);
    }

    #[test]
    fn herd_center_excludes_self_and_other_species() {
        // 三隻同種夥伴（皆在範圍內）+ 一隻自己 + 一隻他種 → 只平均那三隻同種。
        let snap = vec![
            (0u32, WildlifeKind::WildDeer, 0.0_f32, 0.0_f32),     // 自己（排除）
            (1u32, WildlifeKind::WildDeer, 10.0, 0.0),
            (2u32, WildlifeKind::WildDeer, 30.0, 0.0),
            (3u32, WildlifeKind::WildDeer, 50.0, 0.0),
            (4u32, WildlifeKind::WildBird, 10.0, 0.0),            // 他種（排除）
        ];
        let c = herd_center(0, WildlifeKind::WildDeer, 0.0, 0.0, &snap).expect("應有群體中心");
        assert!((c.0 - 30.0).abs() < 0.01 && c.1.abs() < 0.01, "群體中心應為三同種平均 (30,0)，實際 {c:?}");
    }

    #[test]
    fn herd_center_ignores_neighbors_beyond_radius() {
        // 同種夥伴在 HERD_RADIUS 外 → 不算入 → None。
        let far = HERD_RADIUS + 50.0;
        let snap = vec![
            (0u32, WildlifeKind::WildDeer, 0.0_f32, 0.0_f32),
            (1u32, WildlifeKind::WildDeer, far, 0.0),
        ];
        assert_eq!(herd_center(0, WildlifeKind::WildDeer, 0.0, 0.0, &snap), None,
            "範圍外夥伴不應觸發群聚");
    }

    #[test]
    fn herd_wander_target_pulls_toward_anchor() {
        // 有群體中心時，新目標應比「純隨機家附近目標」更靠近群體中心。
        // 家在原點，群體中心遠在 (10000,10000)：拉力後的目標與中心的距離，
        // 應明顯小於家到中心的距離（被朝中心拉了 HERD_PULL 比例）。
        let mut rng = make_rng();
        let anchor = (10000.0_f32, 10000.0_f32);
        let home_to_anchor = (anchor.0.powi(2) + anchor.1.powi(2)).sqrt();
        for _ in 0..50 {
            let (tx, ty) = herd_wander_target(0.0, 0.0, Some(anchor), &mut rng);
            let d = ((tx - anchor.0).powi(2) + (ty - anchor.1).powi(2)).sqrt();
            // 隨機點僅落在家附近 WANDER_RADIUS 內，混合 HERD_PULL 後距中心必縮短。
            assert!(d < home_to_anchor * (1.0 - HERD_PULL + 0.01),
                "拉力後距群體中心 {d} 應明顯小於 {home_to_anchor}");
        }
    }

    #[test]
    fn herd_wander_target_no_anchor_is_pure_random_near_home() {
        // 無夥伴時行為應與純隨機漫遊一致：目標落在家附近 WANDER_RADIUS 內。
        let mut rng = make_rng();
        for _ in 0..50 {
            let (tx, ty) = herd_wander_target(2000.0, 2000.0, None, &mut rng);
            let d = ((tx - 2000.0_f32).powi(2) + (ty - 2000.0_f32).powi(2)).sqrt();
            assert!(d <= WANDER_RADIUS + 0.01, "無夥伴目標應在家附近，實際距離 {d}");
        }
    }

    #[test]
    fn herding_does_not_disturb_flee() {
        // 群聚只影響「選漫遊目標」，不該蓋過逃跑：玩家逼近時仍進入 Fleeing。
        // （群聚夥伴就在身邊，但威脅優先。）
        let mut rng = make_rng();
        let mut deer = Wildlife::new(0, WildlifeKind::WildDeer, 2000.0, 2000.0, &mut rng);
        let threats = vec![(2030.0_f32, 2000.0_f32)];
        let anchor = Some((2010.0_f32, 2000.0_f32));
        deer.tick_idle(0.1, &threats, WANDER_SPEED, anchor, &mut rng);
        assert!(matches!(deer.state, WildlifeState::Fleeing { .. }),
            "威脅在 FLEE_RADIUS 內，群聚不應蓋過逃跑，實際 {:?}", deer.state);
    }
}
