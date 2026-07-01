//! 乙太方界 AI 居民記憶系統 v1——純邏輯 + jsonl 持久化（讓居民記得你、記得聊過什麼）。
//!
//! 這是「AI 棲居世界」的靈魂：居民跨 session 記得對話、記得玩家、記得發生過的事。
//! 本模組**只放與連線/鎖/LLM 無關的確定性純邏輯**（短期對話歷史、長期記憶累積/淘汰/回想、
//! 摘要擷取）與一個輕量 jsonl 持久化層（比照 `npc_agent::append_prayer` 的做法）。
//! 真正的 tick 驅動、廣播、無鎖 async 思考、鎖的取放都在 `voxel_ws.rs`，嚴守 prod 死鎖鐵律：
//! 短鎖快照 → drop → spawn → 下一步套用，**記憶讀寫絕不在持鎖中 await**。
//!
//! 兩層記憶：
//! 1. **短期對話歷史**：每對 `(玩家, 居民)` 維護最近 [`HISTORY_TURNS`] 輪，餵進 prompt → 對話連貫。
//!    純記憶體（session 內），重啟即清——對話連貫只需「這段對話」的脈絡。
//! 2. **長期記憶**：每位居民一份**累積記憶**（記「跟哪個玩家聊過、重點、相對先後」），
//!    capped + 淘汰最舊，**持久化到 `data/voxel_memory.jsonl`**，重啟後載回 → 記得你上次說過的事。
//!
//! 全部抽成可測純函式；不抄外部碼、繁中註解；機敏值不涉入；**append-only、絕不刪既有玩家資料**。

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

/// 短期對話歷史每對 `(玩家, 居民)` 保留的輪數（一輪 = 玩家一句 + 居民一句）。
/// 取小：夠維持「這段對話」連貫，又不讓 prompt 爆長（成本/延遲鐵律）。
pub const HISTORY_TURNS: usize = 6;
/// 每位居民長期記憶的上限筆數：超過就淘汰最舊（記憶體側 cap，prompt 不爆、回想不慢）。
pub const MAX_MEMORIES_PER_RESIDENT: usize = 40;
/// 回想時最多撈幾筆「關於這個玩家」的記憶餵進 prompt（最近 + 相關，別過度工程）。
pub const RECALL_LIMIT: usize = 4;
/// 一筆長期記憶摘要的字元上限：規則擷取後截斷，避免單筆塞爆。
pub const SUMMARY_MAX_CHARS: usize = 80;
/// 餵進對話 system prompt 的「脈絡區塊」總字元上限。超過就只留**最近**那一段
/// （近期對話比舊記憶重要）。每次對話少燒 token、免費額度更耐用（成本鐵律）。
pub const MAX_CONTEXT_CHARS: usize = 700;

/// 一輪對話（短期記憶用）：玩家說的 + 居民回的。
#[derive(Clone, Debug, PartialEq)]
pub struct DialogueTurn {
    pub user: String,
    pub reply: String,
}

/// 一筆長期記憶（持久化單位）。刻意**不寫系統時鐘**（對齊本專案避 `SystemTime::now` 慣例）：
/// 用單調遞增的 `seq` 當「相對先後（何時）」的排序鍵——回想只需「最近」順序，不需牆鐘。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// 居民身份（voxel 居民 id，如 "vox_res_0"）。
    pub resident: String,
    /// 玩家身份鍵（登入者帳號 id 或顯示名；訪客用顯示名）。
    pub player: String,
    /// 這次互動摘要（一兩句重點）。
    pub summary: String,
    /// 單調遞增序號：越大越新，回想時據此排「最近」。
    pub seq: u64,
}

/// 乙太方界記憶 store：短期對話歷史 + 長期累積記憶 + 全域序號。
/// 純資料結構，所有方法皆同步、不 await、不碰連線/LLM；由 `voxel_ws.rs` 包進 `RwLock` 使用。
#[derive(Default)]
pub struct VoxelMemory {
    /// 短期對話歷史：key = (玩家身份鍵, 居民 id) → 最近數輪（front 舊、back 新）。
    convos: HashMap<(String, String), VecDeque<DialogueTurn>>,
    /// 長期記憶：key = 居民 id → 該居民累積記憶（front 舊、back 新，capped）。
    long: HashMap<String, VecDeque<MemoryEntry>>,
    /// 模糊印象計數：key = 居民 id → 累計被 cap 淘汰出 `long` 的舊記憶筆數
    /// （記憶 v2「整併/壓縮/封存」的最小可行版）。**刻意不存原文**——被淘汰的
    /// `MemoryEntry.summary` 可能含玩家私下原話，日記系統的隱私鐵律是輸出永不含
    /// 原話（見 `voxel_diary.rs` 檔頭），因此這裡只記「淡忘了幾段」這個安全信號，
    /// 由日記層轉成一句去識別化的反思，而非把舊記憶原封不動存到別處再外洩一次。
    faded_counts: HashMap<String, usize>,
    /// 全域單調序號（下一筆記憶用）。載入時設為「已存在最大 seq + 1」。
    next_seq: u64,
}

impl VoxelMemory {
    /// 空 store（無 DB / 測試）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史記憶重建 store（重啟後從 jsonl 載回）。
    /// 依 `seq` 排序後分配到各居民、各自 cap 到最新 [`MAX_MEMORIES_PER_RESIDENT`] 筆；
    /// `next_seq` 設為最大 seq + 1，確保新記憶序號續接不撞號。
    pub fn from_entries(mut entries: Vec<MemoryEntry>) -> Self {
        entries.sort_by_key(|e| e.seq);
        let mut long: HashMap<String, VecDeque<MemoryEntry>> = HashMap::new();
        let mut faded_counts: HashMap<String, usize> = HashMap::new();
        let mut max_seq = 0u64;
        for e in entries {
            max_seq = max_seq.max(e.seq);
            let resident = e.resident.clone();
            let q = long.entry(resident.clone()).or_default();
            q.push_back(e);
            // 載入時即守 cap：每位居民只留最新 N 筆（淘汰最舊）——
            // jsonl 本身 append-only、從未刪過任何一行，這裡重播整段淘汰史，
            // 因此「淡忘計數」在重啟後能被精確重建，不必另開持久化格式。
            while q.len() > MAX_MEMORIES_PER_RESIDENT {
                if q.pop_front().is_some() {
                    *faded_counts.entry(resident.clone()).or_insert(0) += 1;
                }
            }
        }
        Self {
            convos: HashMap::new(),
            long,
            faded_counts,
            next_seq: max_seq.wrapping_add(1),
        }
    }

    /// 記一輪對話到短期歷史（玩家一句 + 居民一句），超過 [`HISTORY_TURNS`] 淘汰最舊。
    pub fn record_turn(&mut self, player: &str, resident: &str, user: &str, reply: &str) {
        let q = self
            .convos
            .entry((player.to_string(), resident.to_string()))
            .or_default();
        q.push_back(DialogueTurn {
            user: user.to_string(),
            reply: reply.to_string(),
        });
        while q.len() > HISTORY_TURNS {
            q.pop_front();
        }
    }

    /// 取某對 `(玩家, 居民)` 的近期對話歷史（舊→新）。沒有則空。
    pub fn recent_dialogue(&self, player: &str, resident: &str) -> Vec<DialogueTurn> {
        self.convos
            .get(&(player.to_string(), resident.to_string()))
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// 新增一筆長期記憶並回傳它（呼叫端據此 append 落地）。
    /// 分配 `next_seq`、推進序號；該居民記憶超 cap 即淘汰最舊。
    pub fn add_memory(&mut self, resident: &str, player: &str, summary: &str) -> MemoryEntry {
        let entry = MemoryEntry {
            resident: resident.to_string(),
            player: player.to_string(),
            summary: summary.to_string(),
            seq: self.next_seq,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        let q = self.long.entry(resident.to_string()).or_default();
        q.push_back(entry.clone());
        while q.len() > MAX_MEMORIES_PER_RESIDENT {
            if q.pop_front().is_some() {
                *self.faded_counts.entry(resident.to_string()).or_insert(0) += 1;
            }
        }
        entry
    }

    /// 回想：撈某居民「關於這個玩家」的最近記憶（最多 [`RECALL_LIMIT`] 筆，最新在前）。
    /// 排序＝「最近」（seq 大者優先）；v1 不做語意相關度，先做扎實可用的。
    pub fn recall(&self, resident: &str, player: &str, limit: usize) -> Vec<MemoryEntry> {
        let Some(q) = self.long.get(resident) else {
            return Vec::new();
        };
        let mut hits: Vec<MemoryEntry> = q.iter().filter(|e| e.player == player).cloned().collect();
        // seq 大（新）在前。
        hits.sort_by(|a, b| b.seq.cmp(&a.seq));
        hits.truncate(limit);
        hits
    }

    /// 日記用：取某居民**所有**長期記憶，最新在前（seq 大→小）。
    /// 不過濾玩家、不限筆數——日記要讓人類看到居民完整的記憶足跡。
    pub fn all_memories_for(&self, resident: &str) -> Vec<MemoryEntry> {
        let Some(q) = self.long.get(resident) else {
            return Vec::new();
        };
        let mut entries: Vec<MemoryEntry> = q.iter().cloned().collect();
        entries.sort_by(|a, b| b.seq.cmp(&a.seq));
        entries
    }

    /// 好感度：某位玩家與某位居民之間積累的長期記憶筆數（越多 = 越熟識）。
    /// 純計數、確定性、無副作用——前端可以用這個數字決定顯示哪種指示燈。
    /// - 0     → 陌生人（未曾留下記憶）
    /// - 1–2  → 相識（聊過一兩次）
    /// - 3+   → 友人（多次深入互動）
    pub fn affinity_count(&self, player: &str, resident: &str) -> usize {
        self.long
            .get(resident)
            .map(|q| q.iter().filter(|e| e.player == player).count())
            .unwrap_or(0)
    }

    /// 某居民的長期記憶總筆數（所有玩家合計，純計數不複製資料）。
    /// 用於心情計算——越多記憶代表與玩家互動越頻繁。
    pub fn memory_count(&self, resident: &str) -> usize {
        self.long.get(resident).map(|q| q.len()).unwrap_or(0)
    }

    /// 該居民累計被 cap 淘汰出長期記憶的舊記憶筆數（記憶 v2「整併/壓縮/封存」
    /// 最小可行版的安全信號）。0 = 記憶從未滿載過；純讀取，不改狀態。
    pub fn faded_count(&self, resident: &str) -> usize {
        self.faded_counts.get(resident).copied().unwrap_or(0)
    }
}

/// 記憶回想泡泡的好感度門檻：友人（3+ 筆記憶）才觸發。
pub const RECALL_AFFINITY_THRESHOLD: usize = 3;
/// 回想泡泡擷取的原句最大字元數（不含前綴「我記得你說過」）。
const RECALL_SNIPPET_MAX: usize = 18;

/// 把一筆長期記憶摘要轉成居民「回想泡泡」文字——居民主動說出你當初說過的話。
///
/// 摘要格式為「和X聊過，對方提到「…」」；這裡自動抽出「…」部分，
/// 若無「」結構則取開頭一截，讓泡泡永遠帶有玩家原話的味道。
/// 純函式、確定性、可測；不走 LLM、不持鎖。
pub fn recall_bubble(memory_summary: &str) -> String {
    // 從「找到起始引號，往後截取（含引號讓玩家一眼認出自己說的話）。
    let snippet: String = memory_summary
        .find('\u{300c}') // 「
        .map(|i| &memory_summary[i..])
        .unwrap_or(memory_summary)
        .chars()
        .take(RECALL_SNIPPET_MAX + 2) // +2 為「」各一字
        .collect();
    if snippet.is_empty() {
        "我還記得你……".to_string()
    } else {
        format!("我記得你說過{snippet}")
    }
}

/// 由「玩家這次說的話」規則化擷取一句長期記憶摘要（不另呼 LLM，省成本、確定性、可測）。
/// 形如「和{玩家}聊過，對方提到「…」」；截斷玩家原句到 [`SUMMARY_MAX_CHARS`] 內。
/// 之後若要升級成 LLM 摘要，替換此函式即可、上下游不必動。
pub fn summarize_exchange(player: &str, user_text: &str) -> Option<String> {
    let snippet = user_text.trim();
    if snippet.is_empty() {
        return None;
    }
    // 留給固定模板字的餘裕，剩下給玩家原句。
    let body: String = snippet.chars().take(SUMMARY_MAX_CHARS).collect();
    Some(format!("和{player}聊過，對方提到「{body}」"))
}

/// 把回想到的記憶 + 近期對話組成餵進 system prompt 的「脈絡區塊」（純字串組裝、可測）。
/// 兩段都空 → 回空字串（呼叫端據此決定要不要附加）。面向 LLM 的脈絡，集中於此便於日後調整。
pub fn build_context_block(
    history: &[DialogueTurn],
    memories: &[MemoryEntry],
    player_name: &str,
) -> String {
    let mut out = String::new();
    if !memories.is_empty() {
        out.push_str(&format!("【你對「{player_name}」的記憶（越上面越近期）】\n"));
        for m in memories {
            out.push_str(&format!("- {}\n", m.summary));
        }
    }
    if !history.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("【你和對方剛剛的對話（依先後）】\n");
        for t in history {
            out.push_str(&format!("{player_name}：{}\n", t.user));
            out.push_str(&format!("你：{}\n", t.reply));
        }
    }
    cap_context_chars(out.trim_end().to_string())
}

/// 把脈絡區塊截到 [`MAX_CONTEXT_CHARS`] 以內（純函式、可測）。
/// 超長時保留**尾端**（dialogue 在底部、記憶在頂部；近期對話比舊記憶重要），
/// 並切到下一個換行起點避免切碎一行，前面加一句省略標記。
fn cap_context_chars(block: String) -> String {
    if block.chars().count() <= MAX_CONTEXT_CHARS {
        return block;
    }
    let chars: Vec<char> = block.chars().collect();
    let tail: String = chars[chars.len() - MAX_CONTEXT_CHARS..].iter().collect();
    // 從第一個換行之後起，避免開頭是半行殘字。
    let kept = match tail.find('\n') {
        Some(idx) => tail[idx + 1..].trim().to_string(),
        None => tail.trim().to_string(),
    };
    format!("（脈絡較長，僅保留最近的部分）\n{kept}")
}

// ── jsonl 持久化（比照 npc_agent::append_prayer：輕量同步小檔寫，失敗只記 log 不 panic）──────

/// 居民記憶落地檔（執行期產生、已隨 `data/` gitignore）。
const VOXEL_MEMORY_PATH: &str = "data/voxel_memory.jsonl";

/// 把一筆記憶 append 到 `data/voxel_memory.jsonl`。
///
/// 鐵律：只在**不持任何鎖的（async task 內亦可，這支是同步小檔寫）**情境呼叫。append-only、
/// 絕不覆寫/刪除既有行 → 向後相容、不破壞既有玩家記憶。寫失敗只記 log、不 panic。
pub fn append_memory(entry: &MemoryEntry) {
    // 濾控制字元（對齊 prayers/suggestions：維護者常直接在終端機讀 jsonl，ESC/NUL 可注入轉義）。
    let safe = MemoryEntry {
        resident: sanitize_field(&entry.resident),
        player: sanitize_field(&entry.player),
        summary: sanitize_field(&entry.summary),
        seq: entry.seq,
    };
    if safe.summary.is_empty() {
        return; // 空摘要不落地
    }
    if let Ok(record) = serde_json::to_value(&safe) {
        write_memory_line(VOXEL_MEMORY_PATH, &record);
    }
}

/// 從 `data/voxel_memory.jsonl` 載回所有記憶（啟動時呼叫一次）。檔不存在/壞行皆容忍。
pub fn load_memories() -> Vec<MemoryEntry> {
    read_memory_lines(VOXEL_MEMORY_PATH)
}

/// 濾掉控制字元並 trim（換行也濾掉，記憶是單句）。
fn sanitize_field(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

/// 實際把一行 JSON append 進檔（抽出便於測試指定路徑）。寫失敗只記 log、不 panic。
fn write_memory_line(path: &str, record: &serde_json::Value) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut file) => {
            if let Ok(line) = serde_json::to_string(record) {
                let _ = writeln!(file, "{line}");
            }
        }
        Err(e) => tracing::warn!("無法寫入居民記憶檔 {path}: {e}"),
    }
}

/// 讀回所有記憶行（抽出便於測試指定路徑）。壞行略過、檔缺回空。
fn read_memory_lines(path: &str) -> Vec<MemoryEntry> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(), // 檔不存在（首次啟動）= 沒有記憶，正常
    };
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                None
            } else {
                serde_json::from_str::<MemoryEntry>(line).ok()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_turn_caps_at_history_turns() {
        let mut m = VoxelMemory::new();
        for i in 0..(HISTORY_TURNS + 5) {
            m.record_turn("旅人", "vox_res_0", &format!("問{i}"), &format!("答{i}"));
        }
        let hist = m.recent_dialogue("旅人", "vox_res_0");
        assert_eq!(hist.len(), HISTORY_TURNS, "短期歷史應 cap 在 HISTORY_TURNS");
        // 留下的是最新幾輪（最舊被淘汰）。
        assert_eq!(hist.last().unwrap().user, format!("問{}", HISTORY_TURNS + 4));
        assert_eq!(hist.first().unwrap().user, format!("問{}", 5));
    }

    #[test]
    fn dialogue_is_keyed_per_player_and_resident() {
        let mut m = VoxelMemory::new();
        m.record_turn("阿星", "vox_res_0", "嗨", "你好");
        m.record_turn("小美", "vox_res_0", "哈囉", "歡迎");
        // 不同玩家 → 各自獨立歷史，不串味。
        assert_eq!(m.recent_dialogue("阿星", "vox_res_0").len(), 1);
        assert_eq!(m.recent_dialogue("小美", "vox_res_0").len(), 1);
        // 不同居民 → 空。
        assert!(m.recent_dialogue("阿星", "vox_res_1").is_empty());
    }

    #[test]
    fn add_memory_assigns_increasing_seq_and_caps() {
        let mut m = VoxelMemory::new();
        let mut last = 0;
        for i in 0..(MAX_MEMORIES_PER_RESIDENT + 10) {
            let e = m.add_memory("vox_res_0", "旅人", &format!("記憶{i}"));
            assert!(e.seq >= last, "seq 應單調遞增");
            last = e.seq;
        }
        // 記憶體側 cap：超過上限的最舊被淘汰。
        let all = m.recall("vox_res_0", "旅人", 9999);
        assert_eq!(all.len(), MAX_MEMORIES_PER_RESIDENT);
    }

    // ── 淡忘計數（記憶 v2 最小可行版）───────────────────────────────────────

    #[test]
    fn faded_count_zero_when_never_evicted() {
        let mut m = VoxelMemory::new();
        m.add_memory("vox_res_0", "旅人", "記憶0");
        assert_eq!(m.faded_count("vox_res_0"), 0);
        assert_eq!(m.faded_count("vox_res_不存在"), 0);
    }

    #[test]
    fn add_memory_eviction_increments_faded_count() {
        let mut m = VoxelMemory::new();
        for i in 0..(MAX_MEMORIES_PER_RESIDENT + 3) {
            m.add_memory("vox_res_0", "旅人", &format!("記憶{i}"));
        }
        // 多推 3 筆超過 cap → 3 筆被淘汰。
        assert_eq!(m.faded_count("vox_res_0"), 3);
        // 未超載的居民不受影響。
        assert_eq!(m.faded_count("vox_res_1"), 0);
    }

    #[test]
    fn from_entries_replay_matches_runtime_faded_count() {
        // 一次性重播（重啟載回）算出的淡忘計數，應與逐筆即時淘汰（執行期）完全一致——
        // 計數不是另存的持久化格式，而是從全量 jsonl 歷史精確重建。
        let mut runtime = VoxelMemory::new();
        let mut entries = Vec::new();
        for i in 0..(MAX_MEMORIES_PER_RESIDENT + 8) {
            let e = runtime.add_memory("vox_res_0", "旅人", &format!("往事{i}"));
            entries.push(e);
        }
        let replayed = VoxelMemory::from_entries(entries);
        assert_eq!(runtime.faded_count("vox_res_0"), replayed.faded_count("vox_res_0"));
        assert_eq!(runtime.faded_count("vox_res_0"), 8);
    }

    #[test]
    fn recall_filters_by_player_and_orders_recent_first() {
        let mut m = VoxelMemory::new();
        m.add_memory("vox_res_0", "阿星", "阿星想蓋橋");
        m.add_memory("vox_res_0", "小美", "小美愛釣魚");
        m.add_memory("vox_res_0", "阿星", "阿星找到礦");
        let hits = m.recall("vox_res_0", "阿星", RECALL_LIMIT);
        assert_eq!(hits.len(), 2, "只撈關於阿星的");
        // 最新（阿星找到礦）在前。
        assert_eq!(hits[0].summary, "阿星找到礦");
        assert_eq!(hits[1].summary, "阿星想蓋橋");
        // 小美的不會混進來。
        assert!(hits.iter().all(|e| e.player == "阿星"));
    }

    #[test]
    fn recall_limit_is_respected() {
        let mut m = VoxelMemory::new();
        for i in 0..10 {
            m.add_memory("vox_res_0", "旅人", &format!("事件{i}"));
        }
        assert_eq!(m.recall("vox_res_0", "旅人", 3).len(), 3);
    }

    #[test]
    fn summarize_extracts_snippet_and_truncates() {
        let s = summarize_exchange("阿星", "  我想在河上蓋一座橋  ").unwrap();
        assert!(s.contains("阿星"));
        assert!(s.contains("蓋一座橋"));
        // 空輸入 → None。
        assert!(summarize_exchange("阿星", "   ").is_none());
        // 超長玩家句截斷（按字元，多位元組中文）。
        let long = "字".repeat(SUMMARY_MAX_CHARS + 50);
        let out = summarize_exchange("旅人", &long).unwrap();
        // 摘要含模板字，但內嵌的玩家原句不超過上限。
        assert!(out.matches('字').count() <= SUMMARY_MAX_CHARS);
    }

    #[test]
    fn context_block_renders_memories_and_history() {
        let history = vec![DialogueTurn {
            user: "你還記得我嗎".to_string(),
            reply: "當然記得".to_string(),
        }];
        let memories = vec![MemoryEntry {
            resident: "vox_res_0".to_string(),
            player: "阿星".to_string(),
            summary: "阿星想蓋橋".to_string(),
            seq: 1,
        }];
        let block = build_context_block(&history, &memories, "阿星");
        assert!(block.contains("阿星想蓋橋"), "應含長期記憶");
        assert!(block.contains("你還記得我嗎"), "應含近期對話");
        assert!(block.contains("阿星"));
        // 兩段皆空 → 空字串。
        assert!(build_context_block(&[], &[], "阿星").is_empty());
    }

    #[test]
    fn context_block_capped_keeps_recent_dialogue() {
        // 灌一大堆記憶 + 多輪對話 → 應截到上限內，且保留「最近」的對話（在尾端）。
        let memories: Vec<MemoryEntry> = (0..40)
            .map(|i| MemoryEntry {
                resident: "vox_res_0".to_string(),
                player: "阿星".to_string(),
                summary: format!("很久以前的舊記憶第 {i} 條，填充填充填充填充填充"),
                seq: i as u64,
            })
            .collect();
        let history = vec![DialogueTurn {
            user: "我剛剛說的最新一句話".to_string(),
            reply: "這是居民最新的回覆".to_string(),
        }];
        let block = build_context_block(&history, &memories, "阿星");
        // 截到上限內（加上省略標記那一行的少量字數，仍應遠小於未截前）。
        assert!(block.chars().count() <= MAX_CONTEXT_CHARS + 40, "脈絡應截到上限內：{}", block.chars().count());
        // 最近的對話一定要留著（近期對話比舊記憶重要）。
        assert!(block.contains("我剛剛說的最新一句話"), "截斷後仍須保留最近對話");
        assert!(block.contains("脈絡較長"), "超長時應加省略標記");
    }

    #[test]
    fn from_entries_rebuilds_caps_and_continues_seq() {
        // 模擬從磁碟載回（亂序 + 超過 cap）。
        let mut entries = Vec::new();
        for i in 0..(MAX_MEMORIES_PER_RESIDENT + 5) {
            entries.push(MemoryEntry {
                resident: "vox_res_0".to_string(),
                player: "旅人".to_string(),
                summary: format!("事件{i}"),
                seq: i as u64,
            });
        }
        // 打亂順序，驗證內部會排序。
        entries.reverse();
        let mut m = VoxelMemory::from_entries(entries);
        // 載入即 cap。
        assert_eq!(m.recall("vox_res_0", "旅人", 9999).len(), MAX_MEMORIES_PER_RESIDENT);
        // 新記憶 seq 續接（> 既有最大）。
        let e = m.add_memory("vox_res_0", "旅人", "重啟後的新記憶");
        assert!(e.seq > (MAX_MEMORIES_PER_RESIDENT + 4) as u64, "seq 應續接既有最大值之後");
    }

    #[test]
    fn jsonl_roundtrip_persists_and_loads() {
        // 寫到臨時檔再讀回，驗證持久化格式可往返（模擬重啟）。
        let dir = std::env::temp_dir().join(format!("voxmem_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("voxel_memory.jsonl");
        let pstr = path.to_str().unwrap();
        let _ = std::fs::remove_file(&path);

        let e1 = MemoryEntry {
            resident: "vox_res_0".to_string(),
            player: "阿星".to_string(),
            summary: "阿星想蓋橋".to_string(),
            seq: 0,
        };
        let e2 = MemoryEntry {
            resident: "vox_res_1".to_string(),
            player: "小美".to_string(),
            summary: "小美愛釣魚".to_string(),
            seq: 1,
        };
        write_memory_line(pstr, &serde_json::to_value(&e1).unwrap());
        write_memory_line(pstr, &serde_json::to_value(&e2).unwrap());

        let loaded = read_memory_lines(pstr);
        assert_eq!(loaded.len(), 2);
        assert!(loaded.contains(&e1));
        assert!(loaded.contains(&e2));

        // 壞行容忍：append 一行垃圾，仍只讀回合法兩筆。
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(pstr).unwrap();
            writeln!(f, "這不是 json").unwrap();
        }
        assert_eq!(read_memory_lines(pstr).len(), 2, "壞行應略過");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_missing_file_is_empty() {
        let missing = "data/__definitely_not_here_voxmem__.jsonl";
        assert!(read_memory_lines(missing).is_empty());
    }

    // ── affinity_count 測試 ───────────────────────────────────────────────────

    #[test]
    fn affinity_zero_for_stranger() {
        let store = VoxelMemory::new();
        assert_eq!(store.affinity_count("小明", "vox_res_0"), 0);
    }

    #[test]
    fn affinity_counts_only_matching_player_and_resident() {
        let mut store = VoxelMemory::new();
        store.add_memory("vox_res_0", "小明", "小明說想蓋房子");
        store.add_memory("vox_res_0", "小明", "小明聊到星星");
        // 同居民但不同玩家
        store.add_memory("vox_res_0", "小美", "小美說愛農耕");
        // 不同居民但同玩家
        store.add_memory("vox_res_1", "小明", "小明在居民1處說話");

        assert_eq!(store.affinity_count("小明", "vox_res_0"), 2, "小明與 res_0 有兩筆");
        assert_eq!(store.affinity_count("小美", "vox_res_0"), 1, "小美與 res_0 有一筆");
        assert_eq!(store.affinity_count("小明", "vox_res_1"), 1, "小明與 res_1 有一筆");
        assert_eq!(store.affinity_count("小美", "vox_res_1"), 0, "小美與 res_1 無互動");
    }

    #[test]
    fn affinity_three_or_more_is_friend_tier() {
        let mut store = VoxelMemory::new();
        store.add_memory("vox_res_0", "小明", "a");
        store.add_memory("vox_res_0", "小明", "b");
        store.add_memory("vox_res_0", "小明", "c");
        assert!(store.affinity_count("小明", "vox_res_0") >= 3, "三筆以上應達友人等級");
    }

    // ── recall_bubble 測試 ───────────────────────────────────────────────────

    #[test]
    fn recall_bubble_standard_format() {
        // 典型摘要：含「…」格式 → 抽出引號部分。
        let summary = "和阿星聊過，對方提到「想在這裡蓋觀星塔」";
        let bubble = recall_bubble(summary);
        assert!(bubble.contains("「想在這裡蓋觀星塔"), "應含玩家原話");
        assert!(bubble.starts_with("我記得你說過"), "應有前綴");
    }

    #[test]
    fn recall_bubble_no_bracket_falls_back_to_opening() {
        // 沒有「…」結構 → 取開頭截斷。
        let summary = "某個特殊格式沒有引號的摘要文字";
        let bubble = recall_bubble(summary);
        assert!(!bubble.is_empty(), "不能回空字串");
    }

    #[test]
    fn recall_bubble_empty_summary_returns_fallback() {
        let bubble = recall_bubble("");
        assert_eq!(bubble, "我還記得你……", "空摘要應回備用語");
    }

    #[test]
    fn recall_bubble_snippet_bounded() {
        // 很長的摘要，抽出後不應超出泡泡合理長度。
        let long_inner = "「".to_string() + &"X".repeat(50) + "」";
        let bubble = recall_bubble(&long_inner);
        // RECALL_SNIPPET_MAX+2 = 20 字，前綴「我記得你說過」7 字 → 全文 ≤ 27
        assert!(bubble.chars().count() <= 28, "回想泡泡不應過長");
    }

    #[test]
    fn recall_bubble_non_empty_for_typical_memory() {
        // 確認典型 summarize_exchange 輸出能正常轉換成泡泡。
        let summary = summarize_exchange("阿信", "你這裡的石頭好漂亮").unwrap();
        let bubble = recall_bubble(&summary);
        assert!(!bubble.is_empty(), "典型記憶應能產生非空泡泡");
        assert!(bubble.contains("你這裡的石頭好漂亮")
            || bubble.contains("你這裡"), "應含玩家原話或部份");
    }

    #[test]
    fn recall_affinity_threshold_is_sane() {
        assert!(RECALL_AFFINITY_THRESHOLD >= 2, "門檻不能太低（陌生人不該觸發）");
        assert!(RECALL_AFFINITY_THRESHOLD <= 5, "門檻不能太高（永遠觸發不了）");
    }
}
