//! 星際拍賣行（ROADMAP 522）。
//!
//! 每 AUCTION_INTERVAL_SECS 秒，城鎮廣場自動出現一件傳說遺物；
//! 全服玩家在 AUCTION_DURATION_SECS 秒內競標；出價成功立刻從玩家乙太扣除，
//! 同時退還前一位出價者的乙太；時間到最高出價者得標並收到物品。
//!
//! 設計鐵律：
//! - 純邏輯、零 LLM、零持久化、記憶體模式（重啟清零）。
//! - 出價立即扣乙太、被超標立即退款：確保乙太不憑空消失。
//! - 兩鎖不巢狀：auction 鎖與 players 鎖分開取放（守 prod-deadlock 鐵律）。
//! - 零 migration：拍賣不入存檔，重啟即重置，傳說物非常態收入管道。

use crate::inventory::ItemKind;
use crate::protocol::AuctionView;
use uuid::Uuid;

/// 兩次拍賣間冷卻秒數（2 小時）。
pub const AUCTION_INTERVAL_SECS: f32 = 7200.0;
/// 伺服器啟動後首場拍賣等待秒數（5 分鐘，讓玩家先熱身）。
pub const FIRST_AUCTION_WAIT_SECS: f32 = 300.0;
/// 單場競標持續秒數（15 分鐘）。
pub const AUCTION_DURATION_SECS: f32 = 900.0;
/// 最低加價幅度（乙太）。新出價必須至少比當前最高出價高這麼多。
pub const MIN_BID_INCREMENT: u32 = 5;
/// 拍賣物品座標（城鎮廣場拍賣台，NPC 里長旁）。
pub const AUCTION_WX: f32 = 2300.0;
pub const AUCTION_WY: f32 = 2100.0;
/// 玩家需在此範圍內才能出價（像素）。
pub const AUCTION_REACH: f32 = 200.0;

/// 輪番出場的拍賣物品：(物品種類, 底價乙太, 數量)。
/// 依 `item_cycle % len` 輪換出場，讓每件傳說物都有機會上台。
pub const AUCTION_CATALOG: &[(ItemKind, u32, u32)] = &[
    (ItemKind::AlphaCrystal,    40, 1),
    (ItemKind::EtherOverlordCore, 60, 1),
    (ItemKind::StarCrystalBlade, 80, 1),
    (ItemKind::LegendaryCore,  100, 1),
    (ItemKind::RiftBlade,       70, 1),
];

/// 單場競標狀態（進行中時存在於 `AuctionState.active`）。
#[derive(Debug, Clone)]
pub struct ActiveAuction {
    /// 本場拍賣物品。
    pub item: ItemKind,
    /// 本場拍賣數量（目前均為 1，保留擴展空間）。
    pub qty: u32,
    /// 底價（未出價時的起始金額）。
    pub base_price: u32,
    /// 目前最高出價（含底價；無人出價時等於底價）。
    pub current_bid: u32,
    /// 當前最高出價者 UID（None = 尚無人出價）。
    pub current_bidder: Option<Uuid>,
    /// 當前最高出價者名稱（供廣播顯示；無人時為空字串）。
    pub current_bidder_name: String,
    /// 競標剩餘秒數。
    pub remaining_secs: f32,
}

/// 全局拍賣狀態（純記憶體，重啟清零）。
pub struct AuctionState {
    /// 距下次開場的倒數秒數（active = Some 時此欄無意義）。
    countdown: f32,
    /// 目前進行中的競標（None = 閒置等待）。
    pub active: Option<ActiveAuction>,
    /// 已完成的場次數（用於輪換物品）。
    item_cycle: usize,
}

/// `tick()` 回傳值：描述本幀的拍賣事件。
#[derive(Debug)]
pub enum AuctionTick {
    /// 無事件。
    None,
    /// 剛開場。
    Spawned { item: ItemKind, base_price: u32, qty: u32 },
    /// 競標結束。`winner` = None 代表無人出價（物品流標，無需發放）。
    Finished {
        winner: Option<(Uuid, String, u32)>, // (uid, name, 成交金額)
        item: ItemKind,
        qty: u32,
    },
}

/// `try_bid()` 回傳值。
#[derive(Debug, PartialEq)]
pub enum BidResult {
    /// 出價成功。`refund_to` = Some((uid, 金額)) 表示需要退款給前一位出價者。
    Accepted { refund_to: Option<(Uuid, u32)> },
    /// 無進行中的競標。
    NoActiveAuction,
    /// 出價金額不足（需 ≥ `minimum`）。
    TooLow { minimum: u32 },
}

impl AuctionState {
    pub fn new() -> Self {
        Self {
            countdown: FIRST_AUCTION_WAIT_SECS,
            active: None,
            item_cycle: 0,
        }
    }

    /// 推進時間，回傳本幀事件（Spawned / Finished / None）。
    pub fn tick(&mut self, dt: f32) -> AuctionTick {
        // ─── 競標進行中 ───────────────────────────────────────────────────────
        if let Some(a) = self.active.as_mut() {
            a.remaining_secs -= dt;
            if a.remaining_secs <= 0.0 {
                let finished = AuctionTick::Finished {
                    winner: a.current_bidder.map(|uid| {
                        (uid, a.current_bidder_name.clone(), a.current_bid)
                    }),
                    item: a.item,
                    qty: a.qty,
                };
                self.active = None;
                self.countdown = AUCTION_INTERVAL_SECS;
                return finished;
            }
            return AuctionTick::None;
        }

        // ─── 等待下場 ────────────────────────────────────────────────────────
        self.countdown -= dt;
        if self.countdown <= 0.0 {
            let idx = self.item_cycle % AUCTION_CATALOG.len();
            self.item_cycle = self.item_cycle.wrapping_add(1);
            let (item, base_price, qty) = AUCTION_CATALOG[idx];
            self.active = Some(ActiveAuction {
                item,
                qty,
                base_price,
                current_bid: base_price,
                current_bidder: None,
                current_bidder_name: String::new(),
                remaining_secs: AUCTION_DURATION_SECS,
            });
            return AuctionTick::Spawned { item, base_price, qty };
        }

        AuctionTick::None
    }

    /// 嘗試出價。純邏輯層不扣乙太——呼叫端（ws.rs）負責：
    /// 1. 先確認玩家有足夠乙太再呼叫此函式。
    /// 2. 成功後扣除玩家 `amount` 乙太，並退款 `refund_to` 的前一位出價者。
    pub fn try_bid(&mut self, uid: Uuid, name: &str, amount: u32) -> BidResult {
        let Some(a) = self.active.as_mut() else {
            return BidResult::NoActiveAuction;
        };
        // 第一位出價者只需 ≥ 底價；後續出價者需 ≥ 當前最高 + 最低加價幅度。
        let minimum = if a.current_bidder.is_some() {
            a.current_bid.saturating_add(MIN_BID_INCREMENT)
        } else {
            a.base_price
        };
        if amount < minimum {
            return BidResult::TooLow { minimum };
        }
        let refund_to = a.current_bidder.map(|prev| (prev, a.current_bid));
        a.current_bid = amount;
        a.current_bidder = Some(uid);
        a.current_bidder_name = name.to_string();
        BidResult::Accepted { refund_to }
    }

    /// 提供給 Snapshot 的唯讀視圖（無活躍競標時回 None，節省頻寬）。
    pub fn view(&self) -> Option<AuctionView> {
        let a = self.active.as_ref()?;
        Some(AuctionView {
            item: format!("{:?}", a.item)
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    if i > 0 && c.is_uppercase() {
                        format!("_{}", c.to_lowercase())
                    } else {
                        c.to_lowercase().to_string()
                    }
                })
                .collect(),
            qty: a.qty,
            base_price: a.base_price,
            current_bid: a.current_bid,
            bidder_name: a.current_bidder_name.clone(),
            remaining_secs: a.remaining_secs.ceil() as u32,
        })
    }

    /// 剩餘等待秒數（僅供 HUD 等待提示用，活躍競標時無意義）。
    pub fn countdown_secs(&self) -> f32 {
        self.countdown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_uid() -> Uuid {
        Uuid::new_v4()
    }

    #[test]
    fn no_event_before_first_wait() {
        let mut s = AuctionState::new();
        match s.tick(1.0) {
            AuctionTick::None => {}
            _ => panic!("首秒不應有事件"),
        }
        assert!(s.active.is_none());
    }

    #[test]
    fn spawns_after_first_wait() {
        let mut s = AuctionState::new();
        match s.tick(FIRST_AUCTION_WAIT_SECS + 0.1) {
            AuctionTick::Spawned { .. } => {}
            _ => panic!("應觸發 Spawned"),
        }
        assert!(s.active.is_some());
    }

    #[test]
    fn first_bid_needs_only_base_price() {
        let mut s = AuctionState::new();
        s.tick(FIRST_AUCTION_WAIT_SECS + 0.1);
        let uid = new_uid();
        let base = s.active.as_ref().unwrap().base_price;
        match s.try_bid(uid, "甲", base) {
            BidResult::Accepted { refund_to: None } => {} // 第一位，無前任退款
            r => panic!("預期 Accepted(None)，得 {r:?}"),
        }
        assert_eq!(s.active.as_ref().unwrap().current_bid, base);
    }

    #[test]
    fn first_bid_below_base_rejected() {
        let mut s = AuctionState::new();
        s.tick(FIRST_AUCTION_WAIT_SECS + 0.1);
        let uid = new_uid();
        let base = s.active.as_ref().unwrap().base_price;
        match s.try_bid(uid, "乙", base - 1) {
            BidResult::TooLow { minimum } => assert_eq!(minimum, base),
            r => panic!("預期 TooLow，得 {r:?}"),
        }
    }

    #[test]
    fn second_bid_needs_increment() {
        let mut s = AuctionState::new();
        s.tick(FIRST_AUCTION_WAIT_SECS + 0.1);
        let (a, b) = (new_uid(), new_uid());
        let base = s.active.as_ref().unwrap().base_price;
        s.try_bid(a, "甲", base);
        // 剛好加最低加價：應過
        match s.try_bid(b, "乙", base + MIN_BID_INCREMENT) {
            BidResult::Accepted { refund_to: Some((prev_uid, prev_amt)) } => {
                assert_eq!(prev_uid, a);
                assert_eq!(prev_amt, base);
            }
            r => panic!("預期有退款 Accepted，得 {r:?}"),
        }
    }

    #[test]
    fn second_bid_below_increment_rejected() {
        let mut s = AuctionState::new();
        s.tick(FIRST_AUCTION_WAIT_SECS + 0.1);
        let (a, b) = (new_uid(), new_uid());
        let base = s.active.as_ref().unwrap().base_price;
        s.try_bid(a, "甲", base);
        match s.try_bid(b, "乙", base + MIN_BID_INCREMENT - 1) {
            BidResult::TooLow { minimum } => {
                assert_eq!(minimum, base + MIN_BID_INCREMENT);
            }
            r => panic!("預期 TooLow，得 {r:?}"),
        }
    }

    #[test]
    fn no_active_auction_returns_error() {
        let mut s = AuctionState::new();
        match s.try_bid(new_uid(), "甲", 999) {
            BidResult::NoActiveAuction => {}
            r => panic!("預期 NoActiveAuction，得 {r:?}"),
        }
    }

    #[test]
    fn finishes_when_time_expires_with_winner() {
        let mut s = AuctionState::new();
        s.tick(FIRST_AUCTION_WAIT_SECS + 0.1);
        let uid = new_uid();
        let base = s.active.as_ref().unwrap().base_price;
        s.try_bid(uid, "甲", base);
        match s.tick(AUCTION_DURATION_SECS + 0.1) {
            AuctionTick::Finished { winner: Some((w, name, amt)), .. } => {
                assert_eq!(w, uid);
                assert_eq!(name, "甲");
                assert_eq!(amt, base);
            }
            r => panic!("預期 Finished(Some)，得 {r:?}"),
        }
        assert!(s.active.is_none());
    }

    #[test]
    fn finishes_with_no_winner_if_no_bids() {
        let mut s = AuctionState::new();
        s.tick(FIRST_AUCTION_WAIT_SECS + 0.1);
        match s.tick(AUCTION_DURATION_SECS + 0.1) {
            AuctionTick::Finished { winner: None, .. } => {}
            r => panic!("預期 Finished(None)，得 {r:?}"),
        }
    }

    #[test]
    fn resets_countdown_after_finish() {
        let mut s = AuctionState::new();
        s.tick(FIRST_AUCTION_WAIT_SECS + 0.1);
        s.tick(AUCTION_DURATION_SECS + 0.1);
        // 結束後等待 AUCTION_INTERVAL_SECS 秒才再開場
        match s.tick(AUCTION_INTERVAL_SECS - 1.0) {
            AuctionTick::None => {}
            _ => panic!("冷卻未到應無事件"),
        }
        match s.tick(2.0) {
            AuctionTick::Spawned { .. } => {}
            _ => panic!("冷卻結束後應 Spawned"),
        }
    }

    #[test]
    fn items_cycle_through_catalog() {
        let mut s = AuctionState::new();
        let n = AUCTION_CATALOG.len();
        let mut seen_items = vec![];
        for _ in 0..n {
            // 快速開場並立刻結束
            s.tick(FIRST_AUCTION_WAIT_SECS + 1.0);
            if let Some(a) = &s.active {
                seen_items.push(a.item);
            }
            s.tick(AUCTION_DURATION_SECS + 1.0);
            s.countdown = 0.0; // 強制立刻下一場（測試用）
        }
        assert_eq!(seen_items.len(), n, "應輪完 {} 件物品", n);
        // 每件物品在清單中都出現一次
        for &(expected, ..) in AUCTION_CATALOG {
            assert!(seen_items.contains(&expected), "{expected:?} 沒出現在輪換中");
        }
    }

    #[test]
    fn view_returns_none_when_inactive() {
        let s = AuctionState::new();
        assert!(s.view().is_none());
    }

    #[test]
    fn view_returns_some_when_active() {
        let mut s = AuctionState::new();
        s.tick(FIRST_AUCTION_WAIT_SECS + 0.1);
        assert!(s.view().is_some());
        let v = s.view().unwrap();
        assert!(v.remaining_secs > 0);
        assert!(v.current_bid > 0);
    }

    #[test]
    fn same_player_can_outbid_themselves() {
        // 允許同一位玩家追加出更高的價（提高保護費）
        let mut s = AuctionState::new();
        s.tick(FIRST_AUCTION_WAIT_SECS + 0.1);
        let uid = new_uid();
        let base = s.active.as_ref().unwrap().base_price;
        s.try_bid(uid, "甲", base);
        match s.try_bid(uid, "甲", base + MIN_BID_INCREMENT) {
            BidResult::Accepted { refund_to: Some((prev, _)) } => assert_eq!(prev, uid),
            r => panic!("預期自我超標，得 {r:?}"),
        }
    }
}
