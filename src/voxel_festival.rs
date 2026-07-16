//! 乙太方界·村莊慶典日 v1——世界第一次有了固定的節慶節奏🎉（自主提案切片，ROADMAP 1022；
//! 接續 1017 木筏／1019 沉船地標／1021 騎乘，reviewer 明令「下一刀挑真正不同維度：新玩法動詞／
//! 新地圖區域／或更內聚大塊功能，別再『天氣×下一種生物』」）。
//!
//! **這一刀補的缺口**：世界至今每一場「特別的日子」都是**個體事件**——居民各自的生日
//! （`voxel_birthday`）、婚禮（`voxel_wedding`）、成年禮、圓夢……全部繫在某一位居民身上，
//! 沒發生在誰身上那天就跟平常沒兩樣。玩家隨時可發起的手風琴表演（`voxel_busking`）自己的
//! 頭註解就明講：世界從沒有「集體慶祝」——煙火/生日都是事件驅動的一次性慶典，沒有任何**排定
//! 好、全村共同、一再循環**的節日。本刀補上：世界時鐘每隔固定天數迎來一個「慶典日」，那一整天
//! 閒著的居民會比平常更容易顯露歡慶的心情，讓世界第一次有了屬於**全體**、而非某一位居民的
//! 節慶節奏。
//!
//! **與既有元素 razor-sharp 區隔**：
//! - 不是居民生日（`voxel_birthday`）——那綁定**單一居民**的個人紀念日；慶典日是**全村共同**、
//!   與任何個人身分無關的排定行事曆。
//! - 不是手風琴表演（`voxel_busking`）——那是玩家**隨時可發起**的日常表演，沒有節慶前提；
//!   慶典日是**世界時鐘自己排定**、不需要玩家在場觸發的節奏。
//! - 不是季節輪替（`voxel_season`）——換季是**環境**染色/氣候轉變；慶典日疊加在季節之上，
//!   是**社交/心情**維度的節奏，兩者各自獨立（同一天可能既是換季也是慶典，也可能都不是）。
//!
//! **純函式層**：本模組只有確定性純函式（排程判定、機率門檻、台詞池），零 LLM、零鎖、零 IO、
//! 零 async，可單元測試。世界時鐘讀取／居民冷卻遞減／Feed 落地全留在 `voxel_ws.rs`（比照
//! `voxel_season`/`voxel_humming` 的「純函式 + tick 呼叫端組裝」慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：零 LLM（歡慶台詞/動態牆文案全確定性模板）、不收玩家輸入、不開對外
//! 端點、不動帳號權限——排程完全由伺服器內部世界時鐘決定，玩家無從自報或催發；居民歡慶泡泡
//! 走與哼歌相同的「機率 + 長冷卻」節流，狂刷不了畫面。

/// 慶典日的排程間隔（世界日）：世界時鐘每累積這麼多天，其中一天是慶典日。世界一天現行
/// 600 秒（見 `voxel_time::DAY_DURATION_SECS`），故約每 1 小時真實時間迎來一次慶典日，
/// 頻率介於「季節輪替（每 2 天）太密」與「居民生日（一年一輪）太疏」之間，是專屬於
/// **社交節奏**、而非環境或個人的排程尺度。
pub const FESTIVAL_INTERVAL_DAYS: u64 = 6;

/// 今天（世界時鐘累計日數）是不是慶典日。純函式、確定性——`day` 對間隔取模為 0 即為慶典日，
/// 世界啟動當天（day=0）恰好也是慶典日，讓新世界第一天就帶著歡慶氣氛開場。
pub fn is_festival_day(day: u64) -> bool {
    day % FESTIVAL_INTERVAL_DAYS == 0
}

/// 居民歡慶泡泡的冷卻倒數（秒）：一次歡慶後這麼久才會再顯露，比照哼歌（`vhum::HUM_COOLDOWN_SECS`
/// 同量級）——慶典日整天都在，但同一位居民不該每 tick 都在冒泡泡，偶爾一拍才有節慶感、不洗版。
pub const FESTIVAL_BUBBLE_COOLDOWN_SECS: f32 = 200.0;

/// 每次判定觸發歡慶泡泡的機率（配合長冷卻天然節流，避免每次冷卻歸零就必然觸發、顯得死板）。
pub const FESTIVAL_BUBBLE_CHANCE: f32 = 0.35;

/// 這位居民此刻要不要冒出歡慶泡泡：純函式、確定性，呼叫端負責傳入「是不是慶典日／閒著醒著
/// 沒有更優先的正事／冷卻已過／擲骰」四個既有事實，本函式只做最後的機率門檻判定。
pub fn should_celebrate(is_festival: bool, eligible: bool, cooldown_ok: bool, roll: f32) -> bool {
    is_festival && eligible && cooldown_ok && roll < FESTIVAL_BUBBLE_CHANCE
}

/// 慶典日歡慶泡泡（四句輪替，確定性，🎉/🎊 開頭讓前端一眼辨認）。
pub fn festival_bubble_line(pick: usize) -> &'static str {
    const POOL: &[&str] = &[
        "🎉 今天是慶典日呢，心情特別好！",
        "🎊 慶典日就是要笑得比平常燦爛一點～",
        "🎉 這種日子，連空氣都甜甜的",
        "🎊 好想跟大家一起熱鬧一下！",
    ];
    POOL[pick % POOL.len()]
}

/// 慶典日開始那一刻的城鎮動態牆文案（一天只播一次，供不在線的玩家回來也讀得到）。
pub fn festival_begin_feed_line() -> &'static str {
    "今天是村莊的慶典日🎉，村里的人都比平常更容易眉開眼笑。"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn festival_day_at_multiples_of_interval() {
        assert!(is_festival_day(0));
        assert!(is_festival_day(FESTIVAL_INTERVAL_DAYS));
        assert!(is_festival_day(FESTIVAL_INTERVAL_DAYS * 3));
    }

    #[test]
    fn non_festival_day_between_intervals() {
        for d in 1..FESTIVAL_INTERVAL_DAYS {
            assert!(!is_festival_day(d), "day {d} 不該是慶典日");
        }
    }

    #[test]
    fn should_celebrate_requires_festival_day() {
        assert!(!should_celebrate(false, true, true, 0.0), "非慶典日不該歡慶");
    }

    #[test]
    fn should_celebrate_requires_eligible() {
        assert!(!should_celebrate(true, false, true, 0.0), "沒空/在忙不該歡慶");
    }

    #[test]
    fn should_celebrate_requires_cooldown_ok() {
        assert!(!should_celebrate(true, true, false, 0.0), "冷卻中不該歡慶");
    }

    #[test]
    fn should_celebrate_requires_roll_under_chance() {
        assert!(!should_celebrate(true, true, true, FESTIVAL_BUBBLE_CHANCE), "擲骰未過門檻不該歡慶");
        assert!(should_celebrate(true, true, true, 0.0), "擲骰壓線最小值應觸發");
    }

    #[test]
    fn festival_bubble_line_non_empty_and_all_distinct() {
        let lines: std::collections::HashSet<_> = (0..4).map(festival_bubble_line).collect();
        assert_eq!(lines.len(), 4, "四句應各不相同");
        for pick in [0usize, 1, 2, 3, 99] {
            assert!(!festival_bubble_line(pick).is_empty());
        }
    }

    #[test]
    fn festival_begin_feed_line_mentions_festival() {
        assert!(festival_begin_feed_line().contains('🎉'));
        assert!(!festival_begin_feed_line().is_empty());
    }
}
