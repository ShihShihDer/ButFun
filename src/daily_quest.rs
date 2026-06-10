//! 每日任務系統（ROADMAP 32）：每位玩家每天 3 條個人任務，24 小時後重置。
//!
//! 純邏輯層（無 IO / 無 WebSocket）：任務種類定義、確定性生成、進度推進、重置判斷。
//! 任務在玩家首次觸發（請求查詢或抵達事件）時惰性初始化，不持久化（重啟當天從頭算）。

use crate::combat::EnemyKind;
use crate::inventory::ItemKind;

/// 每條任務完成時，玩家得到的乙太獎勵。
pub const DAILY_TASK_ETHER_REWARD: u32 = 15;

/// 每條任務完成時，玩家得到的 EXP 獎勵。
pub const DAILY_TASK_EXP_REWARD: u32 = 80;

/// 任務自動重置的周期（秒）：24 小時。
pub const DAILY_RESET_SECS: u64 = 86_400;

/// 每位玩家同時持有的每日任務數。
pub const DAILY_TASK_COUNT: usize = 3;

/// 可以出現在擊殺型每日任務的敵人選單（6 種，確定性輪替）。
const KILL_POOL: &[(EnemyKind, u32, &str)] = &[
    (EnemyKind::ScrapDrone,      5, "銹蝕巡邏機"),
    (EnemyKind::EtherWisp,       4, "迷途乙太靈"),
    (EnemyKind::FlutterSprite,   6, "飄舞精靈"),
    (EnemyKind::MushroomStalker, 4, "蕈菇潛行者"),
    (EnemyKind::CrystalGolem,    3, "晶石傀儡"),
    (EnemyKind::RuneGuardian,    3, "符文守衛"),
];

/// 可以出現在採集型每日任務的物品選單（6 種）。
const GATHER_POOL: &[(ItemKind, u32, &str)] = &[
    (ItemKind::Wood,           10, "木材"),
    (ItemKind::Stone,           8, "礦石"),
    (ItemKind::WildflowerSeed,  5, "野花種子"),
    (ItemKind::MushroomSpore,   4, "蕈菇孢子"),
    (ItemKind::CrystalShard,    3, "晶石碎片"),
    (ItemKind::AncientFragment, 3, "古代碎片"),
];

/// 可以出現在旅行型每日任務的星球選單（4 顆）。
const TRAVEL_POOL: &[(&str, &str)] = &[
    ("verdant", "翠幽星"),
    ("crimson", "赤焰星"),
    ("void",    "虛空星"),
    ("aether",  "霧醚星"),
];

// ──────────────────────────────────────────────────────────────────────────────
// 任務目標
// ──────────────────────────────────────────────────────────────────────────────

/// 一條每日任務的目標種類。
#[derive(Debug, Clone, PartialEq)]
pub enum DailyTaskKind {
    Kill { kind: EnemyKind, count: u32 },
    Gather { item: ItemKind, count: u32 },
    Travel { planet: &'static str },
}

/// 一條正在進行中的每日任務。
#[derive(Debug, Clone)]
pub struct DailyTask {
    pub kind: DailyTaskKind,
    pub progress: u32,
    pub completed: bool,
    /// 前端顯示用的說明（繁中）。
    pub description: String,
}

impl DailyTask {
    fn new(kind: DailyTaskKind, description: String) -> Self {
        Self { kind, progress: 0, completed: false, description }
    }

    /// 任務的目標數量（Travel 型固定為 1）。
    pub fn goal(&self) -> u32 {
        match &self.kind {
            DailyTaskKind::Kill   { count, .. } => *count,
            DailyTaskKind::Gather { count, .. } => *count,
            DailyTaskKind::Travel { .. }        => 1,
        }
    }

    /// 推進進度；回傳 true 代表此次推進讓任務剛完成。
    pub fn advance(&mut self, amount: u32) -> bool {
        if self.completed { return false; }
        self.progress = (self.progress + amount).min(self.goal());
        if self.progress >= self.goal() {
            self.completed = true;
            true
        } else {
            false
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 玩家每日狀態
// ──────────────────────────────────────────────────────────────────────────────

/// 一位玩家的每日任務狀態（3 條任務 + 重置時刻 + 全完廣播旗標）。
#[derive(Debug, Clone)]
pub struct PlayerDailyState {
    pub tasks: Vec<DailyTask>,
    /// 此批任務在此 Unix timestamp（秒）後需重置。
    pub reset_at: u64,
    /// 三條全完時是否已廣播一次（避免重複廣播）。
    pub all_done_announced: bool,
}

impl PlayerDailyState {
    /// 以 `user_seed`（通常是玩家 UUID 的 u128 低 64 bit）與當前時刻建立今日任務集合。
    pub fn new(user_seed: u64, now_secs: u64) -> Self {
        let day_index = now_secs / DAILY_RESET_SECS;
        let reset_at  = (day_index + 1) * DAILY_RESET_SECS;
        Self {
            tasks: generate_tasks(user_seed, day_index),
            reset_at,
            all_done_announced: false,
        }
    }

    /// 若已過重置時刻，以今天的新任務取代。
    pub fn check_reset(&mut self, now_secs: u64, user_seed: u64) {
        if now_secs >= self.reset_at {
            *self = Self::new(user_seed, now_secs);
        }
    }

    /// 擊殺事件：回傳剛完成的任務索引（0-2），若無完成回 None。
    pub fn on_kill(&mut self, kind: EnemyKind) -> Option<usize> {
        for (i, task) in self.tasks.iter_mut().enumerate() {
            if let DailyTaskKind::Kill { kind: k, .. } = task.kind {
                if k == kind && task.advance(1) {
                    return Some(i);
                }
            }
        }
        None
    }

    /// 採集事件：回傳剛完成的任務索引，若無回 None。
    pub fn on_gather(&mut self, item: ItemKind, amount: u32) -> Option<usize> {
        for (i, task) in self.tasks.iter_mut().enumerate() {
            if let DailyTaskKind::Gather { item: it, .. } = task.kind {
                if it == item && task.advance(amount) {
                    return Some(i);
                }
            }
        }
        None
    }

    /// 旅行事件：回傳剛完成的任務索引，若無回 None。
    pub fn on_travel(&mut self, planet: &str) -> Option<usize> {
        for (i, task) in self.tasks.iter_mut().enumerate() {
            if let DailyTaskKind::Travel { planet: p } = task.kind {
                if p == planet && task.advance(1) {
                    return Some(i);
                }
            }
        }
        None
    }

    /// 三條任務是否全部完成。
    pub fn all_complete(&self) -> bool {
        self.tasks.len() == DAILY_TASK_COUNT
            && self.tasks.iter().all(|t| t.completed)
    }

    /// 完成數（0-3）。
    pub fn done_count(&self) -> usize {
        self.tasks.iter().filter(|t| t.completed).count()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 確定性任務生成（純函式，可測試）
// ──────────────────────────────────────────────────────────────────────────────

/// 以 `user_seed` 和 `day_index`（Unix 天數）確定性生成 3 條不重複種類的每日任務。
/// 同一玩家同一天永遠產生相同任務；不同玩家同一天任務不同（個人化）。
pub fn generate_tasks(user_seed: u64, day_index: u64) -> Vec<DailyTask> {
    // 分三個 slot 各取一條：Kill / Gather / Travel 各一
    let kill_idx   = cheap_hash(user_seed, day_index, 0) as usize % KILL_POOL.len();
    let gather_idx = cheap_hash(user_seed, day_index, 1) as usize % GATHER_POOL.len();
    let travel_idx = cheap_hash(user_seed, day_index, 2) as usize % TRAVEL_POOL.len();

    let (kind, count, name) = KILL_POOL[kill_idx];
    let kill_task = DailyTask::new(
        DailyTaskKind::Kill { kind, count },
        format!("擊殺 {} 隻{}（{}/{}）", count, name, 0, count),
    );

    let (item, count, name) = GATHER_POOL[gather_idx];
    let gather_task = DailyTask::new(
        DailyTaskKind::Gather { item, count },
        format!("採集 {} 個{}（{}/{}）", count, name, 0, count),
    );

    let (planet, planet_name) = TRAVEL_POOL[travel_idx];
    let travel_task = DailyTask::new(
        DailyTaskKind::Travel { planet },
        format!("旅行前往{}", planet_name),
    );

    vec![kill_task, gather_task, travel_task]
}

/// 簡單整數雜湊（非加密用途），用於確定性任務選取。
fn cheap_hash(seed: u64, day: u64, slot: u64) -> u64 {
    let mut h = seed ^ (day.wrapping_mul(2_654_435_761));
    h ^= slot.wrapping_mul(1_234_567_891);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ceb9fe1a85ec53);
    h ^= h >> 33;
    h
}

// ──────────────────────────────────────────────────────────────────────────────
// 前端 Wire 結構（序列化用）
// ──────────────────────────────────────────────────────────────────────────────

/// 任務資訊，序列化後送給前端顯示。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DailyTaskView {
    pub description: String,
    pub progress: u32,
    pub goal: u32,
    pub completed: bool,
}

impl DailyTask {
    pub fn to_view(&self) -> DailyTaskView {
        DailyTaskView {
            description: self.description.clone(),
            progress: self.progress,
            goal: self.goal(),
            completed: self.completed,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 測試
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_are_deterministic() {
        let seed = 12345_u64;
        let day  = 1000_u64;
        let tasks1 = generate_tasks(seed, day);
        let tasks2 = generate_tasks(seed, day);
        assert_eq!(tasks1.len(), 3);
        assert_eq!(tasks2.len(), 3);
        for (t1, t2) in tasks1.iter().zip(tasks2.iter()) {
            assert_eq!(t1.description, t2.description);
            assert_eq!(t1.goal(), t2.goal());
        }
    }

    #[test]
    fn tasks_differ_by_day() {
        let seed = 42_u64;
        let t0 = generate_tasks(seed, 0);
        let t1 = generate_tasks(seed, 1);
        // 不同天至少有一條任務說明不同（目標不同）
        let same = t0.iter().zip(t1.iter()).filter(|(a, b)| a.description == b.description).count();
        // 三條都相同的機率極低（hash 均勻散布）
        assert!(same < 3, "day 0 和 day 1 任務應至少有一條不同");
    }

    #[test]
    fn tasks_differ_by_seed() {
        let day = 500_u64;
        let ta = generate_tasks(0,    day);
        let tb = generate_tasks(9999, day);
        let same = ta.iter().zip(tb.iter()).filter(|(a, b)| a.description == b.description).count();
        assert!(same < 3, "不同 seed 同一天任務應至少有一條不同");
    }

    #[test]
    fn kill_task_advance_and_complete() {
        let mut state = PlayerDailyState::new(1, 0);
        // 找到擊殺任務並記錄 kind
        let kill_kind = state.tasks.iter().find_map(|t| {
            if let DailyTaskKind::Kill { kind, .. } = t.kind { Some(kind) } else { None }
        }).unwrap();
        let goal = state.tasks.iter().find_map(|t| {
            if let DailyTaskKind::Kill { count, .. } = t.kind { Some(count) } else { None }
        }).unwrap();

        // 推進 goal-1 次，還未完成
        for _ in 0..goal - 1 {
            assert!(state.on_kill(kill_kind).is_none());
        }
        // 第 goal 次完成
        assert!(state.on_kill(kill_kind).is_some());
    }

    #[test]
    fn gather_task_advance_and_complete() {
        let mut state = PlayerDailyState::new(2, 0);
        let (gather_item, goal) = state.tasks.iter().find_map(|t| {
            if let DailyTaskKind::Gather { item, count } = t.kind { Some((item, count)) } else { None }
        }).unwrap();

        // 批量推進
        let result = state.on_gather(gather_item, goal);
        assert!(result.is_some());
    }

    #[test]
    fn travel_task_advance_and_complete() {
        let mut state = PlayerDailyState::new(3, 0);
        let planet = state.tasks.iter().find_map(|t| {
            if let DailyTaskKind::Travel { planet } = t.kind { Some(planet) } else { None }
        }).unwrap();

        let result = state.on_travel(planet);
        assert!(result.is_some());
    }

    #[test]
    fn wrong_enemy_does_not_advance_kill_task() {
        let mut state = PlayerDailyState::new(4, 0);
        // 用和當天任務不同的 kind 打怪（ScrapDrone vs EtherWisp 等）
        // 先找任務的 kind，用另一個打
        let kill_kind = state.tasks.iter().find_map(|t| {
            if let DailyTaskKind::Kill { kind, .. } = t.kind { Some(kind) } else { None }
        }).unwrap();
        let other = if kill_kind == EnemyKind::ScrapDrone {
            EnemyKind::EtherWisp
        } else {
            EnemyKind::ScrapDrone
        };
        assert!(state.on_kill(other).is_none());
    }

    #[test]
    fn all_complete_after_finishing_three_tasks() {
        let mut state = PlayerDailyState::new(5, 0);
        assert!(!state.all_complete());

        // 完成擊殺任務
        let kill_kind = state.tasks.iter().find_map(|t| {
            if let DailyTaskKind::Kill { kind, count } = t.kind { Some((kind, count)) } else { None }
        }).unwrap();
        for _ in 0..kill_kind.1 { state.on_kill(kill_kind.0); }

        // 完成採集任務
        let (gi, gc) = state.tasks.iter().find_map(|t| {
            if let DailyTaskKind::Gather { item, count } = t.kind { Some((item, count)) } else { None }
        }).unwrap();
        state.on_gather(gi, gc);

        // 完成旅行任務
        let planet = state.tasks.iter().find_map(|t| {
            if let DailyTaskKind::Travel { planet } = t.kind { Some(planet) } else { None }
        }).unwrap();
        state.on_travel(planet);

        assert!(state.all_complete());
        assert_eq!(state.done_count(), 3);
    }

    #[test]
    fn check_reset_resets_when_expired() {
        let now = 0_u64;
        let mut state = PlayerDailyState::new(6, now);
        assert_eq!(state.reset_at, DAILY_RESET_SECS); // 第 0 天重置點 = 86400

        // 尚未到期：不重置
        state.check_reset(DAILY_RESET_SECS - 1, 6);
        assert_eq!(state.reset_at, DAILY_RESET_SECS);

        // 剛過期：重置
        state.check_reset(DAILY_RESET_SECS, 6);
        assert_eq!(state.reset_at, 2 * DAILY_RESET_SECS);
    }

    #[test]
    fn already_completed_task_does_not_over_advance() {
        let mut state = PlayerDailyState::new(7, 0);
        let (gi, gc) = state.tasks.iter().find_map(|t| {
            if let DailyTaskKind::Gather { item, count } = t.kind { Some((item, count)) } else { None }
        }).unwrap();
        state.on_gather(gi, gc); // 完成
        // 再推一次，應回 None（已完成不再觸發）
        assert!(state.on_gather(gi, 1).is_none());
    }

    #[test]
    fn generate_tasks_has_exactly_three_tasks() {
        for day in 0..30_u64 {
            let tasks = generate_tasks(day * 7, day);
            assert_eq!(tasks.len(), DAILY_TASK_COUNT);
        }
    }

    #[test]
    fn done_count_increments() {
        let mut state = PlayerDailyState::new(8, 0);
        assert_eq!(state.done_count(), 0);

        let (gi, gc) = state.tasks.iter().find_map(|t| {
            if let DailyTaskKind::Gather { item, count } = t.kind { Some((item, count)) } else { None }
        }).unwrap();
        state.on_gather(gi, gc);
        assert_eq!(state.done_count(), 1);
    }
}
