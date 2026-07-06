//! 乙太方界·村莊集體里程碑 v1（自主提案切片，ROADMAP 856）。
//!
//! **真缺口**：`voxel_milestones.rs`（724）的成就徽章全數綁定**單一玩家**——`unlock(player, id)`
//! 只通知那位玩家自己的連線（`try_unlock_milestone` 走 per-connection `out_tx`）。854 讓居民開始
//! 為你的建造作品取名字、累積成一座座「被記住的地標」，但無論累積到幾座，世界本身從沒有過
//! 任何反應——沒有一個屬於**村莊整體**、**大家一起達成**的里程碑。654 讓村莊有了實體的街廓
//! （廣場/道路/地塊），卻從沒有一刻讓玩家感覺到「這座村莊正在一起長大」。
//!
//! **做法**：用 854 已在追蹤的「被居民命名的地標數量」當村莊成長最直觀、零新開銷的指標——
//! 每累積到一個新地標，只是靜靜地 +1；累積到門檻（3/8/15 座）時，讓**全體居民**一起停下手邊
//! 的事、冒出歡呼泡泡，動態牆同時播一則「全村」等級的公告。與既有個人里程碑刻意區隔：
//! 個人里程碑「你做了什麼」只講給你一個人聽，本模組「村莊做到了什麼」講給所有人聽。
//!
//! 純邏輯層：零 IO、零鎖、零 LLM、零 async；確定性純函式 + 純同步 `VillageMilestoneStore`。
//! 鎖／廣播／持久化 IO 都在 `voxel_ws.rs`（比照 `voxel_milestones` 慣例，短鎖即釋、IO 在鎖外）。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// 持久化路徑（`data/` 已 gitignore；全域一份，不分玩家）。
const VILLAGE_MILESTONES_PATH: &str = "data/voxel_village_milestones.jsonl";

/// 一個村莊集體里程碑門檻的靜態定義。
pub struct TierDef {
    /// 穩定 id（wire 契約 + 持久化鍵）。
    pub id: &'static str,
    /// 達成門檻——累計命名地標數（`>=` 即達成）。
    pub threshold: usize,
    /// 繁中稱號（達成時村莊晉升成什麼）。
    pub name_zh: &'static str,
}

/// 全部門檻，依累計地標數由小到大排列（新增門檻只准往後 append、數字只准遞增）。
pub const TIERS: &[TierDef] = &[
    TierDef { id: "village_hamlet", threshold: 3, name_zh: "小小聚落" },
    TierDef { id: "village_town", threshold: 8, name_zh: "像模像樣的村莊" },
    TierDef { id: "village_city", threshold: 15, name_zh: "熱鬧的市鎮" },
];

/// 查表確認是否為已知門檻 id（守 store 資料乾淨，未知 id 不寫入）。
pub fn is_known(id: &str) -> bool {
    TIERS.iter().any(|t| t.id == id)
}

/// 純函式：地標數 `count` 底下，第一個「已達門檻、但尚未解鎖過」的門檻定義（沒有就回 `None`）。
/// 門檻間距（3/8/15）皆大於單次命名只會 +1 的幅度，正常情況一次呼叫至多命中一個門檻；
/// 就算 `count` 一口氣跳過不只一個門檻（例如追加歷史資料），也只回傳最早那個尚未解鎖的，
/// 呼叫端下次命名再檢查一次即可依序補上，不會漏。
pub fn check_new_tier(count: usize, unlocked: &HashSet<String>) -> Option<&'static TierDef> {
    TIERS.iter().find(|t| count >= t.threshold && !unlocked.contains(t.id))
}

/// 全村慶祝時，居民頭頂輪替的歡呼泡泡（刻意不含任何個人稱謂，適用於任何一位居民）。
pub fn celebrate_say_line(pick: usize) -> &'static str {
    const LINES: &[&str] = &[
        "大家一起把這裡變成家了！",
        "感覺這座村子越來越熱鬧了呢！",
        "這裡越來越有村莊的樣子了！",
        "真高興能跟大家一起住在這裡！",
    ];
    LINES[pick % LINES.len()]
}

/// 全村里程碑動態牆播報句（含達成稱號與當下地標數）。
pub fn celebrate_feed_line(tier_name_zh: &str, count: usize) -> String {
    format!("🎉 村莊迎來第 {count} 座被居民記住的地標——正式成為「{tier_name_zh}」！")
}

// ── 持久化格式 ────────────────────────────────────────────────────────────────

/// 一筆持久化記錄：村莊達成了某個集體門檻（全域，不分玩家）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VillageMilestoneEntry {
    pub id: String,
}

// ── 村莊里程碑 Store ─────────────────────────────────────────────────────────

/// 村莊集體里程碑帳本（純同步資料結構，由呼叫端包進 `RwLock`）——全域一份，不分玩家，
/// 與 `voxel_milestones::MilestoneStore`（per-player）刻意區隔。
#[derive(Default, Debug)]
pub struct VillageMilestoneStore {
    unlocked: HashSet<String>,
}

impl VillageMilestoneStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。未知 id（例如舊版留下的壞資料）安全略過。
    pub fn from_entries(entries: impl IntoIterator<Item = VillageMilestoneEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            if is_known(&e.id) {
                s.unlocked.insert(e.id);
            }
        }
        s
    }

    /// 村莊是否已達成指定門檻。
    pub fn has(&self, id: &str) -> bool {
        self.unlocked.contains(id)
    }

    /// 檢查＋解鎖一氣呵成（單一寫鎖內呼叫，避免檢查與寫入之間的競態）：地標數剛好跨過下一個
    /// 尚未解鎖的門檻時，當場標記解鎖並回傳該門檻定義；已解鎖過或尚未達標則回 `None`（冪等）。
    pub fn try_unlock_new_tier(&mut self, count: usize) -> Option<&'static TierDef> {
        let tier = check_new_tier(count, &self.unlocked)?;
        self.unlocked.insert(tier.id.to_string());
        Some(tier)
    }
}

// ── jsonl 持久化（append-only，比照 voxel_milestones::append_milestone 慣例）───────

/// Append 一筆村莊里程碑記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_village_milestone(entry: &VillageMilestoneEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(VILLAGE_MILESTONES_PATH, &line);
    }
}

/// 載回所有村莊里程碑記錄（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_village_milestones() -> Vec<VillageMilestoneEntry> {
    let content = match std::fs::read_to_string(VILLAGE_MILESTONES_PATH) {
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
                serde_json::from_str::<VillageMilestoneEntry>(l).ok()
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
        Err(e) => tracing::warn!("無法寫入村莊里程碑記錄 {path}: {e}"),
    }
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_ids_unique() {
        let mut ids: Vec<&str> = TIERS.iter().map(|t| t.id).collect();
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), before, "村莊里程碑 id 不應重複");
    }

    #[test]
    fn tier_thresholds_strictly_increasing() {
        for w in TIERS.windows(2) {
            assert!(w[0].threshold < w[1].threshold, "門檻應嚴格遞增");
        }
    }

    #[test]
    fn tiers_have_nonempty_fields() {
        for t in TIERS {
            assert!(!t.id.is_empty());
            assert!(!t.name_zh.is_empty());
            assert!(t.threshold > 0);
        }
    }

    #[test]
    fn is_known_matches_static_list() {
        assert!(is_known("village_hamlet"));
        assert!(is_known("village_city"));
        assert!(!is_known(""));
        assert!(!is_known("village_metropolis"));
    }

    #[test]
    fn check_new_tier_below_first_threshold_is_none() {
        let unlocked = HashSet::new();
        assert!(check_new_tier(0, &unlocked).is_none());
        assert!(check_new_tier(2, &unlocked).is_none());
    }

    #[test]
    fn check_new_tier_at_threshold_returns_first_tier() {
        let unlocked = HashSet::new();
        let t = check_new_tier(3, &unlocked).expect("達到第一門檻應回傳");
        assert_eq!(t.id, "village_hamlet");
    }

    #[test]
    fn check_new_tier_none_when_not_reaching_next_after_first_unlocked() {
        let mut unlocked = HashSet::new();
        unlocked.insert("village_hamlet".to_string());
        assert!(check_new_tier(5, &unlocked).is_none(), "第一門檻已解鎖、未達第二門檻應回 None");
    }

    #[test]
    fn check_new_tier_progresses_to_next_tier() {
        let mut unlocked = HashSet::new();
        unlocked.insert("village_hamlet".to_string());
        let t = check_new_tier(8, &unlocked).expect("達第二門檻應回傳");
        assert_eq!(t.id, "village_town");
    }

    #[test]
    fn check_new_tier_none_when_all_unlocked() {
        let unlocked: HashSet<String> = TIERS.iter().map(|t| t.id.to_string()).collect();
        assert!(check_new_tier(999, &unlocked).is_none());
    }

    #[test]
    fn try_unlock_new_tier_idempotent() {
        let mut s = VillageMilestoneStore::new();
        let first = s.try_unlock_new_tier(3);
        assert!(first.is_some());
        assert!(s.has("village_hamlet"));
        let second = s.try_unlock_new_tier(3);
        assert!(second.is_none(), "同樣的地標數重複檢查應冪等回 None");
    }

    #[test]
    fn try_unlock_new_tier_progresses_through_tiers() {
        let mut s = VillageMilestoneStore::new();
        assert_eq!(s.try_unlock_new_tier(3).unwrap().id, "village_hamlet");
        assert!(s.try_unlock_new_tier(5).is_none(), "尚未達第二門檻");
        assert_eq!(s.try_unlock_new_tier(8).unwrap().id, "village_town");
        assert_eq!(s.try_unlock_new_tier(15).unwrap().id, "village_city");
        assert!(s.try_unlock_new_tier(999).is_none(), "門檻已全數解鎖");
    }

    #[test]
    fn from_entries_restores_unlocked_state() {
        let entries = vec![VillageMilestoneEntry { id: "village_hamlet".to_string() }];
        let s = VillageMilestoneStore::from_entries(entries);
        assert!(s.has("village_hamlet"));
        assert!(!s.has("village_town"));
    }

    #[test]
    fn from_entries_skips_unknown_ids() {
        let entries = vec![VillageMilestoneEntry { id: "bogus".to_string() }];
        let s = VillageMilestoneStore::from_entries(entries);
        assert!(!s.has("bogus"));
    }

    #[test]
    fn celebrate_say_line_nonempty_and_pick_safe() {
        for pick in 0..20 {
            assert!(!celebrate_say_line(pick).is_empty());
        }
    }

    #[test]
    fn celebrate_feed_line_contains_tier_name_and_count() {
        let line = celebrate_feed_line("小小聚落", 3);
        assert!(line.contains("小小聚落"));
        assert!(line.contains('3'));
    }
}
