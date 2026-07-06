//! 乙太方界·玩家熟練度 v1（自主提案切片，接續 724 里程碑）。
//!
//! **真缺口**：玩家里程碑（724）補上了「回頭看看自己走了多遠」的一次性徽章牆——但徽章是
//! **二元**的（做過一次沒），一次解鎖後就靜止不動。玩家日復一日反覆採集/耕種/垂釣，除了
//! 徽章牆上早已勾滿的那幾枚勾勾，**持續投入本身**從沒有任何看得見的累積成長——這正對著
//! `docs/PLAN_ETHERVOX.md`「玩家遊玩」節那句「進度/目標：療癒循環給人想一直玩下去的理由」，
//! 里程碑只解決了「有沒有做過」，卻沒解決「練得多深」。
//!
//! **這一刀補上二元徽章之外的連續進程**：三條熟練度——⛏️採集／🌾耕種／🎣垂釣（療癒循環裡
//! 玩家重複次數最高的三種核心動作）隨反覆遊玩持續攢經驗、升級、練到 Lv.5 起解鎖一份小小的
//! 產出加成（與既有工具加成 790、並肩協作 827 同量級的 +1，各自獨立疊加）——讓「這幾天一直
//! 在採集」這件事本身第一次有看得見的回饋曲線，而不是做完第一次就沒了下文。
//!
//! **與 724 里程碑的區隔**：里程碑＝二元「做過一次沒」、跨系統全覆蓋（合成/贈禮/交易…）；
//! 熟練度＝連續數值「練得多深」、只挑三種玩家重複次數最高的核心動作，兩者互補、不重疊。
//!
//! 純邏輯層：等級曲線、加成解鎖、稱號、經驗值全是確定性純函式，零 LLM／鎖／IO；
//! 持久化 IO（`append_mastery`／`load_mastery`）走既有「append-only、重啟逐行加總還原」
//! 慣例（比照 `voxel_inventory`），與 hub 的 `RwLock` 接線、動作觸發點全在 `voxel_ws.rs`。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 持久化路徑（`data/` 已 gitignore，執行期產生）。
const MASTERY_PATH: &str = "data/voxel_mastery.jsonl";

/// 三條熟練度——刻意只挑玩家重複次數最高的核心動作，別無限擴張成任何動作都算一條。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MasteryKind {
    Gathering,
    Farming,
    Fishing,
}

impl MasteryKind {
    pub const ALL: [MasteryKind; 3] =
        [MasteryKind::Gathering, MasteryKind::Farming, MasteryKind::Fishing];

    /// wire / 持久化用穩定字串鍵。
    pub fn as_str(self) -> &'static str {
        match self {
            MasteryKind::Gathering => "gathering",
            MasteryKind::Farming => "farming",
            MasteryKind::Fishing => "fishing",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "gathering" => Some(MasteryKind::Gathering),
            "farming" => Some(MasteryKind::Farming),
            "fishing" => Some(MasteryKind::Fishing),
            _ => None,
        }
    }

    pub fn display_name_zh(self) -> &'static str {
        match self {
            MasteryKind::Gathering => "採集",
            MasteryKind::Farming => "耕種",
            MasteryKind::Fishing => "垂釣",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            MasteryKind::Gathering => "⛏️",
            MasteryKind::Farming => "🌾",
            MasteryKind::Fishing => "🎣",
        }
    }

    /// 每次動作累積的經驗值：越少發生一次的動作單次給越多經驗（垂釣要等上鉤、耕種要等
    /// 作物熟成，採集最頻繁反而給最少），讓三條熟練度用同樣的遊玩時間大致同步成長，
    /// 不會有一條遙遙領先、其餘形同虛設。
    pub fn xp_per_action(self) -> u32 {
        match self {
            MasteryKind::Gathering => 2,
            MasteryKind::Farming => 6,
            MasteryKind::Fishing => 8,
        }
    }
}

/// 每級所需經驗（累積制、線性）：Lv.N 需要總經驗 >= N * LEVEL_XP_STEP。
pub const LEVEL_XP_STEP: u32 = 60;
/// 頂級——練到頭之後經驗仍可累積但不再升級，避免數字無限跑、稱號/加成永遠停在頂級。
pub const MAX_LEVEL: u32 = 10;
/// 練到這一級起，該熟練度對應的動作解鎖一份小小的產出加成（往後每次動作恆生效，非一次性）。
pub const BONUS_UNLOCK_LEVEL: u32 = 5;

/// 由總經驗算出目前等級（純函式、確定性、封頂 MAX_LEVEL）。
pub fn level_for_xp(xp: u32) -> u32 {
    (xp / LEVEL_XP_STEP).min(MAX_LEVEL)
}

/// 練到 `BONUS_UNLOCK_LEVEL` 起，對應動作額外多收一份既有材料——與工具加成（790）／
/// 並肩協作（827）同量級的 +1，三者各自獨立判定、疊加，不互相取代。
pub fn mastery_yield_bonus(level: u32) -> u32 {
    if level >= BONUS_UNLOCK_LEVEL {
        1
    } else {
        0
    }
}

/// 稱號（純展示，HUD/面板用）。
pub fn title_for_level(level: u32) -> &'static str {
    match level {
        0..=1 => "新手",
        2..=4 => "熟手",
        5..=8 => "好手",
        _ => "大師",
    }
}

/// 升級提示句（自己看得到的溫柔慶祝，面向玩家字串集中此處、i18n 友善）。
pub fn levelup_line(kind: MasteryKind, level: u32) -> String {
    let icon = kind.icon();
    let name = kind.display_name_zh();
    let title = title_for_level(level);
    format!("{icon} {name}熟練度升到 Lv.{level}（{title}）！")
}

/// 產出加成揭曉句（只在真的多收一份時冒出，讓玩家知道「為什麼」多了一份）。
pub fn bonus_line(kind: MasteryKind) -> String {
    let icon = kind.icon();
    let name = kind.display_name_zh();
    format!("{icon} 練出來的{name}手法，多收了一份！")
}

// ── 持久化格式（append-only：一行一筆經驗增量，重啟時逐行加總還原，比照 voxel_inventory）──

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MasteryEntry {
    pub player: String,
    pub kind: String,
    pub xp_delta: u32,
}

/// 玩家熟練度帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct MasteryStore {
    /// key = (玩家名, 種類) → 累積經驗。
    xp: HashMap<(String, MasteryKind), u32>,
}

impl MasteryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化紀錄重建（未知種類字串安全忽略，不 panic、不污染 store）。
    pub fn from_entries(entries: impl IntoIterator<Item = MasteryEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            if let Some(kind) = MasteryKind::from_str(&e.kind) {
                *s.xp.entry((e.player, kind)).or_insert(0) += e.xp_delta;
            }
        }
        s
    }

    /// 目前這位玩家這條熟練度的累積經驗（未紀錄過回 0）。
    pub fn xp_for(&self, player: &str, kind: MasteryKind) -> u32 {
        *self.xp.get(&(player.to_string(), kind)).unwrap_or(&0)
    }

    /// 累加經驗，回傳 `(新總經驗, 是否剛好升級, 新等級)`。
    pub fn add_xp(&mut self, player: &str, kind: MasteryKind, delta: u32) -> (u32, bool, u32) {
        let key = (player.to_string(), kind);
        let before = *self.xp.get(&key).unwrap_or(&0);
        let before_level = level_for_xp(before);
        let after = before.saturating_add(delta);
        self.xp.insert(key, after);
        let after_level = level_for_xp(after);
        (after, after_level > before_level, after_level)
    }
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
        Err(e) => tracing::warn!("無法寫入熟練度記錄 {path}: {e}"),
    }
}

/// append 一筆經驗增量（IO，呼叫端須在鎖外呼叫）。
pub fn append_mastery(entry: &MasteryEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(MASTERY_PATH, &line);
    }
}

/// 載回所有熟練度經驗紀錄（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_mastery() -> Vec<MasteryEntry> {
    let content = match std::fs::read_to_string(MASTERY_PATH) {
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
                serde_json::from_str::<MasteryEntry>(l).ok()
            }
        })
        .collect()
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_for_xp_steps_and_caps() {
        assert_eq!(level_for_xp(0), 0);
        assert_eq!(level_for_xp(59), 0);
        assert_eq!(level_for_xp(60), 1);
        assert_eq!(level_for_xp(300), 5);
        assert_eq!(level_for_xp(600), MAX_LEVEL);
        assert_eq!(level_for_xp(999_999), MAX_LEVEL); // 封頂，不無限跑
    }

    #[test]
    fn mastery_yield_bonus_unlocks_at_threshold() {
        assert_eq!(mastery_yield_bonus(0), 0);
        assert_eq!(mastery_yield_bonus(BONUS_UNLOCK_LEVEL - 1), 0);
        assert_eq!(mastery_yield_bonus(BONUS_UNLOCK_LEVEL), 1);
        assert_eq!(mastery_yield_bonus(MAX_LEVEL), 1);
    }

    #[test]
    fn title_for_level_progression() {
        assert_eq!(title_for_level(0), "新手");
        assert_eq!(title_for_level(1), "新手");
        assert_eq!(title_for_level(2), "熟手");
        assert_eq!(title_for_level(4), "熟手");
        assert_eq!(title_for_level(5), "好手");
        assert_eq!(title_for_level(8), "好手");
        assert_eq!(title_for_level(9), "大師");
        assert_eq!(title_for_level(MAX_LEVEL), "大師");
    }

    #[test]
    fn kind_round_trips_and_all_covers_three() {
        for k in MasteryKind::ALL {
            assert_eq!(MasteryKind::from_str(k.as_str()), Some(k));
        }
        assert_eq!(MasteryKind::ALL.len(), 3);
        assert_eq!(MasteryKind::from_str("bogus"), None);
    }

    #[test]
    fn xp_per_action_all_positive_and_differ_by_rarity() {
        // 越稀有的動作單次經驗越高（垂釣 > 耕種 > 採集），避免頻繁動作的熟練度遙遙領先。
        assert!(MasteryKind::Fishing.xp_per_action() > MasteryKind::Farming.xp_per_action());
        assert!(MasteryKind::Farming.xp_per_action() > MasteryKind::Gathering.xp_per_action());
        for k in MasteryKind::ALL {
            assert!(k.xp_per_action() > 0);
        }
    }

    #[test]
    fn add_xp_accumulates_and_reports_levelup_once() {
        let mut store = MasteryStore::new();
        let (xp1, up1, lvl1) = store.add_xp("nova", MasteryKind::Gathering, 30);
        assert_eq!(xp1, 30);
        assert!(!up1);
        assert_eq!(lvl1, 0);

        let (xp2, up2, lvl2) = store.add_xp("nova", MasteryKind::Gathering, 30);
        assert_eq!(xp2, 60);
        assert!(up2); // 剛好跨過 Lv.1 門檻
        assert_eq!(lvl2, 1);

        let (_, up3, _) = store.add_xp("nova", MasteryKind::Gathering, 5);
        assert!(!up3); // 還沒到下一級
        assert_eq!(store.xp_for("nova", MasteryKind::Gathering), 65);
    }

    #[test]
    fn add_xp_tracks_players_and_kinds_independently() {
        let mut store = MasteryStore::new();
        store.add_xp("nova", MasteryKind::Gathering, 100);
        store.add_xp("nova", MasteryKind::Farming, 10);
        store.add_xp("luna", MasteryKind::Gathering, 5);

        assert_eq!(store.xp_for("nova", MasteryKind::Gathering), 100);
        assert_eq!(store.xp_for("nova", MasteryKind::Farming), 10);
        assert_eq!(store.xp_for("nova", MasteryKind::Fishing), 0);
        assert_eq!(store.xp_for("luna", MasteryKind::Gathering), 5);
        assert_eq!(store.xp_for("stranger", MasteryKind::Gathering), 0);
    }

    #[test]
    fn from_entries_sums_deltas_and_skips_unknown_kind() {
        let entries = vec![
            MasteryEntry { player: "nova".into(), kind: "gathering".into(), xp_delta: 2 },
            MasteryEntry { player: "nova".into(), kind: "gathering".into(), xp_delta: 2 },
            MasteryEntry { player: "nova".into(), kind: "farming".into(), xp_delta: 6 },
            MasteryEntry { player: "luna".into(), kind: "bogus".into(), xp_delta: 999 },
        ];
        let store = MasteryStore::from_entries(entries);
        assert_eq!(store.xp_for("nova", MasteryKind::Gathering), 4);
        assert_eq!(store.xp_for("nova", MasteryKind::Farming), 6);
        assert_eq!(store.xp_for("luna", MasteryKind::Gathering), 0);
    }

    #[test]
    fn lines_mention_kind_and_are_non_empty() {
        for k in MasteryKind::ALL {
            let l = levelup_line(k, 5);
            assert!(!l.is_empty());
            assert!(l.contains(k.display_name_zh()));
            let b = bonus_line(k);
            assert!(!b.is_empty());
            assert!(b.contains(k.display_name_zh()));
        }
    }
}
