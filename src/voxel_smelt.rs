//! 乙太方界·熔爐煨煮 v1——冶煉不再瞬間，放進生料後需時間慢慢煨熟，回來取熱騰騰的成品。
//!
//! **核心信念（PLAN_ETHERVOX·人類也要玩得爽）**：乙太方界的採集手感一路是「對準→敲掉→立刻入袋」，
//! 熔爐冶煉也一直是**瞬間**變出成品——按下去就好，沒有等待、沒有期待。這一刀讓熔爐長出時間感：
//! 你把生魚 / 生馬鈴薯 / 礦石放進熔爐，它**開始煨煮**，過一段時間才熟成——你可以趁空去採礦、種田、
//! 跟居民聊天，回來時成品已烤好、自動入你背包，還跳一則「你的烤魚煨好了」的溫暖提示。
//!
//! 這呼應垂釣的「等」（`voxel_fishing`）與農地持久化 764 那句最有感的「你種的田，回來還在長」——
//! 熔爐第一次也「回來還在烤」：離線 / 重啟都不會讓煨煮中的心意蒸發（append-only jsonl 持久化，
//! 比照農地 764 / 背包 `voxel_inventory`）。
//!
//! **設計鐵律**：
//! - **純邏輯層**：本模組只有確定性純函式與記憶體 store + 輕量 jsonl 持久化；零 LLM、零鎖、零 async。
//!   連線 / 鎖 / 背包寫入 / 廣播 / tick 觸發全留在 `voxel_ws.rs`（沿用農地 764 的短鎖循序慣例，守死鎖鐵律）。
//! - **向後相容**：新檔 `data/voxel_smelt.jsonl` 不存在 / 壞行皆容忍；煨煮是數十秒的短暫過場，
//!   即使重啟丟失也只是少烤一爐，無玩家資料風險。
//! - **僅熔爐配方走煨煮**：背包 2×2 / 工作台 3×3 的合成維持瞬間（手感不變），只有熔爐冶煉需要等——
//!   熔爐的意象本就是「火慢慢燒」。
//! - 面向玩家字串集中（i18n 友善）；繁中註解；不碰玩家資料表。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 煨煮進度 jsonl 持久化路徑（append-only，比照 `voxel_farm::VOXEL_FARM_PATH`）。
pub const VOXEL_SMELT_PATH: &str = "data/voxel_smelt.jsonl";

/// 依熔爐配方 id 回傳煨煮所需秒數（確定性、可測、零 LLM）。
///
/// 節奏刻意分層——食物快、建材中、鐵最慢，讓不同冶煉有不同的等待手感；秒數皆數十秒等級，
/// 足以讓玩家「趁空去忙別的」，但不至於久到惱人。實際交付對齊 tick 節拍（`tick_smelt` 每 ~15 秒
/// 一次），故有效等待會落在該秒數之後的下一個 tick——這也是為何秒數取得比 tick 稍寬裕。
pub fn smelt_secs(recipe_id: &str) -> u64 {
    match recipe_id {
        // 食物：把垂釣 / 種田的收成烤成佳餚——最快，讓「釣到→烤來吃」的回饋不用等太久。
        "smelt_fish" | "smelt_potato" => 12,
        // 莓果醬：小火慢熬凝成一罐甜點，比烤魚 / 烤地薯稍久一點（熬煮的意象要多點耐心），
        // 但仍在「趁空去採一輪莓果就熬好」的舒服節奏內（莓果醬 v1 ROADMAP 808）。
        "smelt_jam" => 15,
        // 鐵：礦石熔成鐵錠是最「硬」的冶煉，煨煮最久，換來的鐵錠也最珍貴。
        "smelt_iron" => 30,
        // 建材（拋光石 / 玻璃 / 石磚）與其它：中等。
        _ => 18,
    }
}

/// 一爐正在煨煮的成品（記憶體狀態；由 jsonl 事件重建）。
#[derive(Clone, Debug, PartialEq)]
pub struct SmeltJob {
    /// 這爐屬於哪位玩家（烤好後成品入這位玩家的背包）。
    pub player: String,
    /// 熔爐配方 id（對齊 `voxel_craft::Recipe.id`；供交付時查名稱）。
    pub recipe_id: String,
    /// 成品方塊 / 物品 id。
    pub output_block: u8,
    /// 成品數量。
    pub output_count: u32,
    /// 開始煨煮的 Unix 秒數。
    pub started_secs: u64,
    /// 這爐需煨煮的秒數（`started_secs + dur_secs` 即熟成時刻）。
    pub dur_secs: u64,
}

/// 一筆煨煮事件（append-only jsonl 最小單元，比照 `voxel_farm::FarmEvent`）。
/// `player` 等欄位皆 Some → 開爐（start）；`player` 為 None → 這爐已交付 / 取消（remove）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmeltEvent {
    /// 這爐的唯一遞增 id（replay key）。
    pub id: u64,
    /// Some=開爐；None=交付移除。以下欄位在開爐時皆 Some、移除時皆 None（向後相容用 default）。
    #[serde(default)]
    pub player: Option<String>,
    #[serde(default)]
    pub recipe_id: Option<String>,
    #[serde(default)]
    pub output_block: Option<u8>,
    #[serde(default)]
    pub output_count: Option<u32>,
    #[serde(default)]
    pub started_secs: Option<u64>,
    #[serde(default)]
    pub dur_secs: Option<u64>,
}

/// 熔爐煨煮 store（append-only jsonl 持久化，重啟後煨煮中的爐續存）。
#[derive(Default)]
pub struct SmeltStore {
    jobs: HashMap<u64, SmeltJob>,
    /// 下一爐的 id（replay 續號）。
    pub next_id: u64,
}

impl SmeltStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 開一爐煨煮：登記 job、配一個遞增 id，回傳待落地的 `SmeltEvent`（呼叫端在鎖外 append）。
    pub fn start(
        &mut self,
        player: &str,
        recipe_id: &str,
        output_block: u8,
        output_count: u32,
        now_secs: u64,
        dur_secs: u64,
    ) -> SmeltEvent {
        let id = self.next_id;
        self.next_id += 1;
        let job = SmeltJob {
            player: player.to_string(),
            recipe_id: recipe_id.to_string(),
            output_block,
            output_count,
            started_secs: now_secs,
            dur_secs,
        };
        self.jobs.insert(id, job.clone());
        SmeltEvent {
            id,
            player: Some(job.player),
            recipe_id: Some(job.recipe_id),
            output_block: Some(output_block),
            output_count: Some(output_count),
            started_secs: Some(now_secs),
            dur_secs: Some(dur_secs),
        }
    }

    /// 目前無任何煨煮中的爐（供 tick 早退、省鎖）。
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// 回傳所有此刻已熟成（`now >= started + dur`）的爐（clone，供呼叫端鎖外交付）。
    pub fn ready(&self, now_secs: u64) -> Vec<(u64, SmeltJob)> {
        self.jobs
            .iter()
            .filter(|(_, j)| now_secs >= j.started_secs.saturating_add(j.dur_secs))
            .map(|(&id, j)| (id, j.clone()))
            .collect()
    }

    /// 移除一爐（交付後 / 取消）。原本存在才回 `Some(SmeltEvent)`（呼叫端 append）；
    /// 不存在 → `None`（已被別的 tick 交付，不落重複事件）。
    pub fn remove(&mut self, id: u64) -> Option<SmeltEvent> {
        if self.jobs.remove(&id).is_some() {
            Some(SmeltEvent {
                id,
                player: None,
                recipe_id: None,
                output_block: None,
                output_count: None,
                started_secs: None,
                dur_secs: None,
            })
        } else {
            None
        }
    }

    /// 由 jsonl 事件列表重建狀態（啟動時 replay，比照 `FarmStore::from_events`）。
    /// 開爐（欄位 Some）→ 登記該爐；移除（player None）→ 清掉該爐。
    pub fn from_events(events: Vec<SmeltEvent>) -> Self {
        let mut store = SmeltStore::default();
        for e in &events {
            match (
                e.player.as_ref(),
                e.recipe_id.as_ref(),
                e.output_block,
                e.output_count,
                e.started_secs,
                e.dur_secs,
            ) {
                (Some(player), Some(recipe_id), Some(ob), Some(oc), Some(ss), Some(ds)) => {
                    store.jobs.insert(
                        e.id,
                        SmeltJob {
                            player: player.clone(),
                            recipe_id: recipe_id.clone(),
                            output_block: ob,
                            output_count: oc,
                            started_secs: ss,
                            dur_secs: ds,
                        },
                    );
                }
                _ => {
                    store.jobs.remove(&e.id);
                }
            }
            if e.id >= store.next_id {
                store.next_id = e.id + 1;
            }
        }
        store
    }
}

// ── jsonl 持久化（比照 voxel_farm：輕量同步小檔 append，不持任何鎖）─────────────

/// 把一筆 SmeltEvent append 到 jsonl（呼叫端須已釋放 smelt 鎖；失敗只記 log、不 panic）。
pub fn append_smelt(event: &SmeltEvent) {
    let Ok(val) = serde_json::to_value(event) else {
        return;
    };
    write_smelt_line(VOXEL_SMELT_PATH, &val);
}

/// 從 jsonl 載回所有事件（啟動時呼叫一次）。檔不存在 / 壞行皆容忍。
pub fn load_smelt() -> Vec<SmeltEvent> {
    let Ok(content) = std::fs::read_to_string(VOXEL_SMELT_PATH) else {
        return vec![];
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<SmeltEvent>(line).ok())
        .collect()
}

fn write_smelt_line(path: &str, record: &serde_json::Value) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        if let Ok(line) = serde_json::to_string(record) {
            let _ = writeln!(f, "{}", line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_layered_and_positive() {
        // 三種節奏皆正值，且食物 < 建材 < 鐵（快/中/慢分層）。
        let food = smelt_secs("smelt_fish");
        let build = smelt_secs("smelt_glass");
        let iron = smelt_secs("smelt_iron");
        assert!(food > 0 && build > 0 && iron > 0);
        assert!(food < build, "食物應比建材快：{food} < {build}");
        assert!(build < iron, "鐵應比建材慢：{build} < {iron}");
        // 未知配方走預設（不 panic、正值）。
        assert!(smelt_secs("unknown") > 0);
        // 兩種食物同節奏。
        assert_eq!(smelt_secs("smelt_fish"), smelt_secs("smelt_potato"));
        // 莓果醬（808）：正值、比烤魚稍久（熬煮多點耐心），但仍比鐵快得多。
        let jam = smelt_secs("smelt_jam");
        assert!(jam > 0, "莓果醬煨煮秒數應為正值");
        assert!(jam > food, "莓果醬熬煮應比烤魚稍久：{jam} > {food}");
        assert!(jam < iron, "莓果醬仍是甜點小食、應比鐵快：{jam} < {iron}");
    }

    #[test]
    fn start_assigns_increasing_ids_and_returns_event() {
        let mut s = SmeltStore::new();
        let e0 = s.start("阿明", "smelt_fish", 63, 1, 1000, 18);
        let e1 = s.start("阿明", "smelt_potato", 64, 1, 1000, 18);
        assert_eq!(e0.id, 0);
        assert_eq!(e1.id, 1);
        assert_eq!(s.next_id, 2);
        // 開爐事件欄位齊全。
        assert_eq!(e0.player.as_deref(), Some("阿明"));
        assert_eq!(e0.output_block, Some(63));
        assert_eq!(e0.dur_secs, Some(18));
        assert!(!s.is_empty());
    }

    #[test]
    fn ready_only_after_duration() {
        let mut s = SmeltStore::new();
        s.start("小美", "smelt_fish", 63, 1, 1000, 18);
        // 還沒到：started(1000)+dur(18)=1018。
        assert!(s.ready(1017).is_empty(), "未到熟成時刻不該交付");
        // 剛好到：now == 1018。
        assert_eq!(s.ready(1018).len(), 1, "熟成時刻應可交付");
        // 遠超過也算熟成（離線很久回來仍收得到）。
        assert_eq!(s.ready(9999).len(), 1);
    }

    #[test]
    fn remove_is_idempotent() {
        let mut s = SmeltStore::new();
        let e = s.start("客", "smelt_iron", 22, 2, 500, 30);
        let done = s.remove(e.id);
        assert!(done.is_some(), "移除既有爐應回事件");
        assert!(done.unwrap().player.is_none(), "移除事件 player 應為 None");
        assert!(s.is_empty());
        // 再移除同一 id → None（不落重複事件，防兩個 tick 同時交付）。
        assert!(s.remove(e.id).is_none());
    }

    #[test]
    fn replay_reconstructs_pending_and_clears_delivered() {
        // 模擬一段 append-only 事件史：開三爐、交付其中一爐。
        let mut s = SmeltStore::new();
        let a = s.start("甲", "smelt_fish", 63, 1, 100, 18);
        let b = s.start("乙", "smelt_potato", 64, 1, 100, 18);
        let _c = s.start("丙", "smelt_iron", 22, 2, 100, 30);
        let done_b = s.remove(b.id).unwrap();
        let history = vec![
            a.clone(),
            b.clone(),
            _c.clone(),
            done_b.clone(),
        ];
        let rebuilt = SmeltStore::from_events(history);
        // 甲、丙仍在煨煮；乙已交付被清。
        let ready = rebuilt.ready(9999);
        assert_eq!(ready.len(), 2, "應剩兩爐未交付（甲、丙）");
        let players: Vec<&str> = ready.iter().map(|(_, j)| j.player.as_str()).collect();
        assert!(players.contains(&"甲"));
        assert!(players.contains(&"丙"));
        assert!(!players.contains(&"乙"), "已交付的乙不該重建");
        // next_id 續號正確（最大 id + 1）。
        assert_eq!(rebuilt.next_id, s.next_id);
    }

    #[test]
    fn replay_of_empty_history_is_empty() {
        let rebuilt = SmeltStore::from_events(vec![]);
        assert!(rebuilt.is_empty());
        assert_eq!(rebuilt.next_id, 0);
    }
}
