//! 乙太方界·玩家里程碑 v1（成就徽章，ROADMAP 724）。
//!
//! **設計依據**：`docs/PLAN_ETHERVOX.md`「玩家遊玩」節——
//! 「進度/目標：療癒循環（採集→合成→蓋造→與居民同住）給人想一直玩下去的理由」。
//! 這條軸線至今從未被實作：玩家的合成/交易/建造/耕種一次次成功，卻從沒有任何
//! 「回頭看看自己走了多遠」的管道——不像居民有技能簿（719）、交情網（708）能查閱，
//! 玩家自己的成長軌跡完全沒有被看見。
//!
//! **換維度**：671~723 疊的是「居民↔居民」到訪劇本（問候/八卦/互助/拌嘴/傳授/交易），
//! 本切片換到全新角度——**玩家自己的旅程**，把療癒循環裡每個「第一次」變成一枚
//! 可回頭翻閱、達成當下有小小慶祝感的徽章。
//!
//! 純邏輯層（`MilestoneStore` 同步資料結構，無鎖/IO/async），IO 與觸發點在 `voxel_ws.rs`。
//! 持久化格式：`data/voxel_milestones.jsonl`（每行一筆 `(player, id)`，append-only、
//! `unlock` 本身冪等——已達成的里程碑重複呼叫不會重複寫檔，因為呼叫端只在 `unlock` 回
//! `true`（本次才第一次達成）時才 append）。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// 持久化路徑（`data/` 已 gitignore）。
const MILESTONES_PATH: &str = "data/voxel_milestones.jsonl";

/// 一枚里程碑的靜態定義。
pub struct MilestoneDef {
    /// 穩定 id（wire 契約，前後端與持久化皆用此鍵）。
    pub id: &'static str,
    /// 繁中名稱。
    pub name_zh: &'static str,
    /// 繁中說明（達成條件白話文）。
    pub desc_zh: &'static str,
    /// emoji 圖示。
    pub icon: &'static str,
}

/// 全部里程碑，依「採集→建造→合成→耕種→贈禮→交易→熟識→安眠」療癒循環順序排列。
/// 新增里程碑只准往後 append（順序即是玩家旅程的敘事順序，別中途插隊重排）。
pub const MILESTONES: &[MilestoneDef] = &[
    MilestoneDef { id: "first_mine",  name_zh: "初次採集", desc_zh: "挖出人生第一塊方塊", icon: "⛏️" },
    MilestoneDef { id: "first_place", name_zh: "初次建造", desc_zh: "在世界放下第一塊方塊", icon: "🧱" },
    MilestoneDef { id: "first_craft", name_zh: "初次合成", desc_zh: "合成出第一件成品", icon: "🔨" },
    MilestoneDef { id: "first_farm",  name_zh: "初次耕種", desc_zh: "種下人生第一顆種子", icon: "🌱" },
    MilestoneDef { id: "first_gift",  name_zh: "初次贈禮", desc_zh: "送給居民第一份禮物", icon: "🎁" },
    MilestoneDef { id: "first_trade", name_zh: "初次交易", desc_zh: "與居民完成第一筆以物易物", icon: "⇌" },
    MilestoneDef { id: "first_bond",  name_zh: "初次熟識", desc_zh: "和一位居民混熟了", icon: "💛" },
    MilestoneDef { id: "first_sleep", name_zh: "初次安眠", desc_zh: "在床上一覺睡到天亮", icon: "🛌" },
];

/// 查表確認是否為已知里程碑 id（守 store 資料乾淨，未知 id 不寫入）。
pub fn is_known(id: &str) -> bool {
    MILESTONES.iter().any(|m| m.id == id)
}

// ── 持久化格式 ────────────────────────────────────────────────────────────────

/// 一筆持久化記錄：某玩家達成某項里程碑。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MilestoneEntry {
    pub player: String,
    pub id: String,
}

// ── 里程碑 Store ─────────────────────────────────────────────────────────────

/// 玩家里程碑帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct MilestoneStore {
    /// key = 玩家名 → 已達成的里程碑 id 集合。
    earned: HashMap<String, HashSet<String>>,
}

impl MilestoneStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。未知 id（例如舊版留下的壞資料）安全略過。
    pub fn from_entries(entries: impl IntoIterator<Item = MilestoneEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            if is_known(&e.id) {
                s.earned.entry(e.player).or_default().insert(e.id);
            }
        }
        s
    }

    /// 標記玩家達成一項里程碑。回傳 `true` 代表「這次才第一次達成」——
    /// 呼叫端只在回 `true` 時才 append 持久化 + 廣播慶祝；已達成過再呼叫安全回 `false`，冪等。
    /// 未知 id 一律不寫入、回 `false`（防呆，不污染 store）。
    pub fn unlock(&mut self, player: &str, id: &str) -> bool {
        if !is_known(id) {
            return false;
        }
        self.earned.entry(player.to_string()).or_default().insert(id.to_string())
    }

    /// 玩家是否已達成指定里程碑。
    pub fn has(&self, player: &str, id: &str) -> bool {
        self.earned.get(player).is_some_and(|s| s.contains(id))
    }

    /// 玩家已達成的里程碑 id 清單（不保證順序，呼叫端可依 `MILESTONES` 順序重排顯示）。
    pub fn earned_ids(&self, player: &str) -> Vec<String> {
        self.earned.get(player).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }
}

// ── jsonl 持久化（append-only，比照 voxel_invent::append_invented_skill 慣例）──────

/// Append 一筆里程碑記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_milestone(entry: &MilestoneEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(MILESTONES_PATH, &line);
    }
}

/// 載回所有里程碑記錄（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_milestones() -> Vec<MilestoneEntry> {
    let content = match std::fs::read_to_string(MILESTONES_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                None
            } else {
                serde_json::from_str::<MilestoneEntry>(l).ok()
            }
        })
        .collect()
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
        Err(e) => tracing::warn!("無法寫入里程碑記錄 {path}: {e}"),
    }
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlock_first_time_true_second_time_false() {
        let mut s = MilestoneStore::new();
        assert!(s.unlock("nova", "first_mine"), "第一次達成應回 true");
        assert!(!s.unlock("nova", "first_mine"), "重複達成應冪等回 false");
    }

    #[test]
    fn unknown_id_never_unlocks() {
        let mut s = MilestoneStore::new();
        assert!(!s.unlock("nova", "not_a_real_id"));
        assert!(!s.has("nova", "not_a_real_id"));
        assert!(s.earned_ids("nova").is_empty());
    }

    #[test]
    fn players_independent() {
        let mut s = MilestoneStore::new();
        s.unlock("alice", "first_mine");
        assert!(s.has("alice", "first_mine"));
        assert!(!s.has("bob", "first_mine"));
    }

    #[test]
    fn has_false_before_unlock() {
        let s = MilestoneStore::new();
        assert!(!s.has("nova", "first_craft"));
    }

    #[test]
    fn earned_ids_reflects_unlocks() {
        let mut s = MilestoneStore::new();
        s.unlock("nova", "first_mine");
        s.unlock("nova", "first_craft");
        let mut ids = s.earned_ids("nova");
        ids.sort();
        assert_eq!(ids, vec!["first_craft".to_string(), "first_mine".to_string()]);
    }

    #[test]
    fn from_entries_rebuilds_state() {
        let entries = vec![
            MilestoneEntry { player: "nova".into(), id: "first_mine".into() },
            MilestoneEntry { player: "nova".into(), id: "first_place".into() },
            MilestoneEntry { player: "luna".into(), id: "first_farm".into() },
        ];
        let s = MilestoneStore::from_entries(entries);
        assert!(s.has("nova", "first_mine"));
        assert!(s.has("nova", "first_place"));
        assert!(s.has("luna", "first_farm"));
        assert!(!s.has("luna", "first_mine"));
    }

    #[test]
    fn from_entries_skips_unknown_ids() {
        let entries = vec![MilestoneEntry { player: "nova".into(), id: "bogus".into() }];
        let s = MilestoneStore::from_entries(entries);
        assert!(s.earned_ids("nova").is_empty());
    }

    #[test]
    fn is_known_matches_static_list() {
        assert!(is_known("first_mine"));
        assert!(is_known("first_sleep"));
        assert!(!is_known(""));
        assert!(!is_known("first_win_lottery"));
    }

    #[test]
    fn all_milestone_ids_unique() {
        let mut ids: Vec<&str> = MILESTONES.iter().map(|m| m.id).collect();
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), before, "里程碑 id 不應重複");
    }

    #[test]
    fn all_milestones_have_nonempty_fields() {
        for m in MILESTONES {
            assert!(!m.id.is_empty());
            assert!(!m.name_zh.is_empty());
            assert!(!m.desc_zh.is_empty());
            assert!(!m.icon.is_empty());
        }
    }

    #[test]
    fn empty_store_earned_ids_empty() {
        let s = MilestoneStore::new();
        assert!(s.earned_ids("nobody").is_empty());
    }
}
