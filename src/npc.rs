//! NPC 商人純邏輯：位置、商品目錄、交易驗證。
//!
//! 新手村固定擺一名商人 NPC：收購玩家採集的素材（→ 乙太），販售工具（← 乙太）。
//! 目的：給剛上線的新玩家一個穩定的「第一桶乙太」出口，
//! 讓採集的木材／石頭有地方換錢，不必等到農地收成或玩家市場有人掛單。
//!
//! 這層只管純邏輯（距離判斷、交易驗證），無 IO、不碰 WebSocket，便於自動測試。

use crate::inventory::{Inventory, ItemKind};
use crate::state::{PUB_FIELD_ORIGIN_X, PUB_FIELD_ORIGIN_Y, VERDANT_SPAWN_X, VERDANT_SPAWN_Y, CRIMSON_SPAWN_X, CRIMSON_SPAWN_Y, VOID_SPAWN_X, VOID_SPAWN_Y, AETHER_SPAWN_X, AETHER_SPAWN_Y, ORIGIN_SPAWN_X, ORIGIN_SPAWN_Y};
use crate::field::{FIELD_ROWS, TILE_SIZE};

/// 玩家離商人多近才能互動（像素）。
pub const SHOP_REACH: f32 = 96.0;

/// 商人在世界上的位置：公共農地左邊、垂直置中，讓新玩家一眼就看得到。
pub fn merchant_pos() -> (f32, f32) {
    let field_height = FIELD_ROWS as f32 * TILE_SIZE;
    (
        PUB_FIELD_ORIGIN_X - 80.0,
        PUB_FIELD_ORIGIN_Y + field_height / 2.0,
    )
}

/// 單筆商品目錄條目：物品種類 + 每單位乙太價格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopEntry {
    pub item: ItemKind,
    /// 每個物品的乙太價格。
    pub price_per: u32,
}

/// NPC **收購**清單（玩家 → NPC，換乙太）。
/// 給採集素材一個現金出口，新玩家不需等農地就能攢起第一桶乙太。
/// 晶石碎片（深層晶洞掉落）以 3 乙太/個溢價收購，鼓勵探索型玩家深入岩地。
/// 蕈菇孢子（森林蕈菇洞掉落）以 2 乙太/個溢價收購，鼓勵探索型玩家深入森林。
/// 古代碎片（沙漠遺跡掉落）以 4 乙太/個高溢價收購，回報最遠、最危險的沙漠探索。
/// 深海珍珠（水域珊瑚礁掉落）以 5 乙太/個最高溢價收購，是所有生態特產中最稀有的。
/// 野花種子（草原野花叢掉落）以 2 乙太/個溢價收購，給穿梭草原的玩家第五條乙太路線。
pub const NPC_BUY_LIST: &[ShopEntry] = &[
    ShopEntry { item: ItemKind::Wood,             price_per: 1 },
    ShopEntry { item: ItemKind::Stone,            price_per: 1 },
    ShopEntry { item: ItemKind::Ether,            price_per: 2 },
    ShopEntry { item: ItemKind::Dirt,             price_per: 1 },
    ShopEntry { item: ItemKind::CrystalShard,     price_per: 3 },
    ShopEntry { item: ItemKind::MushroomSpore,    price_per: 2 },
    ShopEntry { item: ItemKind::AncientFragment,  price_per: 4 },
    ShopEntry { item: ItemKind::DeepSeaPearl,     price_per: 5 },
    ShopEntry { item: ItemKind::WildflowerSeed,   price_per: 2 },
    // 裂縫碎片（宇宙裂縫事件限定）：12 乙太/個，全故鄉商人最高溢價。
    ShopEntry { item: ItemKind::RiftShard,        price_per: 12 },
];

/// NPC **販售**清單（NPC → 玩家，花乙太）。
/// 讓玩家能直接花乙太買工具，不一定要先湊齊合成素材（但自己合成更便宜）。
pub const NPC_SELL_LIST: &[ShopEntry] = &[
    ShopEntry { item: ItemKind::Pickaxe, price_per: 15 },
    ShopEntry { item: ItemKind::Weapon,  price_per: 25 },
];

/// 翠幽星商人在世界上的位置：翠幽星出生點附近偏左，讓玩家傳送後立刻看得到商人。
pub fn verdant_merchant_pos() -> (f32, f32) {
    (VERDANT_SPAWN_X - 120.0, VERDANT_SPAWN_Y)
}

/// 翠幽星商人互動範圍判定。
pub fn is_within_verdant_shop_reach(px: f32, py: f32) -> bool {
    let (mx, my) = verdant_merchant_pos();
    let dx = px - mx;
    let dy = py - my;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 翠幽星 NPC **收購**清單（玩家 → NPC，換乙太）。
/// 翠幽星商人以最高溢價收購翠幽碎片，鼓勵玩家深入翠幽星探索。
/// 也收購故鄉生態特產（給帶著舊存貨來翠幽星的玩家出路）。
pub const VERDANT_BUY_LIST: &[ShopEntry] = &[
    ShopEntry { item: ItemKind::JadeShard,       price_per: 8 },
    ShopEntry { item: ItemKind::CrystalShard,    price_per: 3 },
    ShopEntry { item: ItemKind::MushroomSpore,   price_per: 2 },
    ShopEntry { item: ItemKind::AncientFragment, price_per: 4 },
    ShopEntry { item: ItemKind::DeepSeaPearl,    price_per: 5 },
    ShopEntry { item: ItemKind::WildflowerSeed,  price_per: 2 },
    ShopEntry { item: ItemKind::Wood,            price_per: 1 },
    ShopEntry { item: ItemKind::Stone,           price_per: 1 },
    ShopEntry { item: ItemKind::Ether,           price_per: 2 },
];

/// 翠幽星商人不販售（玩家需自行合成翠幽裝備；沒有販售清單）。
pub const VERDANT_SELL_LIST: &[ShopEntry] = &[];

/// 赤焰星商人在世界上的位置：赤焰星出生點附近偏右，讓玩家傳送後立刻看得到商人。
pub fn crimson_merchant_pos() -> (f32, f32) {
    (CRIMSON_SPAWN_X + 120.0, CRIMSON_SPAWN_Y)
}

/// 赤焰星商人互動範圍判定。
pub fn is_within_crimson_shop_reach(px: f32, py: f32) -> bool {
    let (mx, my) = crimson_merchant_pos();
    let dx = px - mx;
    let dy = py - my;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 赤焰星 NPC **收購**清單（玩家 → NPC，換乙太）。
/// 赤焰星商人以最高溢價收購熔晶碎片，鼓勵玩家深入赤焰星探索。
/// 也收購故鄉與翠幽星生態特產（給帶著舊存貨來赤焰星的玩家出路）。
pub const CRIMSON_BUY_LIST: &[ShopEntry] = &[
    ShopEntry { item: ItemKind::LavaCrystal,     price_per: 10 },
    ShopEntry { item: ItemKind::JadeShard,        price_per: 8 },
    ShopEntry { item: ItemKind::CrystalShard,     price_per: 3 },
    ShopEntry { item: ItemKind::AncientFragment,  price_per: 4 },
    ShopEntry { item: ItemKind::DeepSeaPearl,     price_per: 5 },
    ShopEntry { item: ItemKind::Wood,             price_per: 1 },
    ShopEntry { item: ItemKind::Stone,            price_per: 1 },
    ShopEntry { item: ItemKind::Ether,            price_per: 2 },
];

/// 赤焰星商人不販售（玩家需自行合成赤焰裝備；沒有販售清單）。
pub const CRIMSON_SELL_LIST: &[ShopEntry] = &[];

/// 虛空星商人在世界上的位置：虛空星出生點附近偏左，讓玩家傳送後立刻看得到商人。
pub fn void_merchant_pos() -> (f32, f32) {
    (VOID_SPAWN_X - 120.0, VOID_SPAWN_Y)
}

/// 虛空星商人互動範圍判定。
pub fn is_within_void_shop_reach(px: f32, py: f32) -> bool {
    let (mx, my) = void_merchant_pos();
    let dx = px - mx;
    let dy = py - my;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 虛空星 NPC **收購**清單（玩家 → NPC，換乙太）。
/// 虛空星商人以最高溢價收購虛空碎片，鼓勵玩家深入虛空星探索。
/// 也收購故鄉、翠幽星、赤焰星的素材（給帶著舊存貨來虛空星的玩家出路）。
pub const VOID_BUY_LIST: &[ShopEntry] = &[
    ShopEntry { item: ItemKind::VoidShard,      price_per: 12 },
    ShopEntry { item: ItemKind::LavaCrystal,    price_per: 10 },
    ShopEntry { item: ItemKind::JadeShard,      price_per: 8  },
    ShopEntry { item: ItemKind::CrystalShard,   price_per: 3  },
    ShopEntry { item: ItemKind::AncientFragment, price_per: 4 },
    ShopEntry { item: ItemKind::DeepSeaPearl,   price_per: 5  },
    ShopEntry { item: ItemKind::Wood,           price_per: 1  },
    ShopEntry { item: ItemKind::Stone,          price_per: 1  },
    ShopEntry { item: ItemKind::Ether,          price_per: 2  },
];

/// 虛空星商人不販售（玩家需自行合成虛空裝備；沒有販售清單）。
pub const VOID_SELL_LIST: &[ShopEntry] = &[];

/// 玩家向虛空星商人**賣出** qty 個 item。
/// 純函式，便於測試（caller 負責驗距離）。
pub fn sell_to_void_npc(inv: &mut Inventory, ether: u32, item: ItemKind, qty: u32) -> Option<u32> {
    if qty == 0 {
        return None;
    }
    let price = VOID_BUY_LIST.iter().find(|e| e.item == item)?.price_per;
    if !inv.take(item, qty) {
        return None;
    }
    let earned = price.saturating_mul(qty);
    Some(ether.saturating_add(earned))
}

/// 玩家向赤焰星商人**賣出** qty 個 item。
/// 純函式，便於測試（caller 負責驗距離）。
pub fn sell_to_crimson_npc(inv: &mut Inventory, ether: u32, item: ItemKind, qty: u32) -> Option<u32> {
    if qty == 0 {
        return None;
    }
    let price = CRIMSON_BUY_LIST.iter().find(|e| e.item == item)?.price_per;
    if !inv.take(item, qty) {
        return None;
    }
    let earned = price.saturating_mul(qty);
    Some(ether.saturating_add(earned))
}

/// 玩家向翠幽星商人**賣出** qty 個 item。
/// 純函式，便於測試（caller 負責驗距離）。
pub fn sell_to_verdant_npc(inv: &mut Inventory, ether: u32, item: ItemKind, qty: u32) -> Option<u32> {
    if qty == 0 {
        return None;
    }
    let price = VERDANT_BUY_LIST.iter().find(|e| e.item == item)?.price_per;
    if !inv.take(item, qty) {
        return None;
    }
    let earned = price.saturating_mul(qty);
    Some(ether.saturating_add(earned))
}

/// 霧醚星商人在世界上的位置：霧醚星出生點附近偏左，讓玩家傳送後立刻看得到商人。
pub fn aether_merchant_pos() -> (f32, f32) {
    (AETHER_SPAWN_X - 120.0, AETHER_SPAWN_Y)
}

/// 霧醚星商人互動範圍判定。
pub fn is_within_aether_shop_reach(px: f32, py: f32) -> bool {
    let (mx, my) = aether_merchant_pos();
    let dx = px - mx;
    let dy = py - my;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 霧醚星 NPC **收購**清單（玩家 → NPC，換乙太）。
/// 霧醚星商人以最高溢價收購霧醚碎片，鼓勵玩家深入霧醚星探索。
/// 也收購故鄉、翠幽星、赤焰星、虛空星的素材（給帶著舊存貨來霧醚星的玩家出路）。
pub const AETHER_BUY_LIST: &[ShopEntry] = &[
    ShopEntry { item: ItemKind::AetherShard,    price_per: 15 },
    ShopEntry { item: ItemKind::VoidShard,      price_per: 12 },
    ShopEntry { item: ItemKind::LavaCrystal,    price_per: 10 },
    ShopEntry { item: ItemKind::JadeShard,      price_per: 8  },
    ShopEntry { item: ItemKind::CrystalShard,   price_per: 3  },
    ShopEntry { item: ItemKind::AncientFragment, price_per: 4 },
    ShopEntry { item: ItemKind::DeepSeaPearl,   price_per: 5  },
    ShopEntry { item: ItemKind::Wood,           price_per: 1  },
    ShopEntry { item: ItemKind::Stone,          price_per: 1  },
    ShopEntry { item: ItemKind::Ether,          price_per: 2  },
];

/// 霧醚星商人不販售（玩家需自行合成霧醚裝備；沒有販售清單）。
pub const AETHER_SELL_LIST: &[ShopEntry] = &[];

/// 玩家向霧醚星商人**賣出** qty 個 item。
/// 純函式，便於測試（caller 負責驗距離）。
pub fn sell_to_aether_npc(inv: &mut Inventory, ether: u32, item: ItemKind, qty: u32) -> Option<u32> {
    if qty == 0 {
        return None;
    }
    let price = AETHER_BUY_LIST.iter().find(|e| e.item == item)?.price_per;
    if !inv.take(item, qty) {
        return None;
    }
    let earned = price.saturating_mul(qty);
    Some(ether.saturating_add(earned))
}

/// 星源星商人在世界上的位置：星源星出生點附近偏左，讓玩家傳送後立刻看得到商人。
pub fn origin_merchant_pos() -> (f32, f32) {
    (ORIGIN_SPAWN_X - 120.0, ORIGIN_SPAWN_Y)
}

/// 星源星商人互動範圍判定。
pub fn is_within_origin_shop_reach(px: f32, py: f32) -> bool {
    let (mx, my) = origin_merchant_pos();
    let dx = px - mx;
    let dy = py - my;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 星源星 NPC **收購**清單（玩家 → NPC，換乙太）。
/// 星源星商人以最高溢價收購源晶碎片，鼓勵玩家深入星源星探索。
/// 也收購故鄉、翠幽星、赤焰星、虛空星、霧醚星的素材（五星旅者的終點站）。
pub const ORIGIN_BUY_LIST: &[ShopEntry] = &[
    ShopEntry { item: ItemKind::OriginShard,     price_per: 18 },
    ShopEntry { item: ItemKind::AetherShard,     price_per: 15 },
    ShopEntry { item: ItemKind::VoidShard,       price_per: 12 },
    ShopEntry { item: ItemKind::LavaCrystal,     price_per: 10 },
    ShopEntry { item: ItemKind::JadeShard,       price_per: 8  },
    ShopEntry { item: ItemKind::CrystalShard,    price_per: 3  },
    ShopEntry { item: ItemKind::AncientFragment, price_per: 4  },
    ShopEntry { item: ItemKind::DeepSeaPearl,    price_per: 5  },
    ShopEntry { item: ItemKind::Wood,            price_per: 1  },
    ShopEntry { item: ItemKind::Stone,           price_per: 1  },
    ShopEntry { item: ItemKind::Ether,           price_per: 2  },
];

/// 星源星商人不販售（玩家需自行合成源晶裝備；沒有販售清單）。
pub const ORIGIN_SELL_LIST: &[ShopEntry] = &[];

/// 玩家向星源星商人**賣出** qty 個 item。
/// 純函式，便於測試（caller 負責驗距離）。
pub fn sell_to_origin_npc(inv: &mut Inventory, ether: u32, item: ItemKind, qty: u32) -> Option<u32> {
    if qty == 0 {
        return None;
    }
    let price = ORIGIN_BUY_LIST.iter().find(|e| e.item == item)?.price_per;
    if !inv.take(item, qty) {
        return None;
    }
    let earned = price.saturating_mul(qty);
    Some(ether.saturating_add(earned))
}

/// 玩家是否在商人互動範圍內。純函式，便於測試。
pub fn is_within_shop_reach(px: f32, py: f32) -> bool {
    let (mx, my) = merchant_pos();
    let dx = px - mx;
    let dy = py - my;
    dx * dx + dy * dy <= SHOP_REACH * SHOP_REACH
}

/// 玩家向 NPC **賣出** qty 個 item：
/// 從背包扣除物品，回傳交易後的新乙太量；若物品不在收購清單或背包不足回 None。
/// 純函式，便於測試（caller 負責驗距離與 downed 狀態）。
pub fn sell_to_npc(inv: &mut Inventory, ether: u32, item: ItemKind, qty: u32) -> Option<u32> {
    if qty == 0 {
        return None;
    }
    let price = NPC_BUY_LIST.iter().find(|e| e.item == item)?.price_per;
    if !inv.take(item, qty) {
        return None; // 背包不足
    }
    let earned = price.saturating_mul(qty);
    Some(ether.saturating_add(earned))
}

/// 玩家向 NPC **購買** qty 個 item：
/// 扣乙太並把物品加進背包，回傳交易後的新乙太量；
/// 若物品不在販售清單、乙太不足或背包滿則回 None。
/// 純函式，便於測試（caller 負責驗距離與 downed 狀態）。
pub fn buy_from_npc(inv: &mut Inventory, ether: u32, item: ItemKind, qty: u32) -> Option<u32> {
    if qty == 0 {
        return None;
    }
    let price = NPC_SELL_LIST.iter().find(|e| e.item == item)?.price_per;
    let total = price.saturating_mul(qty);
    if ether < total {
        return None; // 乙太不足
    }
    let added = inv.add(item, qty);
    if added == 0 {
        return None; // 背包滿
    }
    Some(ether - total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv_with(item: ItemKind, qty: u32) -> Inventory {
        let mut inv = Inventory::new();
        inv.add(item, qty);
        inv
    }

    #[test]
    fn merchant_pos_is_in_safe_zone() {
        let (mx, my) = merchant_pos();
        assert!(crate::positions::is_in_safe_zone(mx, my),
            "商人應在新手村安全區內，pos=({mx},{my})");
    }

    #[test]
    fn merchant_pos_is_near_pub_field() {
        let (mx, my) = merchant_pos();
        // 商人應在公共農地水平距離 200 像素內（視覺上看得到）
        let dx = (mx - PUB_FIELD_ORIGIN_X).abs();
        assert!(dx < 200.0, "商人應靠近公共農地 x 邊，dx={dx}");
        let _ = my; // y 軸靈活
    }

    #[test]
    fn within_reach_at_merchant_pos() {
        let (mx, my) = merchant_pos();
        assert!(is_within_shop_reach(mx, my));
    }

    #[test]
    fn out_of_reach_far_away() {
        assert!(!is_within_shop_reach(0.0, 0.0));
    }

    #[test]
    fn sell_wood_gives_ether() {
        let mut inv = inv_with(ItemKind::Wood, 5);
        let new_ether = sell_to_npc(&mut inv, 0, ItemKind::Wood, 3);
        assert_eq!(new_ether, Some(3)); // 1 乙太/個 × 3 = 3
        assert_eq!(inv.count(ItemKind::Wood), 2); // 背包剩 2
    }

    #[test]
    fn sell_fails_if_not_enough_in_inventory() {
        let mut inv = inv_with(ItemKind::Wood, 1);
        let result = sell_to_npc(&mut inv, 10, ItemKind::Wood, 5);
        assert!(result.is_none());
        assert_eq!(inv.count(ItemKind::Wood), 1); // 背包不變
    }

    #[test]
    fn sell_unlisted_item_fails() {
        let mut inv = inv_with(ItemKind::Pickaxe, 3);
        // NPC 不收鎬子
        let result = sell_to_npc(&mut inv, 10, ItemKind::Pickaxe, 1);
        assert!(result.is_none());
        assert_eq!(inv.count(ItemKind::Pickaxe), 3); // 背包不變
    }

    #[test]
    fn sell_qty_zero_fails() {
        let mut inv = inv_with(ItemKind::Wood, 5);
        assert!(sell_to_npc(&mut inv, 10, ItemKind::Wood, 0).is_none());
    }

    #[test]
    fn buy_pickaxe_succeeds_with_enough_ether() {
        let mut inv = Inventory::new();
        let new_ether = buy_from_npc(&mut inv, 20, ItemKind::Pickaxe, 1);
        assert_eq!(new_ether, Some(5)); // 20 - 15 = 5
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn buy_fails_if_not_enough_ether() {
        let mut inv = Inventory::new();
        let result = buy_from_npc(&mut inv, 10, ItemKind::Pickaxe, 1); // 需要 15
        assert!(result.is_none());
        assert!(inv.is_empty());
    }

    #[test]
    fn buy_unlisted_item_fails() {
        let mut inv = Inventory::new();
        // NPC 不賣木材
        let result = buy_from_npc(&mut inv, 100, ItemKind::Wood, 1);
        assert!(result.is_none());
        assert!(inv.is_empty());
    }

    #[test]
    fn buy_qty_zero_fails() {
        let mut inv = Inventory::new();
        assert!(buy_from_npc(&mut inv, 100, ItemKind::Pickaxe, 0).is_none());
    }

    #[test]
    fn ether_item_has_npc_buy_price() {
        // ItemKind::Ether（採礦所得）可以賣給 NPC，確保有非貨幣去處（補 every_item_has_a_sink）
        let mut inv = inv_with(ItemKind::Ether, 3);
        let result = sell_to_npc(&mut inv, 0, ItemKind::Ether, 2);
        assert_eq!(result, Some(4)); // 2 乙太/個 × 2 = 4
    }

    #[test]
    fn npc_buy_list_covers_important_items() {
        // 採集物與重要物資（木／石／乙太）都在收購清單裡——不會有辛勤勞動卻一毛不值的情況。
        let important_items = [
            ItemKind::Wood, ItemKind::Stone, ItemKind::Ether,
            ItemKind::Dirt, ItemKind::CrystalShard, ItemKind::MushroomSpore,
            ItemKind::AncientFragment, ItemKind::DeepSeaPearl, ItemKind::WildflowerSeed,
        ];
        for item in important_items {
            assert!(
                NPC_BUY_LIST.iter().any(|e| e.item == item),
                "物資 {item:?} 不在 NPC 收購清單，玩家賣不出去"
            );
        }
    }

    #[test]
    fn crystal_shard_has_premium_price() {
        // 晶石碎片應比普通礦石（乙太 2 乙太/個）更高價，體現探索溢價。
        let crystal_entry = NPC_BUY_LIST.iter().find(|e| e.item == ItemKind::CrystalShard);
        assert!(crystal_entry.is_some(), "晶石碎片應在收購清單");
        let ether_entry = NPC_BUY_LIST.iter().find(|e| e.item == ItemKind::Ether);
        assert!(crystal_entry.unwrap().price_per > ether_entry.unwrap().price_per,
            "晶石碎片應比乙太礦石更值錢");
    }

    #[test]
    fn mushroom_spore_has_premium_price() {
        // 蕈菇孢子溢價應高於木材（1 乙太/個），體現「探索森林有額外回報」。
        let spore_entry = NPC_BUY_LIST.iter().find(|e| e.item == ItemKind::MushroomSpore);
        assert!(spore_entry.is_some(), "蕈菇孢子應在收購清單");
        let wood_entry = NPC_BUY_LIST.iter().find(|e| e.item == ItemKind::Wood);
        assert!(spore_entry.unwrap().price_per > wood_entry.unwrap().price_per,
            "蕈菇孢子應比木材更值錢");
    }

    #[test]
    fn deep_sea_pearl_has_highest_premium_price() {
        // 深海珍珠應是所有生態特產中收購價最高，反映水域探索的珍稀性。
        let pearl_entry = NPC_BUY_LIST.iter().find(|e| e.item == ItemKind::DeepSeaPearl);
        assert!(pearl_entry.is_some(), "深海珍珠應在收購清單");
        let ancient_entry = NPC_BUY_LIST.iter().find(|e| e.item == ItemKind::AncientFragment);
        assert!(pearl_entry.unwrap().price_per > ancient_entry.unwrap().price_per,
            "深海珍珠應比古代碎片更值錢（最高生態特產溢價）");
    }

    #[test]
    fn npc_sell_list_prices_exceed_craft_incentive() {
        // NPC 賣出工具的價格應高於合成成本（鼓勵自己合成而非直接花錢買）。
        // 鎬子合成：木×3 (3乙太) + 石×2 (2乙太) = 5乙太；NPC 賣 15 乙太，比合成貴 3×，合理。
        let pickaxe = NPC_SELL_LIST.iter().find(|e| e.item == ItemKind::Pickaxe);
        assert!(pickaxe.is_some());
        assert!(pickaxe.unwrap().price_per > 5, "鎬子售價應高於自行採集合成的機會成本");
    }
}
