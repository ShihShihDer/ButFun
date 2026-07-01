//! 乙太方界·居民情誼（ROADMAP 672）——居民彼此拜訪累積情誼，持久化跨重啟。
//!
//! **設計依據**：`docs/PLAN_ETHERVOX.md` §4 居民↔居民關係
//! ——居民記得彼此、有關係（熟識/幫過）→ 友誼自然湧現 = 小社會。
//!
//! 每一次跨域探訪（ROADMAP 671）到達目的地時呼叫 `record_visit(a, b)`；
//! 累積足夠次數後情誼升級（陌生→相識→老朋友），升級時呼叫端可廣播 Feed。
//! 問候語依情誼層級不同（`arrival_line`），讓玩家親眼見到「它們處出交情了」。
//!
//! 純邏輯層（無 IO、無鎖、無 async），IO 在 `voxel_ws.rs`。
//! 持久化格式：`data/voxel_bonds.jsonl`（每行一對 `BondEntry`，append-only 快照最後一行）。

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

// ── 常數 ──────────────────────────────────────────────────────────────────────

/// 拜訪 n 次後升到相識。
pub const ACQUAINTANCE_VISITS: u32 = 3;
/// 拜訪 n 次後升到老朋友。
pub const FRIEND_VISITS: u32 = 8;
/// 每對居民拜訪次數上限（防超長壽世界無限累加）。
pub const VISIT_CAP: u32 = 200;

// ── 情誼層級 ──────────────────────────────────────────────────────────────────

/// 兩位居民之間的情誼層級。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BondTier {
    /// 陌生——第一次打照面。
    Stranger,
    /// 相識——互訪過幾次，叫得出名字來。
    Acquaintance,
    /// 老朋友——常相往來，一眼就認出對方。
    Friend,
}

impl BondTier {
    fn from_visits(visits: u32) -> Self {
        if visits >= FRIEND_VISITS {
            BondTier::Friend
        } else if visits >= ACQUAINTANCE_VISITS {
            BondTier::Acquaintance
        } else {
            BondTier::Stranger
        }
    }
}

// ── 持久化格式 ────────────────────────────────────────────────────────────────

/// 一筆持久化記錄：一對居民的拜訪計數。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BondEntry {
    /// 較小的居民 id（排序鍵，確保 (a,b) == (b,a)）。
    pub id_a: String,
    /// 較大的居民 id。
    pub id_b: String,
    /// 累積拜訪次數（A→B 或 B→A 皆算）。
    pub visits: u32,
}

// ── 情誼帳本 ──────────────────────────────────────────────────────────────────

/// 居民情誼帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct ResidentBonds {
    /// key = (min_id, max_id) → 拜訪次數。
    counts: HashMap<(String, String), u32>,
}

impl ResidentBonds {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。
    pub fn from_entries(entries: impl IntoIterator<Item = BondEntry>) -> Self {
        let mut b = Self::new();
        for e in entries {
            let key = bond_key(&e.id_a, &e.id_b);
            b.counts.insert(key, e.visits.min(VISIT_CAP));
        }
        b
    }

    /// 記錄一次 `a` 拜訪 `b`（或 `b` 拜訪 `a`，對稱）。
    /// 回傳 `(新層級, 是否在此次升級)`——呼叫端依此決定是否廣播。
    /// `a == b` 防呆：自己不和自己累積。
    pub fn record_visit(&mut self, a: &str, b: &str) -> (BondTier, bool) {
        if a == b {
            return (BondTier::Stranger, false);
        }
        let old_visits = self.visit_count(a, b);
        let old_tier = BondTier::from_visits(old_visits);
        let new_visits = (old_visits + 1).min(VISIT_CAP);
        let new_tier = BondTier::from_visits(new_visits);
        let key = bond_key(a, b);
        self.counts.insert(key, new_visits);
        (new_tier, new_tier > old_tier)
    }

    /// 查詢情誼層級（不改狀態）。
    pub fn tier_of(&self, a: &str, b: &str) -> BondTier {
        BondTier::from_visits(self.visit_count(a, b))
    }

    /// 查詢拜訪次數（不改狀態）。
    pub fn visit_count(&self, a: &str, b: &str) -> u32 {
        let key = bond_key(a, b);
        self.counts.get(&key).copied().unwrap_or(0)
    }

    /// 轉成持久化記錄（快照，寫入 jsonl 用）。
    pub fn to_entries(&self) -> Vec<BondEntry> {
        self.counts
            .iter()
            .map(|((a, b), &visits)| BondEntry {
                id_a: a.clone(),
                id_b: b.clone(),
                visits,
            })
            .collect()
    }

    /// 計算某居民與指定居民列表之間的 Friend / Acquaintance 情誼數量。
    /// 用於心情計算（居民↔居民關係反映情緒狀態）。純計數、不修改資料。
    pub fn bond_counts_for(&self, resident: &str, all_ids: &[&str]) -> (usize, usize) {
        let mut friend = 0usize;
        let mut acquaintance = 0usize;
        for &other in all_ids {
            if other == resident {
                continue;
            }
            match self.tier_of(resident, other) {
                BondTier::Friend => friend += 1,
                BondTier::Acquaintance => acquaintance += 1,
                BondTier::Stranger => {}
            }
        }
        (friend, acquaintance)
    }

    /// 清除某位居民的所有情誼記錄（居民退休時用）。
    pub fn forget(&mut self, id: &str) {
        self.counts.retain(|(a, b), _| a != id && b != id);
    }
}

/// 情誼層級 → 穩定字串鍵（供 API/前端序列化用，與顯示文案分開，避免文案改動牽動協議）。
pub fn tier_key(tier: BondTier) -> &'static str {
    match tier {
        BondTier::Stranger => "stranger",
        BondTier::Acquaintance => "acquaintance",
        BondTier::Friend => "friend",
    }
}

/// 對稱鍵：確保 (a,b) == (b,a)。
fn bond_key(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

// ── 問候語（依情誼層級）───────────────────────────────────────────────────────

/// 訪客到達時的問候台詞（依情誼與被訪者名字）。確定性，零 LLM。
pub fn arrival_line(tier: BondTier, host: &str, visitor: &str, pick: usize) -> String {
    match tier {
        BondTier::Stranger => {
            let pool = [
                format!("{host}，你好！我是{visitor}，來打個招呼！"),
                format!("打擾了，{host}！我是{visitor}，路過來拜訪。"),
                format!("{host}，初次登門，請多指教！"),
            ];
            pool[pick % pool.len()].clone()
        }
        BondTier::Acquaintance => {
            let pool = [
                format!("{host}！又是我，{visitor}～"),
                format!("{host}，我來看看你！感覺好久沒見。"),
                format!("嘿，{host}！我想起你就過來了。"),
            ];
            pool[pick % pool.len()].clone()
        }
        BondTier::Friend => {
            let pool = [
                format!("🤝 {host}！我就知道你在這！"),
                format!("🤝 {host}，老朋友！最近過得怎麼樣？"),
                format!("🤝 {host}！每次來這裡都覺得特別自在。"),
            ];
            pool[pick % pool.len()].clone()
        }
    }
}

/// 訪客離開時的告別台詞（依情誼）。確定性，零 LLM。
pub fn departure_line(tier: BondTier, visitor: &str, pick: usize) -> String {
    match tier {
        BondTier::Stranger => {
            let pool = [
                format!("{visitor}：時間到了，先回去了，再見！"),
                format!("{visitor}：感謝招待，改天再來！"),
            ];
            pool[pick % pool.len()].clone()
        }
        BondTier::Acquaintance => {
            let pool = [
                format!("{visitor}：告辭啦，下次再見！"),
                format!("{visitor}：先回去了，保重喔！"),
            ];
            pool[pick % pool.len()].clone()
        }
        BondTier::Friend => {
            let pool = [
                format!("{visitor}：🤝 先走啦，老朋友！下次再聊！"),
                format!("{visitor}：🤝 回去了，你也保重！"),
            ];
            pool[pick % pool.len()].clone()
        }
    }
}

/// 升級里程碑廣播台詞（Feed 用）。
pub fn tier_up_line(tier: BondTier, a: &str, b: &str) -> String {
    match tier {
        BondTier::Stranger => format!("{a} 和 {b} 第一次見面了"),
        BondTier::Acquaintance => format!("{a} 和 {b} 成了相識！彼此走動了幾次，漸漸熟悉起來。"),
        BondTier::Friend => format!("🤝 {a} 和 {b} 成了老朋友！"),
    }
}

/// 情誼升級時寫進訪客長期記憶的摘要文字（ROADMAP 673 社交足跡）。
/// 確定性、純函式、零 LLM；呼叫端把回傳字串餵進 `VoxelMemory::add_memory`。
/// `Stranger` 回空字串（初次見面太平凡，不污染記憶庫），呼叫端需跳過。
pub fn bond_social_memory(host_name: &str, tier: BondTier) -> String {
    match tier {
        BondTier::Stranger => String::new(),
        BondTier::Acquaintance => format!("和{}走動了幾次，我們漸漸相識了", host_name),
        BondTier::Friend => format!("🤝 和{}成了老朋友，每次見面都覺得特別自在", host_name),
    }
}

// ── 持久化 IO（只有函式，鎖在 voxel_ws.rs）──────────────────────────────────

const BONDS_FILE: &str = "data/voxel_bonds.jsonl";

/// 從 `data/voxel_bonds.jsonl` 讀取所有記錄（檔案不存在回空 Vec）。
pub fn load_bonds() -> Vec<BondEntry> {
    let content = match std::fs::read_to_string(BONDS_FILE) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// 把整份帳本快照 append 一行到 `data/voxel_bonds.jsonl`。
/// 使用 append-only 策略：重啟時讀最後快照，老記錄自然被遮蓋。
/// 但為簡單起見，每次升級時寫入整份，防 jsonl 無限長大——
/// 實際上居民情誼只有 4×3/2=6 對，快照極小。
pub fn save_bonds(bonds: &ResidentBonds) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(BONDS_FILE)
    {
        for entry in bonds.to_entries() {
            if let Ok(line) = serde_json::to_string(&entry) {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bonds() -> ResidentBonds {
        ResidentBonds::new()
    }

    #[test]
    fn new_pair_is_stranger() {
        let b = make_bonds();
        assert_eq!(b.tier_of("露娜", "諾娃"), BondTier::Stranger);
        assert_eq!(b.visit_count("露娜", "諾娃"), 0);
    }

    #[test]
    fn tier_key_stable_strings() {
        assert_eq!(tier_key(BondTier::Stranger), "stranger");
        assert_eq!(tier_key(BondTier::Acquaintance), "acquaintance");
        assert_eq!(tier_key(BondTier::Friend), "friend");
    }

    #[test]
    fn symmetry_a_b_equals_b_a() {
        let mut b = make_bonds();
        b.record_visit("露娜", "諾娃");
        assert_eq!(b.visit_count("露娜", "諾娃"), b.visit_count("諾娃", "露娜"));
        assert_eq!(b.tier_of("露娜", "諾娃"), b.tier_of("諾娃", "露娜"));
    }

    #[test]
    fn accumulates_to_acquaintance() {
        let mut b = make_bonds();
        for _ in 0..ACQUAINTANCE_VISITS {
            b.record_visit("露娜", "諾娃");
        }
        assert_eq!(b.tier_of("露娜", "諾娃"), BondTier::Acquaintance);
    }

    #[test]
    fn accumulates_to_friend() {
        let mut b = make_bonds();
        for _ in 0..FRIEND_VISITS {
            b.record_visit("露娜", "諾娃");
        }
        assert_eq!(b.tier_of("露娜", "諾娃"), BondTier::Friend);
    }

    #[test]
    fn record_visit_returns_upgraded_flag() {
        let mut b = make_bonds();
        // 前 ACQUAINTANCE_VISITS-1 次不升級
        for _ in 0..(ACQUAINTANCE_VISITS - 1) {
            let (_, upgraded) = b.record_visit("露娜", "諾娃");
            assert!(!upgraded, "未到門檻不應升級");
        }
        // 第 ACQUAINTANCE_VISITS 次升級
        let (tier, upgraded) = b.record_visit("露娜", "諾娃");
        assert!(upgraded, "到達門檻應升級");
        assert_eq!(tier, BondTier::Acquaintance);
    }

    #[test]
    fn record_visit_returns_friend_upgrade() {
        let mut b = make_bonds();
        for _ in 0..(FRIEND_VISITS - 1) {
            b.record_visit("露娜", "諾娃");
        }
        let (tier, upgraded) = b.record_visit("露娜", "諾娃");
        assert!(upgraded, "到達 Friend 門檻應升級");
        assert_eq!(tier, BondTier::Friend);
    }

    #[test]
    fn self_visit_no_effect() {
        let mut b = make_bonds();
        let (tier, upgraded) = b.record_visit("露娜", "露娜");
        assert_eq!(tier, BondTier::Stranger, "自己拜訪自己不應累積");
        assert!(!upgraded);
        assert_eq!(b.visit_count("露娜", "露娜"), 0);
    }

    #[test]
    fn visit_cap_enforced() {
        let mut b = make_bonds();
        for _ in 0..VISIT_CAP + 10 {
            b.record_visit("露娜", "諾娃");
        }
        assert_eq!(b.visit_count("露娜", "諾娃"), VISIT_CAP, "次數應夾在上限");
    }

    #[test]
    fn forget_clears_entries() {
        let mut b = make_bonds();
        b.record_visit("露娜", "諾娃");
        b.record_visit("露娜", "賽勒");
        b.forget("露娜");
        assert_eq!(b.visit_count("露娜", "諾娃"), 0, "forget 後 露娜-諾娃 清零");
        assert_eq!(b.visit_count("露娜", "賽勒"), 0, "forget 後 露娜-賽勒 清零");
        // 跟露娜無關的對不受影響
        b.record_visit("諾娃", "賽勒");
        b.forget("露娜"); // 再 forget 不影響無關對
        assert_eq!(b.visit_count("諾娃", "賽勒"), 1);
    }

    #[test]
    fn from_entries_restores_state() {
        let entries = vec![
            BondEntry { id_a: "露娜".into(), id_b: "諾娃".into(), visits: FRIEND_VISITS },
            BondEntry { id_a: "賽勒".into(), id_b: "奧瑞".into(), visits: 1 },
        ];
        let b = ResidentBonds::from_entries(entries);
        assert_eq!(b.tier_of("露娜", "諾娃"), BondTier::Friend);
        assert_eq!(b.tier_of("賽勒", "奧瑞"), BondTier::Stranger);
    }

    #[test]
    fn from_entries_caps_excessive_visits() {
        let entries = vec![BondEntry {
            id_a: "露娜".into(),
            id_b: "諾娃".into(),
            visits: VISIT_CAP + 999,
        }];
        let b = ResidentBonds::from_entries(entries);
        assert_eq!(b.visit_count("露娜", "諾娃"), VISIT_CAP);
    }

    #[test]
    fn tier_ordering_stranger_lt_acquaintance_lt_friend() {
        assert!(BondTier::Stranger < BondTier::Acquaintance);
        assert!(BondTier::Acquaintance < BondTier::Friend);
    }

    #[test]
    fn arrival_line_non_empty_all_tiers() {
        for tier in [BondTier::Stranger, BondTier::Acquaintance, BondTier::Friend] {
            let line = arrival_line(tier, "諾娃", "露娜", 0);
            assert!(!line.is_empty(), "tier {tier:?} 問候語不可為空");
        }
    }

    #[test]
    fn arrival_line_friend_has_handshake_emoji() {
        let line = arrival_line(BondTier::Friend, "諾娃", "露娜", 0);
        assert!(line.contains("🤝"), "老朋友問候語應含 🤝");
    }

    #[test]
    fn departure_line_non_empty_all_tiers() {
        for tier in [BondTier::Stranger, BondTier::Acquaintance, BondTier::Friend] {
            let line = departure_line(tier, "露娜", 0);
            assert!(!line.is_empty(), "tier {tier:?} 告別語不可為空");
        }
    }

    #[test]
    fn tier_up_line_non_empty_all_tiers() {
        for tier in [BondTier::Stranger, BondTier::Acquaintance, BondTier::Friend] {
            let line = tier_up_line(tier, "露娜", "諾娃");
            assert!(!line.is_empty(), "tier_up_line tier {tier:?} 不可為空");
        }
    }

    #[test]
    fn tier_up_friend_line_has_handshake() {
        let line = tier_up_line(BondTier::Friend, "露娜", "諾娃");
        assert!(line.contains("🤝"), "升格老朋友廣播應含 🤝");
    }

    #[test]
    fn constants_are_monotone() {
        assert!(ACQUAINTANCE_VISITS < FRIEND_VISITS, "相識門檻應小於友人門檻");
        assert!(FRIEND_VISITS < VISIT_CAP, "友人門檻應小於上限");
    }

    // ── ROADMAP 673：bond_social_memory ────────────────────────────────────────

    #[test]
    fn bond_social_memory_stranger_is_empty() {
        assert!(bond_social_memory("諾娃", BondTier::Stranger).is_empty(), "初次見面不加記憶");
    }

    #[test]
    fn bond_social_memory_acquaintance_contains_host_and_keyword() {
        let s = bond_social_memory("諾娃", BondTier::Acquaintance);
        assert!(!s.is_empty());
        assert!(s.contains("諾娃"), "應含被訪者名字");
        assert!(s.contains("相識"), "應含「相識」關鍵字，讓日記能分類");
    }

    #[test]
    fn bond_social_memory_friend_contains_host_and_keyword() {
        let s = bond_social_memory("賽勒", BondTier::Friend);
        assert!(!s.is_empty());
        assert!(s.contains("賽勒"), "應含被訪者名字");
        assert!(s.contains("老朋友"), "應含「老朋友」關鍵字，讓日記能分類");
    }
}
