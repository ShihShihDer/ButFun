//! 玩家對玩家的世界市場：掛單、購買、取消。
//!
//! 純邏輯層（無 IO / 無 WebSocket），以記憶體 HashMap 為儲存。
//! v1 重啟後掛單清空——賣家需重新掛單。
//! 設計：物品掛單後從賣家背包移出（由 ws.rs 在背包層扣除）；
//! 購買後物品給買家、乙太給賣家（由 ws.rs 處理轉帳）；
//! 取消後物品退回賣家背包。

use std::collections::HashMap;

use uuid::Uuid;

use crate::inventory::ItemKind;

/// 一筆市場掛單。
#[derive(Debug, Clone, PartialEq)]
pub struct MarketListing {
    /// 掛單唯一識別碼。
    pub id: Uuid,
    /// 賣家玩家 id。
    pub seller_id: Uuid,
    /// 賣家顯示名（快照廣播用，不再查 players map）。
    pub seller_name: String,
    /// 販售物品種類。
    pub item: ItemKind,
    /// 數量（1..MAX_STACK）。
    pub qty: u32,
    /// 每單位乙太價格（可為 0 = 贈送）。
    pub price_per: u32,
    /// 掛單時賣家的世界 x 座標（AOI 剔除 + 世界渲染用）。
    pub x: f32,
    /// 掛單時賣家的世界 y 座標。
    pub y: f32,
}

impl MarketListing {
    /// 這筆掛單的總價（price_per * qty，溢位截斷）。
    #[allow(dead_code)]
    pub fn total_price(&self) -> u32 {
        self.price_per.saturating_mul(self.qty)
    }
}

/// 世界市場（記憶體，v1）。
#[derive(Debug, Default)]
pub struct Market {
    listings: HashMap<Uuid, MarketListing>,
}

impl Market {
    pub fn new() -> Self {
        Self::default()
    }

    /// 張貼掛單（物品已在呼叫端從背包移除）。
    pub fn post(&mut self, listing: MarketListing) {
        self.listings.insert(listing.id, listing);
    }

    /// 取消掛單（只有賣家本人有效）。
    /// 回傳托管的物品 `(item, qty)`，呼叫端再還給賣家背包。
    pub fn cancel(&mut self, listing_id: Uuid, requester: Uuid) -> Option<(ItemKind, u32)> {
        if self.listings.get(&listing_id).map_or(false, |l| l.seller_id == requester) {
            let l = self.listings.remove(&listing_id).unwrap();
            Some((l.item, l.qty))
        } else {
            None
        }
    }

    /// 購買掛單（不驗乙太，由呼叫端驗）。
    /// 回傳掛單讓呼叫端：把物品給買家、把乙太轉給賣家。
    pub fn buy(&mut self, listing_id: Uuid) -> Option<MarketListing> {
        self.listings.remove(&listing_id)
    }

    /// 所有掛單（快照廣播用）。
    pub fn all(&self) -> impl Iterator<Item = &MarketListing> {
        self.listings.values()
    }

    /// 取消某賣家的所有掛單，回傳托管物品（玩家刪帳等清理用）。
    #[allow(dead_code)]
    pub fn cancel_all_by(&mut self, seller_id: Uuid) -> Vec<(ItemKind, u32)> {
        let ids: Vec<Uuid> = self
            .listings
            .values()
            .filter(|l| l.seller_id == seller_id)
            .map(|l| l.id)
            .collect();
        ids.into_iter()
            .filter_map(|id| self.listings.remove(&id))
            .map(|l| (l.item, l.qty))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_listing(seller: Uuid, item: ItemKind, qty: u32, price_per: u32) -> MarketListing {
        MarketListing {
            id: Uuid::new_v4(),
            seller_id: seller,
            seller_name: "賣家".into(),
            item,
            qty,
            price_per,
            x: 100.0,
            y: 200.0,
        }
    }

    #[test]
    fn post_and_list() {
        let mut m = Market::new();
        let seller = Uuid::new_v4();
        let l = make_listing(seller, ItemKind::Wood, 5, 3);
        let id = l.id;
        m.post(l);
        let all: Vec<_> = m.all().collect();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
    }

    #[test]
    fn buy_removes_listing() {
        let mut m = Market::new();
        let seller = Uuid::new_v4();
        let l = make_listing(seller, ItemKind::Stone, 3, 2);
        let id = l.id;
        m.post(l);
        let bought = m.buy(id).unwrap();
        assert_eq!(bought.item, ItemKind::Stone);
        assert!(m.all().next().is_none());
    }

    #[test]
    fn cancel_by_owner_returns_item() {
        let mut m = Market::new();
        let seller = Uuid::new_v4();
        let l = make_listing(seller, ItemKind::Ether, 2, 10);
        let id = l.id;
        m.post(l);
        let returned = m.cancel(id, seller).unwrap();
        assert_eq!(returned, (ItemKind::Ether, 2));
        assert!(m.all().next().is_none());
    }

    #[test]
    fn cancel_by_non_owner_fails() {
        let mut m = Market::new();
        let seller = Uuid::new_v4();
        let other = Uuid::new_v4();
        let l = make_listing(seller, ItemKind::Wood, 1, 1);
        let id = l.id;
        m.post(l);
        assert!(m.cancel(id, other).is_none());
        assert_eq!(m.all().count(), 1);
    }

    #[test]
    fn total_price_no_overflow() {
        let l = MarketListing {
            id: Uuid::new_v4(),
            seller_id: Uuid::new_v4(),
            seller_name: "x".into(),
            item: ItemKind::Wood,
            qty: u32::MAX,
            price_per: 2,
            x: 0.0,
            y: 0.0,
        };
        // saturating_mul 不溢位
        assert_eq!(l.total_price(), u32::MAX);
    }
}
