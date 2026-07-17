//! 乙太方界·居民人生志業 v1（純引擎層）——每位居民不再只是「有口吻、有生計」，
//! 而是懷著一條 **F5 著墨的「人生志業」**：把這位居民的靈魂夢想，拆成一條多階段的
//! 里程碑之路（`Milestone`）。每個里程碑對應一項**真實成就**（蓋了某種建物、發明了
//! 幾樣手藝、教了幾個人、與心上人成雙、立起村碑……），而**進展完全由「查真實累積
//! 狀態」湧現得出，零腳本、零硬推**——居民真的做到了那件事，志業才往前一階，並吐出
//! 一句第一人稱的內心獨白（`feed_line`）；走到最後一階，附上那句一生的夢（`final_line`）。
//!
//! **設計信念**（承 `docs/PLAN_ETHERVOX.md`「記憶→行為、讓居民真的活著」）：
//! 志業不是任務系統、不是進度條腳本。它只是一面鏡子——照著居民「真的累積了什麼」，
//! 在牠恰好跨過某個門檻的那一刻，替牠說出「我這一路，原來是往這裡走的」。因此本模組的
//! 核心是一個**確定性純函式** [`trigger_met`]：把里程碑的 `trigger` 字串對照一份由呼叫端
//! 填好的**真實成就快照** [`AchievementSnapshot`]，回答「達成了沒」。
//!
//! **保守鐵律**：看不懂的 `trigger` → 一律回 `false`（寧可志業停在原地，也**絕不誤推進**、
//! 絕不替居民虛構一段沒真正發生的人生）。採集／合成類（`gather:` / `craft:`）在還沒有可信
//! 來源前，快照給的是空計數 → 自然評 `false`——同一條「沒真憑實據就不動」的鐵律。
//!
//! 純邏輯層：結構、解析、評估、逐階推進皆為**純函式 / 純資料**，零 async、零鎖、零 LLM。
//! 持久化只有讀寫 jsonl 的 IO 函式；鎖與接線（每 tick 收集真實快照、廣播、記動態）全在
//! `voxel_ws.rs`——**本模組完全不碰 `voxel_ws.rs`**，接線在後續 PR。不抄外部碼；繁中註解。

use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};

// ── 志業資料結構（唯讀，來自 F5 著墨的 jsonl）─────────────────────────────────

/// 一個里程碑：志業之路上的一階。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Milestone {
    /// 第幾階（1 起算，與 `current_stage` 對齊：`current_stage == stage` 表示這階已達成）。
    pub stage: u32,
    /// 這階的名字（給人看的短標題，如「把手藝傳給米拉」）。
    pub name: String,
    /// 觸發條件字串（對照 [`AchievementSnapshot`]，見 [`trigger_met`] 的文法表）。
    pub trigger: String,
    /// 達成這階時，居民吐出的第一人稱內心獨白。
    pub feed_line: String,
}

/// 一位居民的整條人生志業。
///
/// 欄位對映 jsonl（`pursuit_title` → [`title`]、`final_dream_line` → [`final_line`]），
/// 缺欄靠 serde 容錯；未知欄位忽略——向後相容，日後 F5 加欄不破舊碼。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pursuit {
    /// 居民 id（如 `vox_res_0`），與快照 / 進度帳本的鍵一致。
    pub id: String,
    /// 居民名字（如「露娜」，僅供顯示）。
    pub name: String,
    /// 志業標題（jsonl 欄位 `pursuit_title`）。
    #[serde(rename = "pursuit_title")]
    pub title: String,
    /// 里程碑之路（依 `stage` 遞增；本模組以 `stage` 值定位下一階，不假設陣列索引）。
    pub milestones: Vec<Milestone>,
    /// 走到最後一階時，附上的那句一生的夢（jsonl 欄位 `final_dream_line`）。
    #[serde(rename = "final_dream_line")]
    pub final_line: String,
}

/// 全體居民的志業表（純同步資料，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct PursuitStore {
    /// key = 居民 id → 那條志業。
    pursuits: HashMap<String, Pursuit>,
}

impl PursuitStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 查某居民的志業（未登記回 `None`）。
    pub fn get(&self, id: &str) -> Option<&Pursuit> {
        self.pursuits.get(id)
    }

    /// 收錄一條志業（同 id 覆蓋——後到的優先，比照 jsonl 逐行讀入語意）。
    pub fn insert(&mut self, pursuit: Pursuit) {
        self.pursuits.insert(pursuit.id.clone(), pursuit);
    }

    /// 目前收錄的志業條數。
    pub fn len(&self) -> usize {
        self.pursuits.len()
    }

    /// 是否一條志業都沒有。
    pub fn is_empty(&self) -> bool {
        self.pursuits.is_empty()
    }
}

// ── 志業表 IO（讀 jsonl，鎖在 voxel_ws.rs）──────────────────────────────────

const PURSUITS_FILE: &str = "data/voxel_life_pursuits.jsonl";

/// 從 `data/voxel_life_pursuits.jsonl` 讀入全體志業（**缺檔 → 空表，向後相容**）。
/// 比照 `voxel_livelihood::load_livelihood`：純 IO，解析失敗 / 空白行安靜略過，
/// 不因一行壞掉丟掉整檔——寧可少一條志業，也不讓服務起不來。
pub fn load_pursuits() -> PursuitStore {
    load_pursuits_from(PURSUITS_FILE)
}

/// [`load_pursuits`] 的路徑可注入版本（供測試指向假路徑／假檔）。
fn load_pursuits_from(path: &str) -> PursuitStore {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return PursuitStore::default(), // 缺檔＝空表（向後相容）
    };
    parse_pursuits(&content)
}

/// 把整份 jsonl 內容解析成志業表（每行一條 `Pursuit`；壞行 / 空行略過）。
fn parse_pursuits(content: &str) -> PursuitStore {
    let mut store = PursuitStore::default();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(p) = serde_json::from_str::<Pursuit>(line) {
            store.insert(p);
        }
        // 壞行安靜略過（向後相容，不 panic、不中斷）
    }
    store
}

// ── 志業進度（獨立持久化，append-only last-wins）────────────────────────────

/// 一筆進度快照：某居民推進到第幾階（`stage`）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProgressEntry {
    /// 居民 id。
    pub resident: String,
    /// 當前達成到第幾階（`0` = 尚未達成第 1 個里程碑）。
    pub stage: u32,
}

/// 居民志業進度帳本（純同步資料，由呼叫端包進 `RwLock`）。
///
/// 與志業表**獨立**：志業內容是 F5 著墨的靜態底本，進度則是每位居民走到哪的動態狀態，
/// 兩者分檔存放、互不干擾。進度只前進不倒退（推進語意）。
#[derive(Default, Debug)]
pub struct PursuitProgress {
    /// key = 居民 id → 當前階（`0` = 未達成第 1 個里程碑）。
    stages: HashMap<String, u32>,
}

impl PursuitProgress {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。
    /// **append-only last-wins**：同一居民多筆時，以檔案順序**最後一筆**為準
    /// （後寫的進度自然遮蓋前身，比照 `voxel_livelihood` 的快照語意）。
    pub fn from_entries(entries: impl IntoIterator<Item = ProgressEntry>) -> Self {
        let mut p = Self::new();
        for e in entries {
            // 逐行覆蓋——最後讀到的那筆勝出（append-only 下即最新）。
            p.stages.insert(e.resident, e.stage);
        }
        p
    }

    /// 查某居民當前階（未登記回 `0` ＝ 還沒達成第 1 個里程碑）。
    pub fn stage_of(&self, resident: &str) -> u32 {
        self.stages.get(resident).copied().unwrap_or(0)
    }

    /// 把某居民的進度設到 `stage`（**只前進不倒退**）。
    /// 回傳 `true` 表示真的往前推了（需持久化 / 廣播）；`false` 表示沒前進（維持原狀）。
    pub fn set_stage(&mut self, resident: &str, stage: u32) -> bool {
        if stage > self.stage_of(resident) {
            self.stages.insert(resident.to_string(), stage);
            true
        } else {
            false
        }
    }

    /// 轉成持久化記錄（快照，寫 jsonl 用）。順序不保證（HashMap 迭代）。
    pub fn to_entries(&self) -> Vec<ProgressEntry> {
        self.stages
            .iter()
            .map(|(resident, &stage)| ProgressEntry {
                resident: resident.clone(),
                stage,
            })
            .collect()
    }

    /// 目前登記在案的居民數。
    pub fn len(&self) -> usize {
        self.stages.len()
    }

    /// 是否無任何進度登記。
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
}

const PROGRESS_FILE: &str = "data/voxel_life_pursuit_progress.jsonl";

/// 從 `data/voxel_life_pursuit_progress.jsonl` 讀回進度（缺檔 → 空帳本）。
pub fn load_progress() -> PursuitProgress {
    load_progress_from(PROGRESS_FILE)
}

/// [`load_progress`] 的路徑可注入版本（供測試用臨時檔）。
fn load_progress_from(path: &str) -> PursuitProgress {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return PursuitProgress::default(),
    };
    let entries = content
        .lines()
        .filter_map(|l| serde_json::from_str::<ProgressEntry>(l).ok());
    PursuitProgress::from_entries(entries)
}

/// 把整份進度快照 append 一行到 `data/voxel_life_pursuit_progress.jsonl`
/// （比照 `save_livelihood`：居民數極少，每次推進整份快照重寫也不會無限長大，
/// 讀檔時 last-wins 折疊掉舊筆）。
pub fn save_progress(progress: &PursuitProgress) {
    save_progress_to(PROGRESS_FILE, progress);
}

/// [`save_progress`] 的路徑可注入版本（供測試用臨時檔）。
fn save_progress_to(path: &str, progress: &PursuitProgress) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        for e in progress.to_entries() {
            if let Ok(line) = serde_json::to_string(&e) {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

// ── 真實成就快照（owned、無鎖，呼叫端填）──────────────────────────────────────

/// 一份**當下的真實累積狀態**，由 `voxel_ws.rs` 在每個低頻節拍收集後填好、傳進純函式。
///
/// 刻意設計成 **owned、無任何鎖 / 引用**——呼叫端在鎖內把要用的數字抄進來，出鎖後再交給
/// [`trigger_met`] 評估，純函式完全不碰任何共享狀態。缺的維度給預設（`0` / 空集合 / `None`），
/// 對應「沒有可信來源」——照保守鐵律，那類 `trigger` 自然評 `false`。
#[derive(Default, Debug, Clone)]
pub struct AchievementSnapshot {
    /// 各種建物已蓋座數（key = 建物種類，如 `"Well"`、`"Pavilion"`）。
    pub builds_by_kind: HashMap<String, u32>,
    /// 自己摸索發明出來的手藝總數（`invent:any` / `invent:count:N`）。
    pub invented_count: u32,
    /// 發明出的手藝種類集合（供 `invent:smelt` 這類「發明了某特定門道」）。
    pub invented_kinds: HashSet<String>,
    /// 教會別人的次數（`teach:N`）。
    pub taught_count: u32,
    /// 心上人（戀人）名字集合（`romance:sweetheart:<名>`）。
    pub sweethearts: HashSet<String>,
    /// 締結婚約 / 成親的對象名字集合（`romance:wedding:<名>`）。
    pub marriages: HashSet<String>,
    /// 是否已與戀人蓋起愛巢（`romance:lovenest`）。
    pub has_lovenest: bool,
    /// 孩子數（`family:child:N`）。
    pub child_count: u32,
    /// 是否已立起村碑 / 開拓出新聚落（`colony:found`）。
    pub colony_founded: bool,
    /// 遷居代號（`0` = 未遷居；非 `0` = 已搬到某處，供 `settle:migrate`）。
    pub settlement: u32,
    /// 目前昇華出的主導生活領域（如 `"Caretaker"`、`"Builder"`；無明顯主導 → `None`）。
    /// 對照 `domain:<D>` 時做字串等值比較，`<D>` 用與此處相同的領域名。
    pub dominant_domain: Option<String>,
    /// 各原料的採集累計數（key = 原料，如 `"cactus"`、`"iron"`；`gather:<r>:N`）。
    /// **無可信來源時給空 map**——照保守鐵律自然評 `false`。
    pub gather_counts: HashMap<String, u32>,
    /// 各成品的合成累計數（key = 成品，如 `"iron_ingot"`；`craft:<x>:N`）。
    /// **無可信來源時給空 map**——同上保守。
    pub craft_counts: HashMap<String, u32>,
}

// ── trigger 解析＋評估（確定性純函式）────────────────────────────────────────

/// 判定某里程碑的 `trigger` 是否已被真實成就快照滿足。
///
/// 支援的文法（大小寫敏感，未列出的一律 `false`）：
/// - `build:<kind>`            → 該類建物已蓋 ≥ 1 座
/// - `invent:any`             → 發明數 ≥ 1
/// - `invent:count:N`         → 發明數 ≥ N
/// - `invent:smelt`           → 發明種類集合含 `smelt`
/// - `teach:N`                → 教會人次 ≥ N
/// - `romance:sweetheart:<名>` → 心上人集合含該名
/// - `romance:wedding:<名>`    → 婚約對象集合含該名
/// - `romance:lovenest`        → 已蓋愛巢
/// - `family:child:N`         → 孩子數 ≥ N
/// - `colony:found`           → 已立村碑 / 開拓聚落
/// - `settle:migrate`         → 已遷居（settlement ≠ 0）
/// - `domain:<D>`             → 主導領域正好是 D
/// - `gather:<r>:N`           → 原料 r 採集累計 ≥ N（無來源＝空 map → false）
/// - `craft:<x>:N`            → 成品 x 合成累計 ≥ N（無來源＝空 map → false）
///
/// **保守鐵律**：任何無法解析 / 數字壞掉 / 未知前綴 → 回 `false`，絕不誤推進。
pub fn trigger_met(trigger: &str, snap: &AchievementSnapshot) -> bool {
    // build:<kind>
    if let Some(kind) = trigger.strip_prefix("build:") {
        return snap.builds_by_kind.get(kind).copied().unwrap_or(0) >= 1;
    }
    // invent:* 三種（先比對確切字面，再落到 count:N）
    if trigger == "invent:any" {
        return snap.invented_count >= 1;
    }
    if trigger == "invent:smelt" {
        return snap.invented_kinds.contains("smelt");
    }
    if let Some(n) = trigger.strip_prefix("invent:count:") {
        return match n.parse::<u32>() {
            Ok(n) => snap.invented_count >= n,
            Err(_) => false, // 壞數字 → 保守不推進
        };
    }
    // teach:N
    if let Some(n) = trigger.strip_prefix("teach:") {
        return match n.parse::<u32>() {
            Ok(n) => snap.taught_count >= n,
            Err(_) => false,
        };
    }
    // romance:*（用 strip_prefix 取整段名字，容得下名字含冒號）
    if let Some(name) = trigger.strip_prefix("romance:sweetheart:") {
        return snap.sweethearts.contains(name);
    }
    if let Some(name) = trigger.strip_prefix("romance:wedding:") {
        return snap.marriages.contains(name);
    }
    if trigger == "romance:lovenest" {
        return snap.has_lovenest;
    }
    // family:child:N
    if let Some(n) = trigger.strip_prefix("family:child:") {
        return match n.parse::<u32>() {
            Ok(n) => snap.child_count >= n,
            Err(_) => false,
        };
    }
    // colony:found
    if trigger == "colony:found" {
        return snap.colony_founded;
    }
    // settle:migrate
    if trigger == "settle:migrate" {
        return snap.settlement != 0;
    }
    // domain:<D>（字串等值）
    if let Some(d) = trigger.strip_prefix("domain:") {
        return snap.dominant_domain.as_deref() == Some(d);
    }
    // gather:<r>:N — 無來源（map 無該項）自然評 0 → false，寧可停著不虛構
    if let Some(rest) = trigger.strip_prefix("gather:") {
        return count_trigger_met(rest, &snap.gather_counts);
    }
    // craft:<x>:N — 同上保守
    if let Some(rest) = trigger.strip_prefix("craft:") {
        return count_trigger_met(rest, &snap.craft_counts);
    }
    // 未知 trigger → false（保守，別誤推進）
    false
}

/// 解析 `<key>:N` 尾段並對照計數 map（gather / craft 共用）。
/// 用 `rsplit_once` 從最後一個冒號切，讓 key 可含冒號（現況不會，但穩健）。
/// 格式不對 / 數字壞掉 / map 無該 key → 一律 `false`（保守）。
fn count_trigger_met(rest: &str, counts: &HashMap<String, u32>) -> bool {
    if let Some((key, n)) = rest.rsplit_once(':') {
        if let Ok(n) = n.parse::<u32>() {
            return counts.get(key).copied().unwrap_or(0) >= n;
        }
    }
    false
}

// ── 逐階推進（純函式）────────────────────────────────────────────────────────

/// 一次成功推進的結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvanceResult {
    /// 推進後的新階（= 原 `current_stage` + 1）。
    pub new_stage: u32,
    /// 這階的內心獨白（`Milestone::feed_line`）——供廣播 / 記動態。
    pub milestone_feed: String,
    /// 是否剛好走到志業的最後一階。
    pub is_final: bool,
    /// 若 `is_final`，附上那句一生的夢；否則 `None`。
    pub final_line: Option<String>,
}

/// 看某居民的下一個里程碑是否已達成，達成就回一步推進結果（純函式，**不改任何狀態**）。
///
/// 語意：取 `current_stage`，找 `stage == current_stage + 1` 的里程碑；
/// 若不存在（已走完全程）→ `None`；若其 `trigger` 未滿足 → `None`；
/// 滿足 → 回 [`AdvanceResult`]（`new_stage = current + 1`，附該階 `feed_line`；
/// 若這階之後再無里程碑則 `is_final = true` 並附 `final_line`）。
///
/// 呼叫端拿到結果後，自行 `progress.set_stage(rid, new_stage)` + 持久化 + 廣播。
pub fn advance_if_ready(
    store: &PursuitStore,
    progress: &PursuitProgress,
    snap: &AchievementSnapshot,
    rid: &str,
) -> Option<AdvanceResult> {
    let pursuit = store.get(rid)?;
    let current = progress.stage_of(rid);
    let target = current + 1;

    // 以 stage 值定位下一階，不假設陣列索引（穩健：即便 stage 有跳號也對）。
    let next = pursuit.milestones.iter().find(|m| m.stage == target)?;

    // 真實成就未達 → 志業原地不動（湧現，不硬推）。
    if !trigger_met(&next.trigger, snap) {
        return None;
    }

    // 這階之後再無更高階的里程碑 → 走到終點了。
    let is_final = !pursuit.milestones.iter().any(|m| m.stage > target);

    Some(AdvanceResult {
        new_stage: target,
        milestone_feed: next.feed_line.clone(),
        is_final,
        final_line: if is_final {
            Some(pursuit.final_line.clone())
        } else {
            None
        },
    })
}

// ── 單元測試（cargo test -- --test-threads=1）─────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一份小志業（三階：invent → teach:3 → build:Pavilion）供推進測試。
    fn sample_pursuit() -> Pursuit {
        Pursuit {
            id: "vox_res_x".into(),
            name: "測試居民".into(),
            title: "測試志業".into(),
            milestones: vec![
                Milestone { stage: 1, name: "第一手藝".into(), trigger: "invent:any".into(), feed_line: "我摸索著做成了。".into() },
                Milestone { stage: 2, name: "傳三人".into(), trigger: "teach:3".into(), feed_line: "教會了三個人。".into() },
                Milestone { stage: 3, name: "涼亭".into(), trigger: "build:Pavilion".into(), feed_line: "涼亭立好了。".into() },
            ],
            final_line: "終於成了。".into(),
        }
    }

    fn store_with(p: Pursuit) -> PursuitStore {
        let mut s = PursuitStore::new();
        s.insert(p);
        s
    }

    // ── trigger 解析＋評估（各類）───────────────────────────────────────────

    #[test]
    fn trigger_build_needs_one() {
        let mut snap = AchievementSnapshot::default();
        assert!(!trigger_met("build:Well", &snap), "沒蓋過→false");
        snap.builds_by_kind.insert("Well".into(), 1);
        assert!(trigger_met("build:Well", &snap), "蓋過一座→true");
        assert!(!trigger_met("build:Pavilion", &snap), "別種建物不算");
    }

    #[test]
    fn trigger_invent_any_and_count() {
        let mut snap = AchievementSnapshot::default();
        assert!(!trigger_met("invent:any", &snap));
        snap.invented_count = 1;
        assert!(trigger_met("invent:any", &snap));
        assert!(!trigger_met("invent:count:10", &snap), "才 1 樣，不到 10");
        snap.invented_count = 10;
        assert!(trigger_met("invent:count:10", &snap), "剛好 10 → 達成");
        assert!(trigger_met("invent:count:5", &snap), "超過門檻也算");
    }

    #[test]
    fn trigger_invent_smelt_by_kind() {
        let mut snap = AchievementSnapshot::default();
        assert!(!trigger_met("invent:smelt", &snap));
        snap.invented_kinds.insert("smelt".into());
        assert!(trigger_met("invent:smelt", &snap), "發明種類含 smelt");
    }

    #[test]
    fn trigger_teach_threshold() {
        let mut snap = AchievementSnapshot::default();
        snap.taught_count = 2;
        assert!(!trigger_met("teach:3", &snap));
        snap.taught_count = 3;
        assert!(trigger_met("teach:3", &snap));
        assert!(trigger_met("teach:1", &snap));
    }

    #[test]
    fn trigger_romance_variants() {
        let mut snap = AchievementSnapshot::default();
        assert!(!trigger_met("romance:sweetheart:蕾雅", &snap));
        snap.sweethearts.insert("蕾雅".into());
        assert!(trigger_met("romance:sweetheart:蕾雅", &snap));
        assert!(!trigger_met("romance:sweetheart:米拉", &snap), "別人不算");

        assert!(!trigger_met("romance:wedding:蕾雅", &snap), "心動 ≠ 成親");
        snap.marriages.insert("蕾雅".into());
        assert!(trigger_met("romance:wedding:蕾雅", &snap));

        assert!(!trigger_met("romance:lovenest", &snap));
        snap.has_lovenest = true;
        assert!(trigger_met("romance:lovenest", &snap));
    }

    #[test]
    fn trigger_family_colony_settle() {
        let mut snap = AchievementSnapshot::default();
        assert!(!trigger_met("family:child:2", &snap));
        snap.child_count = 2;
        assert!(trigger_met("family:child:2", &snap));

        assert!(!trigger_met("colony:found", &snap));
        snap.colony_founded = true;
        assert!(trigger_met("colony:found", &snap));

        assert!(!trigger_met("settle:migrate", &snap), "settlement=0 未遷居");
        snap.settlement = 7;
        assert!(trigger_met("settle:migrate", &snap));
    }

    #[test]
    fn trigger_domain_exact_match() {
        let mut snap = AchievementSnapshot::default();
        assert!(!trigger_met("domain:Caretaker", &snap), "無主導領域→false");
        snap.dominant_domain = Some("Caretaker".into());
        assert!(trigger_met("domain:Caretaker", &snap));
        assert!(!trigger_met("domain:Builder", &snap), "別的領域不算");
    }

    #[test]
    fn trigger_gather_and_craft_with_source() {
        let mut snap = AchievementSnapshot::default();
        // 有來源、達門檻才算
        snap.gather_counts.insert("cactus".into(), 3);
        assert!(trigger_met("gather:cactus:3", &snap));
        assert!(!trigger_met("gather:cactus:4", &snap), "不到 4");
        snap.craft_counts.insert("iron_ingot".into(), 1);
        assert!(trigger_met("craft:iron_ingot:1", &snap));
    }

    #[test]
    fn trigger_gather_craft_no_source_is_false() {
        // 空 map（無可信來源）→ 保守評 false，寧可停著不虛構。
        let snap = AchievementSnapshot::default();
        assert!(!trigger_met("gather:cactus:3", &snap), "無來源→false");
        assert!(!trigger_met("gather:berry:60", &snap), "無來源→false");
        assert!(!trigger_met("craft:iron_ingot:1", &snap), "無來源→false");
    }

    #[test]
    fn trigger_unknown_is_false() {
        let snap = AchievementSnapshot::default();
        assert!(!trigger_met("wat:this", &snap), "未知前綴→false");
        assert!(!trigger_met("", &snap), "空字串→false");
        assert!(!trigger_met("invent:count:xyz", &snap), "壞數字→false");
        assert!(!trigger_met("teach:notanumber", &snap), "壞數字→false");
        assert!(!trigger_met("gather:cactus", &snap), "缺 N→false");
    }

    // ── 逐階推進 ─────────────────────────────────────────────────────────────

    #[test]
    fn advance_step_by_step() {
        let store = store_with(sample_pursuit());
        let mut progress = PursuitProgress::new();
        let mut snap = AchievementSnapshot::default();

        // 起始：什麼都沒達成 → 不推進。
        assert_eq!(advance_if_ready(&store, &progress, &snap, "vox_res_x"), None);

        // 達成第 1 階（invent:any）。
        snap.invented_count = 1;
        let r = advance_if_ready(&store, &progress, &snap, "vox_res_x").expect("應推進到 1");
        assert_eq!(r.new_stage, 1);
        assert_eq!(r.milestone_feed, "我摸索著做成了。");
        assert!(!r.is_final);
        assert_eq!(r.final_line, None);
        assert!(progress.set_stage("vox_res_x", r.new_stage));

        // 在第 1 階、第 2 階條件（teach:3）未滿 → 不推進。
        assert_eq!(advance_if_ready(&store, &progress, &snap, "vox_res_x"), None);

        // 達成第 2 階。
        snap.taught_count = 3;
        let r = advance_if_ready(&store, &progress, &snap, "vox_res_x").expect("應推進到 2");
        assert_eq!(r.new_stage, 2);
        assert!(!r.is_final, "還有第 3 階，非終點");
        assert!(progress.set_stage("vox_res_x", r.new_stage));

        // 達成第 3 階（最後一階）→ is_final + final_line。
        snap.builds_by_kind.insert("Pavilion".into(), 1);
        let r = advance_if_ready(&store, &progress, &snap, "vox_res_x").expect("應推進到 3");
        assert_eq!(r.new_stage, 3);
        assert!(r.is_final, "第 3 階是最後一階");
        assert_eq!(r.final_line.as_deref(), Some("終於成了。"));
        assert!(progress.set_stage("vox_res_x", r.new_stage));

        // 已走完全程 → 再無下一階 → None。
        assert_eq!(advance_if_ready(&store, &progress, &snap, "vox_res_x"), None);
    }

    #[test]
    fn advance_unknown_resident_is_none() {
        let store = store_with(sample_pursuit());
        let progress = PursuitProgress::new();
        let snap = AchievementSnapshot::default();
        assert_eq!(advance_if_ready(&store, &progress, &snap, "沒這人"), None);
    }

    #[test]
    fn advance_does_not_skip_stages() {
        // 即使晚階條件已滿，只要當前下一階未達，就不跳階。
        let store = store_with(sample_pursuit());
        let progress = PursuitProgress::new(); // current=0，下一階=1（invent:any）
        let mut snap = AchievementSnapshot::default();
        // 第 3 階條件滿了，但第 1 階（invent）沒滿 → 不推進。
        snap.builds_by_kind.insert("Pavilion".into(), 1);
        snap.taught_count = 5;
        assert_eq!(advance_if_ready(&store, &progress, &snap, "vox_res_x"), None);
    }

    #[test]
    fn advance_gather_no_source_stays_put() {
        // 以 gather 為第一階的志業，無來源 → 永遠停在原地（保守）。
        let p = Pursuit {
            id: "g".into(), name: "g".into(), title: "g".into(),
            milestones: vec![Milestone {
                stage: 1, name: "採集".into(), trigger: "gather:berry:60".into(), feed_line: "攢了一筐莓果。".into(),
            }],
            final_line: "成了。".into(),
        };
        let store = store_with(p);
        let progress = PursuitProgress::new();
        let snap = AchievementSnapshot::default(); // gather_counts 空
        assert_eq!(advance_if_ready(&store, &progress, &snap, "g"), None);
    }

    // ── 志業表 IO：缺檔回空、壞行略過 ────────────────────────────────────────

    #[test]
    fn load_missing_file_returns_empty() {
        let store = load_pursuits_from("data/definitely_not_a_real_file_xyz.jsonl");
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn parse_skips_bad_and_blank_lines() {
        // 一行合法（含 pursuit_title / final_dream_line 欄位改名）、一行壞、一行空。
        let content = r#"{"id":"vox_res_0","name":"露娜","pursuit_title":"村村相連","milestones":[{"stage":1,"name":"第一手藝","trigger":"invent:any","feed_line":"做成了。"}],"final_dream_line":"就算我不在，這些本事還在。"}
{ this is not json }

{"id":"vox_res_1","name":"諾娃","pursuit_title":"發明之母","milestones":[],"final_dream_line":"傳下去。"}"#;
        let store = parse_pursuits(content);
        assert_eq!(store.len(), 2, "兩行合法、壞行與空行略過");
        let luna = store.get("vox_res_0").expect("露娜在");
        assert_eq!(luna.name, "露娜");
        assert_eq!(luna.title, "村村相連", "pursuit_title 改名有生效");
        assert_eq!(luna.final_line, "就算我不在，這些本事還在。", "final_dream_line 改名有生效");
        assert_eq!(luna.milestones.len(), 1);
        assert_eq!(luna.milestones[0].trigger, "invent:any");
    }

    // ── 進度 round-trip ─────────────────────────────────────────────────────

    #[test]
    fn progress_from_entries_last_wins() {
        // append-only：同一居民多筆，最後一筆（後寫）勝出。
        let entries = vec![
            ProgressEntry { resident: "露娜".into(), stage: 1 },
            ProgressEntry { resident: "諾娃".into(), stage: 2 },
            ProgressEntry { resident: "露娜".into(), stage: 4 },
        ];
        let p = PursuitProgress::from_entries(entries);
        assert_eq!(p.stage_of("露娜"), 4, "後寫的 4 勝出");
        assert_eq!(p.stage_of("諾娃"), 2);
        assert_eq!(p.stage_of("沒登記"), 0, "未登記回 0");
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn progress_set_stage_only_advances() {
        let mut p = PursuitProgress::new();
        assert!(p.set_stage("露娜", 2), "0→2 前進");
        assert!(!p.set_stage("露娜", 1), "2→1 不倒退");
        assert!(!p.set_stage("露娜", 2), "同階不算前進");
        assert!(p.set_stage("露娜", 3), "2→3 前進");
        assert_eq!(p.stage_of("露娜"), 3);
    }

    #[test]
    fn progress_file_round_trip() {
        // 寫到臨時檔再讀回，驗證持久化格式來回一致（--test-threads=1 下不互擾）。
        let path = std::env::temp_dir().join(format!(
            "butfun_pursuit_progress_test_{}.jsonl",
            std::process::id()
        ));
        let path = path.to_str().unwrap();
        let _ = std::fs::remove_file(path); // 清乾淨，確保 append 從空檔起

        let mut p = PursuitProgress::new();
        p.set_stage("露娜", 3);
        p.set_stage("諾娃", 1);
        save_progress_to(path, &p);

        let restored = load_progress_from(path);
        assert_eq!(restored.stage_of("露娜"), 3);
        assert_eq!(restored.stage_of("諾娃"), 1);
        assert_eq!(restored.len(), 2);

        // append-only：再存一次更新後的快照，last-wins 折疊掉舊筆。
        let mut p2 = PursuitProgress::new();
        p2.set_stage("露娜", 5);
        p2.set_stage("諾娃", 1);
        save_progress_to(path, &p2);
        let restored2 = load_progress_from(path);
        assert_eq!(restored2.stage_of("露娜"), 5, "後寫的 5 勝出");
        assert_eq!(restored2.stage_of("諾娃"), 1);
        assert_eq!(restored2.len(), 2);

        let _ = std::fs::remove_file(path);
    }
}
