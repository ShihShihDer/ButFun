//! 乙太方界·沉船地標 v1（自主提案切片，接續 1005 世界河流／1006 世界第一座橋／1017 木筏）。
//!
//! **真缺口**：河流（1005）與橋（1006）讓世界第一次有一條值得渡的水系，木筏（1017）讓渡河
//! 有了第二種代步工具——但這條河至今純粹是「路過的地形」：沒有任何一處目的地要求玩家
//! 「先下水」才碰得到。既有六種地標（古代遺跡／溫泉／邊陲營地／野外殖民地／世界樹／地底
//! 遺跡神殿）全部落在乾燥陸地或深層岩壁。本刀在河流固定一處深水核心埋一艘沉船殘骸：全
//! 世界唯一一處「非潛水／乘筏摸不到」的地標，讓河流本身、以及剛拿到的木筏／既有的游泳，
//! 第一次有了「為什麼要去」的理由。
//!
//! **做法**：純地形層（[`crate::voxel::shipwreck_block_at`]，走 `block_at` 既有 world gen
//! 管線）確定性在河流深水核心生成破損甲板＋核心遺物；本模組定義「找到核心後你會得到什麼」
//! 與「世界怎麼讓大家知道這一刻很特別」——刻意與地形生成脫鉤（比照 `voxel_dungeon` 分層
//! 慣例），give 動作、里程碑解鎖、探索紀事、Feed 廣播全在 `voxel_ws.rs`（守鎖/IO 紀律），
//! 本模組零 IO、零鎖、零 LLM、零 async，全確定性純函式，可單元測試。
//!
//! **與既有地標 razor-sharp 區隔**：地表遺跡（838）一望即穿；溫泉（839）泡進去就有功能性
//! 回饋；地底遺跡神殿（975）藏在深層岩壁得先挖穿石牆。本刀是**唯一要求先涉水／潛泳／乘筏
//! 才碰得到**的地標——水域探索這個維度第一次有了明確目的地，非彙整清單、非居民行為。
//!
//! **獎勵定調**：比地底遺跡神殿（20 枚，得先深挖遠征）略少，比天然驚喜（深層寶藏 3 枚）
//! 豐厚許多——沉船只需游泳／乘筏抵達即可，難度介於兩者之間，獎勵量隨之落在中段。

use crate::voxel_craft::COIN_ID;

/// 沉船地標里程碑 id（`voxel_milestones::MILESTONES` 已登記）。
pub const MILESTONE_ID: &str = "first_shipwreck";

/// 找到核心遺物的獎勵——固定內容（非隨機挑選）。
pub const RELIC_COIN_COUNT: u32 = 12;

/// 找到沉船核心後給予的物品與數量。
pub fn relic_reward() -> (u8, u32) {
    (COIN_ID, RELIC_COIN_COUNT)
}

/// 游近核心遺物那一刻，玩家收到的單播提示句。
pub fn relic_discovered_line() -> String {
    format!(
        "你潛下河底，眼前赫然是一艘早已解體的沉船殘骸——斷裂的甲板半埋在泥沙裡，\
         船脊間夾著一顆靜靜發光的核心遺物。你小心翼翼地取下它，得到 {RELIC_COIN_COUNT} 枚乙太幣。"
    )
}

/// 動態 Feed 事件細節文字（比照探索紀事/地標旅人留言慣例，讓世界看見這一刻）——
/// 面向玩家字串，繁中、i18n 預留點集中於此。
pub fn relic_feed_detail() -> String {
    "潛進河流深處，摸到了一艘早已解體的沉船殘骸——世界第一處藏在水下的地標，就此被人找到了。"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relic_reward_gives_coins() {
        let (id, count) = relic_reward();
        assert_eq!(id, COIN_ID, "沉船核心獎勵應為乙太幣");
        assert!(count >= 1, "沉船核心獎勵數量應至少 1 枚");
        assert_eq!(count, RELIC_COIN_COUNT);
    }

    #[test]
    fn relic_reward_between_ordinary_treasure_and_dungeon() {
        // 難度定調：游泳/乘筏可及，比深挖遠征（地底遺跡神殿）易，比天然巧遇的深層寶藏難，
        // 獎勵量理應落在兩者之間。
        assert!(RELIC_COIN_COUNT > crate::voxel_treasure::TREASURE_COIN_COUNT);
        assert!(RELIC_COIN_COUNT < crate::voxel_dungeon::RELIC_COIN_COUNT);
    }

    #[test]
    fn relic_discovered_line_is_nonempty_and_mentions_coin_count() {
        let line = relic_discovered_line();
        assert!(!line.is_empty());
        assert!(line.contains(&RELIC_COIN_COUNT.to_string()), "提示應點名獲得數量：{line}");
    }

    #[test]
    fn relic_feed_detail_is_nonempty_and_mentions_shipwreck() {
        let detail = relic_feed_detail();
        assert!(!detail.is_empty());
        assert!(detail.contains("沉船"));
    }
}
