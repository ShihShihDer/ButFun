//! 旅行商人系統（ROADMAP 135）。
//!
//! 每 2 小時從城外來一位神秘旅行商人，帶著其他生態域的稀有物品，
//! 停留 10 分鐘，限時交易。玩家近身（TRADE_REACH 像素內）可開交易面板，
//! 用乙太購買只有旅行商人才帶來的稀有物品（每次來訪庫存獨立重置）。
//!
//! 成本紀律：純本機邏輯，**不呼叫任何 LLM**；零 migration，記憶體模式，重啟清零。

use crate::inventory::ItemKind;

/// 首次拜訪等待（秒）——伺服器啟動後 5 分鐘。
pub const FIRST_WAIT_SECS: f32 = 300.0;
/// 拜訪間隔（秒）——2 小時。
pub const VISIT_INTERVAL_SECS: f32 = 7200.0;
/// 停留時間（秒）——10 分鐘。
pub const STAY_SECS: f32 = 600.0;
/// 交易有效距離（像素）——玩家走進這個範圍才能開面板。
pub const TRADE_REACH: f32 = 100.0;
/// 旅行商人站立位置（城鎮廣場北緣；遠離一般商人避免混淆）。
pub const WANDERER_X: f32 = 2380.0;
pub const WANDERER_Y: f32 = 2150.0;

/// 旅行商人一個商品條目。
#[derive(Debug, Clone)]
pub struct WanderingItem {
    pub item: ItemKind,
    /// 每次來訪可售數量上限（售完即缺貨）。
    pub stock: u32,
    /// 乙太單價。
    pub price_ether: u32,
    /// 本次來訪已售出數量（到訪重置為 0）。
    pub sold: u32,
}

impl WanderingItem {
    fn new(item: ItemKind, stock: u32, price_ether: u32) -> Self {
        Self { item, stock, price_ether, sold: 0 }
    }

    pub fn remaining(&self) -> u32 {
        self.stock.saturating_sub(self.sold)
    }
}

/// 旅行商人狀態（純記憶體，重啟清零）。
pub struct WanderingMerchantState {
    /// 距下次到訪的冷卻倒數（秒）。
    pub cooldown: f32,
    /// 在場倒計時（秒）；0 = 商人不在城鎮。
    pub active_secs: f32,
    /// 本次來訪商品目錄（到訪時填入，離去後清空）。
    pub catalog: Vec<WanderingItem>,
}

impl WanderingMerchantState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_WAIT_SECS,
            active_secs: 0.0,
            catalog: vec![],
        }
    }

    /// 旅行商人目前是否在城鎮。
    pub fn is_active(&self) -> bool {
        self.active_secs > 0.0
    }

    /// 剩餘整數秒（供快照廣播）；不在城鎮時回 0。
    pub fn remaining_secs(&self) -> u32 {
        self.active_secs.ceil() as u32
    }

    /// 前進 dt 秒。回傳 (arrived, departed)。
    pub fn tick(&mut self, dt: f32) -> (bool, bool) {
        if self.is_active() {
            self.active_secs -= dt;
            if self.active_secs <= 0.0 {
                self.active_secs = 0.0;
                self.catalog.clear();
                self.cooldown = VISIT_INTERVAL_SECS;
                return (false, true);
            }
            return (false, false);
        }

        self.cooldown -= dt;
        if self.cooldown <= 0.0 {
            self.active_secs = STAY_SECS;
            self.catalog = build_catalog();
            return (true, false);
        }
        (false, false)
    }

    /// 玩家購買 qty 單位 item。回傳 Ok(total_ether_cost) 或 Err(描述)。
    pub fn buy(&mut self, item: ItemKind, qty: u32) -> Result<u32, &'static str> {
        if !self.is_active() {
            return Err("旅行商人不在城鎮");
        }
        if qty == 0 {
            return Err("數量必須 >= 1");
        }
        let entry = self
            .catalog
            .iter_mut()
            .find(|e| e.item == item)
            .ok_or("旅行商人沒有這個商品")?;
        if qty > entry.remaining() {
            return Err("商品庫存不足");
        }
        let cost = entry.price_ether.saturating_mul(qty);
        entry.sold += qty;
        Ok(cost)
    }
}

/// 每次來訪的標準商品目錄——帶著其他生態域稀有物品，乙太售價略高於市面。
fn build_catalog() -> Vec<WanderingItem> {
    vec![
        // 裂縫碎片：通常只能在深層戰鬥取得，量少，旅商帶來 2 個
        WanderingItem::new(ItemKind::RiftShard, 2, 20),
        // 岩漿晶石：炎紅星才有，非探索者難以取得
        WanderingItem::new(ItemKind::LavaCrystal, 3, 14),
        // 翠幽碎片：翠幽星才有，旅商偶爾帶來
        WanderingItem::new(ItemKind::JadeShard, 3, 12),
        // 星晶碎片：流星雨少見掉落，旅商帶來穩定補貨渠道
        WanderingItem::new(ItemKind::StarCrystalShard, 5, 8),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_arrival_triggers_after_first_wait() {
        let mut s = WanderingMerchantState::new();
        let (arrived, _) = s.tick(FIRST_WAIT_SECS - 1.0);
        assert!(!arrived, "未到 FIRST_WAIT_SECS 不應到訪");
        assert!(!s.is_active());
        let (arrived, _) = s.tick(1.0);
        assert!(arrived, "剛好超過 FIRST_WAIT_SECS 應到訪");
        assert!(s.is_active());
        assert_eq!(s.catalog.len(), 4);
    }

    #[test]
    fn stays_for_stay_secs_then_departs() {
        let mut s = WanderingMerchantState::new();
        s.cooldown = 0.1;
        s.tick(0.1); // 觸發到訪
        assert!(s.is_active());
        let (_, departed) = s.tick(STAY_SECS);
        assert!(departed, "應在 STAY_SECS 後離去");
        assert!(!s.is_active());
        assert!(s.catalog.is_empty(), "離去後商品目錄應清空");
    }

    #[test]
    fn next_visit_resets_sold() {
        let mut s = WanderingMerchantState::new();
        s.cooldown = 0.1;
        s.tick(0.1);
        s.buy(ItemKind::RiftShard, 1).unwrap();
        s.tick(STAY_SECS); // 商人離去
        s.tick(VISIT_INTERVAL_SECS); // 觸發下一次到訪
        assert!(s.is_active());
        let entry = s.catalog.iter().find(|e| e.item == ItemKind::RiftShard).unwrap();
        assert_eq!(entry.sold, 0, "每次到訪 sold 應重置");
    }

    #[test]
    fn buy_deducts_stock_and_returns_cost() {
        let mut s = WanderingMerchantState::new();
        s.active_secs = STAY_SECS;
        s.catalog = build_catalog();
        let cost = s.buy(ItemKind::RiftShard, 1).unwrap();
        assert_eq!(cost, 20, "RiftShard 售價應為 20 乙太");
        let e = s.catalog.iter().find(|e| e.item == ItemKind::RiftShard).unwrap();
        assert_eq!(e.sold, 1);
        assert_eq!(e.remaining(), 1); // stock=2, sold=1
    }

    #[test]
    fn buy_multi_qty_correct_cost() {
        let mut s = WanderingMerchantState::new();
        s.active_secs = STAY_SECS;
        s.catalog = build_catalog();
        let cost = s.buy(ItemKind::StarCrystalShard, 3).unwrap();
        assert_eq!(cost, 24); // 8 * 3
    }

    #[test]
    fn buy_fails_when_inactive() {
        let mut s = WanderingMerchantState::new();
        assert!(s.buy(ItemKind::RiftShard, 1).is_err());
    }

    #[test]
    fn buy_fails_when_out_of_stock() {
        let mut s = WanderingMerchantState::new();
        s.active_secs = STAY_SECS;
        s.catalog = build_catalog();
        s.buy(ItemKind::RiftShard, 2).unwrap(); // 全買光 (stock=2)
        assert!(s.buy(ItemKind::RiftShard, 1).is_err(), "庫存耗盡後應拒絕購買");
    }

    #[test]
    fn buy_zero_qty_fails() {
        let mut s = WanderingMerchantState::new();
        s.active_secs = STAY_SECS;
        s.catalog = build_catalog();
        assert!(s.buy(ItemKind::RiftShard, 0).is_err());
    }

    #[test]
    fn remaining_secs_returns_ceil() {
        let mut s = WanderingMerchantState::new();
        s.active_secs = 9.3;
        assert_eq!(s.remaining_secs(), 10);
    }

    #[test]
    fn cooldown_resets_after_depart() {
        let mut s = WanderingMerchantState::new();
        s.cooldown = 0.1;
        s.tick(0.1);
        s.tick(STAY_SECS);
        // 冷卻應重置為 VISIT_INTERVAL_SECS
        assert!((s.cooldown - VISIT_INTERVAL_SECS).abs() < 1.0);
    }
}
