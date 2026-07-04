//! 乙太方界·水桶 v1——舀水、引水，把乾涸之地變成自己的水田（自主提案切片）。
//!
//! **缺口 / 為誰做**：乙太方界早有一套完整的水流動模擬（`voxel_water`：來源水無限、流動水
//! 遞減、離源太遠會乾涸）與鄰水加速種田（水耕 686：農田鄰近水源生長減半）。可玩家**至今
//! 無法自己搬動水**——`voxel::can_place` 明文「不能放水」。想在沙漠／遠離水源的高地種田，
//! 只能守著天上偶爾的雨；一片乾涸之地永遠沒法靠自己的雙手變成水田。這正對著維護者 2026-06-30
//! 的話：「記得操作／合成等系統也要，畢竟**我也想享受這世界**。」
//!
//! 本刀補上經典而缺失的玩家工具——**水桶**：用鐵錠在工作台打一只水桶，走到湖邊／海邊
//! **舀一桶水**，再走到自家乾田邊**倒出一汪水源**。倒下的水是「來源水」（`Block::Water`），
//! 會被既有水流模擬當成永不乾涸的源頭自然漫開、把周圍農地接上水耕加速——採集（鐵）→合成
//! （水桶）→引水灌溉的全新玩家動詞，第一次讓玩家能親手把荒地改造成綠洲水田。
//!
//! **與既有機制的分界（換維度·非同軸重複）**：下雨（700）是**伺服器自發**的暫時天象、鄰水
//! 加速（686）要求農地**恰好生在天然水源旁**、乙太沃肥（789）是消耗品**點對點**催熟一株幼苗；
//! 水桶是玩家**主動搬運水、永久改造地形水文**——把「哪裡有水」這件事第一次交到玩家手裡。
//!
//! **這裡只放確定性純邏輯**（可舀／可倒判定、物品 id 轉換、回饋台詞），零 LLM、零鎖、零 IO、
//! 零 async，可單元測試。連線 / 鎖 / delta 寫入 / 水流 enqueue / 廣播全留在 `voxel_ws.rs`
//! （沿用放置 660／破壞的短鎖循序 + 鎖外 `enqueue_water_around` 慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：不觸發 LLM、不開對外端點、不收玩家自由文字——回饋台詞全為確定性模板；
//! 舀／倒由**伺服器權威**判定（目標方塊型別後端複驗、手持水桶查背包真持有才消耗，前端不自報
//! 合法性）；水本是療癒沙盒的無限資源，無經濟破壞面；不動帳號權限、不破壞玩家資料。

/// 空水桶物品 id（純物品，住背包、不可放置於世界；接續營火 70 之後首個空號）。
/// 用鐵錠在工作台打造；對準水源「使用」＝舀水（→滿水桶）。
pub const BUCKET_ID: u8 = 71;

/// 滿水桶物品 id（純物品；空水桶舀水後變此）。對準空格「使用」＝倒水（→空水桶）。
pub const WATER_BUCKET_ID: u8 = 72;

/// 來源水方塊 id（`Block::Water`）——舀水的合法目標、倒水後放下的方塊。
/// 這裡以常數釘住（純模組不引入 voxel_ws 私有的 voxel_water 語意），另有測試對齊 `Block::Water`。
pub const WATER_SOURCE_ID: u8 = 7;

/// 空氣方塊 id（`Block::Air`）——倒水的合法目標之一。
const AIR_ID: u8 = 0;

/// 流動水方塊 id 範圍（`voxel_water::flow_id(1..=7)` == 24..=30）——倒水可覆蓋既有流動水
/// （把一格弱流動水扶正成永久源頭，比照麥塊放源覆蓋流水）。
const FLOW_ID_MIN: u8 = 24;
const FLOW_ID_MAX: u8 = 30;

/// 這格是不是「可舀的水源」——只認來源水（`Block::Water`）。
/// 流動水（會遞減乾涸的過渡態）不可舀，避免玩家舀到轉瞬即逝的流水、也防無限複製漏洞
/// （舀源→放源守恆；流動水由源頭撐著、不是可搬走的水量）。純函式、可測。
pub fn is_fillable_source(block_id: u8) -> bool {
    block_id == WATER_SOURCE_ID
}

/// 這格能不能被倒進一格新水源——空氣或既有流動水（可被更強的源覆蓋）才行；
/// 實心方塊擋水、既有來源水不必重放。純函式、可測（語意對齊 `voxel_water::WaterState::floodable`）。
pub fn is_pourable_target(block_id: u8) -> bool {
    block_id == AIR_ID || (FLOW_ID_MIN..=FLOW_ID_MAX).contains(&block_id)
}

/// 空水桶舀水後應變成的物品 id（空→滿）。只有手持空水桶才回 Some，防止亂用。
pub fn fill_result_item(held_id: u8) -> Option<u8> {
    (held_id == BUCKET_ID).then_some(WATER_BUCKET_ID)
}

/// 滿水桶倒水後應變成的物品 id（滿→空）。只有手持滿水桶才回 Some。
pub fn pour_result_item(held_id: u8) -> Option<u8> {
    (held_id == WATER_BUCKET_ID).then_some(BUCKET_ID)
}

/// 面向玩家的水桶名稱（i18n 集中管理）。
pub fn bucket_name_zh(id: u8) -> &'static str {
    match id {
        BUCKET_ID => "水桶",
        WATER_BUCKET_ID => "滿水桶",
        _ => "水桶",
    }
}

/// 舀水成功的回饋句（3 句輪替，確定性）。
pub fn fill_ok_line(pick: usize) -> &'static str {
    const LINES: [&str; 3] = [
        "🪣 舀了一桶清水～",
        "🪣 滿滿一桶水，沉甸甸的～",
        "🪣 打了一桶水，去給乾田引水吧～",
    ];
    LINES[pick % LINES.len()]
}

/// 倒水成功的回饋句（3 句輪替，確定性）。
pub fn pour_ok_line(pick: usize) -> &'static str {
    const LINES: [&str; 3] = [
        "💧 倒出一汪水，順著地勢流淌開來～",
        "💧 水源落地，乾涸的土地潤了起來～",
        "💧 引來一泓活水，田邊有水了～",
    ];
    LINES[pick % LINES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::Block;

    #[test]
    fn ids_are_distinct_and_align_with_blocks() {
        // 水桶兩態互不相同，且與其他純物品／方塊 id 不撞（71/72 是營火 70 之後的空號）。
        assert_ne!(BUCKET_ID, WATER_BUCKET_ID);
        assert_eq!(BUCKET_ID, 71);
        assert_eq!(WATER_BUCKET_ID, 72);
        // 來源水常數必須對齊 voxel.rs 的真相（Block::Water）。
        assert_eq!(WATER_SOURCE_ID, Block::Water as u8);
        assert_eq!(AIR_ID, Block::Air as u8);
        // 水桶物品 id 不可有對應方塊（純物品，Block::from_u8 應為 None）。
        assert!(Block::from_u8(BUCKET_ID).is_none());
        assert!(Block::from_u8(WATER_BUCKET_ID).is_none());
    }

    #[test]
    fn only_source_water_is_fillable() {
        assert!(is_fillable_source(WATER_SOURCE_ID)); // 來源水可舀
        // 流動水（24..=30）不可舀——會乾涸的過渡態，不是可搬走的水量。
        for id in FLOW_ID_MIN..=FLOW_ID_MAX {
            assert!(!is_fillable_source(id), "流動水 id={id} 不該可舀");
        }
        // 空氣／實心皆不可舀。
        assert!(!is_fillable_source(AIR_ID));
        assert!(!is_fillable_source(Block::Stone as u8));
        assert!(!is_fillable_source(Block::Grass as u8));
    }

    #[test]
    fn only_air_or_flow_is_pourable() {
        assert!(is_pourable_target(AIR_ID)); // 空氣可倒
        for id in FLOW_ID_MIN..=FLOW_ID_MAX {
            assert!(is_pourable_target(id), "流動水 id={id} 應可被源覆蓋");
        }
        // 來源水不必重放（已是源）；實心方塊擋水不可倒。
        assert!(!is_pourable_target(WATER_SOURCE_ID));
        assert!(!is_pourable_target(Block::Stone as u8));
        assert!(!is_pourable_target(Block::Dirt as u8));
    }

    #[test]
    fn item_transitions_only_apply_to_matching_bucket() {
        // 空水桶舀水 → 滿水桶；其他手持物一律 None（不能拿石頭去「舀水」）。
        assert_eq!(fill_result_item(BUCKET_ID), Some(WATER_BUCKET_ID));
        assert_eq!(fill_result_item(WATER_BUCKET_ID), None);
        assert_eq!(fill_result_item(Block::Stone as u8), None);
        // 滿水桶倒水 → 空水桶；空水桶不能「倒水」。
        assert_eq!(pour_result_item(WATER_BUCKET_ID), Some(BUCKET_ID));
        assert_eq!(pour_result_item(BUCKET_ID), None);
        assert_eq!(pour_result_item(0), None);
    }

    #[test]
    fn feedback_lines_rotate_and_nonempty() {
        // 台詞非空、確定性輪替、環繞不出界（含大 pick）。
        for p in [0usize, 1, 2, 3, 100, usize::MAX] {
            assert!(!fill_ok_line(p).is_empty());
            assert!(!pour_ok_line(p).is_empty());
        }
        assert_eq!(fill_ok_line(0), fill_ok_line(3)); // 3 句一輪
        assert_ne!(fill_ok_line(0), fill_ok_line(1));
        assert_ne!(pour_ok_line(0), pour_ok_line(1));
    }

    #[test]
    fn names_are_centralized() {
        assert_eq!(bucket_name_zh(BUCKET_ID), "水桶");
        assert_eq!(bucket_name_zh(WATER_BUCKET_ID), "滿水桶");
        assert_eq!(bucket_name_zh(99), "水桶"); // 未知 id 落回通用稱呼、不 panic
    }
}
