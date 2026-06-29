//! 乙太方界·居民日記——把居民的長期記憶格式化成「人類可讀的生命故事」。
//!
//! **核心信念**：AI 的內在生活要看得見才算活著。
//! 這裡只放確定性純邏輯（格式化 + 序列化）；鎖 / 連線都在 `voxel_ws.rs`。
//! 全部抽成可測純函式；不抄外部碼；繁中註解。

use serde::Serialize;

use crate::voxel_memory::MemoryEntry;

/// 日記裡的單一條目：居民記憶的一個片段。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DiaryEntry {
    /// 單調遞增序號（越大越新），供前端依序排列。
    pub seq: u64,
    /// 是哪位玩家引發這段記憶（可能是玩家顯示名）。
    pub player: String,
    /// 記憶摘要文字（直接從 `MemoryEntry.summary` 來，已是繁中句子）。
    pub text: String,
}

/// 一位居民的完整日記頁：名字 + 當前心願 + 記憶條目列表。
#[derive(Clone, Debug, Serialize)]
pub struct DiaryPage {
    /// 居民系統 id（如 "vox_res_0"）。
    pub resident_id: String,
    /// 居民顯示名（如「露娜」）。
    pub resident_name: String,
    /// 居民目前的心願（`None` = 尚未有任何心願；由玩家對話種下）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desire: Option<String>,
    /// 記憶條目列表，**最新在前**（seq 大→小）。空列表 = 沒有任何記憶。
    pub entries: Vec<DiaryEntry>,
}

/// 把居民的 `MemoryEntry` 列表 + 心願 → 格式化成 `DiaryPage`。
/// `memories` 必須**已是最新在前**（呼叫端自行排序，本函式不改順序）。
/// 純函式：確定性、無副作用、可測。
pub fn format_diary_page(
    resident_id: &str,
    resident_name: &str,
    desire: Option<&str>,
    memories: &[MemoryEntry],
) -> DiaryPage {
    let entries = memories
        .iter()
        .map(|m| DiaryEntry {
            seq: m.seq,
            player: m.player.clone(),
            text: m.summary.clone(),
        })
        .collect();
    DiaryPage {
        resident_id: resident_id.to_string(),
        resident_name: resident_name.to_string(),
        desire: desire.map(|s| s.to_string()),
        entries,
    }
}

/// 居民是否「有日記可看」：有心願或至少一筆記憶才算有內容。
/// 純函式、可測；讓前端決定是否亮出「📖 日記」按鈕。
pub fn has_diary_content(desire: Option<&str>, memory_count: usize) -> bool {
    desire.map_or(false, |d| !d.is_empty()) || memory_count > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(seq: u64, player: &str, summary: &str) -> MemoryEntry {
        MemoryEntry {
            resident: "vox_res_0".into(),
            player: player.into(),
            summary: summary.into(),
            seq,
        }
    }

    #[test]
    fn format_diary_page_basic() {
        let memories = vec![
            make_entry(2, "阿星", "和阿星聊過，對方提到「想看星星」"),
            make_entry(1, "小美", "和小美聊過，對方提到「好美的世界」"),
        ];
        let page = format_diary_page("vox_res_0", "露娜", Some("我想蓋一座觀星塔"), &memories);
        assert_eq!(page.resident_id, "vox_res_0");
        assert_eq!(page.resident_name, "露娜");
        assert_eq!(page.desire.as_deref(), Some("我想蓋一座觀星塔"));
        assert_eq!(page.entries.len(), 2);
        // 順序保留（呼叫端傳入的順序）。
        assert_eq!(page.entries[0].seq, 2);
        assert_eq!(page.entries[0].player, "阿星");
        assert_eq!(page.entries[1].seq, 1);
    }

    #[test]
    fn format_diary_page_no_desire() {
        let memories = vec![make_entry(0, "路人", "和路人聊過，對方提到「你好」")];
        let page = format_diary_page("vox_res_1", "諾娃", None, &memories);
        assert!(page.desire.is_none(), "沒心願時 desire 應為 None");
        assert_eq!(page.entries.len(), 1);
    }

    #[test]
    fn format_diary_page_empty_memories() {
        let page = format_diary_page("vox_res_2", "賽勒", Some("我想釣魚"), &[]);
        assert_eq!(page.entries.len(), 0, "沒記憶時 entries 應為空");
        assert!(page.desire.is_some(), "但仍有心願");
    }

    #[test]
    fn format_diary_page_all_empty() {
        let page = format_diary_page("vox_res_3", "奧瑞", None, &[]);
        assert!(page.desire.is_none());
        assert!(page.entries.is_empty());
    }

    #[test]
    fn has_diary_content_rules() {
        // 有心願 → 有內容。
        assert!(has_diary_content(Some("我想種花"), 0));
        // 有記憶 → 有內容。
        assert!(has_diary_content(None, 1));
        // 兩者皆有 → 有內容。
        assert!(has_diary_content(Some("心願"), 5));
        // 兩者皆無 → 無內容。
        assert!(!has_diary_content(None, 0));
        // 空字串心願 → 視為無心願。
        assert!(!has_diary_content(Some(""), 0));
    }

    #[test]
    fn diary_entry_fields_match_memory() {
        let m = make_entry(99, "小玲", "和小玲聊過，對方提到「謝謝你的記憶」");
        let page = format_diary_page("r", "測試居民", None, &[m]);
        let e = &page.entries[0];
        assert_eq!(e.seq, 99);
        assert_eq!(e.player, "小玲");
        assert!(e.text.contains("謝謝你的記憶"));
    }
}
