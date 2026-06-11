//! 路人 / 居民 NPC 系統（ROADMAP 115 + 116）。
//!
//! 城鎮隨繁榮度自然成長一批「廉價」路人——純模板驅動，不呼叫 LLM。
//! 每位居民在城鎮範圍內緩慢閒晃，讓世界看起來有人氣。
//!
//! 分層架構（成本鐵律）：
//! - 少數深度 AI NPC（merchant / village_chief 等）→ 呼叫 LLM，有個性、記憶、生命週期。
//! - 多數路人居民（本模組）→ 模板行為，零 LLM 費用，只負責讓世界「看起來熱鬧」。
//!
//! 人口規則（湧現，非寫死）：
//! - 每位居民有自己的壽命計時器（ROADMAP 116）：
//!   90% 壽命 → 廣播告別倒數；100% → 回歸乙太，新居民遷入替補。
//! - 每 POPULATION_CHECK_SECS 秒依「全村平均繁榮感」增減人口：
//!   > GROW_THRESHOLD → 新移民遷入（含廣播）；< SHRINK_THRESHOLD → 靜靜離去（含廣播）。
//!
//! 完全記憶體模式，重啟清零，零 migration。

use rand::{Rng, SeedableRng, rngs::StdRng};

// ── 居民生命週期常數（ROADMAP 116）──────────────────────────────────────────
/// 居民壽命預設（秒，真實時間）。約 45 分鐘；可用 BUTFUN_RESIDENT_LIFESPAN_SECS 覆寫。
pub const RESIDENT_LIFESPAN_SECS_DEFAULT: f32 = 2700.0;
const RESIDENT_RETIREMENT_FRACTION: f32 = 0.90;

pub fn resident_lifespan_secs() -> f32 {
    std::env::var("BUTFUN_RESIDENT_LIFESPAN_SECS")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(RESIDENT_LIFESPAN_SECS_DEFAULT)
}

// ── 生命週期公告文字 ──────────────────────────────────────────────────────────
fn retirement_msg(name: &str) -> String {
    format!("🕯️ {} 感到乙太的呼喚，即將離開這片土地……", name)
}

fn farewell_msg(name: &str) -> String {
    format!("✨ {} 在乙太之光中安詳告別，感謝大家這段時間的陪伴。", name)
}

fn arrival_from_predecessor_msg(new_name: &str, old_name: &str) -> String {
    format!("🌱 {} 帶著對 {} 的思念遷入村落，展開全新生活。", new_name, old_name)
}

fn new_arrival_msg(name: &str) -> String {
    format!("🌱 {} 從遠方遷入，為村落帶來新氣象。", name)
}

fn departed_msg(name: &str) -> String {
    format!("🍂 {} 決定離開村落，踏上新的旅途。祝一路平安。", name)
}

// ── 生命週期事件（ROADMAP 116）───────────────────────────────────────────────
/// 居民生命週期事件，由 game.rs 廣播至世界聊天。
pub enum ResidentLifecycleEvent {
    /// 居民即將回歸乙太（90% 壽命），廣播告別公告。
    RetirementSoon { name: &'static str, msg: String },
    /// 居民完成生命週期，新居民遷入替補。
    RetiredToEther { old_name: &'static str, new_name: &'static str, farewell_msg: String, arrival_msg: String },
    /// 繁榮帶來新移民遷入（不是替補，是真的人口增加）。
    NewArrival { name: &'static str, msg: String },
    /// 凋零造成人口外移（靜靜離去）。
    Departed { name: &'static str, msg: String },
}

/// 故鄉城鎮閒晃邊界（像素）。
const WANDER_X_MIN: f32 = 1900.0;
const WANDER_X_MAX: f32 = 3100.0;
const WANDER_Y_MIN: f32 = 1850.0;
const WANDER_Y_MAX: f32 = 3050.0;

/// 居民移動速度（像素/秒）。比玩家慢、比有排程 NPC 慢，看起來悠閒。
const MOVE_SPEED: f32 = 40.0;

/// 到達目標附近後，再等幾秒才換新目標（在附近閒站）。
const WAIT_SECS_MIN: f32 = 3.0;
const WAIT_SECS_MAX: f32 = 12.0;

/// 抵達目標的判定半徑（像素）。
const ARRIVE_DIST: f32 = 8.0;

/// 人口下限：世界最冷清時至少這麼多人。
pub const MIN_POPULATION: usize = 2;

/// 人口上限：繁榮到頂時至多這麼多人。
pub const MAX_POPULATION: usize = 12;

/// 繁榮感觸發「新增居民」的閾值（所有 NPC 繁榮感平均 > 此值）。
const GROW_THRESHOLD: i32 = 60;

/// 繁榮感觸發「移除居民」的閾值（所有 NPC 繁榮感平均 < 此值）。
const SHRINK_THRESHOLD: i32 = 30;

/// 人口檢查週期（秒）。
pub const POPULATION_CHECK_SECS: f32 = 300.0;

/// 居民名字池——純在地化字串，不接 LLM。
static NAME_POOL: &[&str] = &[
    "阿土", "梅子", "石頭", "阿花", "老根",
    "翠蓮", "阿明", "阿珠", "狗蛋", "小紅",
    "二柱", "玉蘭", "三寶", "春花", "老鐵",
    "阿水", "嬌嬌", "大牛", "秀英", "阿發",
];

/// 居民行為類型（決定在哪一帶閒晃）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidentPersona {
    /// 四處遊走，整個城鎮都去。
    Wanderer,
    /// 在市場攤區附近逛。
    MarketBrowser,
    /// 在農田地帶附近勞動。
    FarmWorker,
    /// 主要停留在廣場 / 鎮中心。
    TownSquare,
}

impl ResidentPersona {
    /// 依 persona 決定閒晃的 x/y 邊界（可部分重疊形成自然人流）。
    fn wander_bounds(&self) -> (f32, f32, f32, f32) {
        match self {
            // 市場攤區：npc_schedule 商人附近 (2000~2700, 1950~2450)
            ResidentPersona::MarketBrowser => (2000.0, 2700.0, 1950.0, 2450.0),
            // 農田區：公共農地偏南 (1900~2600, 2300~3050)
            ResidentPersona::FarmWorker    => (1900.0, 2600.0, 2300.0, 3050.0),
            // 廣場：鎮中心 (2200~2800, 2000~2600)
            ResidentPersona::TownSquare    => (2200.0, 2800.0, 2000.0, 2600.0),
            // 全城亂逛
            ResidentPersona::Wanderer      => (WANDER_X_MIN, WANDER_X_MAX, WANDER_Y_MIN, WANDER_Y_MAX),
        }
    }

    /// 依居民 index 分配 persona（讓城鎮人流分佈自然）。
    fn for_index(i: usize) -> Self {
        match i % 4 {
            0 => ResidentPersona::MarketBrowser,
            1 => ResidentPersona::FarmWorker,
            2 => ResidentPersona::TownSquare,
            _ => ResidentPersona::Wanderer,
        }
    }
}

/// 單一居民的完整運行狀態。
#[derive(Debug, Clone)]
pub struct ResidentNpc {
    /// 系統 id（"resident_0"、"resident_1"……）。
    pub id: String,
    /// 顯示名（從 NAME_POOL 取）。
    pub name: &'static str,
    pub persona: ResidentPersona,
    pub x: f32,
    pub y: f32,
    /// 當前行走目標。
    target_x: f32,
    target_y: f32,
    /// 到達目標後的等待秒數。> 0 = 在等、 <= 0 = 可選下個目標。
    wait_timer: f32,
    // ── 生命週期（ROADMAP 116）──────────────────────────
    /// 已活的秒數（真實時間）。
    pub age_secs: f32,
    /// 這一生的壽命（秒）。
    pub lifespan_secs: f32,
    /// 退休公告是否已發送（防重複廣播）。
    retirement_announced: bool,
}

impl ResidentNpc {
    /// 用確定性 seed（依 index）初始化，保證每次重啟位置稍有不同但可預期。
    fn new(index: usize, rng: &mut impl Rng) -> Self {
        let persona = ResidentPersona::for_index(index);
        let (x0, x1, y0, y1) = persona.wander_bounds();
        let x = rng.gen_range(x0..=x1);
        let y = rng.gen_range(y0..=y1);
        let tx = rng.gen_range(x0..=x1);
        let ty = rng.gen_range(y0..=y1);
        let name = NAME_POOL[index % NAME_POOL.len()];
        Self {
            id: format!("resident_{}", index),
            name,
            persona,
            x,
            y,
            target_x: tx,
            target_y: ty,
            wait_timer: rng.gen_range(0.0..WAIT_SECS_MAX),
            age_secs: 0.0,
            lifespan_secs: resident_lifespan_secs(),
            retirement_announced: false,
        }
    }

    /// 壽命是否到了（應回歸乙太）。
    fn should_retire(&self) -> bool {
        self.age_secs >= self.lifespan_secs
    }

    /// 退休公告是否應發送（90% 壽命 且尚未發送）。
    fn should_announce_retirement(&self) -> bool {
        !self.retirement_announced
            && self.age_secs >= self.lifespan_secs * RESIDENT_RETIREMENT_FRACTION
    }

    /// 每幀推進：移動 + 等待計時。
    fn tick(&mut self, dt: f32, rng: &mut impl Rng) {
        if self.wait_timer > 0.0 {
            self.wait_timer -= dt;
            return;
        }
        let dx = self.target_x - self.x;
        let dy = self.target_y - self.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < ARRIVE_DIST {
            // 到了，等一下再換目標
            self.wait_timer = rng.gen_range(WAIT_SECS_MIN..=WAIT_SECS_MAX);
            let (x0, x1, y0, y1) = self.persona.wander_bounds();
            self.target_x = rng.gen_range(x0..=x1);
            self.target_y = rng.gen_range(y0..=y1);
        } else {
            let step = (MOVE_SPEED * dt).min(dist);
            self.x += dx / dist * step;
            self.y += dy / dist * step;
        }
    }
}

/// 居民群體管理器。
pub struct ResidentManager {
    pub residents: Vec<ResidentNpc>,
    /// 人口檢查計時器。
    population_timer: f32,
    /// 下一個新居民的 index（只增不減，保證 id 唯一）。
    next_index: usize,
    /// 隨機源（種子固定，重啟後走同一條路但不重要）。
    rng: StdRng,
}

impl ResidentManager {
    /// 從最小人口出發建立管理器。
    pub fn new() -> Self {
        let mut rng = StdRng::seed_from_u64(42);
        let residents: Vec<ResidentNpc> = (0..MIN_POPULATION)
            .map(|i| ResidentNpc::new(i, &mut rng))
            .collect();
        Self {
            next_index: residents.len(),
            residents,
            population_timer: POPULATION_CHECK_SECS,
            rng,
        }
    }

    /// 每幀推進：移動所有居民 + 生命週期 + 人口增減。
    /// 回傳本幀發生的生命週期事件，供 game.rs 廣播至世界聊天。
    pub fn tick(&mut self, dt: f32, avg_prosperity: i32) -> Vec<ResidentLifecycleEvent> {
        let mut events = Vec::new();

        // 1. 推進每位居民的年齡與移動
        for r in &mut self.residents {
            r.age_secs += dt;
            r.tick(dt, &mut self.rng);
        }

        // 2. 退休公告（90% 壽命，防重複）
        for r in &mut self.residents {
            if r.should_announce_retirement() {
                r.retirement_announced = true;
                events.push(ResidentLifecycleEvent::RetirementSoon {
                    name: r.name,
                    msg: retirement_msg(r.name),
                });
            }
        }

        // 3. 壽命到期：回歸乙太 + 新居民遷入替補（每幀最多處理一位，防同時大量廣播）
        if let Some(pos) = self.residents.iter().position(|r| r.should_retire()) {
            let old = self.residents.remove(pos);
            let new_idx = self.next_index;
            self.next_index += 1;
            let new_r = ResidentNpc::new(new_idx, &mut self.rng);
            let farewell = farewell_msg(old.name);
            let arrival = arrival_from_predecessor_msg(new_r.name, old.name);
            events.push(ResidentLifecycleEvent::RetiredToEther {
                old_name: old.name,
                new_name: new_r.name,
                farewell_msg: farewell,
                arrival_msg: arrival,
            });
            self.residents.push(new_r);
        }

        // 4. 繁榮感驅動的人口增減（每 POPULATION_CHECK_SECS 秒一次）
        self.population_timer -= dt;
        if self.population_timer <= 0.0 {
            self.population_timer = POPULATION_CHECK_SECS;
            if avg_prosperity >= GROW_THRESHOLD && self.residents.len() < MAX_POPULATION {
                let idx = self.next_index;
                self.next_index += 1;
                let r = ResidentNpc::new(idx, &mut self.rng);
                events.push(ResidentLifecycleEvent::NewArrival {
                    name: r.name,
                    msg: new_arrival_msg(r.name),
                });
                self.residents.push(r);
            } else if avg_prosperity < SHRINK_THRESHOLD && self.residents.len() > MIN_POPULATION {
                if let Some(r) = self.residents.pop() {
                    events.push(ResidentLifecycleEvent::Departed {
                        name: r.name,
                        msg: departed_msg(r.name),
                    });
                }
            }
        }

        events
    }

    /// 回傳目前最年長居民的名字（供 AI NPC 收徒時點名使用）。
    pub fn oldest_resident_name(&self) -> Option<&'static str> {
        self.residents
            .iter()
            .max_by(|a, b| a.age_secs.partial_cmp(&b.age_secs).unwrap_or(std::cmp::Ordering::Equal))
            .map(|r| r.name)
    }

    /// 回傳所有居民的 (id, name, x, y)，供快照組裝用。
    pub fn views(&self) -> impl Iterator<Item = (&str, &str, f32, f32)> {
        self.residents.iter().map(|r| (r.id.as_str(), r.name, r.x, r.y))
    }

    /// 目前居民人數（供測試用）。
    pub fn population(&self) -> usize {
        self.residents.len()
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_population_within_bounds() {
        let mgr = ResidentManager::new();
        assert!(mgr.population() >= MIN_POPULATION);
        assert!(mgr.population() <= MAX_POPULATION);
    }

    #[test]
    fn population_grows_when_prosperous() {
        let mut mgr = ResidentManager::new();
        let initial = mgr.population();
        // 直接觸發人口檢查（把計時器歸零）
        mgr.population_timer = 0.0;
        let events = mgr.tick(0.01, GROW_THRESHOLD + 1);
        assert_eq!(mgr.population(), (initial + 1).min(MAX_POPULATION));
        // 繁榮帶來移民事件
        if initial < MAX_POPULATION {
            assert!(events.iter().any(|e| matches!(e, ResidentLifecycleEvent::NewArrival { .. })));
        }
    }

    #[test]
    fn population_shrinks_when_poor() {
        let mut mgr = ResidentManager::new();
        // 先讓人口超過最小值
        mgr.population_timer = 0.0;
        mgr.tick(0.01, GROW_THRESHOLD + 1);
        mgr.population_timer = 0.0;
        mgr.tick(0.01, GROW_THRESHOLD + 1);
        let before = mgr.population();
        mgr.population_timer = 0.0;
        let events = mgr.tick(0.01, SHRINK_THRESHOLD - 1);
        assert_eq!(mgr.population(), (before - 1).max(MIN_POPULATION));
        if before > MIN_POPULATION {
            assert!(events.iter().any(|e| matches!(e, ResidentLifecycleEvent::Departed { .. })));
        }
    }

    #[test]
    fn population_never_below_min() {
        let mut mgr = ResidentManager::new();
        // 多次觸發衰退
        for _ in 0..20 {
            mgr.population_timer = 0.0;
            mgr.tick(0.01, 0);
        }
        assert!(mgr.population() >= MIN_POPULATION);
    }

    #[test]
    fn population_never_above_max() {
        let mut mgr = ResidentManager::new();
        for _ in 0..20 {
            mgr.population_timer = 0.0;
            mgr.tick(0.01, 100);
        }
        assert!(mgr.population() <= MAX_POPULATION);
    }

    #[test]
    fn residents_move_within_bounds() {
        let mut mgr = ResidentManager::new();
        // 跑 60 秒模擬
        for _ in 0..600 {
            mgr.tick(0.1, 50);
        }
        for r in &mgr.residents {
            // 居民不該衝出全城大邊界
            assert!(r.x >= WANDER_X_MIN - 1.0 && r.x <= WANDER_X_MAX + 1.0,
                "x out of bounds: {}", r.x);
            assert!(r.y >= WANDER_Y_MIN - 1.0 && r.y <= WANDER_Y_MAX + 1.0,
                "y out of bounds: {}", r.y);
        }
    }

    #[test]
    fn retirement_announced_at_correct_fraction() {
        let mut mgr = ResidentManager::new();
        // 把第一位居民設到退休門檻前一步
        mgr.residents[0].lifespan_secs = 100.0;
        mgr.residents[0].age_secs = 88.0; // 89% < 90%
        // tick 一下，不應觸發
        let ev = mgr.tick(0.5, 50);
        assert!(!ev.iter().any(|e| matches!(e, ResidentLifecycleEvent::RetirementSoon { .. })));
        // 再 tick 過 90%
        let ev2 = mgr.tick(2.0, 50);
        assert!(ev2.iter().any(|e| matches!(e, ResidentLifecycleEvent::RetirementSoon { .. })));
        // 已標記，再 tick 不重複
        let ev3 = mgr.tick(0.5, 50);
        assert!(!ev3.iter().any(|e| matches!(e, ResidentLifecycleEvent::RetirementSoon { .. })));
    }

    #[test]
    fn retired_to_ether_replaces_resident() {
        let mut mgr = ResidentManager::new();
        let before = mgr.population();
        let old_name = {
            mgr.residents[0].lifespan_secs = 100.0;
            mgr.residents[0].age_secs = 100.0; // 壽命到期
            mgr.residents[0].name
        };
        let ev = mgr.tick(0.01, 50);
        // 人口不變（退休 + 新居民）
        assert_eq!(mgr.population(), before);
        // 事件應存在
        let retired = ev.iter().find(|e| matches!(e, ResidentLifecycleEvent::RetiredToEther { .. }));
        assert!(retired.is_some());
        if let Some(ResidentLifecycleEvent::RetiredToEther { old_name: on, new_name: nn, .. }) = retired {
            assert_eq!(*on, old_name);
            assert_ne!(*nn, old_name, "新居民應使用不同名字");
        }
    }

    #[test]
    fn oldest_resident_name_returns_most_aged() {
        let mut mgr = ResidentManager::new();
        mgr.residents[0].age_secs = 999.0;
        mgr.residents[1].age_secs = 1.0;
        let oldest = mgr.oldest_resident_name().unwrap();
        assert_eq!(oldest, mgr.residents[0].name);
    }
}
