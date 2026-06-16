//! 玩家表情動作（ROADMAP 338）——玩家↔玩家的即時情緒表態。
//!
//! 過去玩家之間要互動，只能打字聊天（Chat / 密語 /w / 公會 /g）。本模組讓玩家能從一個
//! 表情輪選一個情緒，伺服器用玩家**自己的權威座標**廣播 `PlayerEmote`，所有附近玩家就
//! 在那位玩家頭頂看到一枚彈跳浮起的大表情——城鎮社交弧（327～332）把「NPC 之間」與
//! 「玩家↔NPC」做活了之後，這是第一條把「玩家↔玩家」直接連起來的線：不用打字，揮個手、
//! 比個讚、給顆愛心，對面那個人立刻就看得到。
//!
//! 設計取捨：
//! - **純查表、零狀態**：表情是一次性廣播（像 NpcSpeech 泡泡），不寫進玩家快照、不持久化、
//!   不需 migration——伺服器只把「誰、在哪、比了什麼」轉發出去，前端負責彈跳浮起與淡出。
//! - **固定白名單**：表情種類由下面的 `EMOTES` 寫死，伺服器只接受清單內的 wire key，
//!   其餘一律忽略——玩家送不出任意內容（不像聊天會夾帶文字），天然防濫用、零審查負擔。
//! - **i18n**：wire key（如 "wave"）是穩定協議契約、不面向玩家；面向玩家的中文標籤集中在
//!   前端表情輪，未來要在地化只改前端一處。glyph 是 emoji、跨語通用。

/// 表情泡泡在前端顯示的秒數（彈跳浮起＋淡出的總時長）。短而俏皮，不擋畫面。
pub const EMOTE_DISPLAY_SECS: u32 = 4;

/// 表情白名單：`(wire key, emoji glyph)`。
///
/// wire key 是穩定協議契約（前後端共用、序列化進訊息），**不要重排或改名**，只能在尾端追加。
/// 八種涵蓋玩家互相打招呼／鼓勵／表態的常見情緒：揮手、歡呼、愛心、大笑、比讚、哭哭、生氣、想睡。
pub const EMOTES: &[(&str, &str)] = &[
    ("wave", "👋"),     // 揮手——打招呼／道別
    ("cheer", "🎉"),    // 歡呼——慶祝／恭喜
    ("heart", "❤️"),    // 愛心——示好／感謝
    ("laugh", "😆"),    // 大笑——好笑／開心
    ("thumbsup", "👍"), // 比讚——認同／做得好
    ("cry", "😢"),      // 哭哭——難過／求安慰
    ("angry", "😠"),    // 生氣——不滿／抗議（純表態，無任何戰鬥效果）
    ("sleep", "💤"),    // 想睡——掛機／無聊
];

/// 查某個 wire key 對應的 emoji glyph。未知 key 回 `None`（伺服器據此靜默忽略偽造表情）。
pub fn glyph_for(kind: &str) -> Option<&'static str> {
    EMOTES.iter().find(|(k, _)| *k == kind).map(|(_, g)| *g)
}

/// 該 wire key 是否為合法表情。
pub fn is_valid(kind: &str) -> bool {
    glyph_for(kind).is_some()
}

/// 查某個 wire key 在 `EMOTES` 內的索引（0 起算）。未知 key 回 `None`。
///
/// 索引是穩定契約（隨 `EMOTES` 次序，只可尾端追加），記憶體前置欄位（如玩家最近表情）用它
/// 把表情壓成一個小整數、無需在熱路徑搬字串。共鳴偵測（ROADMAP 340）即以同索引判斷
/// 「比的是不是同一個表情」。
pub fn index_of(kind: &str) -> Option<u8> {
    EMOTES.iter().position(|(k, _)| *k == kind).map(|i| i as u8)
}

/// 由 `EMOTES` 索引取 emoji glyph。越界回 `None`。與 `index_of` 互為反向。
pub fn glyph_at(index: u8) -> Option<&'static str> {
    EMOTES.get(index as usize).map(|(_, g)| *g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_keys_resolve_to_nonempty_glyphs() {
        for (key, _) in EMOTES {
            let g = glyph_for(key).expect("白名單內的 key 必有 glyph");
            assert!(!g.is_empty(), "{key} 的 glyph 不可為空");
        }
    }

    #[test]
    fn unknown_key_returns_none() {
        assert_eq!(glyph_for("explode"), None);
        assert_eq!(glyph_for(""), None);
        assert_eq!(glyph_for("WAVE"), None, "大小寫敏感，避免協議模糊");
        assert!(!is_valid("rm -rf"));
    }

    #[test]
    fn wire_keys_are_unique() {
        let mut keys: Vec<&str> = EMOTES.iter().map(|(k, _)| *k).collect();
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n, "表情 wire key 不可重複");
    }

    #[test]
    fn is_valid_matches_glyph_lookup() {
        assert!(is_valid("wave"));
        assert!(is_valid("sleep"));
        assert_eq!(is_valid("nope"), glyph_for("nope").is_some());
    }

    #[test]
    fn index_and_glyph_at_are_inverse() {
        // 每個 wire key 的索引取回的 glyph，要等於直接查表的 glyph（index_of/glyph_at 互逆）。
        for (key, glyph) in EMOTES {
            let i = index_of(key).expect("白名單內的 key 必有索引");
            assert_eq!(glyph_at(i), Some(*glyph), "{key} 索引取回的 glyph 不一致");
        }
        // 未知 key 無索引；越界索引無 glyph。
        assert_eq!(index_of("nope"), None);
        assert_eq!(index_of("WAVE"), None, "大小寫敏感");
        assert_eq!(glyph_at(EMOTES.len() as u8), None, "越界索引回 None");
    }
}
