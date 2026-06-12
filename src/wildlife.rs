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

// ─── ROADMAP 142：乙太微粒常數 ───────────────────────────────────────────────

/// 乙太微粒採集有效距離（像素）。
pub const CARION_COLLECT_RADIUS: f32 = 80.0;
/// 每顆乙太微粒給予的乙太數量。
pub const CARION_ETHER: u32 = 4;
/// 乙太微粒存在時長（秒），逾時自動消失。
const CARION_ORB_TTL: f32 = 90.0;
/// 同時存在乙太微粒的上限（防止無限堆積）。
const MAX_CARION_ORBS: usize = 8;

// ─── 種類與營養階 ────────────────────────────────────────────────────────────

/// 野生動物種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
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
        }
    }

    /// 非追獵行為 tick：閒晃 / 休息 / 逃跑 / 返家。
    /// `flee_threats`：需要逃離的座標（玩家 + 捕食者）；捕食者呼叫時傳空。
    fn tick_idle(&mut self, dt: f32, flee_threats: &[(f32, f32)], speed: f32, rng: &mut StdRng) {
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
                    let (tx, ty) = random_target(self.home_x, self.home_y, WANDER_RADIUS, rng);
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
                    let (tx, ty) = random_target(self.home_x, self.home_y, WANDER_RADIUS, rng);
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

    /// 每幀推進所有野生動物，回傳本幀產生的事件列表。
    pub fn tick(&mut self, dt: f32, player_positions: &[(f32, f32)]) -> Vec<WildlifeEvent> {
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

        // ── Phase 1: 死亡倒數 + 重生 ──────────────────────────────────────────
        for a in &mut self.animals {
            if !a.alive {
                a.respawn_timer -= dt;
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

        // ── Phase 2c: 守衛行為 tick（ROADMAP 143）───────────────────────────
        // 處理所有物種（獵物與捕食者）的 Guarding 狀態。
        for i in 0..self.animals.len() {
            if !self.animals[i].alive { continue; }
            let WildlifeState::Guarding { threat_x, threat_y, guard_timer } = self.animals[i].state else { continue };
            let dx = threat_x - self.animals[i].x;
            let dy = threat_y - self.animals[i].y;
            let dist = (dx * dx + dy * dy).sqrt();
            let remaining = guard_timer - dt;
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
                            // 無獵物，正常閒晃（捕食者不怕玩家，傳空威脅）。
                            let rng = &mut self.rng;
                            let a = &mut self.animals[i];
                            a.tick_idle(dt, &[], PRED_WANDER_SPEED, rng);
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

            // 威脅 = 玩家 + 捕食者。
            let mut threats: Vec<(f32, f32)> = player_positions.to_vec();
            threats.extend_from_slice(&pred_positions);
            let rng = &mut self.rng;
            let a = &mut self.animals[i];
            a.tick_idle(dt, &threats, WANDER_SPEED, rng);
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
            animal.tick_idle(0.1, &[], WANDER_SPEED, &mut rng);
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
        animal.tick_idle(0.1, &threats, WANDER_SPEED, &mut rng);
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
        mgr.tick(0.1, &[]);
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
        let events = mgr.tick(0.1, &[]);
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
        mgr.tick(0.2, &[]);
        assert!(mgr.animals[deer_idx].alive, "野鹿應在計時器結束後重生");
    }

    #[test]
    fn manager_tick_no_panic() {
        let mut mgr = WildlifeManager::new();
        let players = vec![(2200.0f32, 2200.0)];
        for _ in 0..100 {
            mgr.tick(0.1, &players);
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
        mgr.tick(0.1, &[]);
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
        mgr.tick(0.1, &[]);
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
        mgr.tick(0.1, &[]);
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
        mgr.tick(0.1, &players);
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
        let events = mgr.tick(0.1, &players);
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
        let events1 = mgr.tick(0.1, &players);
        assert!(events1.iter().any(|e| matches!(e, WildlifeEvent::ColonyThreatened { .. })));
        // 馬上再觸發：冷卻中，不應再發出事件。
        let events2 = mgr.tick(0.1, &players);
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
        mgr.tick(0.2, &[]);
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
        mgr.tick(0.1, &players);
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
}
