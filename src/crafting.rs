//! 合成配方（Phase 1-C 純邏輯地基）。
//!
//! 玩法鏈缺的那一環：採集／打怪／農地三個來源都在灌背包，木／石／廢料只進不出。
//! 合成給這些素材一個「去處」——查配方、扣素材、產出工具，是 GDD 紀律「乙太有產出
//! 也要有去處」往素材延伸的第一步。
//!
//! 這層純資料 + 純函式，無 IO、不碰 WebSocket／遊戲迴圈，便於自動測試：
//!   - `RECIPES`：靜態配方表（輸入素材 → 產出物品），單一真實來源。
//!   - `Recipe::can_craft(&inv)`：背包夠不夠料、產物放不放得下（UI 反灰用）。
//!   - `Recipe::craft(&mut inv)`：**全有全無**——夠才一次扣全部素材、加產物；任一條件
//!     不滿足回 `false` 且完全不動背包（不會扣到一半卻拿不到產物）。
//!   - `recipe_by_id(id)`：接線時 client 送 `Craft{ recipe: "pickaxe" }`，伺服器查表。
//!
//! additive、不動廣播 shape：背包已隨快照廣播（見 `protocol::InventoryView`），合成只是
//! 多扣／多加背包內容，前端只需多一個合成面板，零契約變更。延續 `inventory.rs` /
//! `gather.rs` / `field.rs` 的前置慣例：純邏輯先落地、標 `allow(dead_code)`，接線輪
//! （ws 收 `Craft` → `recipe_by_id` → `craft` → 背包走既有快照）才有呼叫端。

use crate::inventory::{Inventory, ItemKind, MAX_STACK};

/// 一條合成配方：吃 `inputs` 列出的素材，產出 `output_qty` 個 `output`。
///
/// `id` 是給前端／網路用的穩定字串（snake_case，對齊 `ItemKind` 的序列化命名）：
/// client 送 `Craft{ recipe: id }`，伺服器以 `recipe_by_id` 查回配方，避免讓客戶端
/// 直接送一整份配方內容（素材／產量一律由伺服器這份表說了算，client 只送意圖）。
pub struct Recipe {
    /// 穩定字串 id（網路／前端用），全表唯一。
    pub id: &'static str,
    /// 合成出的物品。
    pub output: ItemKind,
    /// 一次合成產出的數量。
    pub output_qty: u32,
    /// 需要消耗的素材 `(物品, 數量)`。同一物品在一條配方裡只出現一次
    /// （見 `recipe_table_is_well_formed` 測試把關），故 `can_craft` 的逐項檢查無須疊加。
    pub inputs: &'static [(ItemKind, u32)],
}

/// 全部配方表（單一真實來源）。薄切片先只有鎬子一條：木×3 + 石×2 → 鎬子×1，
/// 把採集／打怪堆起來的木石導向第一件工具。日後加配方只要往這個陣列加一筆。
pub const RECIPES: &[Recipe] = &[Recipe {
    id: "pickaxe",
    output: ItemKind::Pickaxe,
    output_qty: 1,
    inputs: &[(ItemKind::Wood, 3), (ItemKind::Stone, 2)],
}];

// 整個模組是前置地基：接線輪（ws 收 `Craft` → 查表 → `craft` → 背包隨快照廣播）才有
// 呼叫端，在此之前公開項目皆無外部呼叫，比照 `inventory.rs` / `gather.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
impl Recipe {
    /// 此刻能否合成：每種素材都夠，**且**產物加得進背包（不會撞 `MAX_STACK`）。
    /// 把「產物放得下」一併納入，是為了讓 `craft` 的全有全無語意成立——否則素材被扣、
    /// 產物卻被堆疊上限夾掉，玩家平白損失素材。
    pub fn can_craft(&self, inv: &Inventory) -> bool {
        let inputs_ok = self.inputs.iter().all(|&(item, qty)| inv.has(item, qty));
        // 產物若正好是某個素材（理論上的自反配方），這裡用「扣掉素材後的餘量」會更精確；
        // 但現有配方產物（工具）與素材（資源）不相交，故用當前數量檢查即可、且更保守。
        let output_fits = inv.count(self.output).saturating_add(self.output_qty) <= MAX_STACK;
        inputs_ok && output_fits
    }

    /// 嘗試合成：**全有全無**。`can_craft` 通過才動手——逐項扣素材（已驗夠、必成功）、
    /// 加產物，回 `true`；否則完全不動背包、回 `false`。
    pub fn craft(&self, inv: &mut Inventory) -> bool {
        if !self.can_craft(inv) {
            return false;
        }
        for &(item, qty) in self.inputs {
            // `can_craft` 已確保每項都夠且素材互不重複，這裡的 `take` 必定成功；
            // debug 下加斷言，防日後改動讓不變式悄悄破裂。
            let took = inv.take(item, qty);
            debug_assert!(took, "can_craft 通過後 take 不該失敗");
        }
        inv.add(self.output, self.output_qty);
        true
    }
}

/// 依字串 id 查配方（接線時伺服器收到 client 的 `Craft` 意圖後用）。未知 id 回 `None`。
#[allow(dead_code)]
pub fn recipe_by_id(id: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::NodeKind;

    /// 把背包灌到剛好夠合成鎬子的素材（木 3 石 2），供多個測試共用。
    fn stocked() -> Inventory {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        inv.add(ItemKind::Stone, 2);
        inv
    }

    fn pickaxe() -> &'static Recipe {
        recipe_by_id("pickaxe").expect("鎬子配方應存在")
    }

    #[test]
    fn recipe_by_id_finds_known_and_rejects_unknown() {
        assert!(recipe_by_id("pickaxe").is_some());
        assert!(recipe_by_id("nonexistent").is_none());
        assert!(recipe_by_id("").is_none());
    }

    #[test]
    fn craft_consumes_inputs_and_yields_output() {
        let mut inv = stocked();
        let r = pickaxe();
        assert!(r.can_craft(&inv));
        assert!(r.craft(&mut inv));
        // 素材扣光、得一把鎬子。
        assert_eq!(inv.count(ItemKind::Wood), 0);
        assert_eq!(inv.count(ItemKind::Stone), 0);
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn craft_keeps_surplus_materials() {
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 5);
        inv.add(ItemKind::Stone, 3);
        assert!(pickaxe().craft(&mut inv));
        // 只扣掉配方所需，多的留著。
        assert_eq!(inv.count(ItemKind::Wood), 2);
        assert_eq!(inv.count(ItemKind::Stone), 1);
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn craft_fails_and_is_unchanged_when_missing_a_material() {
        // 有木沒石：can_craft 為否、craft 不動背包（驗原子性——木不該被扣掉）。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        let r = pickaxe();
        assert!(!r.can_craft(&inv));
        assert!(!r.craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Wood), 3);
        assert_eq!(inv.count(ItemKind::Stone), 0);
        assert_eq!(inv.count(ItemKind::Pickaxe), 0);
    }

    #[test]
    fn craft_fails_when_partially_short() {
        // 木夠石差一個：仍是全有全無，整筆失敗、木原封不動。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Wood, 3);
        inv.add(ItemKind::Stone, 1);
        assert!(!pickaxe().craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Wood), 3);
        assert_eq!(inv.count(ItemKind::Stone), 1);
    }

    #[test]
    fn craft_fails_and_keeps_materials_when_output_would_overflow() {
        // 產物已堆到上限：素材雖夠也不該合（否則扣了料、產物被 MAX_STACK 夾掉而平白損失）。
        let mut full = std::collections::BTreeMap::new();
        full.insert(ItemKind::Wood, 3);
        full.insert(ItemKind::Stone, 2);
        full.insert(ItemKind::Pickaxe, MAX_STACK);
        let mut inv = Inventory::from_raw(full);
        let r = pickaxe();
        assert!(!r.can_craft(&inv));
        assert!(!r.craft(&mut inv));
        // 全有全無：素材一個沒少。
        assert_eq!(inv.count(ItemKind::Wood), 3);
        assert_eq!(inv.count(ItemKind::Stone), 2);
        assert_eq!(inv.count(ItemKind::Pickaxe), MAX_STACK);
    }

    #[test]
    fn gathered_materials_flow_into_crafting() {
        // 端到端模擬玩法鏈：採集產出灌進背包 → 合成。鎖住「採集→背包→合成」同一套物品槽。
        let mut inv = Inventory::new();
        for _ in 0..3 {
            inv.add(NodeKind::Tree.into(), 1); // 採樹得木
        }
        inv.add(NodeKind::Rock.into(), 1); // 採石得石
        inv.add(NodeKind::Rock.into(), 1);
        assert!(pickaxe().craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
    }

    #[test]
    fn recipe_table_is_well_formed() {
        // 配方表健全性（與調校數值無關的不變式，防日後加配方時打錯）：
        let mut seen_ids = std::collections::BTreeSet::new();
        for r in RECIPES {
            // id 唯一。
            assert!(seen_ids.insert(r.id), "配方 id 重複：{}", r.id);
            // 產量為正。
            assert!(r.output_qty > 0, "{} 產量應 > 0", r.id);
            // 至少要有一項素材、每項數量為正、同一物品不重複。
            assert!(!r.inputs.is_empty(), "{} 應至少需要一項素材", r.id);
            let mut seen_items = std::collections::BTreeSet::new();
            for &(item, qty) in r.inputs {
                assert!(qty > 0, "{} 的素材數量應 > 0", r.id);
                assert!(seen_items.insert(item), "{} 的素材 {:?} 重複", r.id, item);
            }
        }
    }

    #[test]
    fn recipe_output_is_disjoint_from_its_own_inputs() {
        // `can_craft` 的「產物放得下」檢查（`inv.count(output) + output_qty <= MAX_STACK`）
        // 刻意用**扣素材前**的當前數量，其正確性靠那行 doc 明言的不變式：「現有配方產物
        // （工具）與素材（資源）不相交」。此前無測試把關這條假設——日後若有人加一條自反
        // 配方（產物同時列在自己的素材裡，如「鎬子+木 → 升級鎬子」），那個捷徑會用「還沒扣
        // 素材的舊量」誤判產物放不放得下（偏保守、可能平白反灰一筆其實做得成的合成），而
        // `recipe_table_is_well_formed` 只驗素材彼此不重複、察覺不到產物撞素材。趁配方表還
        // 只有一條，把 `can_craft` 倚賴的這條前提鎖成測試：日後加自反配方當場紅燈，逼人回去
        // 把那個捷徑改成「扣素材後的餘量」再放行，而不是接線後在線上靜默誤判。
        for r in RECIPES {
            assert!(
                !r.inputs.iter().any(|&(item, _)| item == r.output),
                "配方 `{}` 的產物 {:?} 出現在自己的素材裡——`can_craft` 的產物容量捷徑會失準，\
                 需改用扣素材後的餘量檢查",
                r.id,
                r.output
            );
        }
    }

    /// 防漂移：窮舉所有採集節點種類 → 對應產出的物品，當作「可採集物品」的單一真實來源。
    /// 日後在 `NodeKind` 加變體（新採集資源）時，下面的窮舉 `match` 會**編譯失敗**，逼人
    /// 回來把它補進這份清單——確保 `every_recipe_input_is_obtainable` 賴以判斷的「可採集集合」
    /// 不會與 `NodeKind` 漂移（比照 `inventory.rs` 的 `ItemKind::ALL` 窮舉 match 守則）。
    fn gatherable_items() -> std::collections::BTreeSet<ItemKind> {
        const NODE_KINDS: &[NodeKind] = &[NodeKind::Tree, NodeKind::Rock, NodeKind::EtherOre];
        // 窮舉守衛：新增 NodeKind 變體卻忘了加進 NODE_KINDS 時，此 match 不窮舉、編譯失敗。
        for &n in NODE_KINDS {
            match n {
                NodeKind::Tree | NodeKind::Rock | NodeKind::EtherOre => {}
            }
        }
        NODE_KINDS.iter().map(|&n| ItemKind::from(n)).collect()
    }

    #[test]
    fn every_recipe_input_is_obtainable() {
        // 跨模組不變式（1-A 採集 × 1-B 物品 × 1-C 合成），與 `tools.rs` 的
        // `every_tool_item_is_obtainable` **互補的另一個方向**：那條守「配方產物（工具）
        // 拿得到」（輸出側——每個工具都有配方）；這條守「配方素材湊得齊」（輸入側——每條
        // 配方需要的每樣素材，玩家都有來源取得）。
        //
        // 失敗模式不同：加一條新配方、卻讓它需要一種**既不可採集**（`From<NodeKind>` 只把
        // 採集節點映成 Wood/Stone/Ether 三種資源）、**也沒有任何配方產出**的素材，玩家就
        // 永遠湊不齊料——前端合成面板會把它列出來、卻因 `can_craft` 永遠為否而恆反灰，是條
        // 玩家看得到卻永遠合不出的**死配方**。`recipe_table_is_well_formed` 只驗素材數量為正、
        // 彼此不重複，察覺不到「這素材根本拿不到」。PLAN 自己就指向再加配方（斧／鋤），屆時
        // 這正是會踩的坑。趁配方表還小，把「凡配方素材必有來源」鎖成遍歷整張表的組合測試：
        // 日後加配方時若引用了拿不到的素材當場紅燈，而非接線後玩家對著恆反灰的合成鈕困惑。
        //
        // 「有來源」＝可採集（某 `NodeKind` 產出它）**或**可合成（某條配方產出它，允許
        // 「工具＋素材→升級工具」這類以合成中間物當素材的配方鏈）。
        let gatherable = gatherable_items();
        for r in RECIPES {
            for &(item, _) in r.inputs {
                let craftable = RECIPES.iter().any(|other| other.output == item);
                assert!(
                    gatherable.contains(&item) || craftable,
                    "配方 `{}` 需要素材 {:?}，但它既不可採集（沒有 NodeKind 產出它）、也沒有任何\
                     配方產出它——玩家永遠湊不齊料，這是條看得到卻永遠合不出的死配方；請確認該素材\
                     能由採集／合成取得，或為它補上來源",
                    r.id, item
                );
            }
        }
    }

    #[test]
    fn recipe_ids_are_wire_safe_snake_case() {
        // 線協定契約：`id` 是 client 送 `Craft{ recipe: id }` 跟前端 keying 用的穩定字串，
        // doc 言明「snake_case，對齊 `ItemKind` 的序列化命名」。此前只驗 id 唯一/非空意涵，
        // **沒驗格式**——接線輪（ws 收 `Craft` → `recipe_by_id`）一旦加第二條配方，一個帶
        // 空格／大寫／unicode 的壞 id 會悄悄破壞 JSON 協定或前端對應，且 `recipe_by_id`
        // 只做字串相等比對、不會察覺。趁配方表還小，把這個契約鎖成測試，日後加配方時
        // 打錯 id 當場紅燈，而不是接線後才在線上炸開。
        for r in RECIPES {
            assert!(!r.id.is_empty(), "配方 id 不可為空");
            // 僅允許小寫 ASCII 字母／數字／底線：與 `#[serde(rename_all = "snake_case")]`
            // 產出的 `ItemKind` 名稱同一套字元集，確保跨 ws／快照／前端的字串 key 一致。
            assert!(
                r.id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "配方 id `{}` 含非 snake_case 字元（只允許 a-z 0-9 _）",
                r.id
            );
            // 不以底線開頭／結尾：避免 `_pickaxe`／`pickaxe_` 這類醜陋又易撞的 key。
            assert!(
                !r.id.starts_with('_') && !r.id.ends_with('_'),
                "配方 id `{}` 不該以底線開頭或結尾",
                r.id
            );
            // 自洽：用自己的 id 查回來必定是同一條配方（鎖住 `recipe_by_id` 是接線用的反查
            // 入口；id 唯一已由 `recipe_table_is_well_formed` 把關，故 id 相等即同一條）。
            let looked_up = recipe_by_id(r.id).expect("自己的 id 應查得到");
            assert_eq!(looked_up.id, r.id, "配方 `{}` 用自身 id 查回的不是自己", r.id);
            assert_eq!(
                looked_up.output, r.output,
                "配方 `{}` 反查到的產物不一致",
                r.id
            );
        }
    }
}
