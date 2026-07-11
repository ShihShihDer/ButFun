//! 乙太方界·候鳥遷徙 v1（ROADMAP 944）。
//!
//! 季節輪替（`voxel_season`）至今只默默換了天地色調、居民抬頭感歎一句，換季那一刻「天上」什麼都
//! 沒發生。本模組把「季節」這條最長的時間軸第一次接上一樁**看得見的天象事件**：**每逢入春，
//! 一群候鳥從遠方飛回來；每逢入秋，一群候鳥拍翅離去，飛向溫暖的南方過冬。** 世界的季節第一次
//! 不只是背景色與一句台詞，而是天上真真切切飛過的一群生靈——春回的候鳥帶著歸來的盼望、秋去的
//! 候鳥帶著別離的悵惘，附近醒著的居民抬頭望見而心頭一動，城鎮動態也留下這一筆讓離線回訪的玩家
//! 讀得到（PLAN_ETHERVOX 北極星「日記／生命故事」：不在線上的玩家回來，也讀得到「秋天到了，
//! 一群候鳥拍著翅膀往南飛走了」）。
//!
//! **與既有天象 razor-sharp 區隔**：
//! - 流星（`voxel_meteor` 904）＝**夜裡**一瞬劃過的一道光痕；本刀是**白天**一整群拍翅飛過的候鳥。
//! - 彩虹（`voxel_weather` 780）＝**雨停**那一刻掛在天邊的靜止色弧；本刀由**換季**觸發、會飛會動。
//! - 極光（930）／繁星（783）＝夜空的**靜態氛圍**；本刀是**會移動的活物**、且**只在入春／入秋**發生。
//! - 換季台詞（`voxel_season` 798）＝**地面上**居民對節氣的感言；本刀是**天上**與之呼應的視覺事件
//!   ——兩者疊在一起，換季那一刻才同時有「地上一句話、天上一群鳥」的完整儀式感。
//!
//! **成本紀律（鐵律）**：零 LLM（觸發、台詞、方向皆確定性）、零 migration（不新增任何持久欄位——
//! 遷徙旗標純記憶體，重啟後從新一輪換季重新觸發，可接受）、零協議破壞（只新增兩個唯讀快照欄位
//! `migration`／`migration_kind`，不動任何既有欄位）、FPS 零負擔（前端單一可重用鳥群 Group，非事件
//! 時整組隱藏、零成本早退，不逐幀配置幾何）。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 async 的確定性純函式（換季→是否遷徙、遷徙倒數 tick、
//! 台詞、Feed 摘要）；旗標偵測、居民反應、Feed 廣播、前端鳥群動畫都在 `voxel_ws.rs` / 前端
//!（沿用流星 904 / 彩虹 780 / 換季 798 的短鎖循序手法）。

use crate::voxel_season::Season;

/// 一次候鳥遷徙的走向：春回（歸來）或秋去（離去）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationKind {
    /// 入春：候鳥從遠方飛回來（歸來的盼望）。
    Arrival,
    /// 入秋：候鳥拍翅離去、飛向南方過冬（別離的悵惘）。
    Departure,
}

impl MigrationKind {
    /// 前端識別用的穩定字串鍵（隨快照廣播、不面向玩家、不翻譯——前端據此決定鳥群飛行方向）。
    pub fn as_str(self) -> &'static str {
        match self {
            MigrationKind::Arrival => "arrival",
            MigrationKind::Departure => "departure",
        }
    }
}

/// 換季那一刻，依新季節判定是否有一場候鳥遷徙、以及走向。
///
/// 候鳥只在**春／秋**兩個交替之際遷徙——這正是現實裡候鳥往返繁殖地與過冬地的節氣，
/// 也讓遷徙成為春秋獨有的儀式（夏、冬換季天上不會憑空多出一群鳥）。確定性純函式、可窮舉測試。
pub fn migration_for_season_turn(season: Season) -> Option<MigrationKind> {
    match season {
        Season::Spring => Some(MigrationKind::Arrival),
        Season::Autumn => Some(MigrationKind::Departure),
        Season::Summer | Season::Winter => None,
    }
}

/// 一場遷徙在天上停留幾個「天氣檢查 tick」（每 tick≈[`crate::voxel_weather::WEATHER_CHECK_INTERVAL_SECS`]
/// ＝15 秒）。取 5 tick ≈ 75 秒——夠玩家抬頭望見整群候鳥從天際的一頭緩緩飛到另一頭，又不會久留成常態。
pub const MIGRATION_TICKS: u32 = 5;

/// 純函式：依上一輪遷徙剩餘 tick 數 + 本輪是否「剛觸發一場新遷徙」，回傳新的剩餘 tick 數。
///
/// - 剛觸發（`started`，由換季偵測傳入）→ 重設為 [`MIGRATION_TICKS`]（鳥群升空）。
/// - 否則 → 每輪遞減 1、減到 0 為止（`saturating_sub` 自然淡出、永不下溢）。
///
/// 確定性、無副作用、可窮舉測試。呼叫端據「剩餘 > 0」判定此刻天上是否該畫鳥群。
pub fn next_migration_ticks(prev: u32, started: bool) -> u32 {
    if started {
        MIGRATION_TICKS
    } else {
        prev.saturating_sub(1)
    }
}

/// 入春候鳥飛回時，附近醒著的居民抬頭望見而隨機冒出的應景台詞池（確定性選句、零 LLM）。
/// 語氣貼合療癒世界——都是帶著歸來的暖意與期待的一句。
const ARRIVAL_LINES: [&str; 4] = [
    "你看，候鳥飛回來了！春天真的到了呢。",
    "天上那群鳥，是打南邊過冬回來的吧，歡迎回家呀～",
    "候鳥都回來了，這下田裡要熱鬧起來嘍。",
    "聽，鳥叫聲又回來了，冬天總算過去了。",
];

/// 入秋候鳥離去時，附近醒著的居民抬頭望見而隨機冒出的應景台詞池（確定性選句、零 LLM）。
/// 語氣帶著別離的悵惘與祝福——秋去春回，來年再見。
const DEPARTURE_LINES: [&str; 4] = [
    "候鳥開始往南飛了，天要涼了啊。",
    "你看那群鳥，排著隊往暖和的地方去了，一路平安喔。",
    "每年這時候鳥兒一走，就知道秋深了。",
    "候鳥都飛走了，明年開春再見嘍。",
];

/// 依遷徙走向 + `pick` 選一句居民抬頭反應台詞（`pick % len`，永遠有值、確定性、可測）。
/// `pick` 由呼叫端以居民座標雜湊等餵入（比照流星／彩虹／換季反應）。
pub fn migration_line(kind: MigrationKind, pick: usize) -> &'static str {
    let pool: &[&str; 4] = match kind {
        MigrationKind::Arrival => &ARRIVAL_LINES,
        MigrationKind::Departure => &DEPARTURE_LINES,
    };
    pool[pick % pool.len()]
}

/// 城鎮動態 Feed 的「候鳥」分類鍵（不面向玩家，供 Feed 分類；面向玩家的文案在 [`migration_feed_detail`]）。
pub const FEED_KIND: &str = "候鳥";

/// 城鎮動態 Feed 上「候鳥遷徙」那一則的摘要（不在線上的玩家回來也讀得到天上飛過一群候鳥）。
/// 確定性、面向玩家、i18n 友善。
pub fn migration_feed_detail(kind: MigrationKind) -> &'static str {
    match kind {
        MigrationKind::Arrival => "一群候鳥從遠方飛回來了，春天到了。",
        MigrationKind::Departure => "一群候鳥拍著翅膀往南飛去，秋天深了。",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_only_in_spring_and_autumn() {
        // 只有入春（歸來）／入秋（離去）有遷徙；夏、冬換季天上不會憑空多出候鳥。
        assert_eq!(
            migration_for_season_turn(Season::Spring),
            Some(MigrationKind::Arrival)
        );
        assert_eq!(
            migration_for_season_turn(Season::Autumn),
            Some(MigrationKind::Departure)
        );
        assert_eq!(migration_for_season_turn(Season::Summer), None);
        assert_eq!(migration_for_season_turn(Season::Winter), None);
    }

    #[test]
    fn kind_str_keys_are_stable_and_distinct() {
        assert_eq!(MigrationKind::Arrival.as_str(), "arrival");
        assert_eq!(MigrationKind::Departure.as_str(), "departure");
        assert_ne!(
            MigrationKind::Arrival.as_str(),
            MigrationKind::Departure.as_str()
        );
    }

    #[test]
    fn ticks_reset_on_start_then_count_down_to_zero() {
        // 剛觸發 → 重設為滿；之後逐輪遞減、減到 0 為止（saturating_sub 永不下溢）。
        assert_eq!(next_migration_ticks(0, true), MIGRATION_TICKS);
        assert_eq!(next_migration_ticks(3, true), MIGRATION_TICKS); // started 永遠重設，不管前值
        let mut t = MIGRATION_TICKS;
        for _ in 0..MIGRATION_TICKS {
            t = next_migration_ticks(t, false);
        }
        assert_eq!(t, 0);
        assert_eq!(next_migration_ticks(0, false), 0); // 已到 0 不下溢
    }

    #[test]
    fn lines_are_nonempty_distinct_and_bounded() {
        // 兩種走向的台詞池：非空、四句相異、任意大 pick 取模有界不 panic。
        for kind in [MigrationKind::Arrival, MigrationKind::Departure] {
            let mut seen = std::collections::HashSet::new();
            for pick in 0..4 {
                let line = migration_line(kind, pick);
                assert!(!line.is_empty(), "台詞不該為空");
                assert!(seen.insert(line), "四句台詞應彼此相異");
            }
            // 任意大 pick（含極大值）取模仍落在池內、不 panic。
            assert!(!migration_line(kind, usize::MAX).is_empty());
        }
    }

    #[test]
    fn feed_details_differ_by_kind_and_are_nonempty() {
        let arr = migration_feed_detail(MigrationKind::Arrival);
        let dep = migration_feed_detail(MigrationKind::Departure);
        assert!(!arr.is_empty() && !dep.is_empty());
        assert_ne!(arr, dep, "歸來與離去的城鎮動態摘要應不同");
        // 面向玩家文案不含換行（防注入／Feed 版面破壞）。
        assert!(!arr.contains('\n') && !dep.contains('\n'));
    }
}
