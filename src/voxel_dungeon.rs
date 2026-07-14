//! 乙太方界·地底遺跡神殿 v1（ROADMAP 975，自主提案切片）。
//!
//! **真缺口**：review 在 974（遠征首領）merge 時明令「974 這條線到此收尾，別再往首領上疊
//! 第 N 層縫隙，要的是玩家一眼看得出『世界又長大了一塊』的東西」——地底至今只有「天然」的
//! 東西：深層礦石（682）是挖了進背包的資源、天然洞穴腔室（934）是隨機雕空的岩壁綴發光結晶。
//! 世界從沒有一處**人工鑿建**的地底空間：先民砌起磚牆、留了通道、藏了寶物的密室。這是
//! 「垂直/室內探索」這個維度第一次被打開，與既有地標系統（地表遺跡 838、天然洞穴 934、
//! 世界奇觀 940）皆不同軸——不是格狀重複、不是隨機腔室，是**全世界唯一一座、規則的房間＋
//! 通道**，得先挖穿深層石牆才看得見裡頭。
//!
//! **做法**：純地形層（[`crate::voxel::dungeon_block_at`]，走 `block_at` 既有 world gen
//! 管線）確定性生成入口室—通道—藏寶室；本模組定義「找到藏寶室核心後你會得到什麼」與
//! 「世界怎麼讓大家知道這一刻很特別」——刻意與地形生成脫鉤（比照 [`crate::voxel_treasure`]
//! 深層寶藏的分層慣例），give 動作、里程碑解鎖、探索紀事、Feed 廣播全在 `voxel_ws.rs`
//! （守鎖/IO 紀律），本模組零 IO、零鎖、零 LLM、零 async，全確定性純函式，可單元測試。
//!
//! **與 838/934/940 razor-sharp 區隔**：地表遺跡是一眼望穿的四根裸露殘柱；天然洞穴是隨機
//! 雕空、不規則的腔室；世界奇觀是地表拔天而起、遠遠可見的天然巨物。本刀是**藏在深層岩壁
//! 裡、得先挖穿石牆才看得見**的規則房間＋通道——垂直/室內探索空間感是世界第一次擁有。

use crate::voxel_craft::COIN_ID;

/// 遺跡神殿里程碑 id（`voxel_milestones::MILESTONES` 已登記）。
pub const MILESTONE_ID: &str = "first_dungeon";

/// 找到藏寶室核心的獎勵——固定內容（非隨機挑選），遠高於深層寶藏（790/3枚）的隨機驚喜，
/// 值回一趟深挖遠征的一次性豐厚回報（全世界唯一一座，每位玩家只能領一次）。
pub const RELIC_COIN_COUNT: u32 = 20;

/// 找到遺跡核心後給予的物品與數量。
pub fn relic_reward() -> (u8, u32) {
    (COIN_ID, RELIC_COIN_COUNT)
}

/// 抵達藏寶室核心那一刻，玩家收到的單播提示句。
pub fn relic_discovered_line() -> String {
    format!(
        "你挖穿了深層的一整片磚牆，眼前赫然是一座人工鑿建的地底密室——遺跡核心靜靜發著幽光，\
         周圍散落著先民留下的痕跡。你小心翼翼地取下核心，得到 {RELIC_COIN_COUNT} 枚乙太幣。"
    )
}

/// 動態 Feed 事件細節文字（比照探索紀事/地標旅人留言慣例，讓世界看見這一刻）——
/// 面向玩家字串，繁中、i18n 預留點集中於此。
pub fn relic_feed_detail() -> String {
    "一路往深處挖，鑿穿了一整片石牆，撞見一座人工鑿建的地底遺跡神殿——世界第一座".to_string()
        + "藏在岩壁裡的密室，就此被人找到了。"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relic_reward_gives_coins() {
        let (id, count) = relic_reward();
        assert_eq!(id, COIN_ID, "遺跡核心獎勵應為乙太幣");
        assert!(count >= 1, "遺跡核心獎勵數量應至少 1 枚");
        assert_eq!(count, RELIC_COIN_COUNT);
    }

    #[test]
    fn relic_reward_more_generous_than_ordinary_mining_treasure() {
        // 深層寶藏（voxel_treasure）是隨機巧遇的小驚喜；遺跡核心是全世界唯一一次性目標，
        // 獎勵理應更豐厚才值回一趟專程深挖遠征。
        assert!(RELIC_COIN_COUNT > crate::voxel_treasure::TREASURE_COIN_COUNT);
    }

    #[test]
    fn relic_discovered_line_is_nonempty_and_mentions_coin_count() {
        let line = relic_discovered_line();
        assert!(!line.is_empty());
        assert!(line.contains(&RELIC_COIN_COUNT.to_string()), "提示應點名獲得數量：{line}");
    }

    #[test]
    fn relic_feed_detail_is_nonempty_and_mentions_dungeon() {
        let detail = relic_feed_detail();
        assert!(!detail.is_empty());
        assert!(detail.contains("遺跡"));
    }
}
