//! 乙太方界居民「老友情境問候」——好感度達老友（≥ [`FOND_AFFINITY`] 筆記憶）時，
//! 居民說出記憶驅動的特定台詞（提及送禮/交易/對話），而非通用招呼，
//! 讓玩家第一次感受到「居民**記得我們做過的具體事**，不只是「認識我」的差異。
//!
//! **純邏輯層**：確定性、零 LLM、零持久化、可測；
//! 連線、鎖、廣播全在 `voxel_ws.rs`。

/// 觸發老友情境問候的好感度門檻（居民對玩家的長期記憶筆數）。
/// 高於 `RECALL_AFFINITY_THRESHOLD`（=3）——確保這是「真正的老朋友」才有的問候。
pub const FOND_AFFINITY: usize = 5;

/// 情境類別（由最近記憶摘要關鍵詞推斷，優先序由高到低）。
#[derive(Clone, Debug, PartialEq)]
pub enum FriendContext {
    /// 玩家曾送禮給這位居民（記憶含「送來」/「送我」）。
    PlayerGaveGift,
    /// 雙方有以物易物記錄（記憶含「以物易物」）。
    Traded,
    /// 只有對話記憶，無特定互動（通用老友）。
    JustTalked,
}

/// 從最近幾筆記憶摘要偵測情境（關鍵詞掃描，確定性零 LLM）。
///
/// - `summaries`：[`MemoryEntry.summary`] 列表（傳入前 N 筆，最新在最後）。
/// - 回傳「最有感」情境；優先順序：`PlayerGaveGift` > `Traded` > `JustTalked`。
/// - 空列表安全退回 `JustTalked`。
pub fn detect_context(summaries: &[String]) -> FriendContext {
    // 從最近往舊掃，找到最有感情境即回傳。
    for s in summaries.iter().rev() {
        if contains_any(s, &["送來", "送我"]) {
            return FriendContext::PlayerGaveGift;
        }
        if s.contains("以物易物") {
            return FriendContext::Traded;
        }
    }
    FriendContext::JustTalked
}

/// 依情境與玩家名生成老友問候台詞（≤ 40 字，確定性）。
///
/// - `player_name`：玩家顯示名（空字串安全退回通用句）。
/// - `ctx`：由 [`detect_context`] 推斷的情境。
/// - `pick`：呼叫端提供的雜湊值（確定性選句池，0..usize 安全）。
pub fn fond_greeting_line(player_name: &str, ctx: &FriendContext, pick: usize) -> String {
    let name: String = if player_name.is_empty() {
        "旅人".to_string()
    } else {
        player_name.chars().take(6).collect()
    };
    let raw = match ctx {
        FriendContext::PlayerGaveGift => {
            const LINES: [&str; 3] = [
                "剛還在想{n}帶來的心意，你回來啦！",
                "{n}！你送的那份心意我一直記著。",
                "歡迎{n}！你上次送我的真的很暖心。",
            ];
            LINES[pick % LINES.len()].replace("{n}", &name)
        }
        FriendContext::Traded => {
            const LINES: [&str; 3] = [
                "{n}！上次的交換你滿意嗎？",
                "嗨，{n}！手邊又攢了些材料，想換換。",
                "{n}，有空再來以物易物吧！",
            ];
            LINES[pick % LINES.len()].replace("{n}", &name)
        }
        FriendContext::JustTalked => {
            const LINES: [&str; 3] = [
                "{n}！你說的那些話，我一直放在心上。",
                "嗨，{n}！你回來了，有好多話想說。",
                "{n}回來了！你上次分享的我還記著呢。",
            ];
            LINES[pick % LINES.len()].replace("{n}", &name)
        }
    };
    // 截斷防超長（安全防線，正常不超過 40 字）。
    raw.chars().take(40).collect()
}

fn contains_any(s: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|k| s.contains(k))
}

// ── 測試 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_context_gift_from_keyword_song_lai() {
        let summaries = vec!["收到了旅人送來的木頭，心裡暖暖的".to_string()];
        assert_eq!(detect_context(&summaries), FriendContext::PlayerGaveGift);
    }

    #[test]
    fn detect_context_gift_from_keyword_song_wo() {
        let summaries = vec!["旅人送我麵包——這個人很體貼".to_string()];
        assert_eq!(detect_context(&summaries), FriendContext::PlayerGaveGift);
    }

    #[test]
    fn detect_context_trade() {
        let summaries = vec!["和小美以物易物：我給了種子，換來了木頭，感覺不錯".to_string()];
        assert_eq!(detect_context(&summaries), FriendContext::Traded);
    }

    #[test]
    fn detect_context_fallback_to_just_talked() {
        let summaries = vec!["聊到了星空與夜晚的美麗".to_string(), "分享了旅人的見聞".to_string()];
        assert_eq!(detect_context(&summaries), FriendContext::JustTalked);
    }

    #[test]
    fn detect_context_empty_is_just_talked() {
        assert_eq!(detect_context(&[]), FriendContext::JustTalked);
    }

    #[test]
    fn detect_context_gift_wins_over_trade() {
        // 先有交易記憶、後有送禮記憶（更新）→ 應回 PlayerGaveGift（從最新掃）。
        let summaries = vec![
            "和小明以物易物：我給了石頭，換來了種子".to_string(),
            "收到了小明送來的玻璃，心裡暖暖的".to_string(),
        ];
        assert_eq!(detect_context(&summaries), FriendContext::PlayerGaveGift);
    }

    #[test]
    fn fond_greeting_contains_player_name() {
        for ctx in [FriendContext::PlayerGaveGift, FriendContext::Traded, FriendContext::JustTalked] {
            let line = fond_greeting_line("小美", &ctx, 0);
            assert!(line.contains("小美"), "台詞應含玩家名：{line}");
        }
    }

    #[test]
    fn fond_greeting_stays_within_40_chars() {
        let long_name = "超長玩家名字TEST";
        for ctx in [FriendContext::PlayerGaveGift, FriendContext::Traded, FriendContext::JustTalked] {
            for pick in 0..3 {
                let line = fond_greeting_line(long_name, &ctx, pick);
                let chars = line.chars().count();
                assert!(chars <= 40, "超過 40 字（{chars}）：{line}");
            }
        }
    }

    #[test]
    fn fond_greeting_empty_name_uses_fallback() {
        for ctx in [FriendContext::PlayerGaveGift, FriendContext::Traded, FriendContext::JustTalked] {
            let line = fond_greeting_line("", &ctx, 0);
            // 空名字退回「旅人」——不 panic、不空字串
            assert!(!line.is_empty(), "空名字不該回空字串");
            assert!(line.contains("旅人"), "空名字應退回「旅人」：{line}");
        }
    }

    #[test]
    fn fond_greeting_pick_wraps_safely() {
        // pick 遠大於句池長度不 panic
        for pick in [0, 1, 2, 99, usize::MAX / 2] {
            let line = fond_greeting_line("星星", &FriendContext::JustTalked, pick);
            assert!(!line.is_empty());
        }
    }

    #[test]
    fn fond_affinity_threshold_sane() {
        assert!(FOND_AFFINITY >= 4, "門檻太低——4 筆以下還不夠「老友」");
        assert!(FOND_AFFINITY <= 8, "門檻太高——8 筆以上幾乎沒人能觸發");
    }
}
