//! 乙太方界·深層寶藏 v1（自主提案切片）。
//!
//! **真缺口**：790「工欲善其事」讓手持對的工具挖礦時多掉一份**同一種**素材，是「值得帶
//! 工具」的小確幸，但挖礦本身從沒有過真正「巧遇驚喜」的一刻——每一塊礦看起來都一樣，
//! 挖到底了也不會有什麼特別。873/874 剛讓乙太幣成為玩家↔玩家、玩家↔居民的通用貨幣，
//! 但幣至今只有一種來源：拿原礦去「鑄」。世界裡從沒有「挖到的」貨幣——探勘本該偶爾就有
//! 意外的回報。
//!
//! **做法**：[`crate::voxel::treasure_ore_at`]（純地形層，確定性）判定某個原生礦脈座標
//! 是否祕藏寶藏；本模組定義「挖到寶藏後你會得到什麼」與「世界怎麼讓大家知道這一刻很
//! 特別」——刻意與礦石本身脫鉤（固定豐厚獎勵，不看挖到的是煤/鐵/乙太礦哪一種），give
//! 動作、里程碑解鎖、Feed 廣播全在 `voxel_ws.rs`（守鎖/IO 紀律），本模組零 IO、零鎖、
//! 零 LLM、零 async，全確定性純函式，可單元測試。

use crate::voxel_craft::COIN_ID;

/// 寶藏獎勵——固定內容（非隨機挑選），一次挖到就是實打實一筆意外之財，值得停下歡呼。
pub const TREASURE_COIN_COUNT: u32 = 3;

/// 挖到寶藏後給予的物品與數量。
pub fn treasure_reward() -> (u8, u32) {
    (COIN_ID, TREASURE_COIN_COUNT)
}

/// 動態 Feed 事件細節文字（比照探索紀事/地標旅人留言慣例，讓世界看見這一刻）——
/// 面向玩家字串，繁中、i18n 預留點集中於此。
pub fn treasure_feed_detail() -> String {
    format!("在地底深處挖到了一座意外的寶藏，多得 {TREASURE_COIN_COUNT} 枚乙太幣！")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn treasure_reward_gives_coins() {
        let (id, count) = treasure_reward();
        assert_eq!(id, COIN_ID, "寶藏獎勵應為乙太幣");
        assert!(count >= 1, "寶藏獎勵數量應至少 1 枚");
        assert_eq!(count, TREASURE_COIN_COUNT);
    }

    #[test]
    fn treasure_feed_detail_is_non_empty_and_mentions_coin_count() {
        let detail = treasure_feed_detail();
        assert!(!detail.is_empty());
        assert!(detail.contains(&TREASURE_COIN_COUNT.to_string()), "細節應點名獲得數量：{detail}");
        assert!(detail.contains("寶藏"));
    }
}
