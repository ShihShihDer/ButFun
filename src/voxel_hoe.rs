//! 乙太方界·鋤頭 v1——就地把草地／泥土開墾成農田土（自主提案切片）。
//!
//! **缺口 / 為誰做**：乙太方界早有完整種田系統（小麥／胡蘿蔔／馬鈴薯，`voxel_farm`），
//! 但「開墾一塊田」這件事至今很繞——農田土只能在工作台用 `till` 配方（2 泥土 → 2 農田土）
//! 批量做出來，玩家得先挖泥土、搬回工作台合成、再放回地上。想在草原上就地開一畦田，
//! 沒有一個「走到地面、對著它一鋤、當場翻成農田土」的動詞。這正接續 794 水桶（維護者
//! 2026-06-30「操作／合成系統也要，我也想享受這世界」）：**水桶管引水、鋤頭管開墾**，
//! 一對經典農具，合起來把一片荒地變成自己的水田。
//!
//! 本刀補上經典而缺失的玩家工具——**鋤頭**：用木頭＋木板在背包打一把木鋤頭，走到草地／泥土上
//! **一鋤就地翻成農田土**（`Block::FarmSoil`），馬上能撒種。鋤頭是**工具**（反覆使用、不耗損，
//! 比照鎬／斧採集不消耗工具），只驗背包真持有即可。
//!
//! **與既有機制的分界（換維度·非同軸重複）**：`till` 合成配方（665）是**批量把泥土物料
//! 加工成農田土方塊**（消耗背包泥土、產出可放置的農田土）；鋤頭是**就地改造地形**——不消耗
//! 任何方塊、原地把腳下的草／土**轉換**成農田土，是「工具互動」而非「合成配方」，兩者互補
//! （一個管在工作台造材料、一個管在野地開墾），語意各異。與 794 水桶（搬水改造水文）成對
//! 但不同軸：水桶動水、鋤頭動土。
//!
//! **這裡只放確定性純邏輯**（可鋤判定、鋤頭 id、開墾產物 id、回饋台詞），零 LLM、零鎖、零 IO、
//! 零 async，可單元測試。連線 / 鎖 / delta 寫入 / 廣播全留在 `voxel_ws.rs`（沿用放置／破壞的
//! 短鎖循序慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：不觸發 LLM、不開對外端點、不收玩家自由文字——回饋台詞全為確定性模板；
//! 開墾由**伺服器權威**判定（目標方塊型別後端複驗、手持鋤頭查背包真持有才生效，前端不自報
//! 合法性）；農田土是療癒沙盒的無限資源、無經濟破壞面；不動帳號權限、不破壞玩家資料。

use crate::voxel::Block;

/// 木鋤頭物品 id（純物品，住背包、不可放置於世界；接續滿水桶 72 之後首個空號）。
/// 用木頭＋木板在背包 2×2 打造；對準草地／泥土「使用」＝就地開墾成農田土。
/// 鋤頭是工具、反覆使用不耗損（比照鎬／斧），故只有一階（開墾不需要工具階級）。
pub const HOE_ID: u8 = 73;

/// 開墾產物方塊 id（`Block::FarmSoil`）——鋤過的地變成農田土，可立即撒種。
/// 這裡以常數釘住（純模組不引入 voxel_ws 私有語意），另有測試對齊 `Block::FarmSoil`。
pub const FARM_SOIL_ID: u8 = 11;

/// 這格「地面」能不能被鋤成農田土——只認草地（`Block::Grass`）與泥土（`Block::Dirt`）。
/// 沙／雪／石／農田土（已是田）等一律不可鋤：農田土是泥質耕地，沙漠沙地與雪原積雪不對；
/// 已是農田土也不必重鋤。純函式、可測（比照 Minecraft 鋤頭只把草／土翻成耕地的手感）。
pub fn is_tillable(block_id: u8) -> bool {
    block_id == Block::Grass as u8 || block_id == Block::Dirt as u8
}

/// 手持這個物品是不是鋤頭（決定要不要當開墾動作處理）。純函式、可測。
pub fn is_hoe(held_id: u8) -> bool {
    held_id == HOE_ID
}

/// 面向玩家的鋤頭名稱（i18n 集中管理）。
pub fn hoe_name_zh(id: u8) -> &'static str {
    match id {
        HOE_ID => "木鋤頭",
        _ => "鋤頭",
    }
}

/// 開墾成功的回饋句（3 句輪替，確定性）。
pub fn till_ok_line(pick: usize) -> &'static str {
    const LINES: [&str; 3] = [
        "🪏 一鋤下去，土翻鬆了，成了一畦好田～",
        "🪏 把地開墾成農田土，可以撒種了～",
        "🪏 鬆好了土，這塊地能種東西了～",
    ];
    LINES[pick % LINES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct_and_align() {
        // 鋤頭 id 是滿水桶(72)之後的首個空號 73，且與農田土產物 id 不撞。
        assert_eq!(HOE_ID, 73);
        assert_ne!(HOE_ID, FARM_SOIL_ID);
        // 開墾產物常數必須對齊 voxel.rs 的真相（Block::FarmSoil）。
        assert_eq!(FARM_SOIL_ID, Block::FarmSoil as u8);
        // 鋤頭是純物品、不可有對應方塊（Block::from_u8 應為 None）。
        assert!(Block::from_u8(HOE_ID).is_none());
    }

    #[test]
    fn only_grass_or_dirt_is_tillable() {
        // 草地與泥土可鋤。
        assert!(is_tillable(Block::Grass as u8));
        assert!(is_tillable(Block::Dirt as u8));
        // 沙／雪／石／農田土／空氣／水皆不可鋤。
        assert!(!is_tillable(Block::Sand as u8));
        assert!(!is_tillable(Block::Snow as u8));
        assert!(!is_tillable(Block::Stone as u8));
        assert!(!is_tillable(Block::FarmSoil as u8)); // 已是田、不必重鋤
        assert!(!is_tillable(Block::Air as u8));
        assert!(!is_tillable(Block::Water as u8));
    }

    #[test]
    fn is_hoe_only_matches_hoe() {
        assert!(is_hoe(HOE_ID));
        assert!(!is_hoe(Block::Stone as u8));
        assert!(!is_hoe(FARM_SOIL_ID));
        assert!(!is_hoe(0));
    }

    #[test]
    fn name_is_centralized() {
        assert_eq!(hoe_name_zh(HOE_ID), "木鋤頭");
        assert_eq!(hoe_name_zh(99), "鋤頭"); // 未知 id 落回通用稱呼、不 panic
    }

    #[test]
    fn feedback_lines_rotate_and_nonempty() {
        // 台詞非空、確定性輪替、環繞不出界（含大 pick）。
        for p in [0usize, 1, 2, 3, 100, usize::MAX] {
            assert!(!till_ok_line(p).is_empty());
        }
        assert_eq!(till_ok_line(0), till_ok_line(3)); // 3 句一輪
        assert_ne!(till_ok_line(0), till_ok_line(1));
        assert_ne!(till_ok_line(1), till_ok_line(2));
    }
}
