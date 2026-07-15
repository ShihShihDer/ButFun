//! 乙太方界·殖民地集體里程碑 v1（自主提案切片；補 `voxel_settle.rs` 自己留白的地基缺口）。
//!
//! **真缺口**：殖民地真居住（942）讓拓荒者真的搬去了第二村、殖民地安全網（965）讓風禾屯
//! 也有了自己的庇護圈，但 `voxel_settle.rs` 頭註自己誠實列了三項「留待後續刀」的主村專屬
//! 系統——殖民地方位指標（971）已補上地圖端點那一項，**紀念柱／村莊集體里程碑**這兩項至今
//! 仍是空白：無論風禾屯搬進去幾位居民，世界從沒有任何一刻讓玩家或居民感覺到「這座殖民地
//! 正在一起長大」，更沒有一根屬於它自己的紀念柱——集體成就（856/885）永遠只認**主村**。
//!
//! **做法**：主村的集體里程碑（856）用「被居民命名的地標數」當指標——但殖民地初期地廣人稀，
//! 命名地標本就零星，拿同一把尺量注定長期掛零、失去意義。殖民地最直觀、零新開銷、每次
//! 遷居/添丁都會變動的指標，其實是**人口本身**——`voxel_settle::SettlementStore` 早就在追蹤
//! 「誰住哪個聚落」。本模組讓每座殖民地的人口跨過門檻（2/4/8，8＝殖民地小地塊 `colony_plots`
//! 全滿）時，那座殖民地自己的居民一起停下手邊的事、冒出歡呼泡泡，動態牆播一則屬於那座
//! 聚落的公告，並沿用既有 [`crate::voxel_monument::monument_cells`]（純幾何、零新程式碼）
//! 在殖民地自己的中心廣場立起一根同款紀念柱——每座殖民地各自累計、各自擁有一根屬於自己
//! 的村碑，互不相干。
//!
//! **與既有系統 razor-sharp 區隔（非同軸重複）**：856/885＝**主村專屬**、指標是**命名地標數**、
//! 全體居民（含殖民地拓荒者）一起慶祝；本模組＝**殖民地專屬**（`settlement != MAIN_SETTLEMENT`
//! 才會觸發）、指標是**該聚落人口**、**只有那座殖民地自己的居民**慶祝——主村的居民不會為
//! 遠方殖民地的成長冒泡，兩份帳本各自獨立、互不干擾，不共用任何一個門檻 id。
//!
//! **成本紀律**：零 LLM（純函式門檻判定＋確定性選句）、零新美術（沿用村碑既有方塊組合）、
//! FPS 零影響（人口變動本就低頻：遷居/誕生才會觸發一次檢查）。
//!
//! **濫用防護**：不收玩家輸入、不觸發 LLM、不開對外端點——人口/門檻/紀念柱全由伺服器內部
//! 聚落歸屬資料決定性算出，玩家無從自報或催發。
//!
//! **純邏輯層**：本檔零 IO／零鎖／零 async（除 append-only 持久化小節）；門檻判定、選句、
//! 文案皆確定性純函式，可窮舉測試。鎖／廣播／世界寫入／記憶落地全在 `voxel_ws.rs`
//! （比照 `voxel_village_milestone`／`voxel_monument` 既有短鎖循序慣例，守 prod 死鎖鐵律）。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// 持久化路徑（`data/` 已 gitignore；一份帳本、以聚落 id 分帳）。
const COLONY_MILESTONES_PATH: &str = "data/voxel_colony_milestones.jsonl";

/// 一個殖民地集體里程碑門檻的靜態定義。
pub struct TierDef {
    /// 穩定 id（wire 契約 + 持久化鍵；與 `voxel_village_milestone::TIERS` 的 id 空間刻意
    /// 不重疊，兩套帳本各自獨立判定，不會互相誤判為「已解鎖」）。
    pub id: &'static str,
    /// 達成門檻——該聚落累計人口（`>=` 即達成）。
    pub threshold: usize,
    /// 繁中稱號（達成時這座殖民地被稱作什麼）。
    pub name_zh: &'static str,
}

/// 全部門檻，依人口由小到大排列（新增門檻只准往後 append、數字只准遞增）。
/// 8＝`voxel_settle::colony_plots` 一座殖民地的小地塊上限——住滿即是這座殖民地 v1 的極限人口。
pub const TIERS: &[TierDef] = &[
    TierDef { id: "colony_settling", threshold: 2, name_zh: "剛安頓下來的拓荒地" },
    TierDef { id: "colony_hamlet", threshold: 4, name_zh: "像模像樣的聚落" },
    TierDef { id: "colony_full", threshold: 8, name_zh: "住滿的殖民地" },
];

/// 查表確認是否為已知門檻 id（守 store 資料乾淨，未知 id 不寫入）。
pub fn is_known(id: &str) -> bool {
    TIERS.iter().any(|t| t.id == id)
}

/// 門檻 id 在 [`TIERS`] 中的序位（0-based）——供村碑（`voxel_monument`）依「第幾階」逐段拔高。
/// 未知 id 回 `None`。
pub fn tier_index(id: &str) -> Option<usize> {
    TIERS.iter().position(|t| t.id == id)
}

/// 純函式：某聚落人口 `count` 底下，第一個「已達門檻、但尚未解鎖過」的門檻定義（沒有就回
/// `None`）。門檻間距皆大於單次遷居/誕生只會 +1 的幅度，正常情況一次呼叫至多命中一個門檻。
pub fn check_new_tier(count: usize, unlocked: &HashSet<String>) -> Option<&'static TierDef> {
    TIERS.iter().find(|t| count >= t.threshold && !unlocked.contains(t.id))
}

/// 這座殖民地慶祝時，居民頭頂輪替的歡呼泡泡（刻意不含個人稱謂，適用於任何一位居民；
/// 用詞刻意與主村版本（`voxel_village_milestone::celebrate_say_line`）區隔開拓調性）。
pub fn celebrate_say_line(pick: usize) -> &'static str {
    const LINES: &[&str] = &[
        "這裡漸漸有家的樣子了！",
        "我們的新家越來越熱鬧了呢！",
        "沒想到這片新天地也能這麼溫暖！",
        "真高興能在這裡跟大家重新扎根！",
    ];
    LINES[pick % LINES.len()]
}

/// 殖民地集體里程碑動態牆播報句（含聚落名、達成稱號、當下人口）。
pub fn celebrate_feed_line(colony_name: &str, tier_name_zh: &str, count: usize) -> String {
    format!("🏕️ 「{colony_name}」迎來第 {count} 位定居的居民——正式成為「{tier_name_zh}」！")
}

/// 殖民地村碑動態牆播報句（比照 `voxel_monument::monument_feed_line`，嵌入聚落名以區隔是
/// 哪一座村碑拔高了）。
pub fn colony_monument_feed_line(colony_name: &str, tier_name_zh: &str, height: i32) -> String {
    format!(
        "🗿 「{colony_name}」的居民合力立起了自己的村碑，為晉升「{tier_name_zh}」誌記——如今高達 {height} 格。"
    )
}

/// 參與立碑的殖民地居民寫進記憶的一句（episodic、第一人稱內心，嵌聚落名）。
pub fn colony_monument_memory_line(colony_name: &str, height: i32) -> String {
    format!("今天我們在「{colony_name}」中央，把屬於自己的村碑立高了一截，現在有 {height} 格那麼高了。這裡也是我們的家。")
}

// ── 持久化格式 ────────────────────────────────────────────────────────────────

/// 一筆持久化記錄：某聚落達成了某個集體門檻。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColonyMilestoneEntry {
    pub settlement: u64,
    pub id: String,
}

// ── 殖民地里程碑 Store ───────────────────────────────────────────────────────

/// 殖民地集體里程碑帳本（純同步資料結構，由呼叫端包進 `RwLock`）——以聚落 id 分帳，
/// 與 `voxel_village_milestone::VillageMilestoneStore`（全域一份，主村專屬）刻意區隔。
#[derive(Default, Debug)]
pub struct ColonyMilestoneStore {
    unlocked: HashMap<u64, HashSet<String>>,
}

impl ColonyMilestoneStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。未知 id（例如舊版留下的壞資料）安全略過。
    pub fn from_entries(entries: impl IntoIterator<Item = ColonyMilestoneEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            if is_known(&e.id) {
                s.unlocked.entry(e.settlement).or_default().insert(e.id);
            }
        }
        s
    }

    /// 指定聚落是否已達成指定門檻。
    pub fn has(&self, settlement: u64, id: &str) -> bool {
        self.unlocked.get(&settlement).is_some_and(|set| set.contains(id))
    }

    /// 檢查＋解鎖一氣呵成（單一寫鎖內呼叫，避免檢查與寫入之間的競態）：指定聚落的人口剛好
    /// 跨過下一個尚未解鎖的門檻時，當場標記解鎖並回傳該門檻定義；已解鎖過或尚未達標則回
    /// `None`（冪等）。每座聚落各自累計，互不影響。
    pub fn try_unlock_new_tier(&mut self, settlement: u64, count: usize) -> Option<&'static TierDef> {
        let unlocked = self.unlocked.entry(settlement).or_default();
        let tier = check_new_tier(count, unlocked)?;
        unlocked.insert(tier.id.to_string());
        Some(tier)
    }
}

// ── jsonl 持久化（append-only，比照 voxel_village_milestone 慣例）───────────────

/// Append 一筆殖民地里程碑記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_colony_milestone(entry: &ColonyMilestoneEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(COLONY_MILESTONES_PATH, &line);
    }
}

/// 載回所有殖民地里程碑記錄（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_colony_milestones() -> Vec<ColonyMilestoneEntry> {
    let content = match std::fs::read_to_string(COLONY_MILESTONES_PATH) {
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
                serde_json::from_str::<ColonyMilestoneEntry>(l).ok()
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
        Err(e) => tracing::warn!("無法寫入殖民地里程碑記錄 {path}: {e}"),
    }
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_ids_unique_and_disjoint_from_village_ids() {
        let mut ids: Vec<&str> = TIERS.iter().map(|t| t.id).collect();
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), before, "殖民地里程碑 id 不應重複");
        for t in TIERS {
            assert!(
                !crate::voxel_village_milestone::is_known(t.id),
                "殖民地門檻 id {} 不該與主村門檻 id 撞號",
                t.id
            );
        }
    }

    #[test]
    fn tier_thresholds_strictly_increasing_and_capped_at_colony_size() {
        for w in TIERS.windows(2) {
            assert!(w[0].threshold < w[1].threshold, "門檻應嚴格遞增");
        }
        assert_eq!(TIERS.last().unwrap().threshold, 8, "最高門檻應對齊殖民地小地塊上限");
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
        assert!(is_known("colony_settling"));
        assert!(is_known("colony_full"));
        assert!(!is_known(""));
        assert!(!is_known("colony_metropolis"));
    }

    #[test]
    fn tier_index_matches_position() {
        assert_eq!(tier_index("colony_settling"), Some(0));
        assert_eq!(tier_index("colony_hamlet"), Some(1));
        assert_eq!(tier_index("colony_full"), Some(2));
        assert_eq!(tier_index("bogus"), None);
    }

    #[test]
    fn check_new_tier_below_first_threshold_is_none() {
        let unlocked = HashSet::new();
        assert!(check_new_tier(0, &unlocked).is_none());
        assert!(check_new_tier(1, &unlocked).is_none());
    }

    #[test]
    fn check_new_tier_progresses_through_tiers() {
        let mut unlocked = HashSet::new();
        let t = check_new_tier(2, &unlocked).expect("達第一門檻應回傳");
        assert_eq!(t.id, "colony_settling");
        unlocked.insert(t.id.to_string());
        assert!(check_new_tier(3, &unlocked).is_none(), "未達第二門檻");
        let t2 = check_new_tier(4, &unlocked).expect("達第二門檻應回傳");
        assert_eq!(t2.id, "colony_hamlet");
    }

    #[test]
    fn try_unlock_new_tier_is_per_settlement_independent() {
        let mut s = ColonyMilestoneStore::new();
        assert_eq!(s.try_unlock_new_tier(1, 2).unwrap().id, "colony_settling");
        // 另一座聚落（settlement=2）人口還沒到，不受聚落 1 已解鎖影響。
        assert!(s.try_unlock_new_tier(2, 1).is_none());
        assert_eq!(s.try_unlock_new_tier(2, 2).unwrap().id, "colony_settling");
        assert!(s.has(1, "colony_settling"));
        assert!(s.has(2, "colony_settling"));
        assert!(!s.has(1, "colony_hamlet"));
    }

    #[test]
    fn try_unlock_new_tier_idempotent() {
        let mut s = ColonyMilestoneStore::new();
        assert!(s.try_unlock_new_tier(1, 2).is_some());
        assert!(s.try_unlock_new_tier(1, 2).is_none(), "同樣人口重複檢查應冪等回 None");
    }

    #[test]
    fn try_unlock_new_tier_progresses_to_full() {
        let mut s = ColonyMilestoneStore::new();
        assert_eq!(s.try_unlock_new_tier(1, 2).unwrap().id, "colony_settling");
        assert_eq!(s.try_unlock_new_tier(1, 4).unwrap().id, "colony_hamlet");
        assert_eq!(s.try_unlock_new_tier(1, 8).unwrap().id, "colony_full");
        assert!(s.try_unlock_new_tier(1, 999).is_none(), "門檻已全數解鎖");
    }

    #[test]
    fn from_entries_restores_per_settlement_state() {
        let entries = vec![
            ColonyMilestoneEntry { settlement: 1, id: "colony_settling".to_string() },
            ColonyMilestoneEntry { settlement: 2, id: "colony_hamlet".to_string() },
        ];
        let s = ColonyMilestoneStore::from_entries(entries);
        assert!(s.has(1, "colony_settling"));
        assert!(!s.has(1, "colony_hamlet"));
        assert!(s.has(2, "colony_hamlet"));
        assert!(!s.has(2, "colony_settling"));
    }

    #[test]
    fn from_entries_skips_unknown_ids() {
        let entries = vec![ColonyMilestoneEntry { settlement: 1, id: "bogus".to_string() }];
        let s = ColonyMilestoneStore::from_entries(entries);
        assert!(!s.has(1, "bogus"));
    }

    #[test]
    fn celebrate_say_line_nonempty_and_pick_safe() {
        for pick in 0..20 {
            assert!(!celebrate_say_line(pick).is_empty());
        }
    }

    #[test]
    fn celebrate_feed_line_contains_name_tier_and_count() {
        let line = celebrate_feed_line("風禾屯", "剛安頓下來的拓荒地", 2);
        assert!(line.contains("風禾屯"));
        assert!(line.contains("剛安頓下來的拓荒地"));
        assert!(line.contains('2'));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn colony_monument_feed_line_contains_name_tier_and_height() {
        let line = colony_monument_feed_line("風禾屯", "像模像樣的聚落", 6);
        assert!(line.contains("風禾屯"));
        assert!(line.contains("像模像樣的聚落"));
        assert!(line.contains('6'));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn colony_monument_memory_line_is_single_line_and_embeds_name_height() {
        let line = colony_monument_memory_line("風禾屯", 3);
        assert!(!line.is_empty());
        assert!(!line.contains('\n'));
        assert!(line.contains("風禾屯"));
        assert!(line.contains('3'));
    }
}
