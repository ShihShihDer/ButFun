//! 乙太方界·孤獨尋伴 v1（ROADMAP 678）。
//!
//! 心情低落（Lonely）的居民，冷卻到期且附近有在線玩家時，主動走過去尋求陪伴；
//! 抵達後冒出帶 😔 的求陪泡泡，等待玩家搭話。玩家一旦向這位居民開口，
//! 居民冒出感謝泡泡，並把 1 塊木頭送進玩家背包——
//! 讓世界第一次有「AI 居民**需要你**」的溫柔感。
//!
//! **純邏輯層**：零 IO、零鎖、零 LLM、零 async；確定性純函式。
//! 鎖與副作用在 `voxel_ws.rs`（短鎖即釋、不巢狀、守死鎖鐵律）。

/// 搜尋半徑（方塊，XZ 平面）：居民在此範圍內的玩家才會主動靠近。
pub const SEEK_RANGE: f32 = 28.0;
/// 尋伴冷卻（秒，純記憶體、重啟歸零）：每次觸發後要等這麼久才可再觸發。
pub const SEEK_COMFORT_COOLDOWN: f32 = 300.0;
/// 到達玩家這麼近才冒求陪泡泡（方塊，XZ）。
pub const COMFORT_ARRIVE_DIST: f32 = 5.0;
/// 安慰致謝禮物：木頭（block_id 5）1 塊。
pub const COMFORT_GIFT_BLOCK: u8 = 5;
/// 安慰致謝禮物數量。
pub const COMFORT_GIFT_QTY: u32 = 1;

/// 各居民初始冷卻偏移（秒）：讓四位居民不在同一 tick 同時觸發，免得搶玩家。
/// 確定性純函式，可測。
pub fn seek_cooldown_offset(idx: usize) -> f32 {
    120.0 + idx as f32 * 60.0
}

/// 孤獨居民靠近玩家時冒的求陪泡泡台詞。
/// `pick` 由呼叫端傳入（居民位置的位元運算結果），在台詞池中循環選擇。
pub fn comfort_seek_line(pick: usize) -> &'static str {
    const POOL: &[&str] = &[
        "😔 今天有點寂寞，能陪我說說話嗎？",
        "😔 好孤單……旅人，你願意聊聊嗎？",
        "😔 今天沒什麼人…你來了真好。",
    ];
    POOL[pick % POOL.len()]
}

/// 玩家搭話後居民的感謝台詞（冒泡後清除尋伴狀態）。零 LLM、確定性。
pub fn comfort_thanks_line(pick: usize) -> &'static str {
    const POOL: &[&str] = &[
        "😊 謝謝你！說說話，心情好多了！",
        "🙂 有你陪著，今天不再那麼寂寞了。",
        "😊 你的話讓我暖心許多，謝謝你。",
    ];
    POOL[pick % POOL.len()]
}

/// 致謝禮物的面向玩家名稱（集中可 i18n）。
pub fn comfort_gift_name() -> &'static str {
    "木頭"
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_cooldown_offset_strictly_increasing() {
        let offsets: Vec<f32> = (0..4).map(seek_cooldown_offset).collect();
        for i in 1..offsets.len() {
            assert!(offsets[i] > offsets[i - 1], "偏移應隨 idx 嚴格遞增：{:?}", offsets);
        }
    }

    #[test]
    fn seek_cooldown_offset_all_positive() {
        for i in 0..4 {
            assert!(seek_cooldown_offset(i) > 0.0, "偏移 idx={i} 應 > 0");
        }
    }

    #[test]
    fn comfort_seek_line_non_empty_all_picks() {
        for pick in [0usize, 1, 2, 7, 99, 300] {
            let line = comfort_seek_line(pick);
            assert!(!line.is_empty(), "pick={pick} 不應空");
        }
    }

    #[test]
    fn comfort_seek_line_all_have_lonely_emoji() {
        for pick in 0..3 {
            assert!(
                comfort_seek_line(pick).contains("😔"),
                "求陪台詞應含 😔，pick={pick}"
            );
        }
    }

    #[test]
    fn comfort_seek_line_cycles_not_all_same() {
        let lines: Vec<_> = (0..3).map(comfort_seek_line).collect();
        let unique: std::collections::HashSet<_> = lines.iter().collect();
        assert!(unique.len() >= 2, "應有多條不同台詞");
    }

    #[test]
    fn comfort_thanks_line_non_empty_all_picks() {
        for pick in [0usize, 1, 2, 5, 99] {
            let line = comfort_thanks_line(pick);
            assert!(!line.is_empty(), "pick={pick} 不應空");
        }
    }

    #[test]
    fn comfort_thanks_line_all_have_positive_emoji() {
        for pick in 0..3 {
            let line = comfort_thanks_line(pick);
            assert!(
                line.contains("😊") || line.contains("🙂"),
                "感謝台詞應含正面 emoji，pick={pick}"
            );
        }
    }

    #[test]
    fn comfort_gift_constants_valid() {
        assert_eq!(COMFORT_GIFT_BLOCK, 5, "致謝禮應是木頭（block_id 5）");
        assert!(COMFORT_GIFT_QTY > 0, "禮物數量應 > 0");
        assert!(!comfort_gift_name().is_empty(), "禮物名稱不應空");
    }

    #[test]
    fn constants_sane() {
        assert!(SEEK_RANGE > COMFORT_ARRIVE_DIST, "搜尋半徑應大於抵達距離");
        assert!(SEEK_COMFORT_COOLDOWN > 0.0, "冷卻應 > 0");
        assert!(COMFORT_ARRIVE_DIST > 0.0, "抵達距離應 > 0");
    }
}
