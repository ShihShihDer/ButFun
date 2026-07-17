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
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// 社交台詞的全域輪換計數器（tick/seq 維度）。
///
/// 每次產生一句主動搭話／回應就 +1，讓「同一對居民在不同時候」也用不同模板——
/// 光靠 pair 雜湊只能保證「對不同人不同句」，加上這個計數器才能保證「同一對人、
/// 不同次搭話也會輪替」，把原本 94.7% 的重複攤開。純程序化、零 LLM、無鎖 await。
static SOCIAL_ROTATION: AtomicU64 = AtomicU64::new(0);

/// 取下一個輪換序號並自增（wrapping，永不 panic）。
fn next_rotation() -> u64 {
    SOCIAL_ROTATION.fetch_add(1, Ordering::Relaxed)
}

/// 單一名字的位元組雜湊（FNV-1a 風格；確定性、跨平台一致）。
fn name_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// 把一對居民名字摺成穩定的「pair 雜湊」——順序無關（a↔b 與 b↔a 同值），
/// 讓「這一對人」有自己的一組措辭傾向；再和輪換序號一起決定實際模板索引。
fn pair_hash(a: &str, b: &str) -> u64 {
    // 相加使其對稱（與傳入順序無關），再乘散一下避免碰撞集中。
    name_hash(a).wrapping_add(name_hash(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// 由 (pair 雜湊, 輪換序號) 共同算出模板索引——這是「pair-local 輪換」的核心：
/// 同一對人不同次搭話會遞進換句；不同對人起手索引也不同。
fn pair_rotated_index(a: &str, b: &str, rotation: u64, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (pair_hash(a, b).wrapping_add(rotation) % n as u64) as usize
}

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

/// 把「居民說的話」格式化成另一位居民的旁聽摘要。
///
/// **加觀察脈絡維度**：原本一律「聽到X說『…』」，一整面記憶牆讀起來全長一樣。
/// 改成依「說話者名字 × 話語內容」共同決定旁聽框——同一人不同話、不同人同話，
/// 前綴措辭都會換（「聽到」「路過時聽見」「隱約聽到」…），去除罐頭感。
/// 純確定性、零 LLM：相同 (name, text) 永遠得同一句（記憶穩定、可測）。
///
/// 空文字 → None（不存入記憶）。
pub fn overhear_summary(speaker_name: &str, text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let snippet: String = t.chars().take(SOCIAL_SUMMARY_MAX_CHARS).collect();
    // 觀察脈絡：由 (說話者, 內容) 雜湊選旁聽框，讓摘要不再千篇一律「聽到…說」。
    let frames = [
        format!("聽到{speaker_name}說「{snippet}」"),
        format!("路過時聽見{speaker_name}提起「{snippet}」"),
        format!("隱約聽到{speaker_name}念著「{snippet}」"),
        format!("{speaker_name}在旁邊說「{snippet}」，我都聽見了"),
        format!("恰好聽到{speaker_name}講「{snippet}」"),
    ];
    let idx = (name_hash(speaker_name).wrapping_add(name_hash(&snippet)) % frames.len() as u64) as usize;
    Some(frames[idx].clone())
}

/// 居民主動搭話另一位居民的開場白（程式化、零 LLM）。
/// 若有心願就自然帶進去，否則說生活閒事。字元數控制在 `SOCIAL_SAY_CHARS` 以內。
///
/// **pair-local 輪換**：索引不再只按說話者名字固定，改由 (說話者, 目標, 全域輪換序號)
/// 共同決定——同一人對不同對象、同一對人不同次搭話，都會換句，去除 94.7% 的重複。
pub fn resident_social_initiation(speaker_name: &str, target_name: &str, speaker_desire: Option<&str>) -> String {
    let rot = next_rotation();
    if let Some(desire) = speaker_desire {
        // 帶心願：截短心願到 12 字元，讓整句在泡泡框內顯示完整。
        let d: String = desire.chars().take(12).collect();
        let templates = [
            format!("{target_name}，我一直想著「{d}」，你呢？"),
            format!("{target_name}，能告訴你心願嗎？「{d}」"),
            format!("{target_name}，我有個夢想想說：「{d}」。"),
            format!("{target_name}，你有過夢想嗎？我一直惦記著「{d}」。"),
            format!("{target_name}，說來你可能笑，我還在想「{d}」。"),
            format!("{target_name}，願望這種事，你懂嗎？我想「{d}」。"),
        ];
        let idx = pair_rotated_index(speaker_name, target_name, rot, templates.len());
        clamp_say(&templates[idx])
    } else {
        let templates = [
            format!("{target_name}，今天有去哪走走嗎？"),
            format!("{target_name}，這裡真靜，你也這樣覺得？"),
            format!("{target_name}，碰到你真好。"),
            format!("{target_name}，你有發現什麼有趣的？"),
            format!("{target_name}，最近過得還好嗎？"),
            format!("{target_name}，風有點涼，你不冷吧？"),
            format!("{target_name}，好久沒好好聊聊了呢。"),
            format!("{target_name}，你手上在忙什麼呀？"),
        ];
        let idx = pair_rotated_index(speaker_name, target_name, rot, templates.len());
        clamp_say(&templates[idx])
    }
}

/// 目標居民延遲後的回應（程式化、零 LLM）。
///
/// 同 `resident_social_initiation`：pair-local 輪換——回應對象／輪次不同，措辭就換。
pub fn resident_social_response(responder_name: &str, initiator_name: &str) -> String {
    let rot = next_rotation();
    let templates = [
        format!("{initiator_name}，你說的讓我想了好久。"),
        format!("{initiator_name}，原來你也這樣想！"),
        format!("{initiator_name}，聽到你說這個真好。"),
        format!("{initiator_name}，我也有話想說呢。"),
        format!("{initiator_name}，被你這麼一提，我也有感觸。"),
        format!("{initiator_name}，能碰上你聊這個，真巧。"),
        format!("{initiator_name}，你總能說到我心坎裡。"),
        format!("{initiator_name}，嗯，我懂你的意思。"),
    ];
    let idx = pair_rotated_index(responder_name, initiator_name, rot, templates.len());
    clamp_say(&templates[idx])
}

/// 把台詞截到泡泡字元上限內，防溢框（呼叫端本也會截，這裡多一道保險）。
fn clamp_say(s: &str) -> String {
    s.chars().take(SOCIAL_SAY_CHARS).collect()
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

    // ── pair-local 輪換：同一人對不同對象 → 不同開場（去罐頭核心保證）──────────────

    /// 純函式檢查：pair_rotated_index 對「不同對象」在**同一輪次**會落到不同索引
    /// （雜湊決定，不受全域計數器干擾，可穩定斷言）。
    #[test]
    fn pair_index_differs_by_target_same_rotation() {
        // 固定 rotation=0，只變目標對象。8 個模板足以讓多數目標落到不同槽。
        let n = 8usize;
        let a = pair_rotated_index("露娜", "諾娃", 0, n);
        let b = pair_rotated_index("露娜", "星辰", 0, n);
        let c = pair_rotated_index("露娜", "河圖", 0, n);
        // 至少三者不全相同（同一人對不同人不會固定同句）。
        assert!(
            !(a == b && b == c),
            "同一說話者對不同對象在同輪次不應全落同一模板：{a},{b},{c}"
        );
    }

    /// pair_hash 對稱：a↔b 與 b↔a 同值（「這一對人」概念與傳入順序無關）。
    #[test]
    fn pair_hash_is_symmetric() {
        assert_eq!(pair_hash("露娜", "諾娃"), pair_hash("諾娃", "露娜"));
        assert_ne!(pair_hash("露娜", "諾娃"), pair_hash("露娜", "星辰"));
    }

    /// 輪換序號推進 → 同一對人也會換句（tick/seq 維度）。
    #[test]
    fn pair_index_rotates_over_seq() {
        let n = 8usize;
        let mut seen = std::collections::HashSet::new();
        for rot in 0..(n as u64) {
            seen.insert(pair_rotated_index("露娜", "諾娃", rot, n));
        }
        // 連續 n 個輪次應覆蓋全部 n 個索引（rotation 以 +1 步進、模 n）。
        assert_eq!(seen.len(), n, "同一對人連續輪次應輪遍所有模板");
    }

    /// 端到端：同一發起者對不同對象，連呼叫多次，開場白集合應多樣（非千篇一律）。
    #[test]
    fn initiation_varies_across_targets_and_calls() {
        let mut lines = std::collections::HashSet::new();
        for target in ["諾娃", "星辰", "河圖", "青禾"] {
            for _ in 0..3 {
                lines.insert(resident_social_initiation("露娜", target, None));
            }
        }
        // 4 對象 × 3 次，全域計數器推進 + pair 雜湊 → 應遠多於 1 句。
        assert!(lines.len() >= 4, "開場白應多樣、非罐頭，實得 {}", lines.len());
    }

    /// 回應同理：對不同發起者、多次呼叫，應輪出多樣句子。
    #[test]
    fn response_varies_across_initiators_and_calls() {
        let mut lines = std::collections::HashSet::new();
        for initiator in ["諾娃", "星辰", "河圖", "青禾"] {
            for _ in 0..3 {
                lines.insert(resident_social_response("露娜", initiator));
            }
        }
        assert!(lines.len() >= 4, "回應應多樣、非罐頭，實得 {}", lines.len());
    }

    /// 帶心願的開場也走輪換，且句子仍在泡泡上限內、含心願截短版。
    #[test]
    fn initiation_with_desire_rotates_and_stays_short() {
        let mut lines = std::collections::HashSet::new();
        for _ in 0..8 {
            let l = resident_social_initiation("露娜", "諾娃", Some("蓋一座觀星塔"));
            assert!(l.chars().count() <= SOCIAL_SAY_CHARS, "應在泡泡上限內：{l}");
            assert!(l.contains("蓋一座觀"), "應含心願截短版：{l}");
            lines.insert(l);
        }
        assert!(lines.len() >= 2, "同一對人多次帶心願開場也應輪換，實得 {}", lines.len());
    }

    // ── overhear_summary 加觀察脈絡維度 ────────────────────────────────────────

    /// 不同說話者 → 旁聽框措辭可不同（觀察脈絡維度，非一律「聽到…說」）。
    #[test]
    fn overhear_summary_frame_varies_by_speaker() {
        let mut frames = std::collections::HashSet::new();
        for name in ["露娜", "諾娃", "星辰", "河圖", "青禾", "白露", "子規"] {
            frames.insert(overhear_summary(name, "我想蓋觀星塔").unwrap());
        }
        // 7 位說話者，雜湊選框後應出現多於一種措辭（不再千篇一律）。
        assert!(frames.len() >= 2, "旁聽框應隨脈絡變化，實得 {}", frames.len());
    }

    /// 觀察脈絡是確定性的：相同 (name, text) 永遠得同一句（記憶穩定）。
    #[test]
    fn overhear_summary_is_deterministic() {
        let a = overhear_summary("露娜", "我想蓋觀星塔").unwrap();
        let b = overhear_summary("露娜", "我想蓋觀星塔").unwrap();
        assert_eq!(a, b, "相同輸入應得相同摘要");
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
