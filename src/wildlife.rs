//! 中立野生動物系統（ROADMAP 140）。
//!
//! 世界第一次有「不是來打你的」生命——野鳥、野鹿、小動物（松鼠/野兔）
//! 在城外安靜生活：覓食、漫步、看到玩家靠近就逃。
//!
//! 設計原則：
//! - 純模板行為，零 LLM，零 migration。
//! - 玩家接近 → 驚逃；遠離後緩緩回巢。
//! - 此切片：只逃跑，不可被攻擊（食物鏈/狩獵由後續切片處理）。
//! - 效能：15 隻量小，每幀純算術，不額外分配。

use rand::{Rng, SeedableRng, rngs::StdRng};

// ─── 常數 ───────────────────────────────────────────────────────────────────

/// 野生動物總數（固定刷在世界各角落）。
const WILDLIFE_COUNT: usize = 18;

/// 玩家靠近多少像素觸發驚逃。
const FLEE_RADIUS: f32 = 180.0;
/// 驚逃速度（像素/秒）。
const FLEE_SPEED: f32 = 200.0;
/// 驚逃計時器長度（秒），過後轉回返家。
const FLEE_DURATION: f32 = 4.0;

/// 閒晃速度（像素/秒）。
const WANDER_SPEED: f32 = 35.0;
/// 漫遊半徑：動物在家附近多少範圍內隨機走動。
const WANDER_RADIUS: f32 = 180.0;
/// 換目標計時器最小/最大（秒）。
const WANDER_TIMER_MIN: f32 = 2.5;
const WANDER_TIMER_MAX: f32 = 7.0;
/// 休息計時器最小/最大（秒），到了休息點後靜止一會。
const REST_TIMER_MIN: f32 = 1.5;
const REST_TIMER_MAX: f32 = 4.5;

/// 返回巢穴速度（像素/秒）。
const RETURN_SPEED: f32 = 60.0;
/// 距巢穴多近算「回到家」，切回閒晃。
const HOME_ARRIVE_DIST: f32 = 20.0;

// ─── 種類 ───────────────────────────────────────────────────────────────────

/// 野生動物種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WildlifeKind {
    /// 野鳥：輕盈、羽翼，主要在草原/森林。
    WildBird,
    /// 野鹿：溫馴、優雅，主要在草原/森林。
    WildDeer,
    /// 小動物（松鼠/野兔）：圓滾滾、敏捷。
    SmallCritter,
}

impl WildlifeKind {
    pub fn display_name(self) -> &'static str {
        match self {
            WildlifeKind::WildBird    => "野鳥",
            WildlifeKind::WildDeer    => "野鹿",
            WildlifeKind::SmallCritter => "小動物",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            WildlifeKind::WildBird    => "wild_bird",
            WildlifeKind::WildDeer    => "wild_deer",
            WildlifeKind::SmallCritter => "small_critter",
        }
    }
}

// ─── 狀態 ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum WildlifeState {
    /// 在巢穴附近閒晃，走向當前目標點。
    Wandering { target_x: f32, target_y: f32, wander_timer: f32 },
    /// 停下來休息（短暫靜止）。
    Resting { rest_timer: f32 },
    /// 看到玩家後驚逃（帶逃跑方向向量）。
    Fleeing { vx: f32, vy: f32, flee_timer: f32 },
    /// 驚逃完畢，返回巢穴。
    Returning,
}

// ─── 實體 ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Wildlife {
    pub id: u32,
    pub kind: WildlifeKind,
    pub x: f32,
    pub y: f32,
    /// 家的座標（漫遊中心）。
    home_x: f32,
    home_y: f32,
    state: WildlifeState,
}

impl Wildlife {
    fn new(id: u32, kind: WildlifeKind, hx: f32, hy: f32, rng: &mut StdRng) -> Self {
        // 在家附近小範圍內隨機落地初始位置。
        let offset_x = rng.gen_range(-50.0_f32..50.0);
        let offset_y = rng.gen_range(-50.0_f32..50.0);
        Self {
            id,
            kind,
            x: hx + offset_x,
            y: hy + offset_y,
            home_x: hx,
            home_y: hy,
            state: WildlifeState::Resting {
                rest_timer: rng.gen_range(REST_TIMER_MIN..=REST_TIMER_MAX),
            },
        }
    }

    /// 每幀推進：移動 + 狀態切換。
    /// `player_positions`：當前幀所有線上玩家的世界座標。
    pub fn tick(&mut self, dt: f32, player_positions: &[(f32, f32)], rng: &mut StdRng) {
        // 優先偵測玩家接近（覆蓋任何狀態，除非已在逃跑中）。
        let already_fleeing = matches!(self.state, WildlifeState::Fleeing { .. });
        if !already_fleeing {
            if let Some((px, py)) = nearest_player_in_range(self.x, self.y, player_positions, FLEE_RADIUS) {
                // 逃離玩家方向的反向。
                let dx = self.x - px;
                let dy = self.y - py;
                let len = (dx * dx + dy * dy).sqrt().max(1.0);
                let vx = dx / len * FLEE_SPEED;
                let vy = dy / len * FLEE_SPEED;
                self.state = WildlifeState::Fleeing { vx, vy, flee_timer: FLEE_DURATION };
                return;
            }
        }

        match &self.state.clone() {
            WildlifeState::Fleeing { vx, vy, flee_timer } => {
                self.x += vx * dt;
                self.y += vy * dt;
                let remaining = flee_timer - dt;
                if remaining <= 0.0 {
                    self.state = WildlifeState::Returning;
                } else {
                    // 持續偵測玩家是否仍在範圍內，更新逃跑方向。
                    if let Some((px, py)) = nearest_player_in_range(self.x, self.y, player_positions, FLEE_RADIUS) {
                        let dx = self.x - px;
                        let dy = self.y - py;
                        let len = (dx * dx + dy * dy).sqrt().max(1.0);
                        let nvx = dx / len * FLEE_SPEED;
                        let nvy = dy / len * FLEE_SPEED;
                        self.state = WildlifeState::Fleeing { vx: nvx, vy: nvy, flee_timer: remaining };
                    } else {
                        self.state = WildlifeState::Fleeing { vx: *vx, vy: *vy, flee_timer: remaining };
                    }
                }
            }
            WildlifeState::Returning => {
                let dx = self.home_x - self.x;
                let dy = self.home_y - self.y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= HOME_ARRIVE_DIST {
                    // 回到家，切回閒晃。
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
                let tx = *target_x;
                let ty = *target_y;
                let remaining = wander_timer - dt;
                let dx = tx - self.x;
                let dy = ty - self.y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < 8.0 || remaining <= 0.0 {
                    // 抵達目標或超時，切到休息。
                    let rest = rng.gen_range(REST_TIMER_MIN..=REST_TIMER_MAX);
                    self.state = WildlifeState::Resting { rest_timer: rest };
                } else {
                    self.x += (dx / dist) * WANDER_SPEED * dt;
                    self.y += (dy / dist) * WANDER_SPEED * dt;
                    self.state = WildlifeState::Wandering {
                        target_x: tx,
                        target_y: ty,
                        wander_timer: remaining,
                    };
                }
            }
        }
    }

    /// 供協議層使用的狀態字串。
    pub fn state_str(&self) -> &'static str {
        match &self.state {
            WildlifeState::Wandering { .. } => "wandering",
            WildlifeState::Resting { .. }   => "resting",
            WildlifeState::Fleeing { .. }   => "fleeing",
            WildlifeState::Returning        => "returning",
        }
    }
}

// ─── 管理器 ─────────────────────────────────────────────────────────────────

/// 野生動物管理器——持有所有野生動物並每幀推進。
pub struct WildlifeManager {
    pub animals: Vec<Wildlife>,
    rng: StdRng,
}

impl WildlifeManager {
    pub fn new() -> Self {
        let mut rng = StdRng::seed_from_u64(7654321);
        let animals = spawn_wildlife(&mut rng);
        Self { animals, rng }
    }

    /// 每幀推進所有野生動物。
    pub fn tick(&mut self, dt: f32, player_positions: &[(f32, f32)]) {
        for animal in &mut self.animals {
            let rng = &mut self.rng;
            animal.tick(dt, player_positions, rng);
        }
    }
}

// ─── 輔助函式 ───────────────────────────────────────────────────────────────

fn nearest_player_in_range(
    ax: f32, ay: f32,
    players: &[(f32, f32)],
    radius: f32,
) -> Option<(f32, f32)> {
    let r2 = radius * radius;
    players.iter()
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

/// 建立 WILDLIFE_COUNT 隻分散於城外世界各角落的野生動物。
/// 位置設定在安全區(半徑 ~640px)之外、但離主城還不太遠的地方。
fn spawn_wildlife(rng: &mut StdRng) -> Vec<Wildlife> {
    // 預先設計的「家座標」——分散在四方，脫離安全區，野外感覺。
    let spawns: &[(WildlifeKind, f32, f32)] = &[
        // 草原野鳥群（城鎮北方）
        (WildlifeKind::WildBird,    1900.0, 1600.0),
        (WildlifeKind::WildBird,    2100.0, 1500.0),
        (WildlifeKind::WildBird,    1700.0, 1750.0),
        // 草原野鹿（城鎮西北）
        (WildlifeKind::WildDeer,    1600.0, 1900.0),
        (WildlifeKind::WildDeer,    1750.0, 2100.0),
        // 小動物（草原四散）
        (WildlifeKind::SmallCritter, 1950.0, 2000.0),
        (WildlifeKind::SmallCritter, 2200.0, 1700.0),
        (WildlifeKind::SmallCritter, 1800.0, 1650.0),
        // 森林野鳥（城鎮東北，森林生態）
        (WildlifeKind::WildBird,    2700.0, 1700.0),
        (WildlifeKind::WildBird,    2900.0, 1550.0),
        // 森林野鹿（城鎮東方）
        (WildlifeKind::WildDeer,    2800.0, 2000.0),
        (WildlifeKind::WildDeer,    3000.0, 2200.0),
        // 小動物（森林散布）
        (WildlifeKind::SmallCritter, 2600.0, 1900.0),
        (WildlifeKind::SmallCritter, 2950.0, 1850.0),
        // 南方草原（城鎮南方）
        (WildlifeKind::WildBird,    2200.0, 3000.0),
        (WildlifeKind::WildDeer,    2400.0, 3100.0),
        (WildlifeKind::SmallCritter, 2100.0, 2800.0),
        (WildlifeKind::SmallCritter, 2500.0, 2900.0),
    ];

    spawns.iter().enumerate().map(|(i, &(kind, hx, hy))| {
        Wildlife::new(i as u32, kind, hx, hy, rng)
    }).collect()
}

// ─── 測試 ───────────────────────────────────────────────────────────────────

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
    fn no_player_stays_near_home() {
        let mut rng = make_rng();
        let mut animal = Wildlife::new(0, WildlifeKind::WildBird, 2000.0, 2000.0, &mut rng);
        // 模擬 30 秒，沒有玩家。
        for _ in 0..300 {
            animal.tick(0.1, &[], &mut rng);
        }
        // 應仍在 WANDER_RADIUS 範圍內。
        let dx = animal.x - animal.home_x;
        let dy = animal.y - animal.home_y;
        let dist = (dx * dx + dy * dy).sqrt();
        assert!(dist <= WANDER_RADIUS + 10.0, "漂移超出預期: {dist}");
    }

    #[test]
    fn player_nearby_triggers_flee() {
        let mut rng = make_rng();
        let mut animal = Wildlife::new(0, WildlifeKind::WildDeer, 2000.0, 2000.0, &mut rng);
        // 玩家就在旁邊。
        let players = vec![(2050.0_f32, 2050.0_f32)];
        animal.tick(0.1, &players, &mut rng);
        assert!(matches!(animal.state, WildlifeState::Fleeing { .. }),
            "應轉成 Fleeing，實際: {:?}", animal.state);
    }

    #[test]
    fn flee_state_str_correct() {
        let mut rng = make_rng();
        let mut animal = Wildlife::new(0, WildlifeKind::SmallCritter, 2000.0, 2000.0, &mut rng);
        let players = vec![(2010.0_f32, 2010.0_f32)];
        animal.tick(0.1, &players, &mut rng);
        assert_eq!(animal.state_str(), "fleeing");
    }

    #[test]
    fn after_flee_returns_home() {
        let mut rng = make_rng();
        let mut animal = Wildlife::new(0, WildlifeKind::WildDeer, 2000.0, 2000.0, &mut rng);
        // 觸發逃跑。
        let players = vec![(2010.0_f32, 2010.0_f32)];
        animal.tick(0.1, &players, &mut rng);
        // 讓逃跑計時器跑完（dt=FLEE_DURATION + 一點）。
        animal.tick(FLEE_DURATION + 0.5, &[], &mut rng);
        // 應轉成 Returning。
        assert!(
            matches!(animal.state, WildlifeState::Returning)
            || matches!(animal.state, WildlifeState::Wandering { .. })
            || matches!(animal.state, WildlifeState::Resting { .. }),
            "應進入返家流程，實際: {:?}", animal.state
        );
    }

    #[test]
    fn manager_tick_runs_without_panic() {
        let mut mgr = WildlifeManager::new();
        let players = vec![(2200.0, 2200.0)];
        for _ in 0..50 {
            mgr.tick(0.1, &players);
        }
        // 全部動物都還在（不會消失）。
        assert_eq!(mgr.animals.len(), WILDLIFE_COUNT);
    }
}
