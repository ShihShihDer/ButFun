//! 背包模型（Phase 1-B 背包系統的純邏輯地基）。
//!
//! 這層只管「玩家身上有哪些資源、各有多少」，是純資料 + 純函式，無 IO、不碰
//! WebSocket / 遊戲迴圈，便於自動測試。延續 `gather.rs` / `crops.rs` / `field.rs` /
//! `daynight.rs` 的前置慣例：純邏輯先落地、標 `allow(dead_code)`，接線輪（採集產出
//! 加進背包、ws 廣播背包內容、前端按 I 開面板）才有呼叫端。
//!
//! 之後接上：
//!   - ws：採集（`gather_near` 回 `(NodeKind, 產出量)`）→ `add(item, qty)` 加進背包。
//!   - 快照 / 前端：把背包內容隨快照給該玩家、按 I 開面板顯示。
//!   - 合成（Phase 1-C）：`take(item, qty)` 扣材料，不夠就不給合（回 `false` 不動）。
//!   - 持久化（接 0-E）：序列化整個背包（載入時走 `is_loadable` 驗證）。
//!
//! 刻意把資源種類抽成 `ItemKind` enum（而非散落的字串 id）：採集節點 `NodeKind`
//! 直接 `into()` 對應的物品，型別系統擋掉拼錯的 item id；日後工具 / 合成產物（鎬子…）
//! 只要在這個 enum 加一個變體即可，背包容器本身不用動。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::gather::NodeKind;

/// 單一物品的堆疊上限：每種資源最多累積到這個數，避免無界成長 / 整數溢位，
/// 也讓「背包滿了採不進」這種手感日後接得上。
pub const MAX_STACK: u32 = 9_999;

/// 背包裡的物品種類。目前只有採集三資源；工具 / 合成產物日後加變體即可。
///
/// 用 `BTreeMap` 當容器故需 `Ord`：序列化 / 顯示順序因此確定（不靠插入順序），
/// 重啟前後、跨玩家都一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// 木材（採樹得）。
    Wood,
    /// 礦石（採石得）。
    Stone,
    /// 乙太（採乙太礦得；療癒種田之外的另一條乙太來源）。
    Ether,
    /// 鎬子（合成產物，Phase 1-C／1-D）：背包裡的第一件工具，身上有它日後採礦更快。
    /// 工具也是背包物品（沿用同一個容器），故只在此 enum 加一個變體即可——背包、序列化、
    /// 前端面板都不必為「工具」另開資料結構。放在採集三資源之後，既有 `entries` 排序不動。
    Pickaxe,
}

impl ItemKind {
    /// 全部物品種類（測試用單一真實來源，供跨模組不變式遍歷整個物品宇宙）。
    /// 仿照 `gather.rs` 測試的 `KINDS` 陣列：只在測試建置存在，不增生產面 / dead_code。
    /// 加新變體時，`item_kind_all_lists_every_variant` 的窮舉 match 會編譯失敗、`len`
    /// 斷言會紅燈，逼人同步更新此清單——確保 ALL 與 enum 不漂移。
    #[cfg(test)]
    pub(crate) const ALL: &'static [ItemKind] = &[
        ItemKind::Wood,
        ItemKind::Stone,
        ItemKind::Ether,
        ItemKind::Pickaxe,
    ];
}

/// 採集節點種類 → 對應的背包物品。把「採到什麼」與「背包存什麼」綁在型別層，
/// 接線時 `gather_near` 的產出種類直接 `.into()`，不會對錯資源。
impl From<NodeKind> for ItemKind {
    fn from(kind: NodeKind) -> Self {
        match kind {
            NodeKind::Tree => ItemKind::Wood,
            NodeKind::Rock => ItemKind::Stone,
            NodeKind::EtherOre => ItemKind::Ether,
        }
    }
}

/// 一個玩家的背包：物品種類 → 數量。
///
/// 不變式：map 裡只存「數量 > 0」的條目——數量歸零的物品會被移除，使
/// 「背包是否有某物」永遠等同「key 是否存在」，序列化也不留 0 垃圾條目。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    items: BTreeMap<ItemKind, u32>,
}

// 整個模組是前置地基：接線輪（採集進背包、ws 廣播、合成扣料）才有呼叫端，
// 在此之前公開項目皆無外部呼叫，比照 `gather.rs` / `plots.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
impl Inventory {
    /// 空背包。
    pub fn new() -> Self {
        Self::default()
    }

    /// 加 `qty` 個 `item`，夾在 `MAX_STACK` 上限內；回傳**實際加入的量**
    /// （已滿時可能少於 `qty`，接線時可據此回饋「背包滿了」）。`qty == 0` 為 no-op。
    pub fn add(&mut self, item: ItemKind, qty: u32) -> u32 {
        if qty == 0 {
            return 0;
        }
        let slot = self.items.entry(item).or_insert(0);
        let before = *slot;
        // 飽和加再夾上限，避免 u32 溢位。
        let after = before.saturating_add(qty).min(MAX_STACK);
        *slot = after;
        after - before
    }

    /// 扣 `qty` 個 `item`：**夠才扣**並回 `true`；不夠（或 `item` 不存在）回 `false`
    /// 且完全不改變背包（合成「材料不足不給合」要的全有全無語意）。`qty == 0` 視為
    /// 恆成功的 no-op。扣到 0 的條目會被移除以維持「只存 > 0」不變式。
    pub fn take(&mut self, item: ItemKind, qty: u32) -> bool {
        if qty == 0 {
            return true;
        }
        match self.items.get_mut(&item) {
            Some(have) if *have >= qty => {
                *have -= qty;
                if *have == 0 {
                    self.items.remove(&item);
                }
                true
            }
            _ => false,
        }
    }

    /// 是否擁有至少 `qty` 個 `item`（不改變背包，供合成預先檢查 / UI 反灰）。
    pub fn has(&self, item: ItemKind, qty: u32) -> bool {
        self.count(item) >= qty
    }

    /// 某物品的數量（沒有就是 0）。
    pub fn count(&self, item: ItemKind) -> u32 {
        self.items.get(&item).copied().unwrap_or(0)
    }

    /// 背包是否空（沒有任何數量 > 0 的物品）。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 依物品種類排序逐一列出 `(物品, 數量)`（供前端面板顯示 / 快照）。
    /// 因不變式只存 > 0 條目，這裡每筆數量都 > 0。
    pub fn entries(&self) -> impl Iterator<Item = (ItemKind, u32)> + '_ {
        self.items.iter().map(|(&k, &v)| (k, v))
    }

    /// 從存檔載入的背包是否「健全」：沒有數量為 0 的垃圾條目、且每筆不超過堆疊上限。
    /// 與調校常數無關的最小不變式——正常流程（`add` 夾上限、`take` 歸零即移除）絕不會
    /// 產生 0 條目或界外數量，所以這些只會來自壞檔或被竄改的存檔。`u32` 型別本身擋掉
    /// `NaN`/負值，故只需驗「非 0」與上界。延續 `gather::is_loadable` /
    /// `crops::is_loadable` 的載入時驗證脈絡；接 0-E 載入路徑時連同 `allow(dead_code)` 移除。
    pub fn is_loadable(&self) -> bool {
        self.items.values().all(|&n| n > 0 && n <= MAX_STACK)
    }

    /// 測試用：直接組出指定內容（含壞值）的背包，驗證載入防線。
    #[cfg(test)]
    pub fn from_raw(items: BTreeMap<ItemKind, u32>) -> Self {
        Self { items }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_inventory_is_empty() {
        let inv = Inventory::new();
        assert!(inv.is_empty());
        assert_eq!(inv.count(ItemKind::Wood), 0);
    }

    #[test]
    fn add_accumulates_and_reports_added_amount() {
        let mut inv = Inventory::new();
        assert_eq!(inv.add(ItemKind::Wood, 3), 3);
        assert_eq!(inv.add(ItemKind::Wood, 2), 2);
        assert_eq!(inv.count(ItemKind::Wood), 5);
        assert!(!inv.is_empty());
    }

    #[test]
    fn add_zero_is_noop() {
        let mut inv = Inventory::new();
        assert_eq!(inv.add(ItemKind::Stone, 0), 0);
        assert!(inv.is_empty());
    }

    #[test]
    fn add_caps_at_max_stack_and_reports_actual() {
        let mut inv = Inventory::new();
        assert_eq!(inv.add(ItemKind::Ether, MAX_STACK - 1), MAX_STACK - 1);
        // 只能再加 1 就滿，多送的被夾掉、回報實際加入量。
        assert_eq!(inv.add(ItemKind::Ether, 10), 1);
        assert_eq!(inv.count(ItemKind::Ether), MAX_STACK);
        // 滿了之後再加，實際加入 0。
        assert_eq!(inv.add(ItemKind::Ether, 5), 0);
    }

    #[test]
    fn take_succeeds_when_enough_and_decrements() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 5);
        assert!(inv.take(ItemKind::Wood, 3));
        assert_eq!(inv.count(ItemKind::Wood), 2);
    }

    #[test]
    fn take_fails_and_is_unchanged_when_insufficient() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Stone, 2);
        assert!(!inv.take(ItemKind::Stone, 3));
        // 失敗不動背包。
        assert_eq!(inv.count(ItemKind::Stone), 2);
        // 完全沒有的物品也是失敗、不動。
        assert!(!inv.take(ItemKind::Wood, 1));
    }

    #[test]
    fn take_to_zero_removes_entry() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Ether, 2);
        assert!(inv.take(ItemKind::Ether, 2));
        assert_eq!(inv.count(ItemKind::Ether), 0);
        // 歸零後條目移除，背包回空。
        assert!(inv.is_empty());
    }

    #[test]
    fn take_zero_is_noop_success() {
        let mut inv = Inventory::new();
        assert!(inv.take(ItemKind::Wood, 0));
        assert!(inv.is_empty());
    }

    #[test]
    fn has_reflects_count() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        assert!(inv.has(ItemKind::Wood, 3));
        assert!(!inv.has(ItemKind::Wood, 4));
        assert!(inv.has(ItemKind::Wood, 0)); // 任何背包都「有 0 個」
    }

    #[test]
    fn item_kind_all_lists_every_variant() {
        // ALL 是跨模組不變式（如 tools.rs「每個工具都拿得到」）遍歷物品宇宙的依據，
        // 必須涵蓋 enum 全部變體、且不重複。窮舉 match 是強制同步的核心：日後在
        // `ItemKind` 加變體（如鋤頭）時，下面的 match 會因不窮舉而**編譯失敗**，逼人回來
        // 把新變體加進這個 match（連帶提醒加進 ALL）；`len` 斷言則擋住「加了 enum 卻忘了
        // 加進 ALL」——少一筆 len 不等於變體數即紅燈。
        for &k in ItemKind::ALL {
            match k {
                ItemKind::Wood | ItemKind::Stone | ItemKind::Ether | ItemKind::Pickaxe => {}
            }
        }
        let unique: std::collections::BTreeSet<_> = ItemKind::ALL.iter().collect();
        assert_eq!(unique.len(), ItemKind::ALL.len(), "ItemKind::ALL 有重複條目");
        // 目前共 4 種（木／石／乙太／鎬子）；加變體時連同上面的 match 一起更新。
        assert_eq!(ItemKind::ALL.len(), 4, "ItemKind::ALL 筆數與變體數不一致");
    }

    #[test]
    fn node_kind_maps_to_item_kind() {
        assert_eq!(ItemKind::from(NodeKind::Tree), ItemKind::Wood);
        assert_eq!(ItemKind::from(NodeKind::Rock), ItemKind::Stone);
        assert_eq!(ItemKind::from(NodeKind::EtherOre), ItemKind::Ether);
    }

    #[test]
    fn gather_yield_flows_into_inventory_via_into() {
        // 模擬接線：採到 (種類, 量) 直接灌進背包。
        let mut inv = Inventory::new();
        let (kind, qty) = (NodeKind::EtherOre, 2u32);
        inv.add(kind.into(), qty);
        assert_eq!(inv.count(ItemKind::Ether), 2);
    }

    #[test]
    fn entries_are_sorted_and_nonzero() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Ether, 1);
        inv.add(ItemKind::Wood, 4);
        inv.add(ItemKind::Stone, 2);
        let collected: Vec<_> = inv.entries().collect();
        // BTreeMap 依 enum 宣告順序：Wood < Stone < Ether。
        assert_eq!(
            collected,
            vec![
                (ItemKind::Wood, 4),
                (ItemKind::Stone, 2),
                (ItemKind::Ether, 1),
            ]
        );
        assert!(collected.iter().all(|&(_, n)| n > 0));
    }

    #[test]
    fn is_loadable_accepts_normal_and_rejects_corrupt() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 5);
        assert!(inv.is_loadable());
        assert!(Inventory::new().is_loadable()); // 空背包也健全

        // 壞值：數量為 0 的垃圾條目。
        let mut zero = BTreeMap::new();
        zero.insert(ItemKind::Stone, 0);
        assert!(!Inventory::from_raw(zero).is_loadable());

        // 壞值：超過堆疊上限。
        let mut over = BTreeMap::new();
        over.insert(ItemKind::Ether, MAX_STACK + 1);
        assert!(!Inventory::from_raw(over).is_loadable());
    }

    #[test]
    fn serde_round_trip_preserves_contents() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 7);
        inv.add(ItemKind::Ether, 3);
        let json = serde_json::to_string(&inv).unwrap();
        let back: Inventory = serde_json::from_str(&json).unwrap();
        assert_eq!(inv, back);
    }
}
