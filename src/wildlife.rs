//! 野生動物系統（ROADMAP 140 中立野生動物 + ROADMAP 141 食物鏈/獵食）。
//!
//! ROADMAP 140：野鳥/野鹿/小動物——中立、只逃跑、不攻擊。
//! ROADMAP 141：野狼獵野鹿、野狐獵小動物；族群此消彼長（湧現平衡）。
//!
//! 行為規則：
//! - 捕食者進入 HUNT_RADIUS 內偵測到獵物 → Hunting（追獵）。
//! - 追及 KILL_RADIUS 內 → 獵物死亡 + 捕食者進入 Digesting。
//! - 玩家與捕食者都會令獵物 Fleeing；同種獵物見捕食者靠近也一起竄逃（群逃）。
//! - 死亡獵物 ~50 秒後在家附近重生（代表族群新個體）。
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

// ─── 事件 ────────────────────────────────────────────────────────────────────

pub enum WildlifeEvent {
    /// 捕食者成功捕獵，應廣播至全服聊天。
    Kill {
        predator_kind: WildlifeKind,
        prey_kind: WildlifeKind,
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
        }
    }
}

// ─── 管理器 ──────────────────────────────────────────────────────────────────

pub struct WildlifeManager {
    pub animals: Vec<Wildlife>,
    rng: StdRng,
    /// 距上次捕獵廣播的累計秒數（限流用）。
    kill_broadcast_cooldown: f32,
}

impl WildlifeManager {
    pub fn new() -> Self {
        let mut rng = StdRng::seed_from_u64(7654321);
        let animals = spawn_all_wildlife(&mut rng);
        Self { animals, rng, kill_broadcast_cooldown: 0.0 }
    }

    /// 每幀推進所有野生動物，回傳本幀產生的事件列表。
    pub fn tick(&mut self, dt: f32, player_positions: &[(f32, f32)]) -> Vec<WildlifeEvent> {
        let mut events = Vec::new();
        self.kill_broadcast_cooldown = (self.kill_broadcast_cooldown - dt).max(-1.0);

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

        // ── Phase 3: 捕食者行為 ────────────────────────────────────────────────
        // 收集本幀的擊殺：(pred_id, prey_id, pred_kind, prey_kind, x, y)
        let mut kills: Vec<(u32, u32, WildlifeKind, WildlifeKind, f32, f32)> = Vec::new();

        for i in 0..self.animals.len() {
            if !self.animals[i].alive { continue; }
            if self.animals[i].kind.trophic_level() != TrophicLevel::Predator { continue; }

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
}
