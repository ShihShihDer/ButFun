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
    /// 土磚（C-2 挖土地形格掉落；C-4 放置材料）。放在 Wood 之後，既有排序不動。
    Dirt,
    /// 礦石（採石得）。
    Stone,
    /// 乙太（採乙太礦得；療癒種田之外的另一條乙太來源）。
    Ether,
    /// 鎬子（合成產物，Phase 1-C／1-D）：背包裡的第一件工具，身上有它日後採礦更快。
    /// 工具也是背包物品（沿用同一個容器），故只在此 enum 加一個變體即可——背包、序列化、
    /// 前端面板都不必為「工具」另開資料結構。放在採集三資源之後，既有 `entries` 排序不動。
    Pickaxe,
    /// 強化鎬（升級工具配方產物，Phase 1-C／1-D）：以一把鎬子＋素材升級而成，採礦比
    /// 普通鎬子更快。它是「工具＋素材→升級工具」配方鏈的第一個產物——既給已合成的鎬子
    /// 一個新去處（被升級配方消耗），也讓玩家攢素材有第二層進程目標。放在 `Pickaxe` 之後，
    /// 既有 `entries` 排序不動。
    ReinforcedPickaxe,
    /// 武器（合成產物，Phase 1 武器 MVP）：背包裡的第一件戰鬥裝備。鎬子之於採集，武器之於
    /// 戰鬥——身上有它打怪每下更痛（`combat::weapon_power` 查表，鏡像 `tools` 的採集倍率），
    /// 沒有就維持徒手攻擊力。它給合成素材（礦石／乙太）開出「變強打怪」這條新去處，閉合
    /// 「採集→合成→戰鬥變強」正回饋圈。工具也好、武器也好都只是背包物品（沿用同一容器），
    /// 故只在此 enum 加一個變體即可。放在工具之後，既有 `entries` 排序不動。
    Weapon,
    /// 晶石碎片（挖掘 Crystal 地形格掉落，ROADMAP 10 深層晶洞生態域）。
    /// 可賣給 NPC 換取高額乙太（premium 素材），給探索型玩家一條「深挖有額外回報」的路線。
    CrystalShard,
    /// 蕈菇孢子（挖掘 Mushroom 地形格掉落，ROADMAP 11 森林蕈菇洞生態域）。
    /// 散發神秘異星氣息，NPC 溢價收購，給深入森林的玩家一條新乙太路線。
    MushroomSpore,
    /// 古代碎片（挖掘 AncientRuin 地形格掉落，ROADMAP 12 沙漠遺跡生態域）。
    /// 殘留古老文明的銘刻，NPC 以高溢價收購，給探索沙漠的玩家第三條乙太路線。
    AncientFragment,
    /// 深海珍珠（挖掘 CoralReef 地形格掉落，ROADMAP 13 水域珊瑚礁生態域）。
    /// 海底珊瑚礁孕育的稀世珍珠，NPC 以最高溢價收購，給在海岸邊挖珊瑚的玩家第四條乙太路線。
    DeepSeaPearl,
    /// 野花種子（挖掘 WildFlower 地形格掉落，ROADMAP 14 草原野花叢生態域）。
    /// 草原深處野花叢孕育的種子，NPC 溢價收購，給穿梭草原的玩家補上第五條乙太路線。
    WildflowerSeed,
    /// 活力藥水（合成產物：野花種子×3 → 活力藥水×1）。
    /// 使用後立即回復 6 HP——讓生態資源從「只賣 NPC」多一條「自用保命」的路線，
    /// 閉合「探索草原 → 採野花種子 → 合成藥水 → 戰鬥續航」正回饋圈。
    HealingPotion,
    /// 晶石強化液（合成產物：晶石碎片×5 → 晶石強化液×1）。
    /// 使用後立即回復 12 HP——是活力藥水的兩倍，Premium 探索深層晶洞的回報。
    CrystalPotion,
    /// 蕈菇活化液（合成產物：蕈菇孢子×4 → 蕈菇活化液×1）。
    /// 使用後回復 8 HP 並重置自然回血冷卻，讓回血立刻開始。森林菌絲的生命力。
    MushroomElixir,
    /// 古代乙太丸（合成產物：古代碎片×3 → 古代乙太丸×1）。
    /// 使用後直接獲得 10 乙太——讓沙漠探索者在野外就能轉換遺跡能量，省去回城賣材料的腳程。
    EtherPill,
    /// 珍珠復原藥（合成產物：深海珍珠×1 → 珍珠復原藥×1）。
    /// 使用後回復到滿血——深海孕育的頂級珍珠，完整恢復生命力，最稀有材料換來最強效果。
    PearlPotion,
    /// 晶石之刃（合成產物：晶石碎片×6 → 晶石之刃×1）。
    /// 持有此武器攻擊力 +8，強過基礎武器（+5），Deep Rocky 探索的進階武器。
    CrystalBlade,
    /// 珊瑚矛（合成產物：深海珍珠×3 → 珊瑚矛×1）。
    /// 持有此武器攻擊力 +12，全遊戲最高傷害，最稀有材料換來最強攻擊。
    CoralLance,
    /// 草原護符（合成產物：野花種子×8 → 草原護符×1）。
    /// 持有此護符每次受傷減 1 點傷害——讓採集草原的玩家有防禦出路。
    MeadowAmulet,
    /// 晶石護盾（合成產物：晶石碎片×8 + 石頭×4 → 晶石護盾×1）。
    /// 持有此護盾每次受傷減 2 點傷害——更稀有的材料換更高防禦。
    CrystalShield,
    /// 星圖（合成產物：古代碎片×5 → 星圖×1）。
    /// 使用後展示遠方星球的星圖快照，是多星球旅程的序章。
    StarChart,
    /// 蕈菇杖（合成產物：蕈菇孢子×6 → 蕈菇杖×1）。
    /// 持有此魔杖攻擊力 +7，森林生態域探索的秘密武器，菌絲能量凝聚而成。
    /// 填補森林生態的武器空缺——對稱岩地晶石之刃（+8）的前一階。
    MushroomStaff,
    /// 符文刃（合成產物：古代碎片×4 → 符文刃×1）。
    /// 持有此刃攻擊力 +10，古代沙漠文明的符文鍛造技術，強過晶石之刃（+8）。
    /// 填補沙漠生態的武器空缺——對稱珊瑚矛（+12）的前一階，沙漠探索者的精英裝備。
    RuneBlade,
    /// 翠幽碎片（挖掘 JadeVine 地形格掉落，或擊倒翠幽魅影；ROADMAP 21 翠幽星特產）。
    /// 翠幽星獨有，NPC 以最高溢價收購；也是翠幽刃 / 翠幽精露的合成原料。
    JadeShard,
    /// 翠幽精露（合成產物：翠幽碎片×2 → 翠幽精露×1）。
    /// 使用後回復至等級滿血並立即重置自然回血冷卻——翠幽星異星植物凝聚的頂級精華。
    JadeElixir,
    /// 翠幽刃（合成產物：翠幽碎片×5 → 翠幽刃×1）。
    /// 持有此刃攻擊力 +15，翠幽星域最強武器，超越故鄉最強的珊瑚矛（+12），
    /// 只有踏上翠幽星才能鑄造，象徵星際探索的最高獎賞。
    JadeBlade,
    /// 熔晶碎片（挖掘 LavaRock 地形格掉落，或擊倒蒸汽構裝；ROADMAP 22 赤焰星特產）。
    /// 赤焰星獨有，NPC 以最高溢價收購；也是赤焰刃 / 蒸汽精粹的合成原料。
    LavaCrystal,
    /// 蒸汽精粹（合成產物：熔晶碎片×2 → 蒸汽精粹×1）。
    /// 使用後回復至等級滿血，同時獲得 8 乙太（蒸汽燃料轉換器，異星能量兌換乙太）。
    SteamElixir,
    /// 赤焰刃（合成產物：熔晶碎片×6 → 赤焰刃×1）。
    /// 持有此刃攻擊力 +20，赤焰星域頂級武器，超越翠幽刃（+15），
    /// 只有踏上赤焰星才能鑄造，蒸汽龐克文明的最高武裝結晶。
    CrimsonBlade,
    /// 虛空碎片（挖掘 VoidCrystal 地形格掉落，或擊倒虛空幽靈；ROADMAP 23 虛空星特產）。
    /// 虛空星獨有，NPC 以最高溢價收購；也是虛空精粹 / 虛空刃的合成原料。
    VoidShard,
    /// 虛空精粹（合成產物：虛空碎片×2 → 虛空精粹×1）。
    /// 使用後回復至等級滿血，同時獲得 10 乙太（宇宙深淵能量轉換，雙效加成更強）。
    VoidElixir,
    /// 虛空刃（合成產物：虛空碎片×6 → 虛空刃×1）。
    /// 持有此刃攻擊力 +25，全遊戲目前最強武器，超越赤焰刃（+20），
    /// 只有踏上虛空星才能鑄造，宇宙深淵能量凝聚的高階武裝。
    VoidBlade,
    /// 霧醚碎片（挖掘 AetherMist 地形格掉落，或擊倒霧醚幻靈；ROADMAP 24 霧醚星特產）。
    /// 霧醚星獨有，NPC 以最高溢價收購；也是霧醚精粹 / 霧醚之刃的合成原料。
    AetherShard,
    /// 霧醚精粹（合成產物：霧醚碎片×2 → 霧醚精粹×1）。
    /// 使用後回復至等級滿血，同時獲得 15 乙太（乙太迷霧高密度轉換，四星最強補給）。
    AetherEssence,
    /// 霧醚之刃（合成產物：霧醚碎片×8 → 霧醚之刃×1）。
    /// 持有此刃攻擊力 +30，全遊戲最強武器，超越虛空刃（+25），
    /// 只有踏上霧醚星才能鑄造，乙太迷霧凝結的終極宇宙武裝。
    AetherBlade,
    /// 源晶碎片（挖掘 OriginCrystal 地形格掉落，或擊倒源晶守護者；ROADMAP 25 星源星特產）。
    /// 星源星獨有，NPC 以最高溢價收購；也是源晶精粹 / 源晶之刃的合成原料。
    OriginShard,
    /// 源晶精粹（合成產物：源晶碎片×2 → 源晶精粹×1）。
    /// 使用後回復至等級滿血，同時獲得 20 乙太（宇宙源頭能量轉換，五星最強補給）。
    OriginEssence,
    /// 源晶之刃（合成產物：源晶碎片×10 → 源晶之刃×1）。
    /// 持有此刃攻擊力 +40，全遊戲最強武器，超越霧醚之刃（+30），
    /// 只有踏上星源星才能鑄造，宇宙起源之力凝聚的終極武裝。
    OriginBlade,
    /// 裂縫碎片（擊倒裂縫守護者掉落，ROADMAP 26 宇宙裂縫動態世界事件）。
    /// 宇宙裂縫開啟時的限定掉落，NPC 以 12 乙太收購；也是宇宙護盾的合成原料。
    RiftShard,
    /// 宇宙護盾（合成產物：裂縫碎片×3 → 宇宙護盾×1）。
    /// 持有此護盾每次受傷減 6 點傷害——全遊戲最強防禦裝備，宇宙裂縫能量鍛造。
    CosmicShield,
    /// 合金護盾（工作台配方：礦石×8 + 晶石碎片×4 → 合金護盾×1，ROADMAP 36）。
    /// 持有後每次受傷減 4 點傷害——介於晶石護盾(2)與宇宙護盾(6)之間，工作台特有的中階防具。
    AlloyShield,
    /// 工坊精粹（工作台配方：活力藥水×2 + 蕈菇活化液×1 + 晶石碎片×2 → 工坊精粹×1，ROADMAP 36）。
    /// 使用後回復至等級滿血，同時重置回血冷卻並獲得 12 乙太——工匠將多種材料凝煉的全效補給。
    WorkshopElixir,
}

impl ItemKind {
    /// 全部物品種類（測試用單一真實來源，供跨模組不變式遍歷整個物品宇宙）。
    /// 仿照 `gather.rs` 測試的 `KINDS` 陣列：只在測試建置存在，不增生產面 / dead_code。
    /// 加新變體時，`item_kind_all_lists_every_variant` 的窮舉 match 會編譯失敗、`len`
    /// 斷言會紅燈，逼人同步更新此清單——確保 ALL 與 enum 不漂移。
    #[cfg(test)]
    pub(crate) const ALL: &'static [ItemKind] = &[
        ItemKind::Wood,
        ItemKind::Dirt,
        ItemKind::Stone,
        ItemKind::Ether,
        ItemKind::Pickaxe,
        ItemKind::ReinforcedPickaxe,
        ItemKind::Weapon,
        ItemKind::CrystalShard,
        ItemKind::MushroomSpore,
        ItemKind::AncientFragment,
        ItemKind::DeepSeaPearl,
        ItemKind::WildflowerSeed,
        ItemKind::HealingPotion,
        ItemKind::CrystalPotion,
        ItemKind::MushroomElixir,
        ItemKind::EtherPill,
        ItemKind::PearlPotion,
        ItemKind::CrystalBlade,
        ItemKind::CoralLance,
        ItemKind::MeadowAmulet,
        ItemKind::CrystalShield,
        ItemKind::StarChart,
        ItemKind::MushroomStaff,
        ItemKind::RuneBlade,
        ItemKind::JadeShard,
        ItemKind::JadeElixir,
        ItemKind::JadeBlade,
        ItemKind::LavaCrystal,
        ItemKind::SteamElixir,
        ItemKind::CrimsonBlade,
        ItemKind::VoidShard,
        ItemKind::VoidElixir,
        ItemKind::VoidBlade,
        ItemKind::AetherShard,
        ItemKind::AetherEssence,
        ItemKind::AetherBlade,
        ItemKind::OriginShard,
        ItemKind::OriginEssence,
        ItemKind::OriginBlade,
        ItemKind::RiftShard,
        ItemKind::CosmicShield,
        ItemKind::AlloyShield,
        ItemKind::WorkshopElixir,
    ];
}

/// 採集節點種類 → 對應的背包物品。把「採到什麼」與「背包存什麼」綁在型別層，
/// 接線時 `gather_near` 的產出種類直接 `.into()`，不會對錯資源。
impl From<NodeKind> for ItemKind {
    fn from(kind: NodeKind) -> Self {
        match kind {
            NodeKind::Tree => ItemKind::Wood,
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
                ItemKind::Wood
                | ItemKind::Dirt
                | ItemKind::Stone
                | ItemKind::Ether
                | ItemKind::Pickaxe
                | ItemKind::ReinforcedPickaxe
                | ItemKind::Weapon
                | ItemKind::CrystalShard
                | ItemKind::MushroomSpore
                | ItemKind::AncientFragment
                | ItemKind::DeepSeaPearl
                | ItemKind::WildflowerSeed
                | ItemKind::HealingPotion
                | ItemKind::CrystalPotion
                | ItemKind::MushroomElixir
                | ItemKind::EtherPill
                | ItemKind::PearlPotion
                | ItemKind::CrystalBlade
                | ItemKind::CoralLance
                | ItemKind::MeadowAmulet
                | ItemKind::CrystalShield
                | ItemKind::StarChart
                | ItemKind::MushroomStaff
                | ItemKind::RuneBlade
                | ItemKind::JadeShard
                | ItemKind::JadeElixir
                | ItemKind::JadeBlade
                | ItemKind::LavaCrystal
                | ItemKind::SteamElixir
                | ItemKind::CrimsonBlade
                | ItemKind::VoidShard
                | ItemKind::VoidElixir
                | ItemKind::VoidBlade
                | ItemKind::AetherShard
                | ItemKind::AetherEssence
                | ItemKind::AetherBlade
                | ItemKind::OriginShard
                | ItemKind::OriginEssence
                | ItemKind::OriginBlade
                | ItemKind::RiftShard
                | ItemKind::CosmicShield
                | ItemKind::AlloyShield
                | ItemKind::WorkshopElixir => {}
            }
        }
        let unique: std::collections::BTreeSet<_> = ItemKind::ALL.iter().collect();
        assert_eq!(unique.len(), ItemKind::ALL.len(), "ItemKind::ALL 有重複條目");
        // 目前共 43 種（含 ROADMAP 36 工作台：合金護盾/工坊精粹）；加變體時連同上面的 match 一起更新。
        assert_eq!(ItemKind::ALL.len(), 43, "ItemKind::ALL 筆數與變體數不一致");
    }

    #[test]
    fn node_kind_maps_to_item_kind() {
        assert_eq!(ItemKind::from(NodeKind::Tree), ItemKind::Wood);
    }

    #[test]
    fn gather_yield_flows_into_inventory_via_into() {
        // 模擬接線：採到 (種類, 量) 直接灌進背包。
        let mut inv = Inventory::new();
        let (kind, qty) = (NodeKind::Tree, 1u32);
        inv.add(kind.into(), qty);
        assert_eq!(inv.count(ItemKind::Wood), 1);
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

    #[test]
    fn every_item_has_a_source() {
        // 「無死路」跨模組不變式家族的 capstone（總綱）。家族前幾條各守**某張表裡**的東西：
        //   - gather 的 `every_gathered_resource_has_a_sink`、combat 的
        //     `every_enemy_drop_is_a_usable_economic_resource` 守「產出側有去處」；
        //   - crafting 的 `every_recipe_input_is_obtainable` 守「配方素材有來源」；
        //   - tools 的 `every_tool_item_is_obtainable` 守「工具有配方來源」。
        // 但它們**都只遍歷已落在某張表裡的物品**，獨缺一條遍歷**整個物品宇宙 `ItemKind::ALL`**、
        // 守住「凡玩家可能持有的物品，都至少有一條取得途徑」的總綱。
        //
        // 這條才補得到的縫隙：日後在 `ItemKind` 加一個變體（PLAN 自己就指向再加工具／合成產物），
        // 若它既不可採集、也不是任何配方的產物、也不是敵人掉落——它就是個玩家**永遠拿不到的死
        // 物品**，前端面板可能列它卻無從取得。`every_tool_item_is_obtainable` 只在該物品**是工具**
        // 時才檢查（且要求更強：工具必須有配方）；一個**非工具**的新物品會從所有 per-table 守則
        // 的縫隙裡一起漏掉。`item_kind_all_lists_every_variant` 只保證 `ALL` 不漏列變體、不保證每
        // 個變體有來源。趁物品宇宙還小，把「凡物品必有來源」鎖成遍歷 `ALL` 的總綱：日後加物品卻
        // 忘了給來源時當場紅燈，而非接線後玩家對著一個拿不到的物品困惑。
        //
        // 「有來源」＝可採集（某 `NodeKind` 映成它）**或**可合成（某配方產出它）**或**敵人掉落。
        use crate::combat::EnemyKind;
        use crate::crafting::RECIPES;

        // 採集可得的物品集合。窮舉守衛：新增 `NodeKind` 變體卻忘了納入時，此 match 不窮舉、
        // 編譯失敗，逼人回來把新採集資源納入本遍歷（比照 crafting/combat 同家族的守衛）。
        const NODE_KINDS: &[NodeKind] = &[NodeKind::Tree];
        for &n in NODE_KINDS {
            match n {
                NodeKind::Tree => {}
            }
        }
        let gatherable: std::collections::BTreeSet<ItemKind> =
            NODE_KINDS.iter().map(|&n| ItemKind::from(n)).collect();

        // 敵人掉落可得的物品集合。窮舉守衛同上：新增 `EnemyKind` 變體未納入即編譯失敗。
        const ENEMY_KINDS: &[EnemyKind] = &[
            EnemyKind::ScrapDrone,
            EnemyKind::EtherWisp,
            EnemyKind::FlutterSprite,
            EnemyKind::MushroomStalker,
            EnemyKind::CrystalGolem,
            EnemyKind::RuneGuardian,
            EnemyKind::CoralCrab,
            EnemyKind::JadeWraith,
            EnemyKind::SteamConstruct,
            EnemyKind::VoidPhantom,
            EnemyKind::AetherSpecter,
            EnemyKind::OriginGuardian,
            EnemyKind::RiftGuardian,
        ];
        for &e in ENEMY_KINDS {
            match e {
                EnemyKind::ScrapDrone
                | EnemyKind::EtherWisp
                | EnemyKind::FlutterSprite
                | EnemyKind::MushroomStalker
                | EnemyKind::CrystalGolem
                | EnemyKind::RuneGuardian
                | EnemyKind::CoralCrab
                | EnemyKind::JadeWraith
                | EnemyKind::SteamConstruct
                | EnemyKind::VoidPhantom
                | EnemyKind::AetherSpecter
                | EnemyKind::OriginGuardian
                | EnemyKind::RiftGuardian => {}
            }
        }
        let droppable: std::collections::BTreeSet<ItemKind> =
            ENEMY_KINDS.iter().map(|&e| e.drop_loot().0).collect();

        for &item in ItemKind::ALL {
            let gatherable_src = gatherable.contains(&item);
            let craftable_src = RECIPES.iter().any(|r| r.output == item);
            let droppable_src = droppable.contains(&item);
            // C-2 挖掘地形格可取得的物品（Dig handler：實心格→Empty + 材料入背包）。
            // Dirt/Stone/Ether 由挖掘對應 tile 取得；CrystalShard 挖 Crystal 晶洞格取得；
            // MushroomSpore 挖 Mushroom 蕈菇洞格取得；AncientFragment 挖 AncientRuin 遺跡格取得；
            // DeepSeaPearl 挖 CoralReef 珊瑚礁格取得（水域岸邊 80px 挖掘範圍內可達）；
            // WildflowerSeed 挖 WildFlower 野花叢格取得（草原特產，ROADMAP 14）；
            // JadeShard 挖 JadeVine 翠玉藤格取得（翠幽星特產，ROADMAP 21）；
            // LavaCrystal 挖 LavaRock 熔岩石格取得（赤焰星特產，ROADMAP 22）；
            // VoidShard 挖 VoidCrystal 虛空晶體格取得（虛空星特產，ROADMAP 23）；
            // AetherShard 挖 AetherMist 霧醚晶霧格取得（霧醚星特產，ROADMAP 24）。
            let tile_diggable = item == ItemKind::Dirt || item == ItemKind::Stone
                || item == ItemKind::Ether || item == ItemKind::CrystalShard
                || item == ItemKind::MushroomSpore || item == ItemKind::AncientFragment
                || item == ItemKind::DeepSeaPearl || item == ItemKind::WildflowerSeed
                || item == ItemKind::JadeShard || item == ItemKind::LavaCrystal
                || item == ItemKind::VoidShard || item == ItemKind::AetherShard
                || item == ItemKind::OriginShard;
            assert!(
                gatherable_src || craftable_src || droppable_src || tile_diggable,
                "物品 {item:?} 沒有任何取得途徑（不可採集／無配方產出／非敵人掉落／非地形挖掘）\
                 ——它是玩家永遠拿不到的死物品；請給它一條來源，或更新本不變式"
            );
        }
    }

    #[test]
    fn every_item_has_a_sink() {
        // 「無死路」家族的**去處側總綱**，與上面的 `every_item_has_a_source`（來源側總綱）
        // 嚴格對偶、湊成完整的一對。GDD／PLAN 白紙黑字的紀律是雙向的：「有產出**也**要有
        // 去處」。來源側已有 capstone 守住「凡物品都拿得到」；去處側此前卻只有 per-table 守則
        //   - gather 的 `every_gathered_resource_has_a_sink`、combat 的
        //     `every_enemy_drop_is_a_usable_economic_resource`
        // ——它們**都只遍歷某張產出表裡的原料**（採集物／掉落物），獨缺一條遍歷**整個物品宇宙
        // `ItemKind::ALL`**、守住「凡玩家可能持有的物品，都至少有一個去處」的總綱。
        //
        // 這條才補得到、且 per-table 守則**結構上碰不到**的縫隙：成品工具（如 `Pickaxe`）
        // 既不是任何配方的素材、也不是乙太貨幣，兩條 per-table sink 守則都把它排除在外
        // （它們只看採集／掉落出的**原料**有沒有去處，從不檢查工具本身）。一個工具的「去處」
        // 是它**拿來用有效用**——若日後加一個既不被消耗、又不能花、效用倍率卻沒比徒手快的
        // 「死道具」工具，per-table 守則全數放行、只有這條總綱攔得下。同理，一個**非工具、
        // 非原料、非貨幣**的新物品（例如純裝飾品）也只會在這裡紅燈。
        //
        // 「有去處」＝下列任一：
        //   1. 被某條配方當素材消耗（合成原料的去處）；
        //   2. 是有效用的工具（採集倍率嚴格快過徒手——`tools` 的
        //      `every_craftable_tool_is_worth_crafting` 也守同一條「工具必須真有用」）；
        //   3. 是乙太貨幣（`economy` 的擴地消耗點花掉它）；
        //   4. 是有效用的武器（攻擊力嚴格高過徒手——`combat::weapon_power` 拿它打怪更痛；
        //      與第 2 類對偶，武器之於戰鬥猶如工具之於採集）。
        // 日後若有意加「終端收藏品」之類刻意沒有機械去處的物品，會在此紅燈，逼人確認是有意
        // 設計再更新本不變式（比照來源側總綱與工具／配方家族的逃生口）。
        //
        // 物品宇宙的窮舉由 `item_kind_all_lists_every_variant`（窮舉 match + 筆數斷言）守住：
        // 新增 `ItemKind` 變體必先補進 `ALL`，本總綱隨即遍歷到它、要求它有去處——故無需在此
        // 另立 NodeKind/EnemyKind 式的窮舉守衛。
        use crate::combat::{armor_from_item, weapon_from_item, UNARMED_ATTACK_POWER};
        use crate::crafting::RECIPES;
        use crate::economy::EXPANSION_BASE_COST;
        use crate::npc::NPC_BUY_LIST;
        use crate::tools::{tool_from_item, FIST_MULTIPLIER};

        // 乙太去處（擴地消耗點）真實存在的編譯期錨點：直接引用 `economy` 的擴地基準價，
        // 使「乙太是有去處的貨幣」這個論斷不是空話——若日後 `economy` 連同擴地 sink 一起被
        // 移除，本測試會編譯失敗，逼人為乙太另尋去處或更新本不變式。
        const _: () = assert!(EXPANSION_BASE_COST > 0);

        for &item in ItemKind::ALL {
            // 1. 被某條配方當素材消耗。
            let consumed_by_recipe = RECIPES
                .iter()
                .any(|r| r.inputs.iter().any(|&(i, _)| i == item));
            // 2. 是有效用的工具（嚴格快過徒手；沒效用的「工具」不算有去處）。
            let useful_tool =
                tool_from_item(item).is_some_and(|t| t.gather_multiplier() > FIST_MULTIPLIER);
            // 3. 是乙太貨幣（擴地消耗點花掉它）。
            let spendable_currency = item == ItemKind::Ether;
            // 4. 是有效用的武器（攻擊力嚴格高過徒手）或防具（持有可減傷）。
            let useful_weapon = weapon_from_item(item)
                .is_some_and(|w| w.attack_power() > UNARMED_ATTACK_POWER);
            let useful_armor = armor_from_item(item).is_some_and(|a| a.defense() > 0);
            // 5. 是可放置的建造材料（C-4 Place handler：背包材料→實心格）。
            // C-2 引入 Dirt，C-4 接線後成為真正去處（Place Dirt → 建牆）。
            // 此條確認「設計上有去處」，避免在地基切片就要求去處已上線。
            let building_material = item == ItemKind::Dirt;
            // 6. 可賣給 NPC 商人換乙太（NPC_BUY_LIST 中的素材資源）。
            // 「賣出換乙太」是合法的經濟去處——稀有資源（如晶石碎片）給 NPC 高價收購，
            // 讓探索型玩家有把成果兌換乙太的管道。
            let npc_sellable = NPC_BUY_LIST.iter().any(|e| e.item == item);
            // 7. 是可主動使用的消耗品（UseItem 觸發即消耗，直接對玩家產生效果）。
            // 活力藥水為第一個；各生態特產合成藥水同屬此類。
            let usable_consumable = matches!(
                item,
                ItemKind::HealingPotion
                    | ItemKind::CrystalPotion
                    | ItemKind::MushroomElixir
                    | ItemKind::EtherPill
                    | ItemKind::PearlPotion
                    | ItemKind::JadeElixir
                    | ItemKind::SteamElixir
                    | ItemKind::VoidElixir
                    | ItemKind::AetherEssence
                    | ItemKind::OriginEssence
                    | ItemKind::WorkshopElixir
            );
            // 8. 是導航工具（UseItem 觸發功能但不消耗——持有期間可重複使用）。
            // 星圖屬此類：展開星際旅行界面，直到多星球旅程開啟（ROADMAP 20）都有意義。
            let navigation_tool = item == ItemKind::StarChart;

            assert!(
                consumed_by_recipe || useful_tool || spendable_currency || useful_weapon
                    || building_material || npc_sellable || usable_consumable || useful_armor
                    || navigation_tool,
                "物品 {item:?} 沒有任何去處（不被任何配方消耗／不是有效用的工具／不是乙太貨幣／\
                 不是有效用的武器或防具／不是建造材料／不可賣給 NPC／不是可用消耗品）——玩家持有它卻無處可用，\
                 是只進不出的死庫存，違反 GDD「有產出也要有去處」紀律；請給它一個去處或更新本不變式"
            );
        }
    }
}
