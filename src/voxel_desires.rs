//! 乙太方界 AI 居民渴望系統——玩家的話種下居民的夢想，記憶驅動行為 v1。
//!
//! **核心信念**：記憶不只是「聊天記錄」，而要**驅動居民的日常行為**。
//! 本模組讓居民從對話的 LLM 回覆中「萌生心願」，並把它帶進後續每次的思考與對話——
//! 玩家親眼看到「我說過的話，真的改變了這個居民」（記憶驅動行為 v1）。
//!
//! **純邏輯層**：`extract_desire`（規則擷取，確定性、可測、零 LLM 成本）、
//! `DesireStore`（記憶體側每居民一筆「當前心願」+ jsonl 持久化）。
//! 鎖 / 連線 / LLM 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO 外包。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 居民心願摘要的字元上限（截過長的 LLM 輸出）。
pub const DESIRE_MAX_CHARS: usize = 40;
/// 最短有意義心願字元數——太短（如觸發詞本身）視為無意義，略過。
const DESIRE_MIN_CHARS: usize = 4;

/// `sparked_by` 的自我啟發哨兵值——ROADMAP 6「禱告驅動蓋家」：居民自己的禱告
/// （非玩家聊天）萌生的心願用這個標記，供顯示/思考 prompt 判斷來源、選用不同措辭。
pub const SELF_SPARK: &str = "自己";

/// 一筆居民心願（持久化單位）。
///
/// 刻意不寫系統時鐘（對齊本專案避 `SystemTime::now` 慣例）：用 `seq` 當相對先後。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResidentDesire {
    /// 居民系統 id（如 "vox_res_0"）。
    pub resident: String,
    /// 心願文字（由居民 LLM 回覆自動萃取）。
    pub desire: String,
    /// 啟發者：帶出這個心願的玩家顯示名。
    pub sparked_by: String,
    /// 單調遞增序號（越大越新）；從 jsonl 還原時用來保留「最新那筆」。
    pub seq: u64,
}

/// 居民心願 store：每位居民最多一個「當前心願」（後者覆蓋前者）。
#[derive(Default)]
pub struct DesireStore {
    desires: HashMap<String, ResidentDesire>,
    next_seq: u64,
}

impl DesireStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 由持久化記錄還原（重啟時從 jsonl 載回）。同一居民多筆只保 seq 最大（最新）的一筆。
    pub fn from_entries(entries: Vec<ResidentDesire>) -> Self {
        let mut s = Self::default();
        for e in entries {
            // next_seq 設為比所有已知 seq 大 1，確保新記錄不撞號。
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            let current_seq = s.desires.get(&e.resident).map(|c| c.seq);
            if current_seq.map_or(true, |cs| e.seq > cs) {
                s.desires.insert(e.resident.clone(), e);
            }
        }
        s
    }

    /// 更新（或新建）居民的當前心願。回傳新 entry 供呼叫端落地 jsonl。
    pub fn set_desire(&mut self, resident: &str, desire: &str, sparked_by: &str) -> ResidentDesire {
        let entry = ResidentDesire {
            resident: resident.to_string(),
            desire: desire.to_string(),
            sparked_by: sparked_by.to_string(),
            seq: self.next_seq,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        self.desires.insert(resident.to_string(), entry.clone());
        entry
    }

    /// 取居民的當前心願（`None` = 尚未有任何心願）。
    pub fn get_desire(&self, resident: &str) -> Option<&ResidentDesire> {
        self.desires.get(resident)
    }
}

/// 由 LLM 回覆規則擷取第一個「渴望 / 心願」句（不另呼 LLM，省成本、確定性、可測）。
///
/// 找第一個觸發詞（"我想"、"我希望" 等），取「觸發詞到下一個句尾標點」的片段，
/// 截至 [`DESIRE_MAX_CHARS`] 字元。找不到觸發詞或片段過短（< [`DESIRE_MIN_CHARS`]）→ `None`。
pub fn extract_desire(reply: &str) -> Option<String> {
    // 較長 / 更具體的觸發詞排前，避免 "我想要" 被 "我想" 提早截斷。
    // 口語願望常省略「我」（實測露娜說「真希望有玻璃」「好想要亮晶晶的玻璃」全被漏接、
    // 維護者自己的說法「真希望有玻璃」也一樣）——補上無主詞的口語觸發詞。
    const TRIGGERS: &[&str] = &[
        "我的夢想是",
        "我的心願是",
        "我夢想著",
        "我夢想",
        "我真的很想",
        "我想要",
        "我渴望",
        "我盼望",
        "我期待",
        "我希望",
        "我打算",
        "我想",
        "真希望",
        "好希望",
        "好想要",
        "好想",
        "要是有",
        "如果能有",
    ];

    for trigger in TRIGGERS {
        let Some(pos) = reply.find(trigger) else {
            continue;
        };
        let tail = &reply[pos..];
        // 找最近的句尾標點（掃全尾巴，不預設長度）。
        let end_byte = tail
            .char_indices()
            .find(|(_, c)| matches!(c, '。' | '！' | '？' | '\n' | '!' | '?'))
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(tail.len());
        // 去掉句尾標點 + 空白，再按字元截到上限。
        let phrase = tail[..end_byte]
            .trim_end_matches(|c: char| {
                c.is_whitespace() || matches!(c, '。' | '！' | '？' | '!' | '?')
            });
        let trimmed: String = phrase.chars().take(DESIRE_MAX_CHARS).collect();
        if trimmed.chars().count() >= DESIRE_MIN_CHARS {
            return Some(trimmed);
        }
    }
    None
}

// ── jsonl 持久化（append-only，append 即覆蓋舊願；重啟後 from_entries 取最新一筆）──────────

/// 居民心願落地路徑（`data/` 已 gitignore）。
const VOXEL_DESIRES_PATH: &str = "data/voxel_desires.jsonl";

/// Append 一筆心願到 jsonl。append-only、絕不覆寫 / 刪除既有行；失敗只記 log 不 panic。
///
/// **鐵律**：只在不持任何鎖的情境呼叫（同步小檔寫，不 await）。
pub fn append_desire(entry: &ResidentDesire) {
    let safe = ResidentDesire {
        resident: sanitize_field(&entry.resident),
        desire: sanitize_field(&entry.desire),
        sparked_by: sanitize_field(&entry.sparked_by),
        seq: entry.seq,
    };
    if safe.desire.is_empty() {
        return; // 空心願不落地
    }
    if let Ok(line) = serde_json::to_string(&safe) {
        write_line(VOXEL_DESIRES_PATH, &line);
    }
}

/// 載回所有心願記錄（伺服器啟動時呼叫一次）。檔不存在 / 壞行皆容忍。
pub fn load_desires() -> Vec<ResidentDesire> {
    read_lines(VOXEL_DESIRES_PATH)
}

/// 濾掉控制字元並 trim（避免注入 jsonl 換行）。
fn sanitize_field(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

fn write_line(path: &str, line: &str) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("無法寫入居民心願檔 {path}: {e}"),
    }
}

fn read_lines(path: &str) -> Vec<ResidentDesire> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(), // 首次啟動 = 沒有心願，正常
    };
    content
        .lines()
        .filter_map(|line| {
            let l = line.trim();
            if l.is_empty() {
                None
            } else {
                serde_json::from_str::<ResidentDesire>(l).ok()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_desire 純函式測試 ────────────────────────────────────────────

    #[test]
    fn extract_desire_basic_patterns() {
        // 「我想」後接目的語句，以句號結尾。
        assert_eq!(
            extract_desire("我想在這裡蓋一座觀星塔。"),
            Some("我想在這裡蓋一座觀星塔".to_string())
        );
        // 「我希望」+ 感嘆號結尾。
        assert_eq!(
            extract_desire("我希望有一天能看到滿天星斗！"),
            Some("我希望有一天能看到滿天星斗".to_string())
        );
        // 「我的夢想是」出現在句子中間。
        let r = extract_desire("你好呀！我的夢想是和每個旅人交朋友。");
        assert!(r.as_deref().unwrap_or("").starts_with("我的夢想是"));
    }

    #[test]
    fn extract_desire_longer_trigger_wins() {
        // "我的夢想是" 比 "我想" 更具體，應擷取完整觸發詞開頭。
        let r = extract_desire("我的夢想是種一片花田。");
        assert!(r.as_deref().unwrap_or("").starts_with("我的夢想是"), "應以較長觸發詞開頭");
    }

    #[test]
    fn extract_desire_none_when_no_trigger() {
        assert!(extract_desire("你好，旅人！這裡真美。").is_none());
        assert!(extract_desire("").is_none());
        assert!(extract_desire("   ").is_none());
        // 完全沒有觸發詞的句子。
        assert!(extract_desire("今天天氣不錯，適合散步。").is_none());
    }

    #[test]
    fn extract_desire_truncates_at_max() {
        let long = format!("我想{}", "走".repeat(DESIRE_MAX_CHARS + 20));
        let r = extract_desire(&long).unwrap();
        assert!(r.chars().count() <= DESIRE_MAX_CHARS, "應截到上限");
        assert!(r.starts_with("我想"));
    }

    #[test]
    fn extract_desire_min_length_guard() {
        // 觸發詞後立刻到句尾 → 剩餘太短（< 4 字元）→ None。
        // "我想！" 去標點 = "我想" = 2 字元 < 4。
        assert!(extract_desire("我想！").is_none());
    }

    #[test]
    fn extract_desire_stops_at_first_sentence_boundary() {
        // 「我想蓋塔。然後我想做別的。」應只取到第一個句號。
        let r = extract_desire("我想蓋一座塔。然後我想做別的。").unwrap();
        assert!(!r.contains("然後"), "應只取到第一個句尾");
    }

    #[test]
    fn extract_desire_catches_colloquial_without_subject() {
        // 口語願望常省略「我」——露娜實測原句（曾被漏接，真進化驗證卡在這）。
        let r = extract_desire("露娜，真希望有玻璃啊，好想要亮晶晶的玻璃。").unwrap();
        assert!(r.contains("玻璃"), "「真希望…」該被抽到: {r}");
        // 維護者最早的假想句（「我說真希望有玻璃，NPC 聽到會許願嗎」）。
        let r2 = extract_desire("真希望有玻璃做的窗戶！").unwrap();
        assert!(r2.contains("玻璃"), "{r2}");
        // 「好想要…」也是常見口語。
        let r3 = extract_desire("唉，好想要一張木板做的床呀。").unwrap();
        assert!(r3.contains("木板"), "{r3}");
        // 「要是有…」假設句型。
        let r4 = extract_desire("要是有石磚鋪的路就好了。").unwrap();
        assert!(r4.contains("石磚"), "{r4}");
    }

    // ── DesireStore 純函式測試 ───────────────────────────────────────────────

    #[test]
    fn desire_store_set_and_get() {
        let mut s = DesireStore::new();
        assert!(s.get_desire("vox_res_0").is_none(), "初始應無心願");
        let e = s.set_desire("vox_res_0", "我想蓋一座觀星塔", "旅人");
        assert_eq!(e.desire, "我想蓋一座觀星塔");
        assert_eq!(e.sparked_by, "旅人");
        let got = s.get_desire("vox_res_0").unwrap();
        assert_eq!(got.desire, "我想蓋一座觀星塔");
    }

    #[test]
    fn desire_store_overwrite_keeps_latest() {
        let mut s = DesireStore::new();
        let e1 = s.set_desire("vox_res_0", "我想蓋橋", "阿星");
        let e2 = s.set_desire("vox_res_0", "我想種花田", "小美");
        assert!(e2.seq > e1.seq, "新心願 seq 應更大");
        assert_eq!(s.get_desire("vox_res_0").unwrap().desire, "我想種花田", "後者應覆蓋前者");
    }

    #[test]
    fn desire_store_independent_per_resident() {
        let mut s = DesireStore::new();
        s.set_desire("vox_res_0", "我想蓋塔", "阿星");
        s.set_desire("vox_res_1", "我想種田", "小美");
        assert_eq!(s.get_desire("vox_res_0").unwrap().desire, "我想蓋塔");
        assert_eq!(s.get_desire("vox_res_1").unwrap().desire, "我想種田");
        assert!(s.get_desire("vox_res_2").is_none());
    }

    #[test]
    fn from_entries_keeps_latest_per_resident() {
        let entries = vec![
            ResidentDesire {
                resident: "vox_res_0".into(),
                desire: "舊願望".into(),
                sparked_by: "a".into(),
                seq: 0,
            },
            ResidentDesire {
                resident: "vox_res_0".into(),
                desire: "新願望".into(),
                sparked_by: "b".into(),
                seq: 5,
            },
            ResidentDesire {
                resident: "vox_res_1".into(),
                desire: "另一個".into(),
                sparked_by: "c".into(),
                seq: 3,
            },
        ];
        let s = DesireStore::from_entries(entries);
        assert_eq!(s.get_desire("vox_res_0").unwrap().desire, "新願望", "應保留 seq 最大的");
        assert_eq!(s.get_desire("vox_res_1").unwrap().desire, "另一個");
        assert!(s.get_desire("vox_res_2").is_none());
    }

    #[test]
    fn from_entries_seq_continues_after_max() {
        let entries = vec![ResidentDesire {
            resident: "r".into(),
            desire: "v".into(),
            sparked_by: "p".into(),
            seq: 100,
        }];
        let mut s = DesireStore::from_entries(entries);
        let e = s.set_desire("r", "新的", "p2");
        assert!(e.seq > 100, "新 seq 應接在既有最大 seq 之後：seq={}", e.seq);
    }

    #[test]
    fn jsonl_roundtrip() {
        let dir = std::env::temp_dir().join(format!("voxdes_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("voxel_desires.jsonl");
        let _ = std::fs::remove_file(&path);
        let pstr = path.to_str().unwrap();

        let e1 = ResidentDesire {
            resident: "vox_res_0".into(),
            desire: "我想蓋一座塔".into(),
            sparked_by: "旅人".into(),
            seq: 1,
        };
        let e2 = ResidentDesire {
            resident: "vox_res_0".into(),
            desire: "我想種花田".into(),
            sparked_by: "小美".into(),
            seq: 2,
        };
        if let Ok(l) = serde_json::to_string(&e1) {
            write_line(pstr, &l);
        }
        if let Ok(l) = serde_json::to_string(&e2) {
            write_line(pstr, &l);
        }
        let loaded = read_lines(pstr);
        assert_eq!(loaded.len(), 2, "兩筆都應讀回");
        // from_entries 只保最新一筆。
        let s = DesireStore::from_entries(loaded);
        assert_eq!(s.get_desire("vox_res_0").unwrap().desire, "我想種花田");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bad_line_is_skipped() {
        let dir = std::env::temp_dir().join(format!("voxdes_bad_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("voxel_desires.jsonl");
        let _ = std::fs::remove_file(&path);
        let pstr = path.to_str().unwrap();
        write_line(pstr, "這不是 json");
        let e = ResidentDesire {
            resident: "r".into(),
            desire: "v".into(),
            sparked_by: "p".into(),
            seq: 0,
        };
        if let Ok(l) = serde_json::to_string(&e) {
            write_line(pstr, &l);
        }
        let loaded = read_lines(pstr);
        assert_eq!(loaded.len(), 1, "壞行應略過，只讀回合法一筆");
        let _ = std::fs::remove_file(&path);
    }
}
