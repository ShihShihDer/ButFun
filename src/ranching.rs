//! 牧場系統（ROADMAP 48）——農田地塊養雞產蛋。
//!
//! 玩家在「農田（Farm）」類型的地塊上花乙太購入雞，雞每 60 秒自動下蛋（最多堆積
//! 10 顆）。玩家靠近自己的地塊可「收雞蛋」，每次收雞蛋給 8 點農夫熟練度 XP。
//! 雞蛋可賣給 NPC（2 乙太/顆）或合成「煎蛋」（2 顆 → 1 煎蛋，回血 10）。
//!
//! 這是 Phase 2「深度與自動化」的第一步：在自己買的農田地塊上看到雞在跑，
//! 並多了一條農夫熟練度提升的活動路線。
//!
//! 設計取捨：
//!   - **記憶體模式**：雞與蛋不寫 DB（同 pet.rs 做法），重啟後玩家重新購入。
//!     好處：零 migration 風險，快速上線；代價：每次伺服器重啟要重買雞。
//!   - **每塊農田最多 3 隻雞**：避免堆雞刷蛋；後續版本可鬆綁或加飼料機制。
//!   - **蛋最多堆 10 顆**：防無限堆積，逼玩家定期來收。

use std::collections::HashMap;

/// 購入一隻雞的乙太費用。
pub const BUY_CHICKEN_COST: u32 = 15;

/// 每隻雞每批下蛋的間隔（秒）。多隻雞共用同一計時器、同時下蛋。
pub const EGG_INTERVAL_SECS: f32 = 60.0;

/// 農夫熟練度 XP（每次收雞蛋操作）。
pub const COLLECT_FARMER_XP: u32 = 8;

/// 單塊農田地塊最多可養的雞隻數。
pub const MAX_CHICKENS: u32 = 3;

/// 農田地塊上雞蛋的最大堆積數。
pub const MAX_EGGS: u32 = 10;

/// 單一農田地塊的牧場狀態（記憶體模式）。
#[derive(Debug, Clone)]
pub struct RanchState {
    /// 現有雞隻數（0~MAX_CHICKENS）。
    pub chicken_count: u32,
    /// 目前堆積的蛋數（0~MAX_EGGS）。
    pub egg_count: u32,
    /// 距下一批下蛋還剩幾秒（每 tick 由 game.rs 遞減）。
    pub egg_timer: f32,
    /// 本地塊已完成的下蛋批次（用於偽隨機種子）。
    pub egg_batches: u64,
}

impl RanchState {
    fn new() -> Self {
        Self {
            chicken_count: 0,
            egg_count: 0,
            egg_timer: EGG_INTERVAL_SECS,
            egg_batches: 0,
        }
    }
}

/// 全伺服器所有農田地塊的牧場狀態（記憶體模式）。
#[derive(Default)]
pub struct RanchRegistry {
    plots: HashMap<u32, RanchState>,
}

impl RanchRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 嘗試在 `plot_id` 地塊購入一隻雞。
    /// - 失敗條件：地塊不存在（呼叫端保證農田類型）、已達 MAX_CHICKENS。
    /// - 成功：雞數 +1，回 `true`。
    pub fn buy_chicken(&mut self, plot_id: u32) -> bool {
        let state = self.plots.entry(plot_id).or_insert_with(RanchState::new);
        if state.chicken_count >= MAX_CHICKENS {
            return false;
        }
        state.chicken_count += 1;
        true
    }

    /// 收取 `plot_id` 地塊的雞蛋。
    /// 回傳 `(eggs_collected, farmer_xp)`；無蛋或地塊不存在時回 `(0, 0)`。
    pub fn collect_eggs(&mut self, plot_id: u32) -> (u32, u32) {
        let state = match self.plots.get_mut(&plot_id) {
            Some(s) => s,
            None => return (0, 0),
        };
        let eggs = state.egg_count;
        if eggs == 0 {
            return (0, 0);
        }
        state.egg_count = 0;
        (eggs, COLLECT_FARMER_XP)
    }

    /// 取得某地塊的雞隻數與蛋數（供快照廣播用）。地塊不存在回 `(0, 0)`。
    pub fn state_of(&self, plot_id: u32) -> (u32, u32) {
        self.plots.get(&plot_id).map(|s| (s.chicken_count, s.egg_count)).unwrap_or((0, 0))
    }

    /// 每遊戲 tick 更新所有有雞地塊的蛋計時器。
    pub fn tick(&mut self, dt: f32) {
        for state in self.plots.values_mut() {
            if state.chicken_count == 0 {
                continue;
            }
            state.egg_timer -= dt;
            if state.egg_timer <= 0.0 {
                state.egg_timer = EGG_INTERVAL_SECS;
                // 每批下蛋數：1~2 顆，由確定性種子決定。
                let batch = roll_egg_batch(state.egg_batches);
                state.egg_batches = state.egg_batches.wrapping_add(1);
                // 雞多下得多（雞隻數倍乘），但不超過 MAX_EGGS 上限。
                let produced = (batch * state.chicken_count).min(MAX_EGGS - state.egg_count.min(MAX_EGGS));
                state.egg_count = (state.egg_count + produced).min(MAX_EGGS);
            }
        }
    }

    /// 匯出全部有活躍狀態（有雞或有蛋）的地塊快照（供 Snapshot 廣播）。
    pub fn all_active_views(&self) -> Vec<RanchPlotView> {
        self.plots.iter()
            .filter(|(_, s)| s.chicken_count > 0 || s.egg_count > 0)
            .map(|(&plot_id, s)| RanchPlotView {
                plot_id,
                chicken_count: s.chicken_count,
                egg_count: s.egg_count,
            })
            .collect()
    }
}

/// 確定性種子決定本批下 1 或 2 顆蛋（各 50%）。
pub fn roll_egg_batch(seed: u64) -> u32 {
    if seed % 2 == 0 { 2 } else { 1 }
}

/// 快照裡一塊農田地塊的牧場可見狀態（送給前端）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RanchPlotView {
    pub plot_id: u32,
    pub chicken_count: u32,
    pub egg_count: u32,
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 購雞基本流程：首次購雞成功，雞數增加。
    #[test]
    fn buy_chicken_success() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(0));
        assert_eq!(reg.state_of(0), (1, 0));
    }

    /// 同一地塊可購至 MAX_CHICKENS，超出則拒絕。
    #[test]
    fn buy_chicken_respects_max() {
        let mut reg = RanchRegistry::new();
        for _ in 0..MAX_CHICKENS {
            assert!(reg.buy_chicken(5));
        }
        assert!(!reg.buy_chicken(5), "超過上限應被拒");
        assert_eq!(reg.state_of(5).0, MAX_CHICKENS);
    }

    /// 收蛋：有蛋才能收，收完歸零，回傳蛋數與農夫 XP。
    #[test]
    fn collect_eggs_works() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(1));
        // 直接注入幾顆蛋模擬 tick 完畢。
        reg.plots.get_mut(&1).unwrap().egg_count = 3;
        let (eggs, xp) = reg.collect_eggs(1);
        assert_eq!(eggs, 3);
        assert_eq!(xp, COLLECT_FARMER_XP);
        assert_eq!(reg.state_of(1).1, 0, "收完後應歸零");
    }

    /// 無蛋時收蛋回傳 (0, 0)。
    #[test]
    fn collect_eggs_none_when_empty() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(2));
        let (eggs, xp) = reg.collect_eggs(2);
        assert_eq!((eggs, xp), (0, 0));
    }

    /// 地塊不存在時收蛋回傳 (0, 0)。
    #[test]
    fn collect_eggs_nonexistent_plot() {
        let mut reg = RanchRegistry::new();
        assert_eq!(reg.collect_eggs(999), (0, 0));
    }

    /// tick 使計時器倒數，到期後生蛋。
    #[test]
    fn tick_produces_eggs_after_interval() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(3));
        // 大量 tick 讓計時器跑完。
        reg.tick(EGG_INTERVAL_SECS + 0.1);
        let (_, eggs) = reg.state_of(3);
        assert!(eggs > 0, "計時器到期後應有蛋；實際蛋數 = {eggs}");
    }

    /// 沒有雞的地塊不應累積蛋。
    #[test]
    fn tick_no_eggs_without_chickens() {
        let mut reg = RanchRegistry::new();
        // 手動建 state 但雞數 0（不透過 buy_chicken）。
        reg.plots.insert(4, RanchState::new());
        reg.tick(EGG_INTERVAL_SECS * 5.0);
        assert_eq!(reg.state_of(4).1, 0, "無雞不應產蛋");
    }

    /// 蛋不超過 MAX_EGGS 上限。
    #[test]
    fn egg_count_capped_at_max() {
        let mut reg = RanchRegistry::new();
        for _ in 0..MAX_CHICKENS {
            reg.buy_chicken(6);
        }
        // 跑夠多批，蛋不斷累積。
        for _ in 0..20 {
            reg.tick(EGG_INTERVAL_SECS + 0.1);
        }
        let (_, eggs) = reg.state_of(6);
        assert!(eggs <= MAX_EGGS, "蛋不得超過 MAX_EGGS={MAX_EGGS}；實際 = {eggs}");
    }

    /// roll_egg_batch：覆蓋 1 和 2 兩種結果。
    #[test]
    fn roll_egg_batch_covers_both_values() {
        let has_one = (0..10).any(|i| roll_egg_batch(i) == 1);
        let has_two = (0..10).any(|i| roll_egg_batch(i) == 2);
        assert!(has_one, "應有返回 1 的種子");
        assert!(has_two, "應有返回 2 的種子");
    }

    /// BUY_CHICKEN_COST 在合理範圍（1~100 乙太）。
    #[test]
    fn buy_chicken_cost_is_reasonable() {
        assert!(BUY_CHICKEN_COST >= 1 && BUY_CHICKEN_COST <= 100);
    }
}
