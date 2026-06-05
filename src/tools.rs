//! 工具效用模型（Phase 1-D 的純邏輯地基）。
//!
//! 這層只管「玩家身上有什麼工具、拿來做某個動作能加速多少」，是純資料 + 純函式，無 IO、
//! 不碰 WebSocket / 遊戲迴圈，便於自動測試。延續 `crafting.rs` / `inventory.rs` /
//! `gather.rs` 的前置慣例：純邏輯先落地、標 `allow(dead_code)`，接線輪才有呼叫端。
//!
//! 之後接上：
//!   - ws 採集：玩家採集（Phase 1-A `ResourceNode::gather`）時，依背包裡最好的工具
//!     （`best_tool_for(.., Gather)`）算出加速倍率 `m`——一次採集動作相當於採 `m` 下，
//!     採礦更快；沒鎬子就用拳頭（`m == 1`，很慢）。
//!   - ws 翻土：照顧農地翻土（Phase 0-G `Field::till`）時，依 `best_tool_for(.., Till)` 加速。
//!   - 前端：HUD 顯示手上的工具與它對當前動作的效用。
//!
//! 規則刻意做成「對的工具配對的活才有加成」：鎬子採礦快、鋤頭翻土快，拿錯（鎬子翻土）
//! 只有拳頭等級的基礎速度——鼓勵玩家合出並帶對工具。倍率走整數 `u32`：接線時把「一次
//! 動作」放大成 `m` 下即可，不引入浮點誤差，也與 `gather` 以整數耐久計次的模型咬合。

use crate::inventory::{Inventory, ItemKind};

/// 出力的動作種類——決定哪種工具能加速它。
// 前置地基：接線輪（採集 / 翻土帶 task）才會構造這些變體，比照本模組標 `allow(dead_code)`。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTask {
    /// 採集 / 採礦（採樹、採石、採乙太礦——Phase 1-A 的 `ResourceNode::gather`）。
    Gather,
    /// 翻土（Phase 0-G 耕地的 `Field::till`）。
    Till,
}

/// 玩家用來執行動作的工具。`Fist` 是沒有合適工具時的退路（只有基礎速度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// 徒手——任何動作都能做，但只有基礎速度。
    Fist,
    /// 鎬子（合成產物）：採礦更快。
    Pickaxe,
    /// 鋤頭（合成產物）：翻土更快。
    Hoe,
}

/// 徒手的基礎速度倍率（單位速度）。任何動作的最低速度，沒有對的工具就是這個。
pub const FIST_MULTIPLIER: u32 = 1;

/// 用對工具時的加速倍率（鎬子採礦 / 鋤頭翻土）。對應驗收「採礦速度提升 X 倍」的 X。
pub const TOOL_MULTIPLIER: u32 = 3;

// 整個模組是前置地基：接線輪（採集 / 翻土依工具加速）才有呼叫端，
// 在此之前公開項目皆無外部呼叫，比照 `crafting.rs` / `inventory.rs` 標 `allow(dead_code)`。
#[allow(dead_code)]
impl ToolKind {
    /// 此工具拿來做 `task` 的速度倍率。對的工具配對的活回 `TOOL_MULTIPLIER`，
    /// 其餘（徒手、或拿錯工具）一律回基礎 `FIST_MULTIPLIER`。
    pub fn multiplier(self, task: ToolTask) -> u32 {
        match (self, task) {
            (ToolKind::Pickaxe, ToolTask::Gather) => TOOL_MULTIPLIER,
            (ToolKind::Hoe, ToolTask::Till) => TOOL_MULTIPLIER,
            // 徒手、或工具與動作不對盤（鎬子翻土、鋤頭採礦）：基礎速度。
            _ => FIST_MULTIPLIER,
        }
    }
}

/// 某個背包物品若是工具，回對應的 `ToolKind`；不是工具（資源原料）回 `None`。
/// 刻意用窮舉 `match`（不寫 `_` 萬用分支）：日後在 `ItemKind` 加新工具變體時，
/// 編譯器會強制回來補上它對應的工具，避免漏接。
#[allow(dead_code)]
pub fn tool_from_item(item: ItemKind) -> Option<ToolKind> {
    match item {
        ItemKind::Pickaxe => Some(ToolKind::Pickaxe),
        ItemKind::Hoe => Some(ToolKind::Hoe),
        ItemKind::Wood | ItemKind::Stone | ItemKind::Ether => None,
    }
}

/// 玩家背包裡對 `task` 最有效的工具：挑出持有工具中倍率最高者；都沒有、或持有的工具
/// 對這動作都沒加成（如只帶鎬子卻要翻土）就回 `Fist`。供採集 / 翻土接線時決定加速倍率。
#[allow(dead_code)]
pub fn best_tool_for(inv: &Inventory, task: ToolTask) -> ToolKind {
    inv.entries()
        .filter_map(|(item, _)| tool_from_item(item))
        .max_by_key(|tool| tool.multiplier(task))
        // 取最高倍率的工具後，若那也只有基礎效用（拿錯工具）就退回徒手，語意更清楚。
        .filter(|tool| tool.multiplier(task) > FIST_MULTIPLIER)
        .unwrap_or(ToolKind::Fist)
}

/// 玩家做 `task` 的速度倍率（自動取背包裡最好的工具）。`1`＝徒手基礎速度。
/// 接線時：一次動作相當於連做 `speed_multiplier` 下（採礦 / 翻土更快）。
#[allow(dead_code)]
pub fn speed_multiplier(inv: &Inventory, task: ToolTask) -> u32 {
    best_tool_for(inv, task).multiplier(task)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickaxe_speeds_mining_not_tilling() {
        assert_eq!(ToolKind::Pickaxe.multiplier(ToolTask::Gather), TOOL_MULTIPLIER);
        // 鎬子翻土沒加成，只有基礎速度。
        assert_eq!(ToolKind::Pickaxe.multiplier(ToolTask::Till), FIST_MULTIPLIER);
    }

    #[test]
    fn hoe_speeds_tilling_not_mining() {
        assert_eq!(ToolKind::Hoe.multiplier(ToolTask::Till), TOOL_MULTIPLIER);
        assert_eq!(ToolKind::Hoe.multiplier(ToolTask::Gather), FIST_MULTIPLIER);
    }

    #[test]
    fn fist_is_base_speed_for_everything() {
        assert_eq!(ToolKind::Fist.multiplier(ToolTask::Gather), FIST_MULTIPLIER);
        assert_eq!(ToolKind::Fist.multiplier(ToolTask::Till), FIST_MULTIPLIER);
    }

    #[test]
    fn only_tools_map_from_items() {
        assert_eq!(tool_from_item(ItemKind::Pickaxe), Some(ToolKind::Pickaxe));
        assert_eq!(tool_from_item(ItemKind::Hoe), Some(ToolKind::Hoe));
        // 資源原料不是工具。
        assert_eq!(tool_from_item(ItemKind::Wood), None);
        assert_eq!(tool_from_item(ItemKind::Stone), None);
        assert_eq!(tool_from_item(ItemKind::Ether), None);
    }

    #[test]
    fn empty_inventory_falls_back_to_fist() {
        let inv = Inventory::new();
        assert_eq!(best_tool_for(&inv, ToolTask::Gather), ToolKind::Fist);
        assert_eq!(best_tool_for(&inv, ToolTask::Till), ToolKind::Fist);
        assert_eq!(speed_multiplier(&inv, ToolTask::Gather), FIST_MULTIPLIER);
    }

    #[test]
    fn wrong_tool_for_task_falls_back_to_fist() {
        // 只帶鎬子卻要翻土：等同徒手。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Pickaxe, 1);
        assert_eq!(best_tool_for(&inv, ToolTask::Till), ToolKind::Fist);
        assert_eq!(speed_multiplier(&inv, ToolTask::Till), FIST_MULTIPLIER);
        // 同一支鎬子拿去採礦就有加成。
        assert_eq!(best_tool_for(&inv, ToolTask::Gather), ToolKind::Pickaxe);
        assert_eq!(speed_multiplier(&inv, ToolTask::Gather), TOOL_MULTIPLIER);
    }

    #[test]
    fn picks_the_right_tool_for_each_task() {
        // 背包同時有鎬子和鋤頭：各動作各取對的那把。
        let mut inv = Inventory::new();
        inv.add(ItemKind::Pickaxe, 1);
        inv.add(ItemKind::Hoe, 1);
        assert_eq!(best_tool_for(&inv, ToolTask::Gather), ToolKind::Pickaxe);
        assert_eq!(best_tool_for(&inv, ToolTask::Till), ToolKind::Hoe);
        assert_eq!(speed_multiplier(&inv, ToolTask::Gather), TOOL_MULTIPLIER);
        assert_eq!(speed_multiplier(&inv, ToolTask::Till), TOOL_MULTIPLIER);
    }

    // ── Phase 1 玩法垂直迴圈組合測試 ───────────────────────────────────────
    // 各模組（gather / inventory / crafting / tools）都各有扎實的單元測試，但「採集
    // → 進背包 → 合成工具 → 工具回頭讓採集更快」這條跨模組接縫——正是接線層要組裝、
    // 也正是 bug 最愛藏身的地方——此前沒有任何一個測試保證它合得起來。這個組合測試走
    // 一遍純邏輯層的完整經濟迴圈，鎖住各模組契約能對接：任一模組契約日後漂移
    // （`gather_times` 語意、`NodeKind→ItemKind` 映射、`speed_multiplier` 倍率…）
    // 都會在此處整條斷掉、而非等接線上線才在 ws 裡爆。放在 `tools.rs` 是因為這條迴圈的
    // 高潮正是工具效用本身——「合出的鎬子確實讓真實節點採得更快」是本模組的端到端契約。

    use crate::crafting::PICKAXE;
    use crate::gather::{NodeKind, ResourceNode};

    #[test]
    fn phase1_loop_crafted_pickaxe_speeds_real_gathering() {
        let mut inv = Inventory::new();

        // 一開始徒手：採集一次動作只採 speed_multiplier(=1) 下。
        assert_eq!(speed_multiplier(&inv, ToolTask::Gather), FIST_MULTIPLIER);

        // 徒手採木：鎬子配方要木×3。Tree 每下 1 木，一次動作（徒手＝1 下）採 1 木。
        let mut tree = ResourceNode::new(NodeKind::Tree);
        for _ in 0..3 {
            let got = tree.gather_times(speed_multiplier(&inv, ToolTask::Gather));
            inv.add(NodeKind::Tree.into(), got);
        }
        assert_eq!(inv.count(ItemKind::Wood), 3);

        // 徒手採石：配方要石×2。
        let mut rock = ResourceNode::new(NodeKind::Rock);
        for _ in 0..2 {
            let got = rock.gather_times(speed_multiplier(&inv, ToolTask::Gather));
            inv.add(NodeKind::Rock.into(), got);
        }
        assert_eq!(inv.count(ItemKind::Stone), 2);

        // 合成鎬子：材料剛好夠，扣光木石、得到一支鎬子。
        assert!(PICKAXE.craft(&mut inv));
        assert_eq!(inv.count(ItemKind::Pickaxe), 1);
        assert_eq!(inv.count(ItemKind::Wood), 0);
        assert_eq!(inv.count(ItemKind::Stone), 0);

        // 關鍵：背包裡有鎬子後，採集一次動作的倍率自動升到 TOOL_MULTIPLIER(=3)。
        assert_eq!(speed_multiplier(&inv, ToolTask::Gather), TOOL_MULTIPLIER);

        // 帶鎬子採乙太礦：EtherOre 滿耐久 3、每下產 2 乙太，一次動作（3 下）正好把整座
        // 礦採乾＝3×2＝6 乙太，並進入重生。對照下面徒手版可見鎬子確實 3 倍快。
        let mut ore = ResourceNode::new(NodeKind::EtherOre);
        let with_pickaxe = ore.gather_times(speed_multiplier(&inv, ToolTask::Gather));
        inv.add(NodeKind::EtherOre.into(), with_pickaxe);
        assert_eq!(with_pickaxe, 6);
        assert_eq!(inv.count(ItemKind::Ether), 6);
        assert!(ore.is_depleted(), "一次動作就把整座乙太礦採乾、進重生");

        // 對照組：徒手一次動作只採 1 下＝2 乙太，礦還剩耐久（鎬子確實快、且不超採）。
        let mut bare_ore = ResourceNode::new(NodeKind::EtherOre);
        let by_hand = bare_ore.gather_times(FIST_MULTIPLIER);
        assert_eq!(by_hand, 2);
        assert!(bare_ore.is_harvestable(), "徒手一次只採一下，礦還沒採乾");
        assert_eq!(with_pickaxe, by_hand * TOOL_MULTIPLIER);
    }
}
