//! 萬尾釣魚大賽（ROADMAP 523）。
//!
//! 每隔 `CONTEST_INTERVAL_SECS` 秒，全服開場一次限時釣魚大賽，
//! 持續 `CONTEST_DURATION_SECS` 秒；在此時間內釣到的魚以**總體長（公釐）**累計排行，
//! 結束後廣播前三名並發乙太獎勵。
//!
//! 設計鐵律：
//! - **純邏輯、零 IO、零 LLM**：純函式 + 確定性計時，可完整單元測試。
//! - **純記憶體、零 migration、零持久化**：重啟後重計等待計時器。
//! - **技巧型競技**：既有釣魚小遊戲（346）的收竿準確度影響魚的尺寸（449），
//!   因此大賽排名獎勵的不只是「熬時間」，而是收竿技巧。

use std::collections::HashMap;

use uuid::Uuid;

/// 兩次大賽之間的等待秒數（45 分鐘）。
pub const CONTEST_INTERVAL_SECS: f32 = 45.0 * 60.0;

/// 伺服器啟動後首次大賽的等待秒數（15 分鐘），避免上線立刻觸發。
pub const CONTEST_FIRST_DELAY_SECS: f32 = 15.0 * 60.0;

/// 每場大賽的持續時間（5 分鐘）。
pub const CONTEST_DURATION_SECS: f32 = 5.0 * 60.0;

/// 前三名的乙太獎勵（冠軍 / 亞軍 / 季軍）。
pub const WINNER_REWARDS: [u32; 3] = [40, 20, 10];

/// 一名參賽者的累積記錄。
#[derive(Debug, Clone)]
pub struct ContestCatcher {
    /// 顯示名稱（廣播用）。
    pub name: String,
    /// 本場累積體長總和（公釐）。
    pub total_mm: u32,
    /// 本場釣到的尾數。
    pub catch_count: u32,
}

/// 一場進行中的釣魚大賽狀態。
#[derive(Debug)]
pub struct ActiveContest {
    /// 剩餘秒數。
    pub remaining: f32,
    /// 各玩家的參賽記錄。
    pub catchers: HashMap<Uuid, ContestCatcher>,
}

impl ActiveContest {
    fn new() -> Self {
        Self {
            remaining: CONTEST_DURATION_SECS,
            catchers: HashMap::new(),
        }
    }
}

/// 萬尾釣魚大賽的全域狀態（記憶體、零持久化）。
#[derive(Debug)]
pub struct FishingContest {
    /// 距下次大賽開場的倒數秒數；無活躍大賽時遞減。
    pub countdown: f32,
    /// 當前活躍大賽（None = 休眠中）。
    pub active: Option<ActiveContest>,
}

impl Default for FishingContest {
    fn default() -> Self {
        Self::new()
    }
}

/// 大賽結束時，一名排名參賽者的結果。
#[derive(Debug, Clone)]
pub struct ContestResult {
    /// 玩家顯示名稱。
    pub name: String,
    /// 本場累積體長總和（公釐）。
    pub total_mm: u32,
    /// 本場釣到的尾數。
    pub catch_count: u32,
    /// 名次（1/2/3，四名後不列入）。
    pub rank: u8,
    /// 獲得的乙太獎勵（前三有、四名後 0）。
    pub ether_reward: u32,
}

/// `FishingContest::tick` 每幀回傳的事件。
pub enum FishingContestTick {
    /// 一般推進，無特殊事件。
    None,
    /// 大賽剛開場。
    Started,
    /// 大賽結束；附帶前三名排行（可能為空）。
    Finished(Vec<ContestResult>),
}

impl FishingContest {
    pub fn new() -> Self {
        Self {
            countdown: CONTEST_FIRST_DELAY_SECS,
            active: None,
        }
    }

    /// 每 tick 推進 `dt` 秒（dt 非有限數時保守跳過）。
    pub fn tick(&mut self, dt: f32) -> FishingContestTick {
        if !dt.is_finite() || dt <= 0.0 {
            return FishingContestTick::None;
        }

        if let Some(ref mut ev) = self.active {
            ev.remaining -= dt;
            if ev.remaining <= 0.0 {
                let results = self.finish_event();
                return FishingContestTick::Finished(results);
            }
            return FishingContestTick::None;
        }

        // 休眠中：倒數到下一場
        self.countdown -= dt;
        if self.countdown <= 0.0 {
            self.active = Some(ActiveContest::new());
            self.countdown = CONTEST_INTERVAL_SECS;
            return FishingContestTick::Started;
        }
        FishingContestTick::None
    }

    /// 結束當前大賽並回傳排行（呼叫後 `active` 歸 None）。
    fn finish_event(&mut self) -> Vec<ContestResult> {
        let ev = match self.active.take() {
            Some(e) => e,
            None => return vec![],
        };

        // 依累計體長降序排列，相同時按尾數多的優先，再按名稱字典序（確定性）。
        let mut ranked: Vec<ContestCatcher> = ev.catchers.into_values().collect();
        ranked.sort_by(|a, b| {
            b.total_mm
                .cmp(&a.total_mm)
                .then(b.catch_count.cmp(&a.catch_count))
                .then(a.name.cmp(&b.name))
        });

        ranked
            .into_iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let rank = (i + 1) as u8;
                let ether_reward = WINNER_REWARDS.get(i).copied().unwrap_or(0);
                // 只回傳有魚的參賽者（零尾不列入）
                if c.catch_count == 0 {
                    return None;
                }
                Some(ContestResult {
                    name: c.name,
                    total_mm: c.total_mm,
                    catch_count: c.catch_count,
                    rank,
                    ether_reward,
                })
            })
            .take(3)
            .collect()
    }

    /// 釣到一尾魚時呼叫，記入大賽排行（非大賽期間呼叫無效）。
    /// `size_mm` 為 0 時忽略（非記錄魚種）。
    pub fn record_catch(&mut self, player_id: Uuid, name: &str, size_mm: u32) {
        if size_mm == 0 {
            return;
        }
        let ev = match self.active.as_mut() {
            Some(e) => e,
            None => return,
        };
        let entry = ev.catchers.entry(player_id).or_insert_with(|| ContestCatcher {
            name: name.to_string(),
            total_mm: 0,
            catch_count: 0,
        });
        entry.total_mm = entry.total_mm.saturating_add(size_mm);
        entry.catch_count += 1;
    }

    /// 大賽是否正在進行。
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// 大賽剩餘秒數（非進行中回 None）。
    pub fn remaining_secs(&self) -> Option<f32> {
        self.active.as_ref().map(|ev| ev.remaining)
    }

    /// 當前前三名快照（供快照廣播用）：`(顯示名, 累計體長mm, 尾數)`。
    pub fn top3(&self) -> Vec<(String, u32, u32)> {
        let ev = match &self.active {
            Some(e) => e,
            None => return vec![],
        };
        let mut catchers: Vec<&ContestCatcher> = ev.catchers.values().collect();
        catchers.sort_by(|a, b| {
            b.total_mm
                .cmp(&a.total_mm)
                .then(b.catch_count.cmp(&a.catch_count))
                .then(a.name.cmp(&b.name))
        });
        catchers
            .into_iter()
            .filter(|c| c.catch_count > 0)
            .take(3)
            .map(|c| (c.name.clone(), c.total_mm, c.catch_count))
            .collect()
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_id(n: u64) -> Uuid {
        Uuid::from_u64_pair(0, n)
    }

    #[test]
    fn 初始倒數正確() {
        let fc = FishingContest::new();
        assert!((fc.countdown - CONTEST_FIRST_DELAY_SECS).abs() < f32::EPSILON);
        assert!(fc.active.is_none());
    }

    #[test]
    fn tick倒數遞減_不觸發() {
        let mut fc = FishingContest::new();
        let r = fc.tick(60.0);
        assert!(matches!(r, FishingContestTick::None));
        assert!(!fc.is_active());
    }

    #[test]
    fn tick到零觸發Started() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        let r = fc.tick(1.0);
        assert!(matches!(r, FishingContestTick::Started));
        assert!(fc.is_active());
    }

    #[test]
    fn 大賽中remaining正確() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        fc.tick(1.0); // Started
        let rem0 = fc.remaining_secs().unwrap();
        fc.tick(30.0);
        let rem1 = fc.remaining_secs().unwrap();
        assert!(rem0 > rem1);
        assert!((rem0 - CONTEST_DURATION_SECS).abs() < 1.0);
    }

    #[test]
    fn record_catch累加體長() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        fc.tick(1.0); // Started
        let id = make_id(1);
        fc.record_catch(id, "甲", 200);
        fc.record_catch(id, "甲", 350);
        let top3 = fc.top3();
        assert_eq!(top3.len(), 1);
        assert_eq!(top3[0].1, 550); // total_mm
        assert_eq!(top3[0].2, 2);   // catch_count
    }

    #[test]
    fn top3排序正確() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        fc.tick(1.0); // Started
        fc.record_catch(make_id(1), "甲", 100);
        fc.record_catch(make_id(2), "乙", 500);
        fc.record_catch(make_id(3), "丙", 300);
        let top3 = fc.top3();
        assert_eq!(top3[0].0, "乙");
        assert_eq!(top3[1].0, "丙");
        assert_eq!(top3[2].0, "甲");
    }

    #[test]
    fn 大賽結束回傳ContestResult() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        fc.tick(1.0); // Started
        fc.record_catch(make_id(1), "甲", 400);
        fc.record_catch(make_id(2), "乙", 200);
        // 讓時間跑完
        let mut result = vec![];
        for _ in 0..400 {
            if let FishingContestTick::Finished(r) = fc.tick(1.0) {
                result = r;
                break;
            }
        }
        assert!(!result.is_empty());
        assert_eq!(result[0].name, "甲");
        assert_eq!(result[0].rank, 1);
        assert_eq!(result[1].name, "乙");
        assert_eq!(result[1].rank, 2);
    }

    #[test]
    fn 前三名獎勵正確() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        fc.tick(1.0); // Started
        fc.record_catch(make_id(1), "甲", 400);
        fc.record_catch(make_id(2), "乙", 300);
        fc.record_catch(make_id(3), "丙", 200);
        let mut result = vec![];
        for _ in 0..400 {
            if let FishingContestTick::Finished(r) = fc.tick(1.0) {
                result = r;
                break;
            }
        }
        assert_eq!(result[0].ether_reward, WINNER_REWARDS[0]);
        assert_eq!(result[1].ether_reward, WINNER_REWARDS[1]);
        assert_eq!(result[2].ether_reward, WINNER_REWARDS[2]);
    }

    #[test]
    fn 無人參賽結束回傳空() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        fc.tick(1.0); // Started
        let mut result = vec![];
        for _ in 0..400 {
            if let FishingContestTick::Finished(r) = fc.tick(1.0) {
                result = r;
                break;
            }
        }
        assert!(result.is_empty());
    }

    #[test]
    fn dt非有限數保守跳過() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        let r1 = fc.tick(f32::NAN);
        assert!(matches!(r1, FishingContestTick::None));
        assert!(!fc.is_active()); // 沒有觸發
    }

    #[test]
    fn 非大賽期間record_catch無效() {
        let mut fc = FishingContest::new();
        fc.record_catch(make_id(1), "甲", 500); // 大賽未啟動
        assert!(fc.top3().is_empty());
    }

    #[test]
    fn size_mm為零忽略() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        fc.tick(1.0); // Started
        fc.record_catch(make_id(1), "甲", 0); // 非記錄魚種
        assert!(fc.top3().is_empty()); // 不應入榜
    }

    #[test]
    fn 結束後倒數重置到下一場() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        fc.tick(1.0); // Started
        for _ in 0..400 {
            if matches!(fc.tick(1.0), FishingContestTick::Finished(_)) {
                break;
            }
        }
        assert!(!fc.is_active());
        // countdown 應已重置（不再是首次延遲）
        assert!((fc.countdown - CONTEST_INTERVAL_SECS).abs() < 1.0);
    }

    #[test]
    fn 同人多次釣魚持續累加() {
        let mut fc = FishingContest::new();
        fc.countdown = 0.1;
        fc.tick(1.0); // Started
        let id = make_id(7);
        for _ in 0..5 {
            fc.record_catch(id, "旅人", 100);
        }
        let top3 = fc.top3();
        assert_eq!(top3[0].1, 500); // 5 × 100mm
        assert_eq!(top3[0].2, 5);
    }
}
