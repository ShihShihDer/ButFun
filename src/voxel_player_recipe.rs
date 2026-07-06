//! 乙太方界·居民教你一道獨門配方 v1（voxel-player-recipe，自主提案切片）。
//!
//! **真缺口**：居民互相傳授技能（717，`voxel_teach`）讓「本事」在居民↔居民的朋友網絡裡
//! 流通，但玩家的合成配方從第一天上線起就**全部開放**——沒有任何一條「透過情誼解鎖新配方」
//! 的路。居民↔玩家的互動早就有送禮（670）、回禮（728/731）、關心挨餓（845）、掏心事
//! （781）……唯獨「教你點本事」這件事，是居民↔居民獨有、居民↔玩家完全空白的一塊。
//!
//! **本刀補上**：與某位居民感情深厚（好感度達到 [`TEACH_MIN_AFFINITY`]——高於老友情境問候
//! 的 `FOND_AFFINITY`(5)，象徵比一般老友更深一層的交情）時，她會主動教你一道她的獨門配方——
//! 「護身符」（石頭＋一朵紅花，貼身佩戴的心意信物）。從此你能自己在背包合成台做出來，永久
//! 解鎖、跨連線持久。正中 PLAN_ETHERVOX 核心信念「記憶要驅動行為」：她記得你們多熟，這份
//! 熟識第一次直接變成你手上多出的一件本事。
//!
//! **範圍（v1，誠實的取捨）**：先做**一道**配方，任何一位居民、只要夠熟都可能教（不綁定
//! 特定居民身分）——機制先驗證通過，日後可再往 [`TAUGHT_RECIPE_ID`] 這個池子加更多獨門配方。
//!
//! **與既有系統的定位區隔**：居民互相傳授（717）是居民↔居民、技能發明系統（`voxel_invent`）
//! 的原語組合；本刀是居民↔玩家、既有 2×2 合成配方系統（`voxel_craft::Recipe`）的解鎖開關，
//! 兩套配方系統完全獨立，互不干擾。
//!
//! **純邏輯層**：是否教（[`should_teach_recipe`]）、教學泡泡（[`teach_bubble`]）、Feed（
//! [`teach_feed_line`]）、記憶摘要（[`teach_memory_line`]）皆確定性純函式，零 LLM、零鎖、零 IO。
//! [`PlayerRecipeStore`] 是純同步資料結構，鎖／WS／IO／持久化全在 `voxel_ws.rs`。
//!
//! **成本 / 濫用防護**：句子全走固定模板，不夾帶玩家輸入；教學由**伺服器權威**判定好感度
//! （讀 `voxel_memory` 既有記憶筆數，非客戶端自報）；每位居民教學冷卻 [`TEACH_COOLDOWN_SECS`]
//! 長（600s），稀有有份量、天然防洗版；零 migration（新增 append-only jsonl）、零協議破壞
//! （純新增 WS 訊息型別）、零新美術。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// 目前池子裡唯一的獨門配方 id（`voxel_craft::TAUGHT_RECIPES` 對應項的 `id`，
/// 對應物品為 `voxel_craft::AMULET_ID` 護身符）。
pub const TAUGHT_RECIPE_ID: &str = "amulet";

/// 居民願意教你獨門配方的最低好感（＝關於這位玩家的記憶筆數）。設 8——高於老友情境問候
/// 門檻 `FOND_AFFINITY`(5)，也高於掏心事門檻 `CONFIDE_MIN_AFFINITY`(3)：教一道會跟著你一輩子
/// 的本事，該比隨口聊心事更深交情才發生。
pub const TEACH_MIN_AFFINITY: usize = 8;

/// 同一位居民主動教學的冷卻（秒）。設得長（600s＝10 分鐘）——這是一次性的大事件，不必像
/// 招呼／掏心那樣頻繁嘗試，稀有才有份量。
pub const TEACH_COOLDOWN_SECS: f32 = 600.0;

/// 教學泡泡的字元上限（與其餘泡泡框上限一致，超出截斷不破框）。
pub const TEACH_SAY_MAX_CHARS: usize = 40;

/// 判斷此刻是否要教你一道獨門配方：好感夠 ＋ 你還沒學會這道配方 ＋ 冷卻到期 ＋ 過了機率門檻。
///
/// 純函式、確定性（機率骰由呼叫端傳入）。「有沒有配方可教」（是否還有玩家未學會的配方）
/// 由呼叫端另外查 [`PlayerRecipeStore`] 決定，本函式只把「熟不熟／該不該現在教」的門檻。
pub fn should_teach_recipe(affinity: usize, already_known: bool, cooldown_ok: bool, roll: f32, chance: f32) -> bool {
    affinity >= TEACH_MIN_AFFINITY && !already_known && cooldown_ok && roll < chance
}

/// 教學當下的泡泡台詞，依 `pick` 在幾組模板間確定性輪替。整句以字元截到
/// [`TEACH_SAY_MAX_CHARS`] 內，永不破泡泡框。
pub fn teach_bubble(recipe_name: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 4] = [
        "來，我教你怎麼做{}吧——這是我很珍惜的手藝。",
        "跟你這麼熟了，把{}的做法教給你也好。",
        "我把{}的訣竅教你——以後你自己也做得出來了。",
        "這道{}的做法，我只教真正處得來的朋友。",
    ];
    let line = TEMPLATES[pick % TEMPLATES.len()].replace("{}", recipe_name);
    line.chars().take(TEACH_SAY_MAX_CHARS).collect()
}

/// 教學這件事本身，記進居民「關於這位玩家」的一筆記憶摘要（第一人稱、episodic）。
/// 刻意不含配方細節，維持與 `voxel_confide`/`voxel_admire` 同款輕記憶。
pub fn teach_memory_line(player: &str, recipe_name: &str) -> String {
    format!("我把{recipe_name}的做法教給了{player}，往後牠自己也做得出來了。")
}

/// 教學這件事的城鎮動態牆播報（比照既有 Feed 慣例，只嵌顯示名，無玩家原話）。
pub fn teach_feed_line(resident_name: &str, player_name: &str, recipe_name: &str) -> String {
    format!("{resident_name}教了{player_name}一道獨門配方：{recipe_name}。")
}

// ── 持久化格式 ────────────────────────────────────────────────────────────────

/// 一筆持久化記錄：某玩家學會了某道獨門配方。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerRecipeEntry {
    pub player: String,
    pub recipe_id: String,
}

// ── 玩家獨門配方 Store ───────────────────────────────────────────────────────

/// 玩家已學會的獨門配方帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct PlayerRecipeStore {
    /// key = 玩家名 → 已學會的獨門配方 id 集合。
    known: HashMap<String, HashSet<String>>,
}

impl PlayerRecipeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。
    pub fn from_entries(entries: impl IntoIterator<Item = PlayerRecipeEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            s.known.entry(e.player).or_default().insert(e.recipe_id);
        }
        s
    }

    /// 標記玩家學會一道獨門配方。回傳 `true` 代表「這次才第一次學會」——呼叫端只在回 `true`
    /// 時才 append 持久化 + 廣播慶祝；已學會過再呼叫安全回 `false`，冪等。
    pub fn learn(&mut self, player: &str, recipe_id: &str) -> bool {
        self.known.entry(player.to_string()).or_default().insert(recipe_id.to_string())
    }

    /// 玩家是否已學會指定獨門配方。
    pub fn knows(&self, player: &str, recipe_id: &str) -> bool {
        self.known.get(player).is_some_and(|s| s.contains(recipe_id))
    }
}

// ── jsonl 持久化（append-only，比照 voxel_milestones::append_milestone 慣例）──────

const PLAYER_RECIPES_PATH: &str = "data/voxel_player_recipes.jsonl";

/// Append 一筆學會記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_player_recipe(entry: &PlayerRecipeEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(PLAYER_RECIPES_PATH, &line);
    }
}

/// 載回所有學會記錄（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_player_recipes() -> Vec<PlayerRecipeEntry> {
    let content = match std::fs::read_to_string(PLAYER_RECIPES_PATH) {
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
                serde_json::from_str::<PlayerRecipeEntry>(l).ok()
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
        Err(e) => tracing::warn!("無法寫入玩家獨門配方記錄 {path}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_teach_needs_affinity_unknown_cooldown_and_roll() {
        assert!(should_teach_recipe(TEACH_MIN_AFFINITY, false, true, 0.0, 0.5));
        assert!(should_teach_recipe(20, false, true, 0.49, 0.5));
        // 好感不足 → 否決。
        assert!(!should_teach_recipe(TEACH_MIN_AFFINITY - 1, false, true, 0.0, 0.5));
        // 已學會 → 否決（不重複教）。
        assert!(!should_teach_recipe(20, true, true, 0.0, 0.5));
        // 冷卻未到 → 否決。
        assert!(!should_teach_recipe(20, false, false, 0.0, 0.5));
        // 骰子未過門檻 → 否決。
        assert!(!should_teach_recipe(20, false, true, 0.5, 0.5));
    }

    #[test]
    fn teach_bubble_contains_recipe_name_and_fits_frame() {
        for pick in 0..8 {
            let line = teach_bubble("護身符", pick);
            assert!(line.contains("護身符"), "教學泡泡應含配方名：{line}");
            assert!(line.chars().count() <= TEACH_SAY_MAX_CHARS, "教學泡泡不該破框：{line}");
        }
    }

    #[test]
    fn teach_bubble_deterministic_by_pick() {
        assert_eq!(teach_bubble("護身符", 1), teach_bubble("護身符", 1));
    }

    #[test]
    fn memory_and_feed_lines_contain_names() {
        let m = teach_memory_line("小星", "護身符");
        assert!(m.contains("小星") && m.contains("護身符"));
        let f = teach_feed_line("露娜", "小星", "護身符");
        assert!(f.contains("露娜") && f.contains("小星") && f.contains("護身符"));
    }

    #[test]
    fn store_learn_is_idempotent_and_per_player() {
        let mut s = PlayerRecipeStore::new();
        assert!(!s.knows("小星", TAUGHT_RECIPE_ID));
        assert!(s.learn("小星", TAUGHT_RECIPE_ID), "第一次學會應回 true");
        assert!(s.knows("小星", TAUGHT_RECIPE_ID));
        assert!(!s.learn("小星", TAUGHT_RECIPE_ID), "重複學會應回 false（冪等）");
        assert!(!s.knows("阿明", TAUGHT_RECIPE_ID), "不同玩家互不影響");
    }

    #[test]
    fn store_from_entries_replay_restores_known_set() {
        let entries = vec![
            PlayerRecipeEntry { player: "小星".into(), recipe_id: TAUGHT_RECIPE_ID.into() },
            PlayerRecipeEntry { player: "阿明".into(), recipe_id: TAUGHT_RECIPE_ID.into() },
        ];
        let s = PlayerRecipeStore::from_entries(entries);
        assert!(s.knows("小星", TAUGHT_RECIPE_ID));
        assert!(s.knows("阿明", TAUGHT_RECIPE_ID));
        assert!(!s.knows("陌生人", TAUGHT_RECIPE_ID));
    }

}
