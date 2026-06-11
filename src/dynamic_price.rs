//! NPC 浮動收購價市場（ROADMAP 40：經濟再平衡）。
//!
//! 問題：NPC 固定價無限量收購 ＝ 水龍頭全開、沒有回收閥，任何大額定價都會被快速洗掉。
//! 解法：**浮動收購價**——每物品基準價；賣越多收購價越低（每賣 20 個降 10%，地板 30%）；
//!       隨時間回升（每小時 +5%）；商店面板顯示當前價與 ↘ 趨勢指示。
//!
//! 設計原則：
//! - 純邏輯、無 IO（`now_secs` 由 caller 傳入），便於自動測試。
//! - 只控制**收購價**（NPC 買玩家的物品），販售價（NPC 賣物品）不動。
//! - 狀態存記憶體（重啟歸零；重啟後商人回基準價，行為合理）。
//! - 伺服器權威：前端只能讀商人廣播的當前價，不能自算。

use std::collections::BTreeMap;
use crate::inventory::ItemKind;

/// 基準收購倍率（1000 = 100%）。
pub const BASE_BPS: u32 = 1000;
/// 收購倍率地板（300 = 30%）——保留最低收益，不讓玩家血本無歸。
pub const FLOOR_BPS: u32 = 300;
/// 每賣出幾個觸發一次跌價（累積量）。
pub const DECAY_VOLUME: u32 = 20;
/// 每次跌價幅度（100 bps = 10%）。
pub const DECAY_STEP_BPS: u32 = 100;
/// 每小時回升幅度（50 bps = 5%）。
pub const RECOVERY_PER_HOUR_BPS: u32 = 50;

/// 讀取當前 Unix epoch 秒（用於 `now_secs` 參數）。
pub fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone)]
struct ItemPriceState {
    /// 當前收購倍率（1000 = 基準，300 = 地板）。
    price_bps: u32,
    /// 自上次跌價後的累計賣出量（累滿 DECAY_VOLUME 才跌一次）。
    vol_bucket: u32,
    /// 上次回升計算的 Unix epoch 秒。
    last_recovery_secs: u64,
}

impl ItemPriceState {
    fn new() -> Self {
        Self {
            price_bps: BASE_BPS,
            vol_bucket: 0,
            last_recovery_secs: 0,
        }
    }

    /// 記錄一筆賣出：先計算時間回升、再累積量並觸發跌價。
    fn record_sale(&mut self, qty: u32, now_secs: u64) {
        // 先把時間帶來的回升算進去
        let elapsed = now_secs.saturating_sub(self.last_recovery_secs);
        let hours = (elapsed / 3600) as u32;
        if hours > 0 {
            self.price_bps = self.price_bps.saturating_add(hours * RECOVERY_PER_HOUR_BPS).min(BASE_BPS);
            self.last_recovery_secs = self.last_recovery_secs.saturating_add(hours as u64 * 3600);
        }
        // 累積賣出量，觸發跌價
        self.vol_bucket = self.vol_bucket.saturating_add(qty);
        let steps = self.vol_bucket / DECAY_VOLUME;
        if steps > 0 {
            self.vol_bucket %= DECAY_VOLUME;
            let decay = steps.saturating_mul(DECAY_STEP_BPS);
            self.price_bps = self.price_bps.saturating_sub(decay).max(FLOOR_BPS);
        }
    }
}

/// 全服 NPC 浮動收購價市場。
///
/// 讀取（`current_price`、`current_trend`）使用 `&self`，不修改狀態（純計算含時間回升）；
/// 寫入（`record_sale`）使用 `&mut self`，賣出後才更新狀態。
pub struct DynamicPriceMarket {
    states: BTreeMap<ItemKind, ItemPriceState>,
}

impl DynamicPriceMarket {
    pub fn new() -> Self {
        Self { states: BTreeMap::new() }
    }

    /// 查詢某物品的當前有效收購價（含時間回升計算，不修改狀態）。
    /// 若該物品從未賣過，直接回傳 `base_price`（基準價）。
    pub fn current_price(&self, item: ItemKind, base_price: u32, now_secs: u64) -> u32 {
        match self.states.get(&item) {
            None => base_price,
            Some(s) => {
                // 計算時間帶來的回升（不修改 self）
                let elapsed = now_secs.saturating_sub(s.last_recovery_secs);
                let hours = (elapsed / 3600) as u32;
                let bps = s.price_bps.saturating_add(hours * RECOVERY_PER_HOUR_BPS).min(BASE_BPS);
                // 有效收購價 = base_price × bps / BASE_BPS，至少 1 乙太/個
                let p = base_price as u64 * bps as u64 / BASE_BPS as u64;
                (p as u32).max(1)
            }
        }
    }

    /// 查詢某物品的趨勢字串（不修改狀態）。
    /// - `"stable"`: 收購價處於基準（沒有被壓低）
    /// - `"down"`: 收購價低於基準（被大量賣出壓低中）
    pub fn current_trend(&self, item: ItemKind, now_secs: u64) -> &'static str {
        match self.states.get(&item) {
            None => "stable",
            Some(s) => {
                let elapsed = now_secs.saturating_sub(s.last_recovery_secs);
                let hours = (elapsed / 3600) as u32;
                let bps = s.price_bps.saturating_add(hours * RECOVERY_PER_HOUR_BPS).min(BASE_BPS);
                if bps >= BASE_BPS { "stable" } else { "down" }
            }
        }
    }

    /// 記錄一筆賣出，更新浮動收購價（需要 `&mut self`）。
    pub fn record_sale(&mut self, item: ItemKind, qty: u32, now_secs: u64) {
        self.states
            .entry(item)
            .or_insert_with(ItemPriceState::new)
            .record_sale(qty, now_secs);
    }

    /// 取得批量計算所需的起始狀態：（時間修正後的 bps 倍率，現有桶進度）。
    fn bulk_start_state(&self, item: ItemKind, now_secs: u64) -> (u32, u32) {
        match self.states.get(&item) {
            None => (BASE_BPS, 0),
            Some(s) => {
                let elapsed = now_secs.saturating_sub(s.last_recovery_secs);
                let hours = (elapsed / 3600) as u32;
                let bps = s.price_bps.saturating_add(hours * RECOVERY_PER_HOUR_BPS).min(BASE_BPS);
                (bps, s.vol_bucket)
            }
        }
    }

    /// 計算單筆批量賣出的**總**收益（ROADMAP 102：單筆內隨量遞減）。
    ///
    /// 模擬：每賣 `DECAY_VOLUME` 個，後續物品的收購倍率降 `DECAY_STEP_BPS`（地板 `FLOOR_BPS`）。
    /// 回傳玩家應得的基礎乙太總額（不含職業/議價/急收令加成）；不修改市場狀態。
    pub fn calculate_bulk_total(&self, item: ItemKind, base_price: u32, qty: u32, now_secs: u64) -> u32 {
        if qty == 0 || base_price == 0 {
            return 0;
        }
        let (mut bps, mut bucket) = self.bulk_start_state(item, now_secs);
        let mut total = 0u32;
        let mut remaining = qty;
        while remaining > 0 {
            // 桶還剩多少空間才觸發下一次跌價
            let space = DECAY_VOLUME.saturating_sub(bucket);
            let chunk = remaining.min(space);
            // 此段的每單位有效收購價
            let price_per = ((base_price as u64 * bps as u64) / BASE_BPS as u64).max(1) as u32;
            total = total.saturating_add(price_per.saturating_mul(chunk));
            remaining -= chunk;
            bucket += chunk;
            if bucket >= DECAY_VOLUME {
                bucket = 0;
                bps = bps.saturating_sub(DECAY_STEP_BPS).max(FLOOR_BPS);
            }
        }
        total
    }

    /// 在金庫預算內，以漸降價找出最多可成交的數量與總成本（ROADMAP 102）。
    ///
    /// 回傳 `(actual_qty, total_cost, needs_notice)`：
    /// - `actual_qty`：實際可收購數量（≤ requested_qty）
    /// - `total_cost`：金庫應支付的乙太（不含引擎加成）
    /// - `needs_notice`：因金庫不足而部分成交時為 true
    ///
    /// 不修改市場狀態，caller 在成交後呼叫 `record_sale` 更新狀態。
    pub fn find_bulk_affordable(
        &self,
        item: ItemKind,
        base_price: u32,
        requested_qty: u32,
        treasury_balance: u32,
        now_secs: u64,
    ) -> (u32, u32, bool) {
        if requested_qty == 0 {
            return (0, 0, false);
        }
        if base_price == 0 || treasury_balance == 0 {
            return (0, 0, true);
        }
        // 先試算完整批量；若金庫夠則全量成交
        let full_cost = self.calculate_bulk_total(item, base_price, requested_qty, now_secs);
        if full_cost <= treasury_balance {
            return (requested_qty, full_cost, false);
        }
        // 金庫不足：逐段累積，找到預算耗盡為止
        let (mut bps, mut bucket) = self.bulk_start_state(item, now_secs);
        let mut total_qty = 0u32;
        let mut total_cost = 0u32;
        let mut remaining_budget = treasury_balance;
        let mut remaining_qty = requested_qty;
        while remaining_qty > 0 && remaining_budget > 0 {
            let space = DECAY_VOLUME.saturating_sub(bucket);
            let price_per = ((base_price as u64 * bps as u64) / BASE_BPS as u64).max(1) as u32;
            // 在當前單價下最多買幾個（受預算、桶空間、剩餘數量三重限制）
            let affordable = (remaining_budget / price_per).min(space).min(remaining_qty);
            if affordable == 0 {
                break;
            }
            let chunk_cost = affordable.saturating_mul(price_per);
            total_qty += affordable;
            total_cost = total_cost.saturating_add(chunk_cost);
            remaining_budget = remaining_budget.saturating_sub(chunk_cost);
            remaining_qty -= affordable;
            bucket += affordable;
            if bucket >= DECAY_VOLUME {
                bucket = 0;
                bps = bps.saturating_sub(DECAY_STEP_BPS).max(FLOOR_BPS);
            }
        }
        let needs_notice = total_qty < requested_qty;
        (total_qty, total_cost, needs_notice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::ItemKind;

    #[test]
    fn price_at_base_when_no_sales() {
        // 從未賣出時，收購價等於基準價。
        let market = DynamicPriceMarket::new();
        assert_eq!(market.current_price(ItemKind::Wood, 1, 0), 1);
        assert_eq!(market.current_price(ItemKind::CrystalShard, 3, 0), 3);
        assert_eq!(market.current_price(ItemKind::DeepSeaPearl, 5, 0), 5);
    }

    #[test]
    fn trend_stable_when_no_sales() {
        let market = DynamicPriceMarket::new();
        assert_eq!(market.current_trend(ItemKind::Wood, 0), "stable");
    }

    #[test]
    fn partial_bucket_does_not_trigger_decay() {
        // 賣 15 個（未滿 DECAY_VOLUME=20），收購價不變。
        let mut market = DynamicPriceMarket::new();
        market.record_sale(ItemKind::CrystalShard, 15, 0);
        assert_eq!(market.current_price(ItemKind::CrystalShard, 3, 0), 3);
        assert_eq!(market.current_trend(ItemKind::CrystalShard, 0), "stable");
    }

    #[test]
    fn price_decreases_after_decay_volume_sold() {
        // 賣 20 個（= DECAY_VOLUME），跌 10%（100 bps）。
        let mut market = DynamicPriceMarket::new();
        market.record_sale(ItemKind::CrystalShard, 20, 0);
        // 3 × 900/1000 = 2.7 → 2
        assert_eq!(market.current_price(ItemKind::CrystalShard, 3, 0), 2);
        assert_eq!(market.current_trend(ItemKind::CrystalShard, 0), "down");
    }

    #[test]
    fn price_decreases_multiple_steps() {
        // 賣 60 個 = 3 × DECAY_VOLUME，跌 3 × 10% = 30%。
        let mut market = DynamicPriceMarket::new();
        market.record_sale(ItemKind::CrystalShard, 60, 0);
        // 3 × 700/1000 = 2.1 → 2
        assert_eq!(market.current_price(ItemKind::CrystalShard, 3, 0), 2);
    }

    #[test]
    fn price_floors_at_30_percent() {
        // 大量賣出，收購價降到地板（30%），不能再低。
        let mut market = DynamicPriceMarket::new();
        market.record_sale(ItemKind::CrystalShard, 1000, 0);
        // 地板 30%：3 × 300/1000 = 0.9 → max(1) = 1
        let p = market.current_price(ItemKind::CrystalShard, 3, 0);
        assert_eq!(p, 1); // max(1) 確保至少 1 乙太/個
    }

    #[test]
    fn trend_down_when_depressed() {
        let mut market = DynamicPriceMarket::new();
        market.record_sale(ItemKind::CrystalShard, 20, 0);
        assert_eq!(market.current_trend(ItemKind::CrystalShard, 0), "down");
    }

    #[test]
    fn price_recovers_by_one_step_after_one_hour() {
        let mut market = DynamicPriceMarket::new();
        // 賣 20 個，跌至 900 bps。
        market.record_sale(ItemKind::CrystalShard, 20, 0);
        assert_eq!(market.current_price(ItemKind::CrystalShard, 10, 0), 9); // 10 × 900/1000 = 9

        // 1 小時後：+50 bps → 950 bps
        // 10 × 950/1000 = 9.5 → 9（整數截斷）
        assert_eq!(market.current_price(ItemKind::CrystalShard, 10, 3600), 9);
    }

    #[test]
    fn price_fully_recovers_after_two_hours() {
        let mut market = DynamicPriceMarket::new();
        // 賣 20 個，跌至 900 bps。
        market.record_sale(ItemKind::CrystalShard, 20, 0);
        // 2 小時後：+100 bps → 1000 bps（基準）
        assert_eq!(market.current_price(ItemKind::CrystalShard, 10, 7200), 10);
    }

    #[test]
    fn trend_recovers_to_stable_after_sufficient_time() {
        let mut market = DynamicPriceMarket::new();
        market.record_sale(ItemKind::CrystalShard, 20, 0);
        // 2 小時後，趨勢回穩。
        assert_eq!(market.current_trend(ItemKind::CrystalShard, 7200), "stable");
    }

    #[test]
    fn recovery_cannot_exceed_base() {
        // 長時間未賣出，收購倍率仍鎖在基準，不會超過 100%。
        let market = DynamicPriceMarket::new();
        // 100 小時後，未有任何賣出的物品仍應是基準價。
        assert_eq!(market.current_price(ItemKind::Wood, 1, 360_000), 1);
    }

    #[test]
    fn different_items_have_independent_states() {
        // 大量賣木材，不影響晶石碎片的收購價。
        let mut market = DynamicPriceMarket::new();
        market.record_sale(ItemKind::Wood, 1000, 0);
        assert_eq!(market.current_price(ItemKind::CrystalShard, 3, 0), 3); // 不受影響
        assert_eq!(market.current_trend(ItemKind::CrystalShard, 0), "stable");
    }

    #[test]
    fn sell_across_multiple_calls_accumulates_correctly() {
        // 分兩次賣出各 10 個（共 20），等效於一次賣出 20 個。
        let mut market = DynamicPriceMarket::new();
        market.record_sale(ItemKind::CrystalShard, 10, 0);
        market.record_sale(ItemKind::CrystalShard, 10, 0);
        assert_eq!(market.current_price(ItemKind::CrystalShard, 3, 0), 2);
    }

    // ── ROADMAP 102：批量漸降價 ────────────────────────────────────────────

    #[test]
    fn bulk_total_small_qty_no_decay() {
        // qty < DECAY_VOLUME → 不觸發跌價，總收益 = base_price × qty。
        let market = DynamicPriceMarket::new();
        let total = market.calculate_bulk_total(ItemKind::Wood, 2, 10, 0);
        assert_eq!(total, 20, "10 個 × 2 乙太 = 20，不跌價");
    }

    #[test]
    fn bulk_total_triggers_one_step_at_boundary() {
        // 賣 40 個（= 2 × DECAY_VOLUME）：前 20 全價，後 20 跌 10%。
        // base_price=10：前 20 × 10 = 200，後 20 × 9 = 180，合計 380。
        let market = DynamicPriceMarket::new();
        let total = market.calculate_bulk_total(ItemKind::Wood, 10, 40, 0);
        assert_eq!(total, 200 + 180);
    }

    #[test]
    fn bulk_total_floors_at_minimum() {
        // 大量賣出，倍率觸底 FLOOR_BPS；此後每個至少 1 乙太。
        let market = DynamicPriceMarket::new();
        // 賣 200 個 = 10 × DECAY_VOLUME；10 × 10% = 100% 跌幅，但地板 30%。
        // base_price=1，FLOOR 後 price_per=max(1)=1；所有段落至少 1。
        let total = market.calculate_bulk_total(ItemKind::Wood, 1, 200, 0);
        assert!(total > 0 && total <= 200, "批量總收益應介於 1 和 200 之間");
    }

    #[test]
    fn bulk_total_with_existing_bucket() {
        // 桶已有 15 個（前次賣出餘量），再賣 25 個：
        // 先賣 5 個觸發跌價，再賣 20 個（下一個完整段）再觸發跌價，最後 0 個剩。
        // base_price=10，初始 bps=1000，DECAY_STEP_BPS=100：
        //   段 1（前 5 個）：price=10，cost=50；觸發跌價 bps=900
        //   段 2（後 20 個）：price=9，cost=180；觸發跌價 bps=800
        //   合計 = 230
        let mut market = DynamicPriceMarket::new();
        market.record_sale(ItemKind::Stone, 15, 0); // 桶 =15
        let total = market.calculate_bulk_total(ItemKind::Stone, 10, 25, 0);
        assert_eq!(total, 50 + 180);
    }

    #[test]
    fn find_bulk_affordable_full_within_budget() {
        // 金庫充裕，全量成交，needs_notice = false。
        let market = DynamicPriceMarket::new();
        let (qty, cost, notice) = market.find_bulk_affordable(ItemKind::Wood, 2, 10, 999, 0);
        assert_eq!(qty, 10);
        assert_eq!(cost, 20);
        assert!(!notice);
    }

    #[test]
    fn find_bulk_affordable_treasury_limits_qty() {
        // 金庫只剩 15 乙太、單價 2、DECAY_VOLUME=20：
        // 可買 7 個（7×2=14）≤ 15；第 8 個需 2 乙太但剩 1，買不起。
        let market = DynamicPriceMarket::new();
        let (qty, cost, notice) = market.find_bulk_affordable(ItemKind::Wood, 2, 100, 15, 0);
        assert_eq!(qty, 7, "7 × 2 = 14 ≤ 15，第 8 個 2 > 1");
        assert_eq!(cost, 14);
        assert!(notice, "金庫不足應提示玩家");
    }

    #[test]
    fn find_bulk_affordable_empty_treasury() {
        let market = DynamicPriceMarket::new();
        let (qty, cost, notice) = market.find_bulk_affordable(ItemKind::Wood, 2, 10, 0, 0);
        assert_eq!(qty, 0);
        assert_eq!(cost, 0);
        assert!(notice);
    }

    #[test]
    fn bulk_sale_cheaper_than_single_price_times_qty() {
        // 批量賣出的總收益 ≤ 單一定價 × 數量（因為單筆內在遞減）。
        let market = DynamicPriceMarket::new();
        let single_price = market.current_price(ItemKind::Wood, 2, 0);
        let qty = 100u32;
        let bulk = market.calculate_bulk_total(ItemKind::Wood, 2, qty, 0);
        let uniform = single_price.saturating_mul(qty);
        assert!(bulk <= uniform,
            "批量總收益 {bulk} 應 ≤ 均一價 {uniform}（批量漸降）");
    }
}
