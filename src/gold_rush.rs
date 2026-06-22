//! 黃金礦脈爭奪戰（ROADMAP 521）。
//!
//! 每隔 `SPAWN_INTERVAL_SECS` 秒，全服在固定岩地座標自動湧現一條
//! **黃金礦脈**，限時 `DURATION_SECS` 秒；全員可在範圍內按鍵搶挖
//! （每人 `MINE_COOLDOWN_SECS` 秒限流一次），礦脈耗盡或時間到即結束，
//! 廣播前三名並發乙太大賞。
//!
//! 設計鐵律：
//! - **純邏輯、零 IO、零 LLM**：純函式 + 確定性計時，可完整單元測試。
//! - **純記憶體、零 migration**：不需持久化（重啟自然重計等待計時器）。
//! - **競速不 PvP**：搶的是同一條礦脈，不傷害對方，療癒向。

use std::collections::HashMap;
use uuid::Uuid;

/// 兩次事件之間的等待秒數（30 分鐘）。
pub const SPAWN_INTERVAL_SECS: f32 = 1800.0;

/// 伺服器啟動後首次生成的等待秒數（10 分鐘），避免上線立刻觸發。
pub const FIRST_SPAWN_WAIT_SECS: f32 = 600.0;

/// 每次事件的持續時間（5 分鐘）。
pub const DURATION_SECS: f32 = 300.0;

/// 礦脈總礦量——全員加起來只能搶這麼多。
pub const TOTAL_ORE: u32 = 60;

/// 每人每次挖礦的冷卻秒數，防止一人獨包礦脈。
pub const MINE_COOLDOWN_SECS: f32 = 3.0;

/// 黃金礦脈固定世界座標（城東：距主城約 1500px 的岩地方向）。
/// 距主城 (2336, 2272) 的距離約 1565px（落在岩地探勘令的 min_dist=1400 之外）。
pub const VEIN_WX: f32 = 3800.0;
pub const VEIN_WY: f32 = 2400.0;

/// 互動範圍（像素），與玩家在此半徑內才能搶挖。
pub const MINE_REACH: f32 = 120.0;

/// 前三名的乙太獎勵（冠軍 / 亞軍 / 季軍）。
pub const WINNER_REWARDS: [u32; 3] = [30, 15, 8];

/// 一名挖礦者的累積記錄。
#[derive(Debug, Clone)]
pub struct MinerRecord {
    /// 顯示名稱（廣播用）。
    pub name: String,
    /// 累積挖礦次數。
    pub count: u32,
}

/// 一場進行中的黃金礦脈事件。
#[derive(Debug)]
pub struct ActiveGoldRush {
    /// 剩餘礦量（到 0 立即結束）。
    pub remaining_ore: u32,
    /// 已過去的秒數。
    pub elapsed: f32,
    /// 各玩家的挖礦記錄。
    pub miners: HashMap<Uuid, MinerRecord>,
    /// 各玩家的個別冷卻剩餘秒數（0 = 可再挖）。
    pub mine_cooldowns: HashMap<Uuid, f32>,
}

impl ActiveGoldRush {
    fn new() -> Self {
        Self {
            remaining_ore: TOTAL_ORE,
            elapsed: 0.0,
            miners: HashMap::new(),
            mine_cooldowns: HashMap::new(),
        }
    }
}

/// 黃金礦脈爭奪戰的全域狀態（記憶體、零持久化）。
#[derive(Debug)]
pub struct GoldRushState {
    /// 距下次礦脈生成的倒數秒數；無活躍事件時遞減。
    pub spawn_countdown: f32,
    /// 當前活躍事件（None = 休眠中）。
    pub active: Option<ActiveGoldRush>,
}

impl Default for GoldRushState {
    fn default() -> Self {
        Self::new()
    }
}

/// 事件結束時，一名排名玩家的結果。
#[derive(Debug, Clone)]
pub struct RankedResult {
    /// 玩家顯示名稱。
    pub name: String,
    /// 挖礦總次數。
    pub count: u32,
    /// 獲得的乙太獎勵（前三有、四名後無）。
    pub reward: u32,
}

impl GoldRushState {
    pub fn new() -> Self {
        Self {
            spawn_countdown: FIRST_SPAWN_WAIT_SECS,
            active: None,
        }
    }

    /// 每 tick 推進 `dt` 秒。
    ///
    /// - 無活躍事件：遞減 `spawn_countdown`，歸零即生成新礦脈，回 `GoldRushTick::Spawned`。
    /// - 活躍事件：遞減冷卻、累積 elapsed，到期或礦盡即結束，回 `GoldRushTick::Finished`。
    /// - 其餘正常推進：回 `GoldRushTick::None`。
    pub fn tick(&mut self, dt: f32) -> GoldRushTick {
        if let Some(ref mut ev) = self.active {
            // 遞減所有玩家冷卻。
            for cd in ev.mine_cooldowns.values_mut() {
                *cd = (*cd - dt).max(0.0);
            }
            ev.elapsed += dt;
            let finished = ev.elapsed >= DURATION_SECS || ev.remaining_ore == 0;
            if finished {
                let results = self.finish_event();
                self.spawn_countdown = SPAWN_INTERVAL_SECS;
                return GoldRushTick::Finished(results);
            }
            GoldRushTick::None
        } else {
            self.spawn_countdown -= dt;
            if self.spawn_countdown <= 0.0 {
                self.active = Some(ActiveGoldRush::new());
                GoldRushTick::Spawned
            } else {
                GoldRushTick::None
            }
        }
    }

    /// 結束事件並回傳排名結果（清空活躍事件）。
    fn finish_event(&mut self) -> Vec<RankedResult> {
        let ev = self.active.take().expect("finish_event 只在 active 非空時呼叫");
        let mut sorted: Vec<&MinerRecord> = ev.miners.values().collect();
        sorted.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
        sorted
            .iter()
            .enumerate()
            .map(|(i, r)| RankedResult {
                name: r.name.clone(),
                count: r.count,
                reward: if i < WINNER_REWARDS.len() { WINNER_REWARDS[i] } else { 0 },
            })
            .collect()
    }

    /// 玩家嘗試搶挖。
    ///
    /// 成功時回 `Some(player_total_count)`（該玩家累計次數）。
    /// 失敗（無事件、礦盡、冷卻中、座標非有限）回 `None`。
    pub fn try_mine(&mut self, player_id: Uuid, player_name: &str, px: f32, py: f32) -> Option<u32> {
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        if !Self::is_near_vein(px, py) {
            return None;
        }
        let ev = self.active.as_mut()?;
        if ev.remaining_ore == 0 {
            return None;
        }
        let cd = ev.mine_cooldowns.entry(player_id).or_insert(0.0);
        if *cd > 0.0 {
            return None;
        }
        *cd = MINE_COOLDOWN_SECS;
        ev.remaining_ore -= 1;
        let rec = ev.miners.entry(player_id).or_insert_with(|| MinerRecord {
            name: player_name.to_string(),
            count: 0,
        });
        rec.count += 1;
        Some(rec.count)
    }

    /// 玩家是否在礦脈互動範圍內（純函式，供前後端共用）。
    pub fn is_near_vein(px: f32, py: f32) -> bool {
        if !px.is_finite() || !py.is_finite() {
            return false;
        }
        let dx = px - VEIN_WX;
        let dy = py - VEIN_WY;
        dx * dx + dy * dy <= MINE_REACH * MINE_REACH
    }

    /// 目前剩餘礦量（None = 無活躍事件）。
    pub fn remaining_ore(&self) -> Option<u32> {
        self.active.as_ref().map(|e| e.remaining_ore)
    }

    /// 目前剩餘時間秒數（None = 無活躍事件）。
    pub fn remaining_secs(&self) -> Option<f32> {
        self.active
            .as_ref()
            .map(|e| (DURATION_SECS - e.elapsed).max(0.0))
    }

    /// 是否有活躍事件。
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// 目前前三名（供快照廣播）。
    pub fn top3(&self) -> Vec<(String, u32)> {
        let Some(ev) = &self.active else {
            return vec![];
        };
        let mut sorted: Vec<&MinerRecord> = ev.miners.values().collect();
        sorted.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
        sorted
            .iter()
            .take(3)
            .map(|r| (r.name.clone(), r.count))
            .collect()
    }
}

/// `GoldRushState::tick` 的回傳分類。
#[derive(Debug)]
pub enum GoldRushTick {
    /// 本 tick 無特殊事件。
    None,
    /// 本 tick 剛生成了新礦脈（應廣播 GoldRushStart）。
    Spawned,
    /// 本 tick 事件剛結束（應廣播 GoldRushEnd 並發獎）。
    Finished(Vec<RankedResult>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u64) -> Uuid {
        Uuid::from_u128(n as u128)
    }

    #[test]
    fn initial_state_has_no_active_event() {
        let s = GoldRushState::new();
        assert!(s.active.is_none());
        assert!(!s.is_active());
        assert_eq!(s.remaining_ore(), None);
        assert_eq!(s.remaining_secs(), None);
    }

    #[test]
    fn spawns_after_first_wait() {
        let mut s = GoldRushState::new();
        // 還沒到等待時間
        let t = s.tick(FIRST_SPAWN_WAIT_SECS - 1.0);
        assert!(matches!(t, GoldRushTick::None));
        assert!(!s.is_active());
        // 再 tick 2 秒跨過門檻
        let t = s.tick(2.0);
        assert!(matches!(t, GoldRushTick::Spawned));
        assert!(s.is_active());
        assert_eq!(s.remaining_ore(), Some(TOTAL_ORE));
    }

    #[test]
    fn no_mine_when_no_event() {
        let mut s = GoldRushState::new();
        let result = s.try_mine(pid(1), "玩家", VEIN_WX, VEIN_WY);
        assert_eq!(result, None);
    }

    #[test]
    fn mine_succeeds_when_near_vein() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        assert!(s.is_active());
        let result = s.try_mine(pid(1), "甲", VEIN_WX, VEIN_WY);
        assert_eq!(result, Some(1));
        assert_eq!(s.remaining_ore(), Some(TOTAL_ORE - 1));
    }

    #[test]
    fn mine_rejected_when_far_from_vein() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        // 距礦脈超出範圍
        let result = s.try_mine(pid(1), "甲", VEIN_WX + MINE_REACH + 10.0, VEIN_WY);
        assert_eq!(result, None);
        assert_eq!(s.remaining_ore(), Some(TOTAL_ORE));
    }

    #[test]
    fn mine_cooldown_blocks_rapid_mining() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        let first = s.try_mine(pid(1), "甲", VEIN_WX, VEIN_WY);
        assert_eq!(first, Some(1));
        // 立刻再挖：冷卻未到
        let second = s.try_mine(pid(1), "甲", VEIN_WX, VEIN_WY);
        assert_eq!(second, None);
    }

    #[test]
    fn mine_allowed_after_cooldown_expires() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        s.try_mine(pid(1), "甲", VEIN_WX, VEIN_WY);
        // 冷卻走完
        s.tick(MINE_COOLDOWN_SECS + 0.1);
        let result = s.try_mine(pid(1), "甲", VEIN_WX, VEIN_WY);
        assert_eq!(result, Some(2));
    }

    #[test]
    fn different_players_can_mine_independently() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        let a = s.try_mine(pid(1), "甲", VEIN_WX, VEIN_WY);
        let b = s.try_mine(pid(2), "乙", VEIN_WX, VEIN_WY);
        assert_eq!(a, Some(1));
        assert_eq!(b, Some(1));
        assert_eq!(s.remaining_ore(), Some(TOTAL_ORE - 2));
    }

    #[test]
    fn event_ends_on_time_expiry() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        // 推進到快結束
        let t = s.tick(DURATION_SECS + 1.0);
        assert!(matches!(t, GoldRushTick::Finished(_)));
        assert!(!s.is_active());
    }

    #[test]
    fn event_ends_when_ore_depleted() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        assert!(s.is_active());
        // 把礦脈全挖光（每 3.01 秒一次，共 TOTAL_ORE 次）。
        for _ in 0..TOTAL_ORE {
            s.tick(MINE_COOLDOWN_SECS + 0.01);
            s.try_mine(pid(1), "甲", VEIN_WX, VEIN_WY);
        }
        // 再 tick 一下讓結束邏輯跑完（礦量歸零後下一 tick 回 Finished）。
        let t = s.tick(0.01);
        assert!(matches!(t, GoldRushTick::Finished(_)));
        assert!(!s.is_active());
    }

    #[test]
    fn top3_returns_sorted_by_count() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        // 甲挖 2 次、乙 1 次、丙 1 次
        s.try_mine(pid(1), "甲", VEIN_WX, VEIN_WY);
        s.try_mine(pid(2), "乙", VEIN_WX, VEIN_WY);
        s.try_mine(pid(3), "丙", VEIN_WX, VEIN_WY);
        s.tick(MINE_COOLDOWN_SECS + 0.01);
        s.try_mine(pid(1), "甲", VEIN_WX, VEIN_WY);
        let top = s.top3();
        assert!(!top.is_empty());
        assert_eq!(top[0].0, "甲");
        assert_eq!(top[0].1, 2);
    }

    #[test]
    fn is_near_vein_boundary() {
        // 恰在邊界內
        assert!(GoldRushState::is_near_vein(VEIN_WX + MINE_REACH - 1.0, VEIN_WY));
        // 恰在邊界外
        assert!(!GoldRushState::is_near_vein(VEIN_WX + MINE_REACH + 1.0, VEIN_WY));
    }

    #[test]
    fn bad_coordinates_rejected() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        assert_eq!(s.try_mine(pid(1), "甲", f32::NAN, VEIN_WY), None);
        assert_eq!(s.try_mine(pid(1), "甲", VEIN_WX, f32::INFINITY), None);
    }

    #[test]
    fn finish_gives_rewards_to_top3() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        s.try_mine(pid(1), "冠", VEIN_WX, VEIN_WY);
        s.tick(MINE_COOLDOWN_SECS + 0.01);
        s.try_mine(pid(1), "冠", VEIN_WX, VEIN_WY);
        s.try_mine(pid(2), "亞", VEIN_WX, VEIN_WY);
        // 時間到結束
        let t = s.tick(DURATION_SECS + 1.0);
        let GoldRushTick::Finished(results) = t else { panic!("應回 Finished") };
        assert!(!results.is_empty());
        // 冠軍有獎勵
        let champ = results.iter().find(|r| r.name == "冠").unwrap();
        assert_eq!(champ.reward, WINNER_REWARDS[0]);
    }

    #[test]
    fn spawn_countdown_resets_after_event_ends() {
        let mut s = GoldRushState::new();
        s.tick(FIRST_SPAWN_WAIT_SECS + 1.0);
        // 事件結束
        s.tick(DURATION_SECS + 1.0);
        assert!(!s.is_active());
        // spawn_countdown 應重置為 SPAWN_INTERVAL_SECS
        assert!((s.spawn_countdown - SPAWN_INTERVAL_SECS).abs() < 1.0);
    }
}
