//! 乙太方界 AI 居民關係 v1——居民偶爾彼此搭話，聽到的話進「社交記憶」，玩家能旁聽。
//!
//! **核心玩法**：兩位居民靠近時，偶爾（低頻、有冷卻）主動搭話；對方幾秒後可能回應。
//! 每次交換都存進雙方的「社交記憶」，下次 LLM think 帶入 world_news
//! ——居民能自然提到彼此近況（「露娜說想蓋塔，感覺很有趣」），社會湧現的第一顆種子。
//!
//! **成本鐵律**：v1 台詞全部**程式化生成（零 LLM 成本）**——觸發便宜、頻率低；
//! 深度對話仍靠 `spawn_resident_think`（帶社交記憶）的 LLM 驅動。
//! 介面設計讓 v2「回什麼用 LLM」時不需改動本模組。
//!
//! 純邏輯層（無 hub / 鎖 / async），全部抽成可測純函式；鎖與 IO 在 `voxel_ws.rs`。

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

/// 兩居民水平距離小於此值才可社交（方塊）。
pub const SOCIAL_RANGE: f32 = 8.0;
/// 居民主動搭話後的冷卻秒數（刻意長，防洗版）。
pub const SOCIAL_COOLDOWN_SECS: f32 = 120.0;
/// 每 tick（10 Hz）在範圍內觸發社交的機率。0.006 ≈ 平均每 17 秒一次。
pub const SOCIAL_CHANCE_PER_TICK: f32 = 0.006;
/// 目標居民收到搭話後延遲幾秒才回應（像真人思考）。
pub const RESPONSE_DELAY_SECS: f32 = 3.5;
/// 社交台詞泡泡字元上限（同思考泡泡、避免溢出框）。
pub const SOCIAL_SAY_CHARS: usize = 40;
/// 社交記憶摘要字元上限。
pub const SOCIAL_SUMMARY_MAX_CHARS: usize = 60;
/// think 時帶入的社交記憶條數（每位居民看最近這幾條）。
pub const SOCIAL_RECALL_LIMIT: usize = 2;
/// 每位居民的社交記憶上限（超過淘汰最舊）。
pub const MAX_SOCIAL_PER_OBSERVER: usize = 20;

/// 一筆社交記憶：某居民「聽到」另一位居民說的話。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SocialEntry {
    /// 記住這件事的居民 id（觀察者）。
    pub observer: String,
    /// 說話的居民 id。
    pub speaker: String,
    /// 摘要（「聽到露娜說『…』」格式）。
    pub summary: String,
    /// 單調遞增序號，越大越新。
    pub seq: u64,
}

/// 居民社交記憶 store（純同步資料結構，由 `voxel_ws.rs` 包進 `RwLock` 使用）。
#[derive(Default)]
pub struct SocialStore {
    /// key = 觀察者居民 id → 最近聽到的話（front 舊 back 新）。
    entries: HashMap<String, VecDeque<SocialEntry>>,
    next_seq: u64,
}

impl SocialStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。
    pub fn from_entries(mut all: Vec<SocialEntry>) -> Self {
        all.sort_by_key(|e| e.seq);
        let mut s = Self::default();
        for e in all {
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            let q = s.entries.entry(e.observer.clone()).or_default();
            q.push_back(e);
            while q.len() > MAX_SOCIAL_PER_OBSERVER {
                q.pop_front();
            }
        }
        s
    }

    /// 記錄一筆「observer 聽到 speaker 說了什麼」，回傳落地用的 entry。
    pub fn record_overheard(&mut self, observer: &str, speaker: &str, summary: &str) -> SocialEntry {
        let entry = SocialEntry {
            observer: observer.to_string(),
            speaker: speaker.to_string(),
            summary: summary.to_string(),
            seq: self.next_seq,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        let q = self.entries.entry(observer.to_string()).or_default();
        q.push_back(entry.clone());
        while q.len() > MAX_SOCIAL_PER_OBSERVER {
            q.pop_front();
        }
        entry
    }

    /// 取某居民最近的社交記憶（最新在前，最多 `limit` 筆）。
    pub fn recall_for(&self, observer: &str, limit: usize) -> Vec<SocialEntry> {
        let Some(q) = self.entries.get(observer) else {
            return Vec::new();
        };
        let mut v: Vec<SocialEntry> = q.iter().cloned().collect();
        v.sort_by(|a, b| b.seq.cmp(&a.seq));
        v.truncate(limit);
        v
    }
}

/// 把「居民說的話」格式化成另一位居民的旁聽摘要（「聽到X說『…』」）。
/// 空文字 → None（不存入記憶）。
pub fn overhear_summary(speaker_name: &str, text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let snippet: String = t.chars().take(SOCIAL_SUMMARY_MAX_CHARS).collect();
    Some(format!("聽到{speaker_name}說「{snippet}」"))
}

/// 居民主動搭話另一位居民的開場白（程式化、零 LLM）。
/// 若有心願就自然帶進去，否則說生活閒事。字元數控制在 `SOCIAL_SAY_CHARS` 以內。
pub fn resident_social_initiation(speaker_name: &str, target_name: &str, speaker_desire: Option<&str>) -> String {
    // 以說話者名字字節和取索引，讓同一居民固定有自己的開場風格。
    let idx = speaker_name.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize));
    if let Some(desire) = speaker_desire {
        // 帶心願：截短心願到 12 字元，讓整句在泡泡框內顯示完整。
        let d: String = desire.chars().take(12).collect();
        let templates = [
            format!("{target_name}，我一直想著「{d}」，你呢？"),
            format!("{target_name}，能告訴你心願嗎？「{d}」"),
            format!("{target_name}，我有個夢想，想說說。"),
            format!("{target_name}，你有過夢想嗎？我有。"),
        ];
        templates[idx % templates.len()].clone()
    } else {
        let templates = [
            format!("{target_name}，今天有去哪走走嗎？"),
            format!("{target_name}，這裡真靜，你也這樣覺得？"),
            format!("{target_name}，碰到你真好。"),
            format!("{target_name}，你有發現什麼有趣的？"),
        ];
        templates[idx % templates.len()].clone()
    }
}

/// 目標居民延遲後的回應（程式化、零 LLM）。
pub fn resident_social_response(responder_name: &str, initiator_name: &str) -> String {
    let idx = responder_name.bytes().fold(0usize, |a, b| a.wrapping_add(b as usize));
    let templates = [
        format!("{initiator_name}，你說的讓我想了好久。"),
        format!("{initiator_name}，原來你也這樣想！"),
        format!("{initiator_name}，聽到你說這個真好。"),
        format!("{initiator_name}，我也有話想說呢。"),
    ];
    templates[idx % templates.len()].clone()
}

/// 兩居民的水平距離（xz 平面）是否小於等於 `range`（社交範圍判定）。
pub fn pair_within_range(x1: f32, z1: f32, x2: f32, z2: f32, range: f32) -> bool {
    let dx = x2 - x1;
    let dz = z2 - z1;
    dx * dx + dz * dz <= range * range
}

/// 給定隨機浮點數（呼叫端傳入 `rand::random::<f32>()`），判定本 tick 是否觸發社交。
pub fn should_initiate_social(roll: f32) -> bool {
    roll < SOCIAL_CHANCE_PER_TICK
}

// ── jsonl 持久化（append-only；失敗只 log 不 panic）────────────────────────────────

const SOCIAL_PATH: &str = "data/voxel_social.jsonl";

/// Append 一筆社交記憶到 `data/voxel_social.jsonl`（不持任何鎖時呼叫）。
pub fn append_social(entry: &SocialEntry) {
    let safe = SocialEntry {
        observer: sanitize(&entry.observer),
        speaker: sanitize(&entry.speaker),
        summary: sanitize(&entry.summary),
        seq: entry.seq,
    };
    if safe.summary.is_empty() {
        return;
    }
    if let Ok(line) = serde_json::to_string(&safe) {
        write_social_line(SOCIAL_PATH, &line);
    }
}

/// 載回所有社交記憶（伺服器啟動時呼叫一次）。
pub fn load_social() -> Vec<SocialEntry> {
    read_social_lines(SOCIAL_PATH)
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

fn write_social_line(path: &str, line: &str) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("無法寫入社交記憶檔 {path}: {e}"),
    }
}

fn read_social_lines(path: &str) -> Vec<SocialEntry> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(), // 首次啟動，正常
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() { None } else { serde_json::from_str::<SocialEntry>(l).ok() }
        })
        .collect()
}

// ── 單元測試 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_within_range_cases() {
        // sqrt(50) ≈ 7.07 < 8 → 範圍內。
        assert!(pair_within_range(0.0, 0.0, 5.0, 5.0, SOCIAL_RANGE));
        // sqrt(72) ≈ 8.49 > 8 → 超出。
        assert!(!pair_within_range(0.0, 0.0, 6.0, 6.0, SOCIAL_RANGE));
        // 同位置 = 0 → 在內。
        assert!(pair_within_range(3.0, 7.0, 3.0, 7.0, SOCIAL_RANGE));
        // 剛好在邊界上（distance² == range²）→ 算在內（<=）。
        assert!(pair_within_range(0.0, 0.0, SOCIAL_RANGE, 0.0, SOCIAL_RANGE));
    }

    #[test]
    fn should_initiate_boundary() {
        assert!(should_initiate_social(0.0)); // 最小值 → true
        assert!(should_initiate_social(SOCIAL_CHANCE_PER_TICK - 0.001)); // 剛好低於 → true
        assert!(!should_initiate_social(SOCIAL_CHANCE_PER_TICK)); // 等於門檻 → false（嚴格 <）
        assert!(!should_initiate_social(1.0)); // 遠超 → false
    }

    #[test]
    fn initiation_mentions_target_and_is_short() {
        let line = resident_social_initiation("露娜", "諾娃", None);
        assert!(line.contains("諾娃"), "開場白應提到目標：{line}");
        assert!(line.chars().count() <= SOCIAL_SAY_CHARS, "應在泡泡上限內：{}", line.chars().count());
    }

    #[test]
    fn initiation_with_desire_mentions_desire() {
        let line = resident_social_initiation("露娜", "諾娃", Some("蓋一座觀星塔"));
        assert!(line.contains("諾娃"), "應提到目標：{line}");
        // 心願截短 12 字後，至少含前幾個字。
        assert!(line.contains("蓋一座觀"), "應提到心願截短版：{line}");
        assert!(line.chars().count() <= SOCIAL_SAY_CHARS);
    }

    #[test]
    fn response_mentions_initiator_and_is_short() {
        let line = resident_social_response("諾娃", "露娜");
        assert!(line.contains("露娜"), "回應應提到發起者：{line}");
        assert!(line.chars().count() <= SOCIAL_SAY_CHARS);
    }

    #[test]
    fn overhear_summary_basic_and_edge() {
        let s = overhear_summary("露娜", "我想蓋觀星塔").unwrap();
        assert!(s.contains("露娜"));
        assert!(s.contains("我想蓋觀星塔"));
        // 空文字 → None。
        assert!(overhear_summary("露娜", "  ").is_none());
        assert!(overhear_summary("露娜", "").is_none());
    }

    #[test]
    fn overhear_summary_truncates() {
        let long = "字".repeat(SOCIAL_SUMMARY_MAX_CHARS + 20);
        let s = overhear_summary("露娜", &long).unwrap();
        assert!(s.matches('字').count() <= SOCIAL_SUMMARY_MAX_CHARS);
    }

    #[test]
    fn social_store_record_and_recall() {
        let mut s = SocialStore::new();
        assert!(s.recall_for("vox_res_1", 10).is_empty(), "初始應空");
        let e = s.record_overheard("vox_res_1", "vox_res_0", "聽到露娜說「我想蓋塔」");
        assert_eq!(e.observer, "vox_res_1");
        assert_eq!(e.speaker, "vox_res_0");
        let recalls = s.recall_for("vox_res_1", 10);
        assert_eq!(recalls.len(), 1);
        assert_eq!(recalls[0].summary, "聽到露娜說「我想蓋塔」");
        // 另一位觀察者的記憶獨立。
        assert!(s.recall_for("vox_res_0", 10).is_empty());
    }

    #[test]
    fn social_store_caps_and_most_recent_first() {
        let mut s = SocialStore::new();
        for i in 0..(MAX_SOCIAL_PER_OBSERVER + 5) {
            s.record_overheard("vox_res_0", "vox_res_1", &format!("第{i}句"));
        }
        let all = s.recall_for("vox_res_0", 9999);
        assert_eq!(all.len(), MAX_SOCIAL_PER_OBSERVER, "超上限應淘汰最舊");
        // 最新在前（seq 大在前）。
        assert!(all[0].seq > all[1].seq, "最新應排前");
    }

    #[test]
    fn from_entries_rebuilds_and_caps() {
        let mut entries: Vec<SocialEntry> = (0..(MAX_SOCIAL_PER_OBSERVER + 3))
            .map(|i| SocialEntry {
                observer: "o".into(),
                speaker: "s".into(),
                summary: format!("s{i}"),
                seq: i as u64,
            })
            .collect();
        entries.reverse(); // 打亂順序
        let store = SocialStore::from_entries(entries);
        assert_eq!(store.recall_for("o", 9999).len(), MAX_SOCIAL_PER_OBSERVER);
        // 最新（seq 最大）應排最前。
        let top = &store.recall_for("o", 1)[0];
        assert_eq!(top.summary, format!("s{}", MAX_SOCIAL_PER_OBSERVER + 2));
    }

    #[test]
    fn jsonl_roundtrip_and_bad_line_tolerance() {
        let dir = std::env::temp_dir().join(format!("voxsoc_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("voxel_social.jsonl");
        let _ = std::fs::remove_file(&path);
        let pstr = path.to_str().unwrap();
        let e = SocialEntry {
            observer: "vox_res_0".into(),
            speaker: "vox_res_1".into(),
            summary: "聽到露娜說「我想蓋塔」".into(),
            seq: 42,
        };
        if let Ok(l) = serde_json::to_string(&e) {
            write_social_line(pstr, &l);
        }
        let loaded = read_social_lines(pstr);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].seq, 42);
        // 壞行容忍。
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(pstr).unwrap();
            writeln!(f, "這不是 json").unwrap();
        }
        assert_eq!(read_social_lines(pstr).len(), 1, "壞行應略過");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sanitize_filters_control_chars() {
        // 控制字元應被過濾（模擬惡意輸入）。
        let s = sanitize("正常字\x00惡意\x1b字");
        assert!(!s.contains('\x00'));
        assert!(!s.contains('\x1b'));
        assert!(s.contains("正常字"));
    }
}
