//! 乙太方界·居民互相打氣 v1（ROADMAP 679）。
//!
//! 心情好（Joyful/Content）的居民，在範圍內偵測到心情低落（Lonely）的同伴時，
//! 主動走過去說一句溫暖的話。對方透過 `pending_response` 機制延遲回謝。
//! 雙方各寫一筆記憶（最低 mems 計數提升，可讓 Lonely → Curious）。
//! Feed 廣播「相互打氣」，讓玩家看見 AI 社群自我支撐——不靠玩家介入。
//!
//! **純邏輯層**：零 IO、零鎖、零 LLM、零 async；確定性純函式。
//! 鎖與副作用在 `voxel_ws.rs`（短鎖即釋、不巢狀、守死鎖鐵律）。

/// 觸發打氣的搜尋半徑（方塊，XZ 平面）。
pub const CHEER_RANGE: f32 = 15.0;
/// 每 tick 的觸發機率（CHEER_RANGE 內有 Lonely 同伴時才計算；低頻不洗版）。
pub const CHEER_CHANCE: f32 = 0.003;
/// 打氣後冷卻秒數（純記憶體，重啟歸零）。
pub const CHEER_COOLDOWN: f32 = 300.0;
/// 走到這麼近才冒打氣泡泡（方塊，XZ）。
pub const CHEER_ARRIVE_DIST: f32 = 4.5;
/// Feed 事件種類名稱。
pub const FEED_KIND: &str = "相互打氣";

/// 各居民的初始打氣冷卻偏移（秒）：讓四位居民不在同一 tick 同時掃描，
/// 並讓打氣系統在入場穩定後才啟動（避免第一輪就全部觸發）。
pub fn cheer_cooldown_offset(idx: usize) -> f32 {
    180.0 + idx as f32 * 45.0
}

/// 心情好的居民走到 Lonely 同伴旁時冒的鼓勵台詞（零 LLM、確定性）。
/// `pick` 由呼叫端傳入（位元運算結果），在台詞池中循環。
pub fn cheer_line(pick: usize) -> &'static str {
    const POOL: &[&str] = &[
        "😊 別難過，我來陪你了！",
        "🌟 看起來有點寂寞？說說話吧！",
        "💛 我在這裡！你不是一個人。",
    ];
    POOL[pick % POOL.len()]
}

/// 打氣者的記憶摘要（寫進 voxel_memory，提升 mems 計數）。
/// 用被打氣者的名字讓記憶更具體，可讓對方 Lonely→Curious 的計算受益。
pub fn cheer_memory_for_cheerful(lonely_name: &str) -> String {
    format!("去陪伴了{lonely_name}，感覺做了一件溫暖的事。")
}

/// 被打氣者的記憶摘要（寫進 voxel_memory，升 mems 計數→可從 Lonely 升至 Curious）。
pub fn cheer_memory_for_lonely(cheerful_name: &str) -> String {
    format!("{cheerful_name}特地來陪我說話，心裡暖了不少。")
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cheer_cooldown_offset_strictly_increasing() {
        let offsets: Vec<f32> = (0..4).map(cheer_cooldown_offset).collect();
        for i in 1..offsets.len() {
            assert!(offsets[i] > offsets[i - 1], "偏移應隨 idx 嚴格遞增：{:?}", offsets);
        }
    }

    #[test]
    fn cheer_cooldown_offset_all_positive() {
        for i in 0..4 {
            assert!(cheer_cooldown_offset(i) > 0.0, "idx={i} 偏移應 > 0");
        }
    }

    #[test]
    fn cheer_line_non_empty_various_picks() {
        for pick in [0usize, 1, 2, 3, 7, 99, 300] {
            assert!(!cheer_line(pick).is_empty(), "pick={pick} 台詞不應空");
        }
    }

    #[test]
    fn cheer_line_all_have_positive_emoji() {
        for pick in 0..3 {
            let line = cheer_line(pick);
            let has_emoji = line.contains("😊") || line.contains("🌟") || line.contains("💛");
            assert!(has_emoji, "打氣台詞應含正面 emoji，pick={pick}");
        }
    }

    #[test]
    fn cheer_line_cycles_not_all_same() {
        let lines: Vec<_> = (0..3).map(cheer_line).collect();
        let unique: std::collections::HashSet<_> = lines.iter().collect();
        assert!(unique.len() >= 2, "應有多條不同台詞，讓打氣不單調");
    }

    #[test]
    fn cheer_line_within_40_chars() {
        for pick in 0..3 {
            let count = cheer_line(pick).chars().count();
            assert!(count <= 40, "台詞應在 40 字內（泡泡上限），pick={pick}，len={count}");
        }
    }

    #[test]
    fn cheer_memory_for_cheerful_contains_lonely_name() {
        let mem = cheer_memory_for_cheerful("諾娃");
        assert!(mem.contains("諾娃"), "記憶應包含被打氣者名字");
        assert!(!mem.is_empty());
    }

    #[test]
    fn cheer_memory_for_lonely_contains_cheerful_name() {
        let mem = cheer_memory_for_lonely("露娜");
        assert!(mem.contains("露娜"), "記憶應包含打氣者名字");
        assert!(!mem.is_empty());
    }

    #[test]
    fn constants_sane() {
        assert!(CHEER_RANGE > CHEER_ARRIVE_DIST, "搜尋半徑應大於抵達距離");
        assert!(CHEER_COOLDOWN > 0.0, "冷卻應 > 0");
        assert!(CHEER_CHANCE > 0.0 && CHEER_CHANCE < 1.0, "觸發機率應在 (0,1)");
        assert!(!FEED_KIND.is_empty());
    }

    #[test]
    fn cheer_memory_different_for_each_direction() {
        let for_cheerful = cheer_memory_for_cheerful("諾娃");
        let for_lonely = cheer_memory_for_lonely("露娜");
        assert_ne!(for_cheerful, for_lonely, "兩邊記憶應不同（角色立場不同）");
    }

    #[test]
    fn cheer_memory_for_lonely_multiple_names() {
        for name in ["露娜", "諾娃", "賽勒", "奧瑞"] {
            let mem = cheer_memory_for_lonely(name);
            assert!(mem.contains(name), "記憶應含 {name}");
        }
    }

    #[test]
    fn cheer_memory_for_cheerful_multiple_names() {
        for name in ["露娜", "諾娃", "賽勒", "奧瑞"] {
            let mem = cheer_memory_for_cheerful(name);
            assert!(mem.contains(name), "記憶應含 {name}");
        }
    }
}
