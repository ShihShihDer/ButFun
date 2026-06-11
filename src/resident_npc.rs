//! 路人 / 居民 NPC 系統（ROADMAP 115）。
//!
//! 城鎮隨繁榮度自然成長一批「廉價」路人——純模板驅動，不呼叫 LLM。
//! 每位居民在城鎮範圍內緩慢閒晃，讓世界看起來有人氣。
//!
//! 分層架構（成本鐵律）：
//! - 少數深度 AI NPC（merchant / village_chief 等）→ 呼叫 LLM，有個性、記憶、生命週期。
//! - 多數路人居民（本模組）→ 模板行為，零 LLM 費用，只負責讓世界「看起來熱鬧」。
//!
//! 人口增長規則（湧現，非寫死）：
//! - 每 POPULATION_CHECK_SECS 秒檢查一次「全村平均繁榮感」。
//! - 繁榮感 > GROW_THRESHOLD → 新增一個居民（不超過 MAX_POPULATION）。
//! - 繁榮感 < SHRINK_THRESHOLD → 移除最後一個居民（不低於 MIN_POPULATION）。
//!
//! 完全記憶體模式，重啟清零，零 migration。

use rand::{Rng, SeedableRng, rngs::StdRng};

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
        }
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

    /// 每幀推進：移動所有居民 + 人口增減。
    /// `avg_prosperity`: 所有深度 AI NPC 的繁榮感平均值（0~100）。
    pub fn tick(&mut self, dt: f32, avg_prosperity: i32) {
        // 移動居民
        for r in &mut self.residents {
            r.tick(dt, &mut self.rng);
        }
        // 人口檢查
        self.population_timer -= dt;
        if self.population_timer > 0.0 {
            return;
        }
        self.population_timer = POPULATION_CHECK_SECS;
        if avg_prosperity >= GROW_THRESHOLD && self.residents.len() < MAX_POPULATION {
            let idx = self.next_index;
            self.next_index += 1;
            let r = ResidentNpc::new(idx, &mut self.rng);
            self.residents.push(r);
        } else if avg_prosperity < SHRINK_THRESHOLD && self.residents.len() > MIN_POPULATION {
            self.residents.pop();
        }
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
        mgr.tick(0.01, GROW_THRESHOLD + 1);
        assert_eq!(mgr.population(), (initial + 1).min(MAX_POPULATION));
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
        mgr.tick(0.01, SHRINK_THRESHOLD - 1);
        assert_eq!(mgr.population(), (before - 1).max(MIN_POPULATION));
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
}
