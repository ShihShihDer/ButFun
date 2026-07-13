//! 乙太方界 AI 居民記憶系統 v2——兩層記憶（短期原始 + 長期精華提煉）。
//!
//! v1（扁平 FIFO 40 筆）升級：重要的事（玩家名字、計劃、偏好、承諾）提煉進**長期精華層**，
//! 不被短期原始流量淘汰；寒暄瑣事只進短期原始層、被 cap 擠掉也無妨。
//!
//! ## 兩層架構
//! **A. 短期原始層（episodic）**：最近 [`EPISODIC_CAP`] 筆，FIFO cap，提供「近況上下文」。
//! **B. 長期精華層（semantic）**：每對 (居民, 玩家) 最多 [`SEMANTIC_CAP`] 條高價值事實，
//!    不受 A 層淘汰，身份/目標/偏好/承諾在此永久留存。
//!
//! ## 重要性判定
//! 純函式 [`classify_importance`]：寒暄/瑣事 → 只進 A 層；名字宣告/目標/偏好/承諾 → 提煉進 B 層。
//! 零 LLM、確定性、可測；LLM 提煉路徑留擴充點，不燒 token。
//!
//! 本模組**只放與連線/鎖/LLM 無關的確定性純邏輯**；真正的 tick 驅動、廣播、無鎖 async 思考、
//! 鎖的取放都在 `voxel_ws.rs`，嚴守 prod 死鎖鐵律：
//! 短鎖快照 → drop → spawn → 下一步套用，**記憶讀寫絕不在持鎖中 await**。
//!
//! 全部抽成可測純函式；不抄外部碼、繁中註解；機敏值不涉入；**append-only、絕不刪既有玩家資料**。

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

/// 短期對話歷史每對 `(玩家, 居民)` 保留的輪數（一輪 = 玩家一句 + 居民一句）。
pub const HISTORY_TURNS: usize = 6;
/// 短期原始層（episodic）每位居民的 cap：超過即淘汰最舊。v2 縮小為 24（v1 為 40）。
pub const EPISODIC_CAP: usize = 24;
/// 向後相容別名（外部若有引用）。
pub const MAX_MEMORIES_PER_RESIDENT: usize = EPISODIC_CAP;
/// 長期精華層（semantic）每對 (居民, 玩家) 的最大事實條數。
pub const SEMANTIC_CAP: usize = 12;
/// 回想 episodic 時最多撈幾筆（近期上下文，別過度工程）。
pub const RECALL_LIMIT: usize = 4;
/// 「這句話讓你想起了什麼」——每輪對話最多帶回幾筆被勾起的舊記憶（RECALL_LIMIT 窗外的）。
/// 別過度工程：不是找回所有相關記憶，只是最像的一兩筆，避免洗版 prompt、蓋過真正的近期脈絡。
pub const RELEVANT_RECALL_LIMIT: usize = 2;
/// 「被勾起」的字元 bigram Jaccard 相似度門檻——低於這個分數視為雜訊、不觸發
/// （寧可少想起、也別答非所問地硬掰「這讓我想起…」）。
pub const RELEVANCE_MIN_SCORE: f32 = 0.15;
/// 一筆 episodic 摘要的字元上限：規則擷取後截斷。
pub const SUMMARY_MAX_CHARS: usize = 80;
/// 淡忘印象最多保留幾個主題標籤（記憶 v2「整併/壓縮」最小可行版）。
/// 別過度工程：只留最近淘汰前出現的幾個主題，夠日記造一句「常聊到…」就好。
pub const IMPRESSION_TOPIC_CAP: usize = 3;
/// 餵進對話 system prompt 的「脈絡區塊（episodic + 對話）」總字元上限。
/// semantic 精華層不受此 cap 限制，總是帶上。
pub const MAX_CONTEXT_CHARS: usize = 700;

// ── 資料型別 ─────────────────────────────────────────────────────────────────

/// 一輪對話（短期記憶用）：玩家說的 + 居民回的。
#[derive(Clone, Debug, PartialEq)]
pub struct DialogueTurn {
    pub user: String,
    pub reply: String,
}

/// 一筆短期原始記憶（持久化單位，episodic 層）。刻意不寫系統時鐘：
/// 用單調遞增的 `seq` 當排序鍵——回想只需「最近」順序。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// 居民 id（如 "vox_res_0"）。
    pub resident: String,
    /// 玩家身份鍵。
    pub player: String,
    /// 這次互動摘要（規則生成，不含 LLM）。
    pub summary: String,
    /// 單調遞增序號：越大越新。
    pub seq: u64,
}

/// 長期精華事實的類別（規則判定，零 LLM）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FactCategory {
    /// 身份：名字/自稱（「我叫X」「可以叫我X」）。
    Identity,
    /// 目標/計劃（「我要建設X」「想蓋橋」）。
    Goal,
    /// 偏好/喜好（「最喜歡看星星」）。
    Preference,
    /// 承諾/交代（「答應你」「記住了」）。
    Promise,
}

impl FactCategory {
    /// 脈絡區塊顯示標籤（i18n 預留點：集中在此一處替換）。
    fn label(&self) -> &'static str {
        match self {
            Self::Identity   => "身份",
            Self::Goal       => "目標",
            Self::Preference => "偏好",
            Self::Promise    => "承諾",
        }
    }
}

/// 一條長期精華事實（不被 episodic cap 淘汰）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticFact {
    pub category: FactCategory,
    /// 提煉後的事實內容（精簡、可直接餵進 prompt）。
    pub content: String,
}

/// 重要性判定結果——[`classify_importance`] 的回傳值。
#[derive(Clone, Debug, PartialEq)]
pub enum Importance {
    /// 寒暄/瑣事：只進 episodic 層、可被淘汰。
    Ephemeral,
    /// 高價值事實：同時提煉進 semantic 精華層。
    Persistent(SemanticFact),
}

// ── 記憶 Store ───────────────────────────────────────────────────────────────

/// 乙太方界記憶 store（v2 兩層）：短期對話歷史 + episodic + semantic + 全域序號。
/// 純資料結構，所有方法皆同步、不 await；由 `voxel_ws.rs` 包進 `RwLock` 使用。
#[derive(Default)]
pub struct VoxelMemory {
    /// 短期對話歷史：key = (玩家, 居民) → 最近數輪（front 舊、back 新）。
    convos: HashMap<(String, String), VecDeque<DialogueTurn>>,
    /// 短期原始層（episodic）：key = 居民 id → 累積記憶（front 舊、back 新，capped）。
    long: HashMap<String, VecDeque<MemoryEntry>>,
    /// 長期精華層（semantic）：key = (居民 id, 玩家) → 高價值事實（cap = SEMANTIC_CAP）。
    /// 不持久化——重啟時從 episodic jsonl 重建（零新格式、向後相容）。
    semantic: HashMap<(String, String), Vec<SemanticFact>>,
    /// 淡忘計數：key = 居民 id → 被 episodic cap 淘汰的總筆數。
    faded_counts: HashMap<String, usize>,
    /// 淡忘印象：key = 居民 id → 淘汰前留下的主題標籤（記憶 v2「整併/壓縮」最小可行版，
    /// 見 [`impression_topic`]）。front 舊、back 新，capped 在 [`IMPRESSION_TOPIC_CAP`]。
    /// 固定標籤集合、絕不含玩家原話——守 `voxel_diary` 「輸出永不含玩家原話」的隱私鐵律。
    impression_topics: HashMap<String, VecDeque<&'static str>>,
    /// 全域單調序號（下一筆記憶用）。
    next_seq: u64,
}

impl VoxelMemory {
    /// 空 store（無 DB / 測試）。
    pub fn new() -> Self { Self::default() }

    /// 從磁碟載回記憶並重建兩層（重啟時呼叫一次）。
    ///
    /// **向後相容**：舊格式（只有 summary 無精華層）→ episodic 照收、對每筆跑重要性判定重建 semantic。
    /// **遷移清污染**：`is_test_identity(player) == true` 的紀錄全數丟棄（維護者確認為 QA 測試污染）。
    /// 真玩家（濕濕的、旅人等）和有實質內容的記錄完整保留。
    pub fn from_entries(mut entries: Vec<MemoryEntry>) -> Self {
        // ── 遷移：過濾測試身份污染 ──────────────────────────────────────────
        entries.retain(|e| !is_test_identity(&e.player));

        entries.sort_by_key(|e| e.seq);
        let mut long: HashMap<String, VecDeque<MemoryEntry>> = HashMap::new();
        let mut semantic: HashMap<(String, String), Vec<SemanticFact>> = HashMap::new();
        let mut faded_counts: HashMap<String, usize> = HashMap::new();
        let mut impression_topics: HashMap<String, VecDeque<&'static str>> = HashMap::new();
        let mut max_seq = 0u64;

        for e in entries {
            max_seq = max_seq.max(e.seq);
            let resident = e.resident.clone();
            let player = e.player.clone();

            // 重建精華層：對每筆舊 episodic 記憶跑重要性判定（零 LLM）。
            if let Importance::Persistent(fact) = classify_importance(&e.summary) {
                let store = semantic.entry((resident.clone(), player.clone())).or_default();
                merge_into_semantic(store, fact);
            }

            let q = long.entry(resident.clone()).or_default();
            q.push_back(e);
            // 載入即守 episodic cap，精確重建淡忘計數 + 淡忘印象（與 add_memory 同一套邏輯，
            // 確保重啟重建跟線上運行產生一致結果）。
            while q.len() > EPISODIC_CAP {
                if let Some(evicted) = q.pop_front() {
                    record_fade(&mut faded_counts, &mut impression_topics, &resident, &evicted.summary);
                }
            }
        }

        Self {
            convos: HashMap::new(),
            long,
            semantic,
            faded_counts,
            impression_topics,
            next_seq: max_seq.wrapping_add(1),
        }
    }

    /// 記一輪對話到短期歷史，超過 [`HISTORY_TURNS`] 淘汰最舊。
    pub fn record_turn(&mut self, player: &str, resident: &str, user: &str, reply: &str) {
        let q = self
            .convos
            .entry((player.to_string(), resident.to_string()))
            .or_default();
        q.push_back(DialogueTurn { user: user.to_string(), reply: reply.to_string() });
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

    /// 新增一筆記憶：
    /// - 進 episodic 層（超 cap 淘汰最舊），回傳供呼叫端 append 落地。
    /// - 重要的同時提煉進 semantic 精華層（不落地，重啟時從 episodic 重建）。
    pub fn add_memory(&mut self, resident: &str, player: &str, summary: &str) -> MemoryEntry {
        let entry = MemoryEntry {
            resident: resident.to_string(),
            player: player.to_string(),
            summary: summary.to_string(),
            seq: self.next_seq,
        };
        self.next_seq = self.next_seq.wrapping_add(1);

        // ── A 層：episodic ───────────────────────────────────────────────────
        let q = self.long.entry(resident.to_string()).or_default();
        q.push_back(entry.clone());
        while q.len() > EPISODIC_CAP {
            if let Some(evicted) = q.pop_front() {
                record_fade(&mut self.faded_counts, &mut self.impression_topics, resident, &evicted.summary);
            }
        }

        // ── B 層：semantic（重要性判定 → 合併/更新精華事實）─────────────────
        if let Importance::Persistent(fact) = classify_importance(summary) {
            let store = self
                .semantic
                .entry((resident.to_string(), player.to_string()))
                .or_default();
            merge_into_semantic(store, fact);
        }

        entry
    }

    /// 回想 episodic：撈某居民「關於這個玩家」的最近記憶（最多 limit 筆，最新在前）。
    /// 供近期上下文使用；重要的持久事實請用 [`semantic_facts_for`]。
    pub fn recall(&self, resident: &str, player: &str, limit: usize) -> Vec<MemoryEntry> {
        let Some(q) = self.long.get(resident) else { return Vec::new(); };
        let mut hits: Vec<MemoryEntry> = q.iter().filter(|e| e.player == player).cloned().collect();
        hits.sort_by(|a, b| b.seq.cmp(&a.seq));
        hits.truncate(limit);
        hits
    }

    /// 回想第三種管道——「相關」：[`recall`] 只看**近期**（最近 RECALL_LIMIT 筆），
    /// [`semantic_facts_for`] 只看**規則判定為重要**的事實；但玩家隨口提起的舊話題，若既不
    /// 是最近幾句、也沒被 `classify_importance` 判成身份/目標/偏好/承諾，就會永遠沉在
    /// episodic 佇列深處、再也不會浮上對話——即使那件事跟這句話明明很像。
    ///
    /// 本函式用字元 bigram Jaccard 相似度（中文無空白斷詞的粗略近似，零外部依賴/零向量服務）
    /// 從**不在 `exclude_seqs`（通常＝已經被 `recall` 撈出的近期窗）內**的舊記憶裡，挑出跟
    /// `query`（玩家這句話）夠像的幾筆。分數過低（< [`RELEVANCE_MIN_SCORE`]）一律不算，
    /// 寧可少想起也別答非所問。
    pub fn relevant_memories(
        &self,
        resident: &str,
        player: &str,
        query: &str,
        exclude_seqs: &[u64],
        limit: usize,
    ) -> Vec<MemoryEntry> {
        let query_grams = char_bigrams(query);
        if query_grams.is_empty() {
            return Vec::new();
        }
        let Some(q) = self.long.get(resident) else { return Vec::new(); };
        let mut scored: Vec<(f32, MemoryEntry)> = q
            .iter()
            .filter(|e| e.player == player && !exclude_seqs.contains(&e.seq))
            .map(|e| {
                // 比對前先剝殼：只比玩家原話，別讓「和X聊過，對方提到」的固定前綴稀釋分數。
                // 非對話類 summary（採集/建造事件等，無引號）沒有引號 → fallback 回整串。
                let body = extract_inner_quote(&e.summary).unwrap_or(&e.summary);
                (bigram_jaccard(&query_grams, &char_bigrams(body)), e.clone())
            })
            .filter(|(score, _)| *score >= RELEVANCE_MIN_SCORE)
            .collect();
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.1.seq.cmp(&a.1.seq))
        });
        scored.truncate(limit);
        scored.into_iter().map(|(_, e)| e).collect()
    }

    /// 取某居民**關於某位玩家**的所有 episodic 記憶（不設上限、最新在前）。
    /// [`recall`] 會截斷成最近 N 筆供對話上下文；本函式不截斷，供「昇華聚合印象」（如
    /// `voxel_playerepithet` 把一位玩家的全部作為統計成主導角色）用。純讀、確定性。
    pub fn all_player_memories(&self, resident: &str, player: &str) -> Vec<MemoryEntry> {
        let Some(q) = self.long.get(resident) else { return Vec::new(); };
        let mut hits: Vec<MemoryEntry> = q.iter().filter(|e| e.player == player).cloned().collect();
        hits.sort_by(|a, b| b.seq.cmp(&a.seq));
        hits
    }

    /// 取某對 `(居民, 玩家)` 的長期精華事實（B 層）——供對話 prompt 使用，不受 episodic cap 淘汰。
    pub fn semantic_facts_for(&self, resident: &str, player: &str) -> Vec<SemanticFact> {
        self.semantic
            .get(&(resident.to_string(), player.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// 日記用：取某居民**所有** episodic 記憶，最新在前（不過濾玩家）。
    pub fn all_memories_for(&self, resident: &str) -> Vec<MemoryEntry> {
        let Some(q) = self.long.get(resident) else { return Vec::new(); };
        let mut entries: Vec<MemoryEntry> = q.iter().cloned().collect();
        entries.sort_by(|a, b| b.seq.cmp(&a.seq));
        entries
    }

    /// 好感度：某位玩家與某位居民之間積累的 episodic 記憶筆數（越多 = 越熟識）。
    pub fn affinity_count(&self, player: &str, resident: &str) -> usize {
        self.long
            .get(resident)
            .map(|q| q.iter().filter(|e| e.player == player).count())
            .unwrap_or(0)
    }

    /// 某居民的 episodic 記憶總筆數（所有玩家合計）。
    pub fn memory_count(&self, resident: &str) -> usize {
        self.long.get(resident).map(|q| q.len()).unwrap_or(0)
    }

    /// 某居民累計被 episodic cap 淘汰的筆數（0 = 從未滿載）。
    pub fn faded_count(&self, resident: &str) -> usize {
        self.faded_counts.get(resident).copied().unwrap_or(0)
    }

    /// 某居民淘汰前留下的「淡忘印象」主題標籤（記憶 v2「整併/壓縮」最小可行版，
    /// 見 [`impression_topic`]）。最舊在前、最新在後，最多 [`IMPRESSION_TOPIC_CAP`] 個；
    /// 從未淘汰過或淘汰的內容都認不出主題 → 空。純讀、確定性，固定標籤集合、
    /// 絕不含玩家原話——供 `voxel_diary` 造「淡忘印象」句使用。
    pub fn impression_topics(&self, resident: &str) -> Vec<&'static str> {
        self.impression_topics
            .get(resident)
            .map(|q| q.iter().copied().collect())
            .unwrap_or_default()
    }
}

/// 記一筆 episodic 記憶被 cap 淘汰：累計淡忘計數 + 嘗試從內容留下一個去識別化的主題印象
/// （記憶 v2「整併/壓縮」最小可行版）。供 [`VoxelMemory::add_memory`]（線上運行）與
/// [`VoxelMemory::from_entries`]（重啟重建）的淘汰迴圈共用，確保兩條路徑產生一致結果——
/// 寫成自由函式（非 `&mut self` 方法）是刻意的：呼叫端在 episodic 佇列（`self.long` 的某個
/// entry）仍被可變借用時呼叫，若寫成 `&mut self` 方法會撞上借用檢查（field-level 借用拆分
/// 只在直接存取欄位時成立，經方法呼叫就不成立）。
fn record_fade(
    faded_counts: &mut HashMap<String, usize>,
    impression_topics: &mut HashMap<String, VecDeque<&'static str>>,
    resident: &str,
    evicted_summary: &str,
) {
    *faded_counts.entry(resident.to_string()).or_insert(0) += 1;
    if let Some(topic) = impression_topic(evicted_summary) {
        let bag = impression_topics.entry(resident.to_string()).or_default();
        // 同主題不重複記，即使交錯出現也一樣（「常聊到星空、蓋造」比「星空、蓋造、星空」
        // 更像一句人話；只比 back() 會漏非相鄰重複，見 review PR #1254 退回意見）。
        if !bag.contains(&topic) {
            bag.push_back(topic);
            while bag.len() > IMPRESSION_TOPIC_CAP {
                bag.pop_front();
            }
        }
    }
}

/// 淡忘印象的主題詞庫——供 [`impression_topic`] 用。只列出固定的一組去識別化標籤，
/// 絕不是玩家原話（見 `voxel_diary` 「輸出永不含玩家原話」的隱私鐵律）。
const IMPRESSION_TOPIC_KEYWORDS: &[(&str, &[&str])] = &[
    ("星空", &["星星", "星空", "流星", "夜空"]),
    ("蓋造", &["蓋", "建造", "房子", "小屋", "高塔", "橋"]),
    ("種植", &["種", "花", "田", "作物"]),
    ("挖礦", &["礦", "挖", "洞穴", "石頭"]),
    ("釣魚", &["釣魚", "魚", "河邊", "水邊"]),
    ("情誼", &["朋友", "想念", "老朋友", "熟識", "陪伴"]),
];

/// 從一筆**即將被 episodic cap 淘汰**的記憶摘要，粗略辨識出一個去識別化的「主題標籤」——
/// 淘汰前留下一個主題印記，而非讓整句原話船過水無痕（記憶 v2「整併/壓縮」最小可行版）。
///
/// 只回傳 [`IMPRESSION_TOPIC_KEYWORDS`] 裡固定的標籤字串，**絕不**回傳/拼接玩家原話——
/// 守 `voxel_diary` 「輸出永不含玩家原話」的隱私鐵律；日後若要升級成向量語意分群，
/// 替換本函式即可，呼叫端（[`record_fade`]）與上下游不動。零 LLM、確定性、可測。
fn impression_topic(summary: &str) -> Option<&'static str> {
    let body = extract_inner_quote(summary).unwrap_or(summary);
    IMPRESSION_TOPIC_KEYWORDS
        .iter()
        .find(|(_, kws)| kws.iter().any(|kw| body.contains(kw)))
        .map(|(label, _)| *label)
}

// ── 測試身份過濾（遷移用純函式）─────────────────────────────────────────────

/// 判斷某 player 名是否為已知的 QA 測試污染身份——純函式、確定性、可測。
///
/// 過濾規則（維護者確認）：
/// - `Shih\d+`（Shih0、Shih136、Shih978 等）
/// - `T\d+`（T81、T95、T452 等）
/// - `DbgTest`、`ShihTest`、`小石`（精確匹配）
///
/// **保留**：`濕濕的`（真玩家）、`旅人`（有實質內容）及其他所有非測試身份。
pub fn is_test_identity(player: &str) -> bool {
    let p = player.trim();
    // 精確匹配的固定名單
    if matches!(p, "DbgTest" | "ShihTest" | "小石") {
        return true;
    }
    // Shih 後接全數字（Shih0、Shih136、Shih978 …）
    if let Some(rest) = p.strip_prefix("Shih") {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    // T 後接全數字（T81、T95、T452 …）
    if let Some(rest) = p.strip_prefix('T') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

// ── 相關性回想（純函式、零外部依賴/零向量服務、可測）───────────────────────

/// 字元 bigram 集合——中文沒有空白斷詞，用「相鄰兩字」當最小語意單位的粗略近似
/// （不追求精準語意相似度，只求便宜、確定性、抓得住「講同一件事」的重疊）。
fn char_bigrams(s: &str) -> HashSet<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 2 {
        return chars.iter().map(|c| c.to_string()).collect();
    }
    chars.windows(2).map(|w| w.iter().collect()).collect()
}

/// 兩個 bigram 集合的 Jaccard 相似度（交集大小 / 聯集大小），範圍 0.0~1.0。
fn bigram_jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count();
    let union = a.union(b).count();
    inter as f32 / union as f32
}

// ── 重要性判定 + 精華提煉（純函式、零 LLM、可測）──────────────────────────

/// 從 episodic 摘要格式「和X聊過，對方提到「…」」抽出引號內玩家原話。
fn extract_inner_quote(summary: &str) -> Option<&str> {
    let start = summary.find('\u{300c}')?; // 「
    let start_byte = start + '\u{300c}'.len_utf8(); // 「 是 3 bytes
    let remaining = &summary[start_byte..];
    let end = remaining.find('\u{300d}')?; // 」
    Some(&remaining[..end])
}

/// 從文字中抽出 keyword 之後的短名稱（到第一個標點/空格為止，最多 20 字）。
fn try_extract_name_after<'a>(text: &'a str, kw: &str) -> Option<&'a str> {
    let idx = text.find(kw)?;
    let after = text[idx + kw.len()..].trim_start();
    if after.is_empty() { return None; }
    // 截到下一個標點或空白（最多 20 字）
    let end_bytes: usize = after
        .char_indices()
        .take(20)
        .find(|(_, c)| matches!(c, '，' | '。' | '！' | '？' | '、' | ',' | ' ' | '\n'))
        .map(|(i, _)| i)
        .unwrap_or_else(|| after.chars().take(20).map(char::len_utf8).sum());
    let name = &after[..end_bytes];
    if name.is_empty() { None } else { Some(name) }
}

/// 重要性判定：純規則、零 LLM。回傳 [`Importance`]。
///
/// 判定順序：身份 > 目標 > 偏好 > 承諾 > 瑣事。
/// 抽出引號內玩家原話再判，避免被模板前綴（「和X聊過」）干擾。
pub fn classify_importance(summary: &str) -> Importance {
    let inner = extract_inner_quote(summary).unwrap_or(summary);

    // 1. 身份：名字宣告關鍵詞
    for kw in &["我叫", "我是", "可以叫我", "叫我"] {
        if let Some(name) = try_extract_name_after(inner, kw) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return Importance::Persistent(SemanticFact {
                    category: FactCategory::Identity,
                    content: format!("{kw}{trimmed}"),
                });
            }
        }
    }

    // 2. 目標/計劃
    const GOAL_KWS: &[&str] = &[
        "想要", "計劃", "要建設", "要蓋", "要建", "打算", "準備", "目標",
        "從小村莊", "城市國家", "要做", "從基礎", "要把", "我要",
    ];
    for kw in GOAL_KWS {
        if inner.contains(kw) {
            return Importance::Persistent(SemanticFact {
                category: FactCategory::Goal,
                content: inner.chars().take(60).collect(),
            });
        }
    }

    // 3. 偏好/喜好
    const PREF_KWS: &[&str] = &["最喜歡", "喜歡", "最愛", "偏好", "愛看", "愛吃", "愛玩"];
    for kw in PREF_KWS {
        if inner.contains(kw) {
            return Importance::Persistent(SemanticFact {
                category: FactCategory::Preference,
                content: inner.chars().take(60).collect(),
            });
        }
    }

    // 4. 承諾/交代
    const PROMISE_KWS: &[&str] = &["答應", "承諾", "交代", "記住", "一定會", "不會忘"];
    for kw in PROMISE_KWS {
        if inner.contains(kw) {
            return Importance::Persistent(SemanticFact {
                category: FactCategory::Promise,
                content: inner.chars().take(60).collect(),
            });
        }
    }

    Importance::Ephemeral
}

/// 把一條新精華事實合併進 store（最多 [`SEMANTIC_CAP`] 條）。
///
/// 合併規則（純函式、確定性、可測）：
/// - **Identity**：全局唯一，永遠取代舊的（名字只有一個）。
/// - **其他類別**：同類別且內容前 20 字相符 → 就地更新（避免重複堆疊）；否則新增。
/// - 已達 cap：優先移除同類別最舊，若無同類別則移除整體最舊（index 0）。
pub fn merge_into_semantic(store: &mut Vec<SemanticFact>, new_fact: SemanticFact) {
    // Identity：全局唯一
    if new_fact.category == FactCategory::Identity {
        if let Some(existing) = store.iter_mut().find(|f| f.category == FactCategory::Identity) {
            *existing = new_fact;
            return;
        }
    } else {
        // 其他類別：內容前 20 字相符 → 就地更新
        let prefix: String = new_fact.content.chars().take(20).collect();
        if let Some(existing) = store.iter_mut().find(|f| {
            f.category == new_fact.category
                && f.content.chars().take(20).collect::<String>() == prefix
        }) {
            *existing = new_fact;
            return;
        }
    }

    // 新增（若已達 cap 先騰出位置）
    if store.len() >= SEMANTIC_CAP {
        if let Some(idx) = store.iter().position(|f| f.category == new_fact.category) {
            store.remove(idx);
        } else {
            store.remove(0); // 移除最舊的整體記錄
        }
    }
    store.push(new_fact);
}

// ── 對話 prompt 脈絡組裝（純函式、可測）─────────────────────────────────────

/// 記憶回想泡泡的好感度門檻。
pub const RECALL_AFFINITY_THRESHOLD: usize = 3;
/// 回想泡泡擷取的原句最大字元數（不含前綴）。
const RECALL_SNIPPET_MAX: usize = 18;

/// 把一筆 episodic 摘要轉成居民「回想泡泡」文字——純函式、可測。
pub fn recall_bubble(memory_summary: &str) -> String {
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

/// 由「玩家這次說的話」規則化擷取 episodic 摘要（零 LLM，省成本、確定性、可測）。
pub fn summarize_exchange(player: &str, user_text: &str) -> Option<String> {
    let snippet = user_text.trim();
    if snippet.is_empty() { return None; }
    let body: String = snippet.chars().take(SUMMARY_MAX_CHARS).collect();
    Some(format!("和{player}聊過，對方提到「{body}」"))
}

/// 把兩層記憶 + 近期對話組成餵進 system prompt 的「脈絡區塊」（純字串組裝、可測）。
///
/// 輸出結構（優先順序）：
/// 1. **B 層精華事實**（總是帶上，不受 cap 截斷）
/// 2. A 層 episodic 近期記憶 + **被這句話勾起的相關舊記憶**（`relevant`，見 [`Memory::relevant_memories`]）
///    + 本輪對話（合計受 [`MAX_CONTEXT_CHARS`] cap，保留尾端/近期）
///
/// 三層皆空 → 回空字串。
pub fn build_context_block(
    history: &[DialogueTurn],
    episodic: &[MemoryEntry],
    relevant: &[MemoryEntry],
    semantic: &[SemanticFact],
    player_name: &str,
) -> String {
    // ── B 層：長期精華事實（總是帶上，不受 cap 截斷）──────────────────────
    let mut semantic_block = String::new();
    if !semantic.is_empty() {
        semantic_block.push_str(&format!(
            "【你對「{player_name}」了解的重要事實（請始終記住）】\n"
        ));
        for fact in semantic {
            semantic_block.push_str(&format!("- [{}] {}\n", fact.category.label(), fact.content));
        }
    }

    // ── A 層 episodic + 相關舊記憶 + 本輪對話（可被 cap 截斷）───────────────
    let mut body = String::new();
    if !episodic.is_empty() {
        body.push_str(&format!(
            "【你對「{player_name}」的近期記憶（越上面越近期）】\n"
        ));
        for m in episodic {
            body.push_str(&format!("- {}\n", m.summary));
        }
    }
    if !relevant.is_empty() {
        if !body.is_empty() { body.push('\n'); }
        body.push_str("【這句話讓你想起一件很久以前的事】\n");
        for m in relevant {
            body.push_str(&format!("- {}\n", m.summary));
        }
    }
    if !history.is_empty() {
        if !body.is_empty() { body.push('\n'); }
        body.push_str("【你和對方剛剛的對話（依先後）】\n");
        for t in history {
            body.push_str(&format!("{player_name}：{}\n", t.user));
            body.push_str(&format!("你：{}\n", t.reply));
        }
    }
    let capped_body = cap_context_chars(body.trim_end().to_string());

    // ── 組裝 ──────────────────────────────────────────────────────────────
    match (semantic_block.is_empty(), capped_body.is_empty()) {
        (true, true)   => String::new(),
        (true, false)  => capped_body,
        (false, true)  => semantic_block.trim_end().to_string(),
        (false, false) => format!("{}\n{}", semantic_block.trim_end(), capped_body),
    }
}

/// 把 episodic + 對話合計截到 [`MAX_CONTEXT_CHARS`] 以內，保留**尾端**（近期對話在底部最重要）。
fn cap_context_chars(block: String) -> String {
    if block.chars().count() <= MAX_CONTEXT_CHARS {
        return block;
    }
    let chars: Vec<char> = block.chars().collect();
    let tail: String = chars[chars.len() - MAX_CONTEXT_CHARS..].iter().collect();
    let kept = match tail.find('\n') {
        Some(idx) => tail[idx + 1..].trim().to_string(),
        None => tail.trim().to_string(),
    };
    format!("（脈絡較長，僅保留最近的部分）\n{kept}")
}

// ── jsonl 持久化（episodic，append-only）─────────────────────────────────────
// semantic 精華層不持久化，重啟時從 episodic jsonl 重建。

/// 居民記憶落地檔（執行期產生、已隨 `data/` gitignore）。
const VOXEL_MEMORY_PATH: &str = "data/voxel_memory.jsonl";

/// 把一筆 episodic 記憶 append 到 `data/voxel_memory.jsonl`。
/// append-only、絕不覆寫/刪除既有行 → 向後相容、不破壞玩家記憶。
/// 鐵律：只在**不持任何鎖**的情境呼叫（防 prod 死鎖）。寫失敗只記 log、不 panic。
pub fn append_memory(entry: &MemoryEntry) {
    let safe = MemoryEntry {
        resident: sanitize_field(&entry.resident),
        player: sanitize_field(&entry.player),
        summary: sanitize_field(&entry.summary),
        seq: entry.seq,
    };
    if safe.summary.is_empty() { return; }
    if let Ok(record) = serde_json::to_value(&safe) {
        write_memory_line(VOXEL_MEMORY_PATH, &record);
    }
}

/// 從 `data/voxel_memory.jsonl` 載回所有 episodic 記憶（啟動時呼叫一次）。
pub fn load_memories() -> Vec<MemoryEntry> {
    read_memory_lines(VOXEL_MEMORY_PATH)
}

/// 濾掉控制字元並 trim（換行也濾掉，記憶是單句）。
fn sanitize_field(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

/// 實際把一行 JSON append 進檔。寫失敗只記 log、不 panic。
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

/// 讀回所有記憶行。壞行略過、檔缺回空。
fn read_memory_lines(path: &str) -> Vec<MemoryEntry> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() { None } else { serde_json::from_str::<MemoryEntry>(line).ok() }
        })
        .collect()
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ════════════════════════════════════════════════════════════════════════
    // 既有測試（邏輯不變、常數更新為 EPISODIC_CAP）
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn record_turn_caps_at_history_turns() {
        let mut m = VoxelMemory::new();
        for i in 0..(HISTORY_TURNS + 5) {
            m.record_turn("旅人", "vox_res_0", &format!("問{i}"), &format!("答{i}"));
        }
        let hist = m.recent_dialogue("旅人", "vox_res_0");
        assert_eq!(hist.len(), HISTORY_TURNS, "短期歷史應 cap 在 HISTORY_TURNS");
        assert_eq!(hist.last().unwrap().user, format!("問{}", HISTORY_TURNS + 4));
        assert_eq!(hist.first().unwrap().user, format!("問{}", 5));
    }

    #[test]
    fn dialogue_is_keyed_per_player_and_resident() {
        let mut m = VoxelMemory::new();
        m.record_turn("阿星", "vox_res_0", "嗨", "你好");
        m.record_turn("小美", "vox_res_0", "哈囉", "歡迎");
        assert_eq!(m.recent_dialogue("阿星", "vox_res_0").len(), 1);
        assert_eq!(m.recent_dialogue("小美", "vox_res_0").len(), 1);
        assert!(m.recent_dialogue("阿星", "vox_res_1").is_empty());
    }

    #[test]
    fn add_memory_assigns_increasing_seq_and_caps_episodic() {
        let mut m = VoxelMemory::new();
        let mut last = 0;
        for i in 0..(EPISODIC_CAP + 10) {
            let e = m.add_memory("vox_res_0", "旅人", &format!("記憶{i}"));
            assert!(e.seq >= last, "seq 應單調遞增");
            last = e.seq;
        }
        let all = m.recall("vox_res_0", "旅人", 9999);
        assert_eq!(all.len(), EPISODIC_CAP, "episodic 應 cap 在 EPISODIC_CAP");
    }

    // ── 淡忘計數 ────────────────────────────────────────────────────────────

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
        for i in 0..(EPISODIC_CAP + 3) {
            m.add_memory("vox_res_0", "旅人", &format!("記憶{i}"));
        }
        assert_eq!(m.faded_count("vox_res_0"), 3, "多推 3 筆超過 cap → 3 筆被淘汰");
        assert_eq!(m.faded_count("vox_res_1"), 0, "未超載的居民不受影響");
    }

    #[test]
    fn from_entries_replay_matches_runtime_faded_count() {
        let mut runtime = VoxelMemory::new();
        let mut entries = Vec::new();
        for i in 0..(EPISODIC_CAP + 8) {
            let e = runtime.add_memory("vox_res_0", "旅人", &format!("往事{i}"));
            entries.push(e);
        }
        let replayed = VoxelMemory::from_entries(entries);
        assert_eq!(runtime.faded_count("vox_res_0"), replayed.faded_count("vox_res_0"));
        assert_eq!(runtime.faded_count("vox_res_0"), 8);
    }

    // ── 淡忘印象（記憶 v2「整併/壓縮」最小可行版）────────────────────────────

    #[test]
    fn impression_topic_matches_known_categories_and_none_for_unrecognized() {
        assert_eq!(impression_topic("和旅人聊過，對方提到「我最喜歡看星星」"), Some("星空"));
        assert_eq!(impression_topic("和旅人聊過，對方提到「我想蓋一座高塔」"), Some("蓋造"));
        assert_eq!(impression_topic("和旅人聊過，對方提到「田裡的花開了」"), Some("種植"));
        assert_eq!(impression_topic("和旅人聊過，對方提到「洞穴裡的礦挖不完」"), Some("挖礦"));
        assert_eq!(impression_topic("和旅人聊過，對方提到「河邊釣魚真悠閒」"), Some("釣魚"));
        assert_eq!(impression_topic("和旅人聊過，對方提到「好想念老朋友」"), Some("情誼"));
        assert_eq!(impression_topic("和旅人聊過，對方提到「今天天氣真好」"), None, "無法辨識主題應回 None");
        assert_eq!(impression_topic(""), None);
    }

    #[test]
    fn impression_topic_never_echoes_raw_quote() {
        // 隱私鐵律：即使原話命中關鍵字，回傳值也必須是固定標籤集合裡的字，絕不是玩家原話本身。
        let raw = "和旅人聊過，對方提到「這句超級獨特絕不會被當成標籤star星星ZzyXk9」";
        let topic = impression_topic(raw).expect("含「星星」關鍵字應命中主題");
        assert_ne!(topic, raw, "回傳值不該是原話");
        assert_eq!(topic, "星空");
        assert!(IMPRESSION_TOPIC_KEYWORDS.iter().any(|(label, _)| *label == topic));
    }

    #[test]
    fn impression_topics_empty_when_never_evicted() {
        let mut m = VoxelMemory::new();
        m.add_memory("vox_res_0", "旅人", "和旅人聊過，對方提到「我最喜歡看星星」");
        assert!(m.impression_topics("vox_res_0").is_empty(), "沒淘汰過就沒有淡忘印象");
    }

    #[test]
    fn eviction_populates_impression_topics_capped_and_deduped() {
        let mut m = VoxelMemory::new();
        // 先塞會被淘汰的「星空」×2（應去重）與「蓋造」×1，**排在佇列最前面**；
        // 接著塞滿 EPISODIC_CAP 筆瑣事墊底，把上面三筆真的擠出佇列（FIFO 淘汰最舊）。
        m.add_memory("vox_res_0", "旅人", "和旅人聊過，對方提到「我最喜歡看星星」");
        m.add_memory("vox_res_0", "旅人", "和旅人聊過，對方提到「昨晚的星空好美」");
        m.add_memory("vox_res_0", "旅人", "和旅人聊過，對方提到「我想蓋一座高塔」");
        for i in 0..EPISODIC_CAP {
            m.add_memory("vox_res_0", "旅人", &format!("和旅人聊過，對方提到「瑣事{i}」"));
        }
        assert_eq!(m.faded_count("vox_res_0"), 3, "三筆前置記憶應已被擠出佇列");
        let topics = m.impression_topics("vox_res_0");
        assert!(topics.len() <= IMPRESSION_TOPIC_CAP, "應守 IMPRESSION_TOPIC_CAP：{topics:?}");
        assert!(topics.contains(&"星空"), "應留下星空主題：{topics:?}");
        assert!(topics.contains(&"蓋造"), "應留下蓋造主題：{topics:?}");
        // 兩筆連續「星空」只留一個（去重），不是「星空、星空」。
        assert_eq!(topics.iter().filter(|t| **t == "星空").count(), 1, "連續同主題不重複記");
    }

    #[test]
    fn eviction_dedupes_non_adjacent_repeat_topic() {
        // 交錯淘汰順序「星空 → 蓋造 → 星空」（玩家聊天的常態：話題會繞回來）——
        // 只比對 bag.back() 會漏掉非相鄰重複，讓「星空、蓋造、星空」跑進日記句。
        let mut m = VoxelMemory::new();
        m.add_memory("vox_res_0", "旅人", "和旅人聊過，對方提到「我最喜歡看星星」"); // 星空
        m.add_memory("vox_res_0", "旅人", "和旅人聊過，對方提到「我想蓋一座高塔」"); // 蓋造
        m.add_memory("vox_res_0", "旅人", "和旅人聊過，對方提到「昨晚的星空好美」"); // 星空（非相鄰重複）
        for i in 0..EPISODIC_CAP {
            m.add_memory("vox_res_0", "旅人", &format!("和旅人聊過，對方提到「瑣事{i}」"));
        }
        assert_eq!(m.faded_count("vox_res_0"), 3, "三筆前置記憶應已被擠出佇列");
        let topics = m.impression_topics("vox_res_0");
        assert_eq!(
            topics.iter().filter(|t| **t == "星空").count(),
            1,
            "非相鄰重複的「星空」也只留一個，不是「星空、蓋造、星空」：{topics:?}"
        );
        assert_eq!(topics, vec!["星空", "蓋造"], "順序應保留首次出現的位置");
    }

    #[test]
    fn from_entries_replay_matches_runtime_impression_topics() {
        let mut runtime = VoxelMemory::new();
        let mut entries = Vec::new();
        entries.push(runtime.add_memory("vox_res_0", "旅人", "和旅人聊過，對方提到「我最喜歡看星星」"));
        entries.push(runtime.add_memory("vox_res_0", "旅人", "和旅人聊過，對方提到「田裡的花開了」"));
        for i in 0..EPISODIC_CAP {
            entries.push(runtime.add_memory("vox_res_0", "旅人", &format!("和旅人聊過，對方提到「瑣事{i}」")));
        }
        let replayed = VoxelMemory::from_entries(entries);
        assert_eq!(
            runtime.impression_topics("vox_res_0"),
            replayed.impression_topics("vox_res_0"),
            "重啟重建應與線上運行產生一致的淡忘印象"
        );
        assert!(!replayed.impression_topics("vox_res_0").is_empty());
    }

    #[test]
    fn recall_filters_by_player_and_orders_recent_first() {
        let mut m = VoxelMemory::new();
        m.add_memory("vox_res_0", "阿星", "阿星想蓋橋");
        m.add_memory("vox_res_0", "小美", "小美愛釣魚");
        m.add_memory("vox_res_0", "阿星", "阿星找到礦");
        let hits = m.recall("vox_res_0", "阿星", RECALL_LIMIT);
        assert_eq!(hits.len(), 2, "只撈關於阿星的");
        assert_eq!(hits[0].summary, "阿星找到礦", "最新在前");
        assert_eq!(hits[1].summary, "阿星想蓋橋");
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
    fn relevant_memories_finds_similar_old_memory_outside_recall_window() {
        let mut m = VoxelMemory::new();
        m.add_memory("vox_res_0", "阿星", "阿星說他最想要的東西是一把乙太鑰匙");
        // 塞滿 RECALL_LIMIT 筆新記憶，把上面那筆擠出近期回想窗。
        for i in 0..RECALL_LIMIT {
            m.add_memory("vox_res_0", "阿星", &format!("閒聊瑣事{i}"));
        }
        let episodic = m.recall("vox_res_0", "阿星", RECALL_LIMIT);
        assert!(
            episodic.iter().all(|e| !e.summary.contains("乙太鑰匙")),
            "舊記憶應已滑出近期窗，才是這條測試要驗的情境"
        );
        let exclude: Vec<u64> = episodic.iter().map(|e| e.seq).collect();
        let hits = m.relevant_memories(
            "vox_res_0",
            "阿星",
            "你之前是不是說想要一把乙太鑰匙",
            &exclude,
            RELEVANT_RECALL_LIMIT,
        );
        assert_eq!(hits.len(), 1, "應撈回那筆被勾起的舊記憶");
        assert!(hits[0].summary.contains("乙太鑰匙"));
    }

    #[test]
    fn relevant_memories_finds_match_through_real_summarize_exchange_format() {
        // 真實寫入路徑：summary 不是測試餵的乾淨句子，而是 summarize_exchange 產出的
        // 「和X聊過，對方提到「…」」固定殼——比對前沒剝殼會被殼裡的噪音 bigram 稀釋掉分數。
        let summary = summarize_exchange("阿星", "我最喜歡看流星了").unwrap();
        let mut m = VoxelMemory::new();
        m.add_memory("vox_res_0", "阿星", &summary);
        let hits = m.relevant_memories(
            "vox_res_0",
            "阿星",
            "你之前是不是說過很喜歡看流星",
            &[],
            RELEVANT_RECALL_LIMIT,
        );
        assert_eq!(hits.len(), 1, "剝殼後應撈回這筆帶固定前綴格式的真實記憶");
        assert!(hits[0].summary.contains("流星"));
    }

    #[test]
    fn relevant_memories_ignores_unrelated_query() {
        let mut m = VoxelMemory::new();
        m.add_memory("vox_res_0", "阿星", "阿星說他最想要的東西是一把乙太鑰匙");
        let hits = m.relevant_memories(
            "vox_res_0",
            "阿星",
            "今天天氣真好呢",
            &[],
            RELEVANT_RECALL_LIMIT,
        );
        assert!(hits.is_empty(), "字面不像的話不該硬掰出一段舊記憶");
    }

    #[test]
    fn relevant_memories_excludes_given_seqs_and_respects_limit_and_player() {
        let mut m = VoxelMemory::new();
        m.add_memory("vox_res_0", "阿星", "阿星說想要乙太鑰匙一號");
        m.add_memory("vox_res_0", "阿星", "阿星說想要乙太鑰匙二號");
        m.add_memory("vox_res_0", "阿星", "阿星說想要乙太鑰匙三號");
        m.add_memory("vox_res_0", "小美", "小美說想要乙太鑰匙四號");
        let exclude = vec![0u64]; // 排除「一號」那筆（模擬已被 recall 撈走）
        let hits = m.relevant_memories("vox_res_0", "阿星", "乙太鑰匙", &exclude, 1);
        assert_eq!(hits.len(), 1, "limit 應限制回傳筆數");
        assert!(!hits[0].summary.contains("一號"), "exclude_seqs 應排除掉");
        assert!(hits.iter().all(|e| !e.summary.contains("小美")), "不應撈到別的玩家的記憶");
    }

    #[test]
    fn summarize_extracts_snippet_and_truncates() {
        let s = summarize_exchange("阿星", "  我想在河上蓋一座橋  ").unwrap();
        assert!(s.contains("阿星"));
        assert!(s.contains("蓋一座橋"));
        assert!(summarize_exchange("阿星", "   ").is_none());
        let long = "字".repeat(SUMMARY_MAX_CHARS + 50);
        let out = summarize_exchange("旅人", &long).unwrap();
        assert!(out.matches('字').count() <= SUMMARY_MAX_CHARS);
    }

    #[test]
    fn context_block_renders_memories_and_history() {
        let history = vec![DialogueTurn {
            user: "你還記得我嗎".to_string(),
            reply: "當然記得".to_string(),
        }];
        let episodic = vec![MemoryEntry {
            resident: "vox_res_0".to_string(),
            player: "阿星".to_string(),
            summary: "阿星想蓋橋".to_string(),
            seq: 1,
        }];
        // 含 semantic 精華
        let semantic = vec![SemanticFact {
            category: FactCategory::Goal,
            content: "阿星說要蓋橋".to_string(),
        }];
        let block = build_context_block(&history, &episodic, &[], &semantic, "阿星");
        assert!(block.contains("阿星想蓋橋"), "應含 episodic 記憶");
        assert!(block.contains("你還記得我嗎"), "應含近期對話");
        assert!(block.contains("阿星說要蓋橋"), "應含 semantic 精華");
        // 四層皆空 → 空字串
        assert!(build_context_block(&[], &[], &[], &[], "阿星").is_empty());
    }

    #[test]
    fn context_block_renders_relevant_section_distinctly() {
        let relevant = vec![MemoryEntry {
            resident: "vox_res_0".to_string(),
            player: "阿星".to_string(),
            summary: "阿星很久以前提過想要一把乙太鑰匙".to_string(),
            seq: 0,
        }];
        let block = build_context_block(&[], &[], &relevant, &[], "阿星");
        assert!(block.contains("這句話讓你想起一件很久以前的事"), "應有專屬標籤區隔近期記憶");
        assert!(block.contains("乙太鑰匙"));
        // 空 relevant → 不出現該標籤
        assert!(!build_context_block(&[], &[], &[], &[], "阿星").contains("這句話讓你想起"));
    }

    #[test]
    fn context_block_capped_keeps_recent_dialogue() {
        let episodic: Vec<MemoryEntry> = (0..40)
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
        let block = build_context_block(&history, &episodic, &[], &[], "阿星");
        // cap 只截 episodic + 對話，不截 semantic（這裡 semantic 為空）
        assert!(
            block.chars().count() <= MAX_CONTEXT_CHARS + 40,
            "脈絡應截到上限內：{}",
            block.chars().count()
        );
        assert!(block.contains("我剛剛說的最新一句話"), "截斷後仍須保留最近對話");
        assert!(block.contains("脈絡較長"), "超長時應加省略標記");
    }

    #[test]
    fn from_entries_rebuilds_caps_and_continues_seq() {
        let mut entries = Vec::new();
        for i in 0..(EPISODIC_CAP + 5) {
            entries.push(MemoryEntry {
                resident: "vox_res_0".to_string(),
                player: "旅人".to_string(),
                summary: format!("事件{i}"),
                seq: i as u64,
            });
        }
        entries.reverse(); // 打亂順序，驗證內部會排序
        let mut m = VoxelMemory::from_entries(entries);
        assert_eq!(m.recall("vox_res_0", "旅人", 9999).len(), EPISODIC_CAP, "載入即 cap");
        let e = m.add_memory("vox_res_0", "旅人", "重啟後的新記憶");
        assert!(e.seq > (EPISODIC_CAP + 4) as u64, "seq 應續接既有最大值之後");
    }

    #[test]
    fn jsonl_roundtrip_persists_and_loads() {
        let dir = std::env::temp_dir().join(format!("voxmem_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("voxel_memory.jsonl");
        let pstr = path.to_str().unwrap();
        let _ = std::fs::remove_file(&path);

        let e1 = MemoryEntry {
            resident: "vox_res_0".to_string(), player: "阿星".to_string(),
            summary: "阿星想蓋橋".to_string(), seq: 0,
        };
        let e2 = MemoryEntry {
            resident: "vox_res_1".to_string(), player: "小美".to_string(),
            summary: "小美愛釣魚".to_string(), seq: 1,
        };
        write_memory_line(pstr, &serde_json::to_value(&e1).unwrap());
        write_memory_line(pstr, &serde_json::to_value(&e2).unwrap());

        let loaded = read_memory_lines(pstr);
        assert_eq!(loaded.len(), 2);
        assert!(loaded.contains(&e1));
        assert!(loaded.contains(&e2));

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
        assert!(read_memory_lines("data/__definitely_not_here_voxmem__.jsonl").is_empty());
    }

    // ── affinity_count ───────────────────────────────────────────────────────

    #[test]
    fn affinity_zero_for_stranger() {
        assert_eq!(VoxelMemory::new().affinity_count("小明", "vox_res_0"), 0);
    }

    #[test]
    fn affinity_counts_only_matching_player_and_resident() {
        let mut store = VoxelMemory::new();
        store.add_memory("vox_res_0", "小明", "小明說想蓋房子");
        store.add_memory("vox_res_0", "小明", "小明聊到星星");
        store.add_memory("vox_res_0", "小美", "小美說愛農耕");
        store.add_memory("vox_res_1", "小明", "小明在居民1處說話");
        assert_eq!(store.affinity_count("小明", "vox_res_0"), 2);
        assert_eq!(store.affinity_count("小美", "vox_res_0"), 1);
        assert_eq!(store.affinity_count("小明", "vox_res_1"), 1);
        assert_eq!(store.affinity_count("小美", "vox_res_1"), 0);
    }

    #[test]
    fn affinity_three_or_more_is_friend_tier() {
        let mut store = VoxelMemory::new();
        store.add_memory("vox_res_0", "小明", "a");
        store.add_memory("vox_res_0", "小明", "b");
        store.add_memory("vox_res_0", "小明", "c");
        assert!(store.affinity_count("小明", "vox_res_0") >= 3);
    }

    // ── recall_bubble ────────────────────────────────────────────────────────

    #[test]
    fn recall_bubble_standard_format() {
        let summary = "和阿星聊過，對方提到「想在這裡蓋觀星塔」";
        let bubble = recall_bubble(summary);
        assert!(bubble.contains("「想在這裡蓋觀星塔"), "應含玩家原話");
        assert!(bubble.starts_with("我記得你說過"));
    }

    #[test]
    fn recall_bubble_no_bracket_falls_back_to_opening() {
        let bubble = recall_bubble("某個特殊格式沒有引號的摘要文字");
        assert!(!bubble.is_empty());
    }

    #[test]
    fn recall_bubble_empty_summary_returns_fallback() {
        assert_eq!(recall_bubble(""), "我還記得你……");
    }

    #[test]
    fn recall_bubble_snippet_bounded() {
        let long_inner = "「".to_string() + &"X".repeat(50) + "」";
        let bubble = recall_bubble(&long_inner);
        assert!(bubble.chars().count() <= 28, "回想泡泡不應過長");
    }

    #[test]
    fn recall_bubble_non_empty_for_typical_memory() {
        let summary = summarize_exchange("阿信", "你這裡的石頭好漂亮").unwrap();
        let bubble = recall_bubble(&summary);
        assert!(!bubble.is_empty());
        assert!(bubble.contains("你這裡的石頭好漂亮") || bubble.contains("你這裡"));
    }

    #[test]
    fn recall_affinity_threshold_is_sane() {
        assert!(RECALL_AFFINITY_THRESHOLD >= 2);
        assert!(RECALL_AFFINITY_THRESHOLD <= 5);
    }

    // ════════════════════════════════════════════════════════════════════════
    // 新增測試：is_test_identity
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn is_test_identity_filters_shih_digits() {
        assert!(is_test_identity("Shih0"));
        assert!(is_test_identity("Shih136"));
        assert!(is_test_identity("Shih978"));
        assert!(!is_test_identity("Shih"));        // 無數字後綴不濾
        assert!(is_test_identity("ShihTest"));     // ShihTest 在精確名單中，應被濾掉
    }

    #[test]
    fn is_test_identity_filters_t_digits() {
        assert!(is_test_identity("T81"));
        assert!(is_test_identity("T95"));
        assert!(is_test_identity("T452"));
        assert!(!is_test_identity("T"));           // 無數字不濾
        assert!(!is_test_identity("Ta"));          // 非全數字不濾
    }

    #[test]
    fn is_test_identity_filters_exact_names() {
        assert!(is_test_identity("DbgTest"));
        assert!(is_test_identity("ShihTest"));
        assert!(is_test_identity("小石"));
    }

    #[test]
    fn is_test_identity_preserves_real_players() {
        assert!(!is_test_identity("濕濕的"), "真玩家不能被過濾");
        assert!(!is_test_identity("旅人"),   "旅人不能被過濾");
        assert!(!is_test_identity("阿星"));
        assert!(!is_test_identity(""));
    }

    // ════════════════════════════════════════════════════════════════════════
    // 字元 bigram Jaccard（相關性回想的底層純函式）
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn char_bigrams_handles_short_and_normal_strings() {
        assert_eq!(char_bigrams(""), HashSet::new());
        assert_eq!(char_bigrams("a"), HashSet::from(["a".to_string()]));
        assert_eq!(
            char_bigrams("abc"),
            HashSet::from(["ab".to_string(), "bc".to_string()])
        );
    }

    #[test]
    fn bigram_jaccard_identical_is_one_disjoint_is_zero() {
        let a = char_bigrams("乙太鑰匙");
        assert_eq!(bigram_jaccard(&a, &a), 1.0, "自己跟自己相似度應為 1");
        let b = char_bigrams("完全不同的字句");
        assert_eq!(bigram_jaccard(&a, &b), 0.0, "無重疊應為 0");
        assert_eq!(bigram_jaccard(&HashSet::new(), &a), 0.0, "空集合視為 0（非除以零 panic）");
    }

    // ════════════════════════════════════════════════════════════════════════
    // 新增測試：classify_importance（重要性判定）
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn classify_identity_from_quote() {
        let summary = summarize_exchange("旅人", "我叫小明").unwrap();
        match classify_importance(&summary) {
            Importance::Persistent(f) => {
                assert_eq!(f.category, FactCategory::Identity);
                assert!(f.content.contains("小明"), "事實應含名字：{}", f.content);
            }
            Importance::Ephemeral => panic!("名字宣告應被判為 Persistent"),
        }
    }

    #[test]
    fn classify_identity_keke_form() {
        let summary = summarize_exchange("旅人", "可以叫我阿惠").unwrap();
        match classify_importance(&summary) {
            Importance::Persistent(f) => {
                assert_eq!(f.category, FactCategory::Identity);
                assert!(f.content.contains("阿惠"));
            }
            _ => panic!("可以叫我X 應被判為 Identity"),
        }
    }

    #[test]
    fn classify_goal_detected() {
        let summary = summarize_exchange("旅人", "我想要蓋一座橋").unwrap();
        match classify_importance(&summary) {
            Importance::Persistent(f) => assert_eq!(f.category, FactCategory::Goal),
            _ => panic!("目標陳述應被判為 Persistent Goal"),
        }
    }

    #[test]
    fn classify_preference_detected() {
        let summary = summarize_exchange("旅人", "我最喜歡看星星").unwrap();
        match classify_importance(&summary) {
            Importance::Persistent(f) => assert_eq!(f.category, FactCategory::Preference),
            _ => panic!("偏好陳述應被判為 Persistent Preference"),
        }
    }

    #[test]
    fn classify_promise_detected() {
        let summary = summarize_exchange("旅人", "我答應你一定會回來").unwrap();
        match classify_importance(&summary) {
            Importance::Persistent(f) => assert_eq!(f.category, FactCategory::Promise),
            _ => panic!("承諾陳述應被判為 Persistent Promise"),
        }
    }

    #[test]
    fn classify_small_talk_is_ephemeral() {
        for text in &["你好", "再見", "1+1等於多少", "今天天氣真好", "哈哈哈"] {
            let summary = summarize_exchange("旅人", text).unwrap();
            assert_eq!(
                classify_importance(&summary),
                Importance::Ephemeral,
                "寒暄應為 Ephemeral：{text}"
            );
        }
    }

    // ════════════════════════════════════════════════════════════════════════
    // 新增測試：merge_into_semantic（精華層合併）
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn merge_identity_replaces_existing() {
        let mut store = Vec::new();
        merge_into_semantic(&mut store, SemanticFact {
            category: FactCategory::Identity,
            content: "我叫小明".to_string(),
        });
        merge_into_semantic(&mut store, SemanticFact {
            category: FactCategory::Identity,
            content: "我叫大明".to_string(), // 更新名字
        });
        let ids: Vec<_> = store.iter().filter(|f| f.category == FactCategory::Identity).collect();
        assert_eq!(ids.len(), 1, "身份只保留一條");
        assert_eq!(ids[0].content, "我叫大明", "應取代為最新的名字");
    }

    #[test]
    fn merge_similar_goal_updates_in_place() {
        // 兩個目標的前 20 字必須相同才會就地更新（合併條件）。
        // 用長度 > 20 字且前 20 字完全一致的字串驗證。
        // 前 20 字：「我想在世界的東方蓋一座連接兩片大陸的橋梁」（20 char）
        let base = "我想在世界的東方蓋一座連接兩片大陸的橋梁";
        let v1 = format!("{base}（舊版方案）");
        let v2 = format!("{base}（新版方案）");
        assert_eq!(base.chars().count(), 20, "前綴剛好 20 字");

        let mut store = Vec::new();
        merge_into_semantic(&mut store, SemanticFact {
            category: FactCategory::Goal, content: v1,
        });
        // 前 20 字相同 → 就地更新，不堆疊
        merge_into_semantic(&mut store, SemanticFact {
            category: FactCategory::Goal, content: v2,
        });
        let goals: Vec<_> = store.iter().filter(|f| f.category == FactCategory::Goal).collect();
        assert_eq!(goals.len(), 1, "相似目標應就地更新，不堆疊");
        assert!(goals[0].content.contains("新版方案"), "應保留最新版本：{:?}", goals[0].content);
    }

    #[test]
    fn merge_different_goals_accumulate_to_cap() {
        let mut store = Vec::new();
        // 推入 SEMANTIC_CAP + 2 條不同目標，確認不超過 cap
        for i in 0..(SEMANTIC_CAP + 2) {
            merge_into_semantic(&mut store, SemanticFact {
                category: FactCategory::Goal,
                content: format!("唯一目標編號{i:04}"), // 前 20 字各異，不合併
            });
        }
        assert!(store.len() <= SEMANTIC_CAP, "精華層不應超過 SEMANTIC_CAP：{}", store.len());
    }

    // ════════════════════════════════════════════════════════════════════════
    // 核心整合測試：塞爆 episodic → B 層精華仍在
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn semantic_survives_episodic_cap_overflow() {
        // 情境：玩家先聲告名字、說明目標，然後講一大堆寒暄，塞爆 episodic cap。
        // 預期：episodic 裡名字/目標那幾筆被擠掉，但 semantic 精華層仍保有名字/目標。
        let mut m = VoxelMemory::new();
        let resident = "vox_res_0";
        let player = "旅人";

        // 1. 講名字（重要）
        let s1 = summarize_exchange(player, "我叫濕濕的").unwrap();
        m.add_memory(resident, player, &s1);

        // 2. 說明計劃（重要）
        let s2 = summarize_exchange(player, "我要建設一個從小村莊到城市國家的大世界").unwrap();
        m.add_memory(resident, player, &s2);

        // 3. 塞爆 episodic cap 的寒暄（EPISODIC_CAP 倍）
        for i in 0..(EPISODIC_CAP + 5) {
            let s = summarize_exchange(player, &format!("你好啊第{i}次")).unwrap();
            m.add_memory(resident, player, &s);
        }

        // ── 驗證 episodic 只保留最新 EPISODIC_CAP 筆（名字/目標已被擠掉）
        let episodic = m.recall(resident, player, 9999);
        assert_eq!(episodic.len(), EPISODIC_CAP, "episodic 應 cap 在 EPISODIC_CAP");
        // 名字/目標的 episodic 記憶確實已不在
        assert!(
            !episodic.iter().any(|e| e.summary.contains("我叫濕濕的")),
            "名字的 episodic 記憶應已被擠掉"
        );

        // ── 驗證 B 層精華仍完整保有名字和目標
        let semantic = m.semantic_facts_for(resident, player);
        assert!(!semantic.is_empty(), "semantic 精華層不應為空");
        let identity = semantic.iter().find(|f| f.category == FactCategory::Identity);
        assert!(identity.is_some(), "精華層應有身份事實");
        assert!(
            identity.unwrap().content.contains("濕濕的"),
            "精華層身份應含名字「濕濕的」：{:?}",
            identity.unwrap().content
        );
        let goal = semantic.iter().find(|f| f.category == FactCategory::Goal);
        assert!(goal.is_some(), "精華層應有目標事實");
        assert!(
            goal.unwrap().content.contains("建設") || goal.unwrap().content.contains("村莊"),
            "精華層目標應含關鍵字：{:?}",
            goal.unwrap().content
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // 遷移 + 測試污染清除測試
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn migration_filters_test_identities_preserves_real_players() {
        let entries = vec![
            MemoryEntry { resident: "r0".to_string(), player: "Shih0".to_string(),
                          summary: "測試污染".to_string(), seq: 0 },
            MemoryEntry { resident: "r0".to_string(), player: "T81".to_string(),
                          summary: "測試污染2".to_string(), seq: 1 },
            MemoryEntry { resident: "r0".to_string(), player: "DbgTest".to_string(),
                          summary: "測試污染3".to_string(), seq: 2 },
            MemoryEntry { resident: "r0".to_string(), player: "濕濕的".to_string(),
                          summary: "我叫濕濕的".to_string(), seq: 3 },
            MemoryEntry { resident: "r0".to_string(), player: "旅人".to_string(),
                          summary: "旅人的真實記憶".to_string(), seq: 4 },
        ];
        let m = VoxelMemory::from_entries(entries);

        // 測試身份記錄全數丟棄
        assert_eq!(m.recall("r0", "Shih0", 9999).len(), 0, "Shih0 應被過濾");
        assert_eq!(m.recall("r0", "T81", 9999).len(), 0, "T81 應被過濾");
        assert_eq!(m.recall("r0", "DbgTest", 9999).len(), 0, "DbgTest 應被過濾");

        // 真玩家記錄完整保留
        assert_eq!(m.recall("r0", "濕濕的", 9999).len(), 1, "真玩家 濕濕的 應保留");
        assert_eq!(m.recall("r0", "旅人", 9999).len(), 1, "旅人應保留");
    }

    #[test]
    fn migration_rebuilds_semantic_from_old_episodic() {
        // 模擬舊格式 jsonl（只有 episodic，無 semantic）載入時重建精華層
        let entries = vec![
            MemoryEntry {
                resident: "r0".to_string(),
                player: "旅人".to_string(),
                summary: "和旅人聊過，對方提到「我叫阿偉」".to_string(),
                seq: 0,
            },
            MemoryEntry {
                resident: "r0".to_string(),
                player: "旅人".to_string(),
                summary: "和旅人聊過，對方提到「今天天氣真好」".to_string(),
                seq: 1,
            },
        ];
        let m = VoxelMemory::from_entries(entries);
        let semantic = m.semantic_facts_for("r0", "旅人");
        let id = semantic.iter().find(|f| f.category == FactCategory::Identity);
        assert!(id.is_some(), "舊格式載入後應重建身份精華");
        assert!(id.unwrap().content.contains("阿偉"), "身份精華應含名字：{:?}", id);
    }

    // ════════════════════════════════════════════════════════════════════════
    // build_context_block 兩層整合測試
    // ════════════════════════════════════════════════════════════════════════

    #[test]
    fn context_block_includes_both_layers() {
        let episodic = vec![MemoryEntry {
            resident: "r0".to_string(), player: "旅人".to_string(),
            summary: "旅人說了什麼瑣事".to_string(), seq: 0,
        }];
        let semantic = vec![
            SemanticFact { category: FactCategory::Identity, content: "我叫阿偉".to_string() },
            SemanticFact { category: FactCategory::Goal,     content: "我想蓋城堡".to_string() },
        ];
        let block = build_context_block(&[], &episodic, &[], &semantic, "旅人");

        assert!(block.contains("我叫阿偉"),   "B 層身份應出現在脈絡中");
        assert!(block.contains("我想蓋城堡"), "B 層目標應出現在脈絡中");
        assert!(block.contains("旅人說了什麼瑣事"), "A 層 episodic 應出現在脈絡中");
        // B 層優先（出現在 A 層之前）
        let b_pos = block.find("我叫阿偉").unwrap();
        let a_pos = block.find("旅人說了什麼瑣事").unwrap();
        assert!(b_pos < a_pos, "B 層精華應排在 A 層 episodic 之前");
    }

    #[test]
    fn context_block_semantic_not_truncated_by_cap() {
        // 即使 body 過長被截，semantic 精華層仍完整出現
        let big_episodic: Vec<MemoryEntry> = (0..40)
            .map(|i| MemoryEntry {
                resident: "r0".to_string(), player: "旅人".to_string(),
                summary: format!("長段舊記憶第{i:02}條填充填充填充填充填充填充"),
                seq: i as u64,
            })
            .collect();
        let semantic = vec![SemanticFact {
            category: FactCategory::Identity,
            content: "我叫濕濕的".to_string(),
        }];
        let block = build_context_block(&[], &big_episodic, &[], &semantic, "旅人");
        assert!(block.contains("我叫濕濕的"), "semantic 精華層不應被 episodic cap 截掉");
    }

    #[test]
    fn semantic_facts_for_returns_empty_for_unknown() {
        let m = VoxelMemory::new();
        assert!(m.semantic_facts_for("vox_res_0", "旅人").is_empty());
    }

    #[test]
    fn all_player_memories_filters_by_player_newest_first_no_cap() {
        let mut m = VoxelMemory::new();
        // 交錯登記兩位玩家的記憶。
        m.add_memory("r0", "露娜", "露娜送來木頭");     // seq 0
        m.add_memory("r0", "諾娃", "諾娃隨便聊聊");     // seq 1
        m.add_memory("r0", "露娜", "露娜幫我蓋牆");     // seq 2
        m.add_memory("r0", "露娜", "露娜親手蓋起方塊"); // seq 3
        let luna = m.all_player_memories("r0", "露娜");
        assert_eq!(luna.len(), 3, "只回露娜名下的三筆");
        assert!(luna.iter().all(|e| e.player == "露娜"));
        // 最新在前（seq 遞減）
        assert!(luna[0].seq > luna[1].seq && luna[1].seq > luna[2].seq);
        // 未知玩家 / 居民 → 空
        assert!(m.all_player_memories("r0", "無此人").is_empty());
        assert!(m.all_player_memories("無此居民", "露娜").is_empty());
    }
}
