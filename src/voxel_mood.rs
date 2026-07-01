//! 乙太方界·居民心情指示 v1（ROADMAP 676）。
//!
//! 根據居民的情誼關係（bonds）與玩家互動記憶（memories）動態計算「心情」，
//! 以 emoji 顯示在居民名牌左側——讓玩家一眼看出「這個居民現在過得怎樣」，
//! 世界第一次有了可見的情感溫度計。
//!
//! ## 心情層級（從高到低）
//! | 層級 | emoji | 條件 |
//! |------|-------|------|
//! | Joyful  | 😊 | 有老友（Friend bond ≥1）且玩家記憶豐厚（≥5筆）|
//! | Content | 🙂 | 有老友（≥1）或多個相識（Acquaintance ≥2）|
//! | Neutral | 😐 | 有相識（Acquaintance ≥1）|
//! | Curious | 🤔 | 全陌生但玩家有互動記憶（≥2筆）|
//! | Lonely  | 😔 | 全陌生且互動記憶稀少（<2筆）|
//!
//! **純邏輯層**：零 IO、零鎖、零 LLM、零 async；`compute_mood` 確定性純函式。
//! 鎖與廣播在 `voxel_ws.rs`。

/// 居民的心情層級。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoodTier {
    Joyful,
    Content,
    Neutral,
    Curious,
    Lonely,
}

/// 根據居民的情誼與記憶計算心情層級（確定性純函式）。
///
/// - `friend_bonds`：與其他居民處於 Friend 層的數量。
/// - `acquaintance_bonds`：與其他居民處於 Acquaintance 層的數量。
/// - `total_memories`：居民長期記憶總筆數（涵蓋所有玩家的互動）。
pub fn compute_mood(
    friend_bonds: usize,
    acquaintance_bonds: usize,
    total_memories: usize,
) -> MoodTier {
    if friend_bonds >= 1 && total_memories >= 5 {
        MoodTier::Joyful
    } else if friend_bonds >= 1 || acquaintance_bonds >= 2 {
        MoodTier::Content
    } else if acquaintance_bonds >= 1 {
        MoodTier::Neutral
    } else if total_memories >= 2 {
        MoodTier::Curious
    } else {
        MoodTier::Lonely
    }
}

/// 心情對應的 emoji（廣播給前端，顯示在名牌旁）。
pub fn mood_emoji(tier: MoodTier) -> &'static str {
    match tier {
        MoodTier::Joyful => "😊",
        MoodTier::Content => "🙂",
        MoodTier::Neutral => "😐",
        MoodTier::Curious => "🤔",
        MoodTier::Lonely => "😔",
    }
}

/// 心情的中文描述（注入 LLM world_news，讓居民思考時帶著情境）。
pub fn mood_description_zh(tier: MoodTier) -> &'static str {
    match tier {
        MoodTier::Joyful => "心情很好——有老朋友相伴，旅人也常來看你，內心充實而溫暖",
        MoodTier::Content => "心情不錯——有朋友在，感覺挺好的",
        MoodTier::Neutral => "心情平靜——認識一些鄰居，生活有些規律",
        MoodTier::Curious => "有點好奇——跟旅人聊過，對鄰居還不太熟",
        MoodTier::Lonely => "有點寂寞——剛來到這世界，還在慢慢認識周遭",
    }
}

/// 心情自語台詞（ROADMAP 677）——冷卻到期時居民自發冒出的泡泡，零 LLM、純確定性。
///
/// `pick` 由呼叫端傳入（如居民位置的位元運算結果），用於在台詞池中循環選擇。
/// `Neutral` 心情平靜不說話，回 `None`；其餘層級各有 3–5 條台詞輪流。
pub fn spontaneous_line(tier: MoodTier, pick: usize) -> Option<&'static str> {
    let pool: &[&'static str] = match tier {
        MoodTier::Joyful => &[
            "✨ 今天特別開心！",
            "哈哈，感覺什麼都做得到！",
            "嗯哼～這世界真美。",
            "能認識這麼多朋友，真幸福。",
            "心情好，連方塊都比較好看！",
        ],
        MoodTier::Content => &[
            "嗯，今天過得挺好的。",
            "生活有規律，感覺不錯。",
            "偶爾抬頭看看天，挺好。",
        ],
        MoodTier::Neutral => return None,
        MoodTier::Curious => &[
            "這個世界有點有趣…",
            "嗯，我在想一些事情。",
            "附近好像有什麼動靜…？",
        ],
        MoodTier::Lonely => &[
            "有點寂寞啊…",
            "今天好像沒什麼人…",
            "😔 希望有人來聊聊天。",
        ],
    };
    if pool.is_empty() {
        return None;
    }
    Some(pool[pick % pool.len()])
}

/// 依心情計算居民建造間隔（秒）——心情好能量充沛、建造加速；心情差心不在焉、建造放緩。
/// 純函式：確定性、零副作用、可測。與 `BUILD_INTERVAL_SECS`（8.0 預設）對稱。
pub fn build_interval_secs(tier: MoodTier) -> f32 {
    match tier {
        MoodTier::Joyful  => 5.0,  // 活力充沛，每 5 秒一塊（預設 8 秒的 62%）
        MoodTier::Content => 7.0,  // 心情不錯，略快
        MoodTier::Neutral => 8.0,  // 平靜，維持預設
        MoodTier::Curious => 9.0,  // 好奇分心，略慢
        MoodTier::Lonely  => 12.0, // 心不在焉，明顯放緩
    }
}

/// 把心情轉成 `SenseInput.mood` 整數值（0–100）。
/// 替換原先的硬編碼 `70`，讓 LLM 能感知居民當下的情緒狀態。
pub fn mood_to_sense_value(tier: MoodTier) -> i32 {
    match tier {
        MoodTier::Joyful => 90,
        MoodTier::Content => 75,
        MoodTier::Neutral => 60,
        MoodTier::Curious => 50,
        MoodTier::Lonely => 35,
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_bonds_no_memories_is_lonely() {
        assert_eq!(compute_mood(0, 0, 0), MoodTier::Lonely);
    }

    #[test]
    fn no_bonds_one_memory_is_lonely() {
        assert_eq!(compute_mood(0, 0, 1), MoodTier::Lonely);
    }

    #[test]
    fn no_bonds_two_memories_is_curious() {
        assert_eq!(compute_mood(0, 0, 2), MoodTier::Curious);
    }

    #[test]
    fn no_bonds_many_memories_is_curious() {
        assert_eq!(compute_mood(0, 0, 10), MoodTier::Curious);
    }

    #[test]
    fn one_acquaintance_is_neutral() {
        assert_eq!(compute_mood(0, 1, 0), MoodTier::Neutral);
    }

    #[test]
    fn two_acquaintances_is_content() {
        assert_eq!(compute_mood(0, 2, 0), MoodTier::Content);
    }

    #[test]
    fn one_friend_few_memories_is_content() {
        // 有老友但記憶不足 5 筆 → Content（非 Joyful）
        assert_eq!(compute_mood(1, 0, 4), MoodTier::Content);
    }

    #[test]
    fn one_friend_five_memories_is_joyful() {
        assert_eq!(compute_mood(1, 0, 5), MoodTier::Joyful);
    }

    #[test]
    fn two_friends_many_memories_is_joyful() {
        assert_eq!(compute_mood(2, 1, 10), MoodTier::Joyful);
    }

    #[test]
    fn mood_emoji_non_empty_for_all_tiers() {
        for tier in [
            MoodTier::Joyful,
            MoodTier::Content,
            MoodTier::Neutral,
            MoodTier::Curious,
            MoodTier::Lonely,
        ] {
            assert!(!mood_emoji(tier).is_empty(), "{tier:?} emoji 不能空");
        }
    }

    #[test]
    fn mood_description_non_empty_for_all_tiers() {
        for tier in [
            MoodTier::Joyful,
            MoodTier::Content,
            MoodTier::Neutral,
            MoodTier::Curious,
            MoodTier::Lonely,
        ] {
            assert!(!mood_description_zh(tier).is_empty(), "{tier:?} 描述不能空");
        }
    }

    #[test]
    fn mood_sense_value_in_range() {
        for tier in [
            MoodTier::Joyful,
            MoodTier::Content,
            MoodTier::Neutral,
            MoodTier::Curious,
            MoodTier::Lonely,
        ] {
            let v = mood_to_sense_value(tier);
            assert!((0..=100).contains(&v), "{tier:?} mood 值 {v} 超出範圍");
        }
    }

    #[test]
    fn joyful_mood_sense_value_highest() {
        // Joyful > Content > Neutral > Curious > Lonely
        assert!(mood_to_sense_value(MoodTier::Joyful) > mood_to_sense_value(MoodTier::Content));
        assert!(mood_to_sense_value(MoodTier::Content) > mood_to_sense_value(MoodTier::Neutral));
        assert!(mood_to_sense_value(MoodTier::Neutral) > mood_to_sense_value(MoodTier::Curious));
        assert!(mood_to_sense_value(MoodTier::Curious) > mood_to_sense_value(MoodTier::Lonely));
    }

    // ── spontaneous_line（ROADMAP 677）────────────────────────────────────────

    #[test]
    fn neutral_mood_no_spontaneous_line() {
        // 心情平靜不自語
        assert!(spontaneous_line(MoodTier::Neutral, 0).is_none());
        assert!(spontaneous_line(MoodTier::Neutral, 99).is_none());
    }

    #[test]
    fn joyful_always_returns_some() {
        for pick in [0, 1, 2, 3, 4, 7, 100] {
            assert!(
                spontaneous_line(MoodTier::Joyful, pick).is_some(),
                "Joyful pick={pick} 應回台詞"
            );
        }
    }

    #[test]
    fn content_always_returns_some() {
        for pick in [0, 1, 2, 5] {
            assert!(spontaneous_line(MoodTier::Content, pick).is_some());
        }
    }

    #[test]
    fn curious_always_returns_some() {
        for pick in [0, 1, 2, 9] {
            assert!(spontaneous_line(MoodTier::Curious, pick).is_some());
        }
    }

    #[test]
    fn lonely_always_returns_some() {
        for pick in [0, 1, 2] {
            assert!(spontaneous_line(MoodTier::Lonely, pick).is_some());
        }
    }

    // ── build_interval_secs（ROADMAP 680）────────────────────────────────────

    #[test]
    fn build_interval_joyful_fastest() {
        // Joyful 建造間隔應最短
        assert!(
            build_interval_secs(MoodTier::Joyful) < build_interval_secs(MoodTier::Content),
            "Joyful 應比 Content 快"
        );
        assert!(
            build_interval_secs(MoodTier::Content) < build_interval_secs(MoodTier::Neutral),
            "Content 應比 Neutral 快"
        );
    }

    #[test]
    fn build_interval_lonely_slowest() {
        // Lonely 建造間隔應最長
        assert!(
            build_interval_secs(MoodTier::Lonely) > build_interval_secs(MoodTier::Curious),
            "Lonely 應比 Curious 慢"
        );
        assert!(
            build_interval_secs(MoodTier::Curious) > build_interval_secs(MoodTier::Neutral),
            "Curious 應比 Neutral 慢"
        );
    }

    #[test]
    fn build_interval_neutral_matches_default() {
        // Neutral 應等於預設 BUILD_INTERVAL_SECS（8.0）
        assert!(
            (build_interval_secs(MoodTier::Neutral) - 8.0).abs() < f32::EPSILON,
            "Neutral 建造間隔應為 8.0"
        );
    }

    #[test]
    fn build_interval_all_positive() {
        for tier in [
            MoodTier::Joyful,
            MoodTier::Content,
            MoodTier::Neutral,
            MoodTier::Curious,
            MoodTier::Lonely,
        ] {
            assert!(build_interval_secs(tier) > 0.0, "{tier:?} 建造間隔必須大於零");
        }
    }

    #[test]
    fn joyful_lines_cycle_with_different_picks() {
        // 不同 pick 值應產生不同台詞（5 條台詞，pick 0~4 各不同）
        let lines: Vec<_> = (0..5)
            .map(|p| spontaneous_line(MoodTier::Joyful, p).unwrap())
            .collect();
        // 至少要有 2 種不同台詞（5 條不可能全一樣）
        let unique: std::collections::HashSet<_> = lines.iter().collect();
        assert!(unique.len() >= 2, "Joyful 應有多條不同台詞");
    }

    #[test]
    fn lonely_line_non_empty() {
        let line = spontaneous_line(MoodTier::Lonely, 0).unwrap();
        assert!(!line.is_empty(), "Lonely 台詞不能是空字串");
    }

    #[test]
    fn content_line_non_empty() {
        let line = spontaneous_line(MoodTier::Content, 1).unwrap();
        assert!(!line.is_empty());
    }
}
