//! 乙太方界·季節輪替 v1（ROADMAP 798）。
//!
//! 世界至今的環境維度只有「晝夜」（`voxel_time`）與「晴／雨／彩虹」（`voxel_weather`）——
//! 兩者都在一日之內循環，世界隔了幾天回來看仍是同一副模樣，沒有「季節流轉」這條更長的時間感。
//! 本模組補上乙太方界第一條**跨日的季節循環**：世界時鐘累計的「遊戲日數」決定當前季節
//! （春 → 夏 → 秋 → 冬 循環），前端據季節為整片天地微微換上不同色調，居民在**換季那一刻**
//! 抬頭感到季節更迭、心情微亮、並把這一刻記進城鎮動態——季節不只是背景色，而是居民生活與
//! 世界故事的一部分（PLAN_ETHERVOX 北極星「日記／生命故事」：不在線上的玩家回來，也讀得到
//! 「秋天到了，露娜看著飄落的葉子出了神」）。
//!
//! **與既有天氣的定位區隔**：晝夜（`voxel_time`）是一日之內的光影循環、天氣（`voxel_weather`）
//! 是分鐘級的晴雨與雨後彩虹（一日內反覆），本模組是**以「遊戲日」為單位、四季循環的更長時間軸**
//! ——三者疊在一起，世界才有了從「此刻」到「今日」再到「這個季節」的完整時間層次。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式（季節推算、換季台詞、
//! 換季心情、Feed 摘要）；日數偵測、換季旗標、居民反應、Feed 廣播、前端染色都在 `voxel_ws.rs`
//! / 前端（沿用下雨反應 701 / 雨後彩虹 780 的短鎖循序手法）。

use serde::{Deserialize, Serialize};

/// 每個季節持續幾個「遊戲日」（一遊戲日 = [`crate::voxel_time::DAY_DURATION_SECS`] = 600 秒
/// = 10 分鐘真實時間）。取 2 日：每季約 20 分鐘、一整年（四季）約 80 分鐘——長到有「季節流轉」
/// 的餘韻、又短到玩家單場遊玩就有機會親眼撞見一次換季。
pub const DAYS_PER_SEASON: u64 = 2;

/// 一年四季（順序即輪替順序：春 → 夏 → 秋 → 冬 → 春…）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    /// 面向玩家的季節名（繁中；i18n 友善，集中此處統一替換）。
    pub fn display_name(self) -> &'static str {
        match self {
            Season::Spring => "春天",
            Season::Summer => "夏天",
            Season::Autumn => "秋天",
            Season::Winter => "冬天",
        }
    }

    /// 前端識別用的穩定字串鍵（隨快照廣播、不面向玩家、不翻譯——前端據此挑季節色調）。
    pub fn as_str(self) -> &'static str {
        match self {
            Season::Spring => "spring",
            Season::Summer => "summer",
            Season::Autumn => "autumn",
            Season::Winter => "winter",
        }
    }
}

/// 依世界累計遊戲日數，回傳當前季節。
///
/// day 0 = 初春；每過 [`DAYS_PER_SEASON`] 個遊戲日進一季，四季循環（春→夏→秋→冬→春…）。
/// 確定性純函式：同一日數永遠得同一季節，可窮舉測試。
pub fn season_for_day(day: u64) -> Season {
    match (day / DAYS_PER_SEASON) % 4 {
        0 => Season::Spring,
        1 => Season::Summer,
        2 => Season::Autumn,
        _ => Season::Winter,
    }
}

/// 換季那一刻，附近醒著的居民抬頭感到季節更迭、隨機冒出的應景台詞池（確定性選句、零 LLM）。
/// 每季 4 句，語氣貼合療癒世界（都是溫柔而帶著期待／感懷的一句）。
const SPRING_LINES: [&str; 4] = [
    "春天來了，你看，芽都冒出來了呢。",
    "空氣裡有股新泥土的味道，是春天到了。",
    "冰雪化了，萬物都醒過來了。",
    "春天了，該去田裡忙活了。",
];
const SUMMER_LINES: [&str; 4] = [
    "夏天到了，日頭曬得暖洋洋的。",
    "蟬叫起來了，是夏天了呢。",
    "這麼熱的天，正好去溪邊乘個涼。",
    "夏天了，作物長得可歡快了。",
];
const AUTUMN_LINES: [&str; 4] = [
    "秋天到了，你聞，風裡都是熟穀子的香。",
    "葉子開始黃了，秋天悄悄來了。",
    "秋高氣爽，是收成的好時節。",
    "又到秋天了，天涼了，記得多添件衣裳。",
];
const WINTER_LINES: [&str; 4] = [
    "冬天來了，第一場雪快要下了吧。",
    "天冷了，好想圍著火堆烤烤手。",
    "冬天到了，萬物都靜下來歇息了。",
    "冷歸冷，圍在一起就暖和了。",
];

/// 依季節 + `pick` 選一句換季台詞（`pick % len`，永遠有值、確定性、可測）。
/// `pick` 由呼叫端以居民位置雜湊等餵入（比照雨天／彩虹反應）。
pub fn season_turn_line(season: Season, pick: usize) -> &'static str {
    let pool: &[&str; 4] = match season {
        Season::Spring => &SPRING_LINES,
        Season::Summer => &SUMMER_LINES,
        Season::Autumn => &AUTUMN_LINES,
        Season::Winter => &WINTER_LINES,
    };
    pool[pick % pool.len()]
}

/// 城鎮動態 Feed 上「換季」那一則的摘要（不在線上的玩家回來讀得到世界換了季）。
/// 確定性、面向玩家、i18n 友善。
pub fn season_feed_detail(season: Season) -> &'static str {
    match season {
        Season::Spring => "春天來了，萬物復甦。",
        Season::Summer => "夏天到了，綠意正濃。",
        Season::Autumn => "秋天到了，落葉紛紛。",
        Season::Winter => "冬天來了，天地靜謐。",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_zero_is_spring() {
        assert_eq!(season_for_day(0), Season::Spring);
    }

    #[test]
    fn seasons_cycle_in_order() {
        // 每 DAYS_PER_SEASON 日進一季，四季循環。逐日檢查頭一整年 + 跨年回春。
        let expected = [
            Season::Spring,
            Season::Summer,
            Season::Autumn,
            Season::Winter,
        ];
        for day in 0..(DAYS_PER_SEASON * 4 * 3) {
            let idx = ((day / DAYS_PER_SEASON) % 4) as usize;
            assert_eq!(
                season_for_day(day),
                expected[idx],
                "第 {day} 日季節不符"
            );
        }
    }

    #[test]
    fn season_persists_within_its_span() {
        // 同一季內每一日都是同一季節（不會日日跳季）。
        for base in 0..4 {
            let start = base * DAYS_PER_SEASON;
            let s = season_for_day(start);
            for d in start..(start + DAYS_PER_SEASON) {
                assert_eq!(season_for_day(d), s, "第 {d} 日不該換季");
            }
            // 跨過該季末日就換季。
            assert_ne!(season_for_day(start + DAYS_PER_SEASON), s);
        }
    }

    #[test]
    fn turn_lines_non_empty_and_deterministic() {
        for &s in &[Season::Spring, Season::Summer, Season::Autumn, Season::Winter] {
            for pick in 0..12usize {
                let line = season_turn_line(s, pick);
                assert!(!line.is_empty(), "{s:?} 換季台詞不該為空");
                // 確定性：同輸入同輸出；pick 以 4 為週期輪替。
                assert_eq!(line, season_turn_line(s, pick + 4));
            }
        }
    }

    #[test]
    fn each_season_has_distinct_key_and_name() {
        let seasons = [Season::Spring, Season::Summer, Season::Autumn, Season::Winter];
        for (i, &a) in seasons.iter().enumerate() {
            assert!(!a.display_name().is_empty());
            assert!(!a.as_str().is_empty());
            assert!(!season_feed_detail(a).is_empty());
            for &b in &seasons[i + 1..] {
                assert_ne!(a.as_str(), b.as_str(), "季節鍵不可重複");
                assert_ne!(a.display_name(), b.display_name(), "季節名不可重複");
            }
        }
    }
}
