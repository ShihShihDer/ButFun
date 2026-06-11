// ROADMAP 104: 有限庫存 + 補貨——賣商會賣完、隨時間補貨
// 每個販售 NPC 對每樣商品有庫存上限；賣完缺貨；隨時間補貨。
// 「買多了賣價漲」：庫存低時成本上升（稀缺溢價），反映自然供需。

use std::collections::BTreeMap;
use crate::inventory::ItemKind;
use crate::npc_treasury::{
    MERCHANT_HOME, MERCHANT_VERDANT, MERCHANT_CRIMSON,
    MERCHANT_VOID, MERCHANT_AETHER, MERCHANT_ORIGIN,
};

/// 補貨間隔（秒）——每 3 分鐘補一次貨。
pub const STOCK_RESTOCK_INTERVAL_SECS: u64 = 180;

/// 庫存稀缺溢價閾值（以分比計算，庫存比例 = current / max）：
/// - 庫存 ≥ 60% → 正常基準價（×1.0）
/// - 30% ≤ 庫存 < 60% → 輕度溢價（×1.5）
/// - 庫存 < 30% → 缺貨溢價（×2.0）
const SCARCITY_MID_BPS: u32 = 600;  // 60%
const SCARCITY_LOW_BPS: u32 = 300;  // 30%
const PRICE_MULTIPLIER_MID: u32 = 150;   // ×1.5（基準 100 = ×1.0）
const PRICE_MULTIPLIER_HIGH: u32 = 200;  // ×2.0

/// 單一商品的庫存設定。
#[derive(Debug, Clone)]
pub struct StockConfig {
    /// 初始庫存與最大庫存上限。
    pub max: u32,
    /// 每次補貨回補量。
    pub restock_per_tick: u32,
}

/// 取得故鄉商人各商品的庫存設定。
fn home_stock_configs() -> Vec<(ItemKind, StockConfig)> {
    vec![
        (ItemKind::Pickaxe, StockConfig { max: 8,  restock_per_tick: 1 }),
        (ItemKind::Weapon,  StockConfig { max: 8,  restock_per_tick: 1 }),
        (ItemKind::Wood,    StockConfig { max: 50, restock_per_tick: 10 }),
        (ItemKind::Stone,   StockConfig { max: 50, restock_per_tick: 10 }),
        (ItemKind::Dirt,    StockConfig { max: 50, restock_per_tick: 10 }),
    ]
}

/// 單一商品的庫存狀態。
#[derive(Debug, Clone)]
struct ItemStock {
    current: u32,
    max: u32,
    restock_per_tick: u32,
}

impl ItemStock {
    fn new(cfg: &StockConfig) -> Self {
        Self {
            current: cfg.max, // 初始滿庫
            max: cfg.max,
            restock_per_tick: cfg.restock_per_tick,
        }
    }

    /// 庫存比例（千分比，0–1000）。
    fn ratio_bps(&self) -> u32 {
        if self.max == 0 { return 1000; }
        (self.current as u64 * 1000 / self.max as u64) as u32
    }

    /// 當前稀缺溢價乘數（100 = ×1.0，150 = ×1.5，200 = ×2.0）。
    fn price_multiplier(&self) -> u32 {
        let ratio = self.ratio_bps();
        if ratio >= SCARCITY_MID_BPS {
            100  // 庫存充足：正常價
        } else if ratio >= SCARCITY_LOW_BPS {
            PRICE_MULTIPLIER_MID  // 輕度稀缺：×1.5
        } else {
            PRICE_MULTIPLIER_HIGH // 嚴重缺貨：×2.0
        }
    }
}

/// NPC 販售庫存管理（ROADMAP 104）。
pub struct NpcStockState {
    /// 鍵：(merchant_key, ItemKind)。ItemKind 實作 Ord 但未實作 Hash，故用 BTreeMap。
    stocks: BTreeMap<(&'static str, ItemKind), ItemStock>,
}

impl NpcStockState {
    pub fn new() -> Self {
        let mut stocks = BTreeMap::new();
        for (item, cfg) in home_stock_configs() {
            stocks.insert((MERCHANT_HOME, item), ItemStock::new(&cfg));
        }
        // 其他星球商人目前只收購不販售，不初始化庫存。
        let _ = MERCHANT_VERDANT;
        let _ = MERCHANT_CRIMSON;
        let _ = MERCHANT_VOID;
        let _ = MERCHANT_AETHER;
        let _ = MERCHANT_ORIGIN;
        Self { stocks }
    }

    /// 查詢庫存量（0 = 缺貨）。
    pub fn available(&self, merchant: &str, item: ItemKind) -> u32 {
        self.stocks.iter()
            .find(|((m, i), _)| *m == merchant && *i == item)
            .map(|(_, s)| s.current)
            .unwrap_or(0)
    }

    /// 查詢最大庫存。
    pub fn max_stock(&self, merchant: &str, item: ItemKind) -> u32 {
        self.stocks.iter()
            .find(|((m, i), _)| *m == merchant && *i == item)
            .map(|(_, s)| s.max)
            .unwrap_or(0)
    }

    /// 計算當前有效販售價（含稀缺溢價）。
    /// `base_price` 是正常狀況下的售價（即 NPC_SELL_LIST 上的 price_per）。
    pub fn effective_sell_price(&self, merchant: &str, item: ItemKind, base_price: u32) -> u32 {
        let multiplier = self.stocks.iter()
            .find(|((m, i), _)| *m == merchant && *i == item)
            .map(|(_, s)| s.price_multiplier())
            .unwrap_or(100);
        // 有效價 = base × multiplier / 100，至少 base（不能低於原價）。
        ((base_price as u64 * multiplier as u64) / 100).max(base_price as u64) as u32
    }

    /// 嘗試購買：確認有足夠庫存、扣除，回傳 (actual_qty, needs_notice)。
    /// actual_qty 可能少於 qty（庫存不足時），needs_notice=true 時呼叫方回傳提示給玩家。
    pub fn try_purchase(
        &mut self,
        merchant: &'static str,
        item: ItemKind,
        qty: u32,
    ) -> PurchaseResult {
        // 找到對應庫存
        let entry = self.stocks.iter_mut()
            .find(|((m, i), _)| *m == merchant && *i == item);
        let Some((_, stock)) = entry else {
            // 沒有設定庫存的商人/物品（不應發生）：視作庫存無限，直接通過。
            return PurchaseResult { actual_qty: qty, notice: None };
        };
        if stock.current == 0 {
            return PurchaseResult {
                actual_qty: 0,
                notice: Some("庫存已售罄！等商隊補貨後再來。"),
            };
        }
        let actual_qty = qty.min(stock.current);
        stock.current = stock.current.saturating_sub(actual_qty);
        let notice = if actual_qty < qty {
            Some("庫存不足，只買到部分數量，等商隊補貨後再來。")
        } else {
            None
        };
        PurchaseResult { actual_qty, notice }
    }

    /// 退庫存：購買後因乙太不足而失敗時，把已扣除的庫存補回（至上限）。
    pub fn refund(&mut self, merchant: &'static str, item: ItemKind, qty: u32) {
        if let Some((_, stock)) = self.stocks.iter_mut()
            .find(|((m, i), _)| *m == merchant && *i == item)
        {
            stock.current = (stock.current + qty).min(stock.max);
        }
    }

    /// 商隊補貨：所有商品補充 restock_per_tick 數量（至各自上限）。
    pub fn tick_restock(&mut self) {
        for stock in self.stocks.values_mut() {
            stock.current = (stock.current + stock.restock_per_tick).min(stock.max);
        }
    }

    /// 取得故鄉商人所有在售商品的庫存快照（用於廣播給前端顯示）。
    /// 回傳 Vec<(ItemKind, current, max, effective_price)>。
    pub fn home_sell_snapshot(&self, sell_list: &[(ItemKind, u32)]) -> Vec<StockSnapshot> {
        sell_list.iter().map(|(item, base_price)| {
            let current = self.available(MERCHANT_HOME, *item);
            let max = self.max_stock(MERCHANT_HOME, *item);
            let effective_price = self.effective_sell_price(MERCHANT_HOME, *item, *base_price);
            StockSnapshot {
                item: *item,
                current,
                max,
                effective_price,
            }
        }).collect()
    }
}

/// 庫存快照（用於序列化廣播）。
#[derive(Debug, Clone)]
pub struct StockSnapshot {
    pub item: ItemKind,
    pub current: u32,
    pub max: u32,
    pub effective_price: u32,
}

/// 購買嘗試結果。
pub struct PurchaseResult {
    /// 實際成交數量（可能少於請求數量）。
    pub actual_qty: u32,
    /// 若有庫存問題，要通知玩家的訊息；None = 正常。
    pub notice: Option<&'static str>,
}

// ─── 純邏輯單元測試 ────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> NpcStockState { NpcStockState::new() }

    #[test]
    fn initial_stock_is_full() {
        let s = make();
        let pickaxe = s.available(MERCHANT_HOME, ItemKind::Pickaxe);
        let max = s.max_stock(MERCHANT_HOME, ItemKind::Pickaxe);
        assert_eq!(pickaxe, max, "初始庫存應滿");
        assert!(pickaxe > 0, "初始鎬子庫存應大於零");
    }

    #[test]
    fn purchase_deducts_stock() {
        let mut s = make();
        let before = s.available(MERCHANT_HOME, ItemKind::Pickaxe);
        let r = s.try_purchase(MERCHANT_HOME, ItemKind::Pickaxe, 2);
        assert_eq!(r.actual_qty, 2);
        assert!(r.notice.is_none());
        assert_eq!(s.available(MERCHANT_HOME, ItemKind::Pickaxe), before - 2);
    }

    #[test]
    fn purchase_partial_when_stock_low() {
        let mut s = make();
        // 手動設低庫存
        s.stocks.iter_mut()
            .find(|((m, i), _)| *m == MERCHANT_HOME && *i == ItemKind::Pickaxe)
            .unwrap().1.current = 2;
        let r = s.try_purchase(MERCHANT_HOME, ItemKind::Pickaxe, 5);
        assert_eq!(r.actual_qty, 2, "庫存只剩 2，最多買 2 個");
        assert!(r.notice.is_some(), "應提示庫存不足");
        assert_eq!(s.available(MERCHANT_HOME, ItemKind::Pickaxe), 0);
    }

    #[test]
    fn purchase_zero_stock_declines() {
        let mut s = make();
        s.stocks.iter_mut()
            .find(|((m, i), _)| *m == MERCHANT_HOME && *i == ItemKind::Pickaxe)
            .unwrap().1.current = 0;
        let r = s.try_purchase(MERCHANT_HOME, ItemKind::Pickaxe, 1);
        assert_eq!(r.actual_qty, 0, "庫存為零應婉拒");
        assert!(r.notice.is_some(), "應提示售罄");
    }

    #[test]
    fn restock_increases_stock() {
        let mut s = make();
        s.stocks.iter_mut()
            .find(|((m, i), _)| *m == MERCHANT_HOME && *i == ItemKind::Pickaxe)
            .unwrap().1.current = 0;
        s.tick_restock();
        let after = s.available(MERCHANT_HOME, ItemKind::Pickaxe);
        assert!(after > 0, "補貨後庫存應增加");
    }

    #[test]
    fn restock_capped_at_max() {
        let mut s = make();
        s.tick_restock();
        let avail = s.available(MERCHANT_HOME, ItemKind::Wood);
        let max = s.max_stock(MERCHANT_HOME, ItemKind::Wood);
        assert_eq!(avail, max, "補貨後不超過上限");
    }

    #[test]
    fn price_normal_when_stock_sufficient() {
        let s = make();
        // 初始庫存滿（≥60%）→ 正常價
        let p = s.effective_sell_price(MERCHANT_HOME, ItemKind::Pickaxe, 15);
        assert_eq!(p, 15, "庫存充足時價格應等於基準價");
    }

    #[test]
    fn price_multiplied_when_stock_low() {
        let mut s = make();
        // 設庫存低於 30%（最大 8，設 2 = 25%）
        s.stocks.iter_mut()
            .find(|((m, i), _)| *m == MERCHANT_HOME && *i == ItemKind::Pickaxe)
            .unwrap().1.current = 2; // 2/8 = 25% < 30%
        let p = s.effective_sell_price(MERCHANT_HOME, ItemKind::Pickaxe, 15);
        // ×2.0 → 30
        assert_eq!(p, 30, "嚴重缺貨時應為 2× 溢價");
    }

    #[test]
    fn price_mid_multiplier_between_30_60_percent() {
        let mut s = make();
        // 設庫存在 30%~60% 之間（最大 8，設 4 = 50%）
        s.stocks.iter_mut()
            .find(|((m, i), _)| *m == MERCHANT_HOME && *i == ItemKind::Pickaxe)
            .unwrap().1.current = 4; // 4/8 = 50%，在 30%~60% 之間
        let p = s.effective_sell_price(MERCHANT_HOME, ItemKind::Pickaxe, 15);
        // ×1.5 → 22（floor）
        assert_eq!(p, 22, "輕度稀缺時應為 1.5× 溢價（15×150/100=22）");
    }

    #[test]
    fn unknown_item_has_unlimited_stock() {
        // 不在販售清單的物品（如 CrystalShard）沒有庫存設定 → try_purchase 視作無限
        let mut s = make();
        let r = s.try_purchase(MERCHANT_HOME, ItemKind::CrystalShard, 10);
        assert_eq!(r.actual_qty, 10, "無庫存設定的物品視作無限");
    }

    #[test]
    fn home_sell_snapshot_returns_all_items() {
        let s = make();
        let sell_list = vec![
            (ItemKind::Pickaxe, 15u32),
            (ItemKind::Weapon, 25u32),
            (ItemKind::Wood, 3u32),
        ];
        let snaps = s.home_sell_snapshot(&sell_list);
        assert_eq!(snaps.len(), 3);
        assert!(snaps.iter().all(|sn| sn.max > 0), "所有快照的 max 應大於零");
    }

    #[test]
    fn wood_material_has_higher_stock_than_tools() {
        let s = make();
        let wood_max = s.max_stock(MERCHANT_HOME, ItemKind::Wood);
        let pickaxe_max = s.max_stock(MERCHANT_HOME, ItemKind::Pickaxe);
        assert!(wood_max > pickaxe_max, "基礎素材庫存上限應高於工具");
    }
}
