// ROADMAP 100: 商人有限金庫——收購從金庫付，終結無限印鈔（經濟地基）
// 每個收購 NPC 有一個乙太金庫，ShopSell 從金庫扣；金庫不足→部分收或婉拒；
// 商隊收入每 RESTOCK_INTERVAL_SECS 秒緩慢回補，不超過各自上限。

use std::collections::HashMap;

/// 商人金庫鍵（與 ws.rs merchant_name 相同字串）。
pub const MERCHANT_HOME:    &str = "故鄉";
pub const MERCHANT_VERDANT: &str = "翠幽星";
pub const MERCHANT_CRIMSON: &str = "赤焰星";
pub const MERCHANT_VOID:    &str = "虛空星";
pub const MERCHANT_AETHER:  &str = "霧醚星";
pub const MERCHANT_ORIGIN:  &str = "星源星";

/// 各商人初始金庫（乙太）。故鄉業務最繁，初始最豐。
const INITIAL: &[(&str, u32)] = &[
    (MERCHANT_HOME,    600),
    (MERCHANT_VERDANT, 250),
    (MERCHANT_CRIMSON, 250),
    (MERCHANT_VOID,    250),
    (MERCHANT_AETHER,  250),
    (MERCHANT_ORIGIN,  250),
];

/// 各商人金庫上限（乙太）。
const MAX_TREASURY: &[(&str, u32)] = &[
    (MERCHANT_HOME,    1200),
    (MERCHANT_VERDANT,  500),
    (MERCHANT_CRIMSON,  500),
    (MERCHANT_VOID,     500),
    (MERCHANT_AETHER,   500),
    (MERCHANT_ORIGIN,   500),
];

/// 商隊每次回補金額（乙太）。
const RESTOCK_PER_TICK: u32 = 25;

/// 商隊回補間隔（秒）。
pub const RESTOCK_INTERVAL_SECS: u64 = 120;

/// ShopSell 收購結果。
pub struct TreasuryPayResult {
    /// 商人實際收購數量（可能少於請求數量）。
    pub actual_qty: u32,
    /// 商人實際支付乙太（不含引擎加成）。
    pub actual_paid: u32,
    /// 若金庫不足需通知玩家的文字；None = 正常無需提示。
    pub notice: Option<&'static str>,
}

/// 商人收購金庫狀態（ROADMAP 100）。
pub struct NpcTreasuryState {
    treasury: HashMap<&'static str, u32>,
}

impl NpcTreasuryState {
    pub fn new() -> Self {
        let mut treasury = HashMap::new();
        for (key, amount) in INITIAL {
            treasury.insert(*key, *amount);
        }
        Self { treasury }
    }

    /// 查詢金庫餘額。
    pub fn balance(&self, npc_key: &str) -> u32 {
        self.treasury.get(npc_key).copied().unwrap_or(0)
    }

    /// 嘗試從金庫支付：qty 個物品、單價 price_per（動態收購價）。
    /// 若金庫不足，則部分收購；若完全為零則婉拒（actual_qty = 0）。
    /// 注意：僅扣減基礎收購費（引擎加成如職業/急收令由引擎承擔，不從金庫扣）。
    pub fn try_pay(&mut self, npc_key: &'static str, qty: u32, price_per: u32) -> TreasuryPayResult {
        if price_per == 0 {
            // 免費收購（不應出現），當正常全量成交。
            return TreasuryPayResult { actual_qty: qty, actual_paid: 0, notice: None };
        }
        let balance = self.treasury.get(npc_key).copied().unwrap_or(0);
        if balance == 0 {
            return TreasuryPayResult {
                actual_qty: 0,
                actual_paid: 0,
                notice: Some("今天現金已見底，明天商隊回來後再來吧！"),
            };
        }
        let max_affordable = balance / price_per;
        let actual_qty = qty.min(max_affordable);
        let actual_paid = actual_qty * price_per;
        let e = self.treasury.entry(npc_key).or_insert(0);
        *e = e.saturating_sub(actual_paid);
        let notice = if actual_qty < qty {
            Some("現金快見底了，只收了部分！改天再來吧。")
        } else {
            None
        };
        TreasuryPayResult { actual_qty, actual_paid, notice }
    }

    /// 從金庫扣減指定金額（saturating，不能變負）。確認交易成功後呼叫。
    pub fn deduct(&mut self, npc_key: &str, amount: u32) {
        if let Some(e) = self.treasury.get_mut(npc_key) {
            *e = e.saturating_sub(amount);
        }
    }

    /// 查金庫能承擔的最大收購數量（qty 個、單價 price_per）。
    /// 回傳 (actual_qty, needs_notice)；needs_notice=true 表示金庫不足需通知玩家。
    pub fn afforded_qty(&self, npc_key: &str, qty: u32, price_per: u32) -> (u32, bool) {
        if price_per == 0 { return (qty, false); }
        let balance = self.treasury.get(npc_key).copied().unwrap_or(0);
        if balance == 0 {
            return (0, true);
        }
        let max = balance / price_per;
        let actual = qty.min(max);
        (actual, actual < qty)
    }

    /// 商隊回補：所有商人金庫補充 RESTOCK_PER_TICK 乙太（至各自上限）。
    pub fn tick_restock(&mut self) {
        for (key, max) in MAX_TREASURY {
            let e = self.treasury.entry(key).or_insert(0);
            *e = (*e + RESTOCK_PER_TICK).min(*max);
        }
    }
}

// ─── 純邏輯單元測試 ────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> NpcTreasuryState { NpcTreasuryState::new() }

    #[test]
    fn initial_balances_positive() {
        let t = make();
        assert!(t.balance(MERCHANT_HOME) > 0, "故鄉商人應有初始金庫");
        assert!(t.balance(MERCHANT_VERDANT) > 0, "翠幽星商人應有初始金庫");
        assert!(t.balance(MERCHANT_ORIGIN) > 0, "星源星商人應有初始金庫");
    }

    #[test]
    fn full_pay_deducts_treasury() {
        let mut t = make();
        let before = t.balance(MERCHANT_HOME);
        let r = t.try_pay(MERCHANT_HOME, 10, 5);
        assert_eq!(r.actual_qty, 10);
        assert_eq!(r.actual_paid, 50);
        assert!(r.notice.is_none());
        assert_eq!(t.balance(MERCHANT_HOME), before - 50);
    }

    #[test]
    fn partial_pay_when_treasury_low() {
        let mut t = make();
        // 手動清空金庫到剛好只夠 3 個的量（單價 10）
        *t.treasury.entry(MERCHANT_HOME).or_insert(0) = 30;
        let r = t.try_pay(MERCHANT_HOME, 10, 10);
        assert_eq!(r.actual_qty, 3, "只夠買 3 個");
        assert_eq!(r.actual_paid, 30);
        assert!(r.notice.is_some(), "應提示現金不足");
        assert_eq!(t.balance(MERCHANT_HOME), 0);
    }

    #[test]
    fn zero_treasury_declines_entirely() {
        let mut t = make();
        *t.treasury.entry(MERCHANT_HOME).or_insert(0) = 0;
        let r = t.try_pay(MERCHANT_HOME, 5, 3);
        assert_eq!(r.actual_qty, 0, "金庫為零應完全婉拒");
        assert_eq!(r.actual_paid, 0);
        assert!(r.notice.is_some());
        // 金庫仍為 0，不能變負
        assert_eq!(t.balance(MERCHANT_HOME), 0);
    }

    #[test]
    fn restock_increases_balance() {
        let mut t = make();
        *t.treasury.entry(MERCHANT_HOME).or_insert(0) = 0;
        t.tick_restock();
        assert_eq!(t.balance(MERCHANT_HOME), RESTOCK_PER_TICK,
            "回補後應增加 RESTOCK_PER_TICK");
    }

    #[test]
    fn restock_capped_at_max() {
        let mut t = make();
        // 設定到接近上限
        let max = MAX_TREASURY.iter().find(|(k, _)| *k == MERCHANT_HOME).unwrap().1;
        *t.treasury.entry(MERCHANT_HOME).or_insert(0) = max - 1;
        t.tick_restock();
        assert_eq!(t.balance(MERCHANT_HOME), max, "回補不應超過上限");
    }

    #[test]
    fn star_merchants_have_lower_cap_than_home() {
        let home_max = MAX_TREASURY.iter().find(|(k, _)| *k == MERCHANT_HOME).unwrap().1;
        for (key, max) in MAX_TREASURY {
            if *key != MERCHANT_HOME {
                assert!(*max < home_max, "{key} 上限應低於故鄉商人");
            }
        }
    }

    #[test]
    fn price_zero_treated_as_full_qty() {
        let mut t = make();
        let r = t.try_pay(MERCHANT_HOME, 5, 0);
        assert_eq!(r.actual_qty, 5, "單價為零時視作全量成交");
    }

    #[test]
    fn unknown_npc_key_treated_as_empty() {
        let mut t = make();
        // 若用不存在的鍵，應視作金庫為零，婉拒收購
        // 注意：實際 ws.rs 只會用已知鍵，這是防衛性測試
        // 這個測試驗 balance() 回 0
        assert_eq!(t.balance("不存在的商人"), 0);
    }
}
