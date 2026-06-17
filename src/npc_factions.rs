//! NPC 派系自主湧現（ROADMAP 71：湧現派系第三塊）。
//!
//! 基於 NPC 人際關係網（ROADMAP 70）的好惡值，
//! 當兩個 NPC 的雙向平均好惡值越過門檻時，引擎偵測到「派系事件」並廣播到聊天頻道——
//! 讓玩家看到村落的社會結構自然湧現，而非由設計者寫死。
//!
//! 設計鐵律：
//! - 純邏輯，無 LLM 呼叫（LLM 影響對話語境，派系事件廣播由引擎純文字處理）。
//! - 零 DB migration，純記憶體模式，重啟清零（派系從當前關係值重新湧現）。
//! - 引擎事件廣播到 tx_chat，不阻塞 15Hz 迴圈。
//! - 每對 NPC 有冷卻時間，避免頻繁刷頻。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::npc_relations::NpcRelationsState;

/// 兩個 NPC 雙向平均好惡值 >= 此閾值 → 宣告結盟。
const ALLIANCE_THRESHOLD: i32 = 80;
/// 雙向平均好惡值 <= 此閾值 → 宣告競爭對立。
const RIVALRY_THRESHOLD: i32 = 22;
/// 同一對 NPC 宣告同一種派系事件的最短間隔（秒）。防洗頻。
pub const FACTION_ANNOUNCE_COOLDOWN_SECS: u64 = 900; // 15 分鐘

/// 七大 NPC ID（與 npc_relations.rs 一致）。
const ALL_NPCS: &[&str] = &[
    "merchant",
    "workshop_npc",
    "bounty_npc",
    "expedition_npc",
    "procurement_npc",
    "farm_fair_npc",
    "village_chief",
];

/// 派系關係類型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactionBond {
    Alliance,
    Rivalry,
    Neutral,
}

impl FactionBond {
    /// 對前端的穩定 wire 字串（ROADMAP 355）。前端依此鏡像顯示圖示／文案，後端不送面向玩家文字，
    /// 保留 i18n 空間。`Neutral` 不會被送到前端（`current_standings` 只收錄非中性配對）。
    pub fn wire_key(self) -> &'static str {
        match self {
            FactionBond::Alliance => "alliance",
            FactionBond::Rivalry => "rivalry",
            FactionBond::Neutral => "neutral",
        }
    }
}

/// 鎮民派系一覽中的一筆「當前現況」配對（ROADMAP 355）。
///
/// 與 `FactionEvent`（偵測「變化」、帶冷卻、用於一次性廣播）不同：本結構是「此刻完整現況」的快照，
/// 純讀取、無狀態、無冷卻——供 snapshot 持續送給前端「鎮民派系」面板顯示，讓玩家隨時看得到
/// 村落社會結構的當下樣貌（不必剛好在那 15 分鐘一次的廣播瞬間在線）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactionStanding {
    pub npc_a: &'static str,
    pub npc_b: &'static str,
    pub bond: FactionBond,
    /// 雙向平均好惡值（0–100）；結盟越高越鐵、敵對越低越僵。
    pub affinity: i32,
}

/// 計算「此刻」七大 NPC 之間所有明顯的結盟／敵對配對（ROADMAP 355）。
///
/// 純函式、只讀關係、確定性可測：
/// - 只掃有序對（`ALL_NPCS` 次序，`a` 在 `b` 前），同一對只算一次、不重複。
/// - 只收錄非 `Neutral`（結盟或敵對）——和平相處的配對不入列，面板才不被中性對子灌爆。
/// - 排序：結盟（高 avg）在前、敵對（低 avg）在後；結盟內由親到疏、敵對內由僵到緩，
///   讓面板最上是「最鐵的盟友」、最下是「最深的對立」，閱讀次序穩定。
pub fn current_standings(relations: &NpcRelationsState) -> Vec<FactionStanding> {
    let mut out: Vec<FactionStanding> = Vec::new();
    for i in 0..ALL_NPCS.len() {
        for j in (i + 1)..ALL_NPCS.len() {
            let a = ALL_NPCS[i];
            let b = ALL_NPCS[j];
            let avg = mutual_avg(relations, a, b);
            let bond = if avg >= ALLIANCE_THRESHOLD {
                FactionBond::Alliance
            } else if avg <= RIVALRY_THRESHOLD {
                FactionBond::Rivalry
            } else {
                FactionBond::Neutral
            };
            if bond == FactionBond::Neutral {
                continue;
            }
            out.push(FactionStanding { npc_a: a, npc_b: b, bond, affinity: avg });
        }
    }
    out.sort_by(|x, y| {
        // 結盟排在敵對前。
        let rank = |b: FactionBond| match b {
            FactionBond::Alliance => 0,
            FactionBond::Rivalry => 1,
            FactionBond::Neutral => 2,
        };
        rank(x.bond).cmp(&rank(y.bond)).then_with(|| match x.bond {
            // 結盟：親密（高）在前；敵對：最僵（低）在前。
            FactionBond::Alliance => y.affinity.cmp(&x.affinity),
            _ => x.affinity.cmp(&y.affinity),
        })
    });
    out
}

/// 一次派系事件（引擎偵測到後廣播到聊天頻道）。
#[derive(Debug, Clone)]
pub struct FactionEvent {
    pub npc_a: String,
    pub npc_b: String,
    pub bond: FactionBond,
}

impl FactionEvent {
    /// 生成聊天廣播文字。Neutral 回空字串（不廣播）。
    pub fn announce_text(&self) -> String {
        let name_a = npc_display_name(&self.npc_a);
        let name_b = npc_display_name(&self.npc_b);
        match self.bond {
            FactionBond::Alliance => {
                format!(
                    "🤝 [村落派系] {} 與 {} 建立了公認的夥伴關係！",
                    name_a, name_b
                )
            }
            FactionBond::Rivalry => {
                format!(
                    "⚡ [村落派系] {} 與 {} 之間的分歧已人盡皆知...",
                    name_a, name_b
                )
            }
            FactionBond::Neutral => String::new(),
        }
    }
}

/// 追蹤已宣告的派系事件，防止在冷卻期內重複廣播（記憶體模式，重啟清零）。
#[derive(Default)]
pub struct NpcFactionState {
    /// 已宣告的派系鍵：(字母序較小的 NPC, 另一個 NPC) → (上次宣告的 bond 類型, 時間)。
    last_announced: HashMap<(String, String), (FactionBond, Instant)>,
}

impl NpcFactionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 基於當前關係值，偵測新出現的（或已超過冷卻期的）派系事件。
    ///
    /// 不修改 relations，只讀取。呼叫端在 tick 後呼叫此方法，
    /// 回傳的每個 FactionEvent 都應廣播到 tx_chat。
    pub fn detect_changes(&mut self, relations: &NpcRelationsState) -> Vec<FactionEvent> {
        let now = Instant::now();
        let cooldown = Duration::from_secs(FACTION_ANNOUNCE_COOLDOWN_SECS);
        let mut events = Vec::new();

        // 只掃有序對（字母序），避免 (A,B) 和 (B,A) 重複處理
        for i in 0..ALL_NPCS.len() {
            for j in (i + 1)..ALL_NPCS.len() {
                let a = ALL_NPCS[i];
                let b = ALL_NPCS[j];

                let avg = mutual_avg(relations, a, b);
                let current_bond = if avg >= ALLIANCE_THRESHOLD {
                    FactionBond::Alliance
                } else if avg <= RIVALRY_THRESHOLD {
                    FactionBond::Rivalry
                } else {
                    FactionBond::Neutral
                };

                if current_bond == FactionBond::Neutral {
                    continue;
                }

                let key = (a.to_string(), b.to_string());
                let should_announce = match self.last_announced.get(&key) {
                    None => true,
                    Some((last_bond, last_time)) => {
                        // 同類型但已超過冷卻，或類型改變（中性→結盟/競爭，或結盟→競爭）
                        *last_bond != current_bond || now.duration_since(*last_time) >= cooldown
                    }
                };

                if should_announce {
                    self.last_announced.insert(key, (current_bond, now));
                    events.push(FactionEvent {
                        npc_a: a.to_string(),
                        npc_b: b.to_string(),
                        bond: current_bond,
                    });
                }
            }
        }

        events
    }

    /// 產生 system prompt 段落：此 NPC 目前已公開的派系關係。
    ///
    /// 只列涉及 `speaker_id` 的對子；空字串表示無公開派系，不注入 prompt。
    pub fn to_prompt_section(&self, speaker_id: &str) -> String {
        let mut parts: Vec<String> = self
            .last_announced
            .iter()
            .filter_map(|((a, b), (bond, _))| {
                let other_id = if a == speaker_id {
                    b.as_str()
                } else if b == speaker_id {
                    a.as_str()
                } else {
                    return None;
                };
                let other_name = npc_display_name(other_id);
                let desc = match bond {
                    FactionBond::Alliance => format!("{}：公認盟友", other_name),
                    FactionBond::Rivalry => format!("{}：公開摩擦", other_name),
                    FactionBond::Neutral => return None,
                };
                Some(desc)
            })
            .collect();

        if parts.is_empty() {
            return String::new();
        }

        parts.sort(); // 排序讓 prompt 穩定
        format!(
            "\n\n【你目前已公開的派系關係（自然流露在提及他們時的口吻中，無需直說）】{}",
            parts.join("・")
        )
    }
}

/// 計算兩個 NPC 之間的雙向平均好惡值。
pub fn mutual_avg(relations: &NpcRelationsState, a: &str, b: &str) -> i32 {
    let a_to_b = relations.get(a, b).unwrap_or(50);
    let b_to_a = relations.get(b, a).unwrap_or(50);
    (a_to_b + b_to_a) / 2
}

/// NPC ID → 顯示名（與 npc_relations.rs 保持一致）。
pub fn npc_display_name(id: &str) -> &'static str {
    match id {
        "merchant" => "商人薇拉",
        "workshop_npc" => "工匠鐸恩",
        "bounty_npc" => "獵手蘭卡",
        "expedition_npc" => "探勘員芙利亞",
        "procurement_npc" => "採購代理人諾亞",
        "farm_fair_npc" => "評審卡特",
        "village_chief" => "凱爾長老",
        _ => "某位居民",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutual_avg_symmetric() {
        let r = NpcRelationsState::new();
        // bounty_npc ↔ expedition_npc 初始均為 72
        let avg_ab = mutual_avg(&r, "bounty_npc", "expedition_npc");
        let avg_ba = mutual_avg(&r, "expedition_npc", "bounty_npc");
        assert_eq!(avg_ab, avg_ba, "雙向平均值應具對稱性");
    }

    #[test]
    fn mutual_avg_correct_value() {
        let r = NpcRelationsState::new();
        // bounty → expedition = 72, expedition → bounty = 72 → avg = 72
        let avg = mutual_avg(&r, "bounty_npc", "expedition_npc");
        assert_eq!(avg, 72);
    }

    #[test]
    fn no_event_for_initial_state() {
        // 初始關係值均在 43~75 之間，未超過 80（結盟）或低於 22（競爭）
        let r = NpcRelationsState::new();
        let mut fs = NpcFactionState::new();
        let events = fs.detect_changes(&r);
        assert!(events.is_empty(), "初始狀態不應有派系事件，最高好惡值 75 < 80");
    }

    #[test]
    fn alliance_detected_after_horde_repelled() {
        // HordeRepelled：bounty↔expedition 各 +8 直接 + +2 全域 = +10
        // 初始 72 + 10 = 82 ≥ 80 → 應觸發結盟
        let mut r = NpcRelationsState::new();
        r.apply_world_event(crate::npc_relations::RelationsEvent::HordeRepelled);
        let mut fs = NpcFactionState::new();
        let events = fs.detect_changes(&r);
        let has_alliance = events.iter().any(|e| {
            e.bond == FactionBond::Alliance
                && ((e.npc_a == "bounty_npc" && e.npc_b == "expedition_npc")
                    || (e.npc_a == "expedition_npc" && e.npc_b == "bounty_npc"))
        });
        assert!(has_alliance, "獵手↔探勘員打退獸潮後應建立結盟");
    }

    #[test]
    fn no_rivalry_in_initial_state() {
        // 初始最低好惡值為 43（bounty↔workshop），高於競爭門檻 22
        let r = NpcRelationsState::new();
        let mut fs = NpcFactionState::new();
        let events = fs.detect_changes(&r);
        let has_rivalry = events.iter().any(|e| e.bond == FactionBond::Rivalry);
        assert!(!has_rivalry, "初始狀態不應有競爭對立事件");
    }

    #[test]
    fn standings_empty_in_initial_state() {
        // ROADMAP 355：初始好惡值皆在中性帶（43~75），無結盟（≥80）也無敵對（≤22）。
        let r = NpcRelationsState::new();
        assert!(current_standings(&r).is_empty(), "初始村民相處平和，現況快照應為空");
    }

    #[test]
    fn standings_pick_up_alliance() {
        // 打退獸潮後 獵手↔探勘員 升到 82 ≥ 80 → 現況應出現一筆結盟（含正確 affinity）。
        let mut r = NpcRelationsState::new();
        r.apply_world_event(crate::npc_relations::RelationsEvent::HordeRepelled);
        let standings = current_standings(&r);
        let allied = standings.iter().find(|s| {
            s.bond == FactionBond::Alliance
                && ((s.npc_a == "bounty_npc" && s.npc_b == "expedition_npc")
                    || (s.npc_a == "expedition_npc" && s.npc_b == "bounty_npc"))
        });
        let allied = allied.expect("打退獸潮後獵手↔探勘員應結盟");
        assert!(allied.affinity >= ALLIANCE_THRESHOLD, "結盟 affinity 應 ≥ 門檻");
    }

    #[test]
    fn standings_dedup_ordered_pairs() {
        // 同一對 NPC 只算一次（有序對），不會 (A,B) 與 (B,A) 各一筆。
        let mut r = NpcRelationsState::new();
        r.apply_world_event(crate::npc_relations::RelationsEvent::HordeRepelled);
        let standings = current_standings(&r);
        for s in &standings {
            let ia = ALL_NPCS.iter().position(|x| *x == s.npc_a).unwrap();
            let ib = ALL_NPCS.iter().position(|x| *x == s.npc_b).unwrap();
            assert!(ia < ib, "每筆配對皆應 a 在 b 之前（有序、不重複）");
        }
    }

    #[test]
    fn standings_alliance_before_rivalry_sorted() {
        // 結盟排在敵對之前；結盟內由親到疏。手動把關係值推成「一筆結盟 + 一筆敵對」。
        let mut r = NpcRelationsState::new();
        r.set_pair_for_test("merchant", "workshop_npc", 95, 95); // → 結盟
        r.set_pair_for_test("bounty_npc", "farm_fair_npc", 5, 5); // → 敵對
        let standings = current_standings(&r);
        assert!(standings.len() >= 2);
        assert_eq!(standings[0].bond, FactionBond::Alliance, "首筆應為結盟");
        assert_eq!(standings.last().unwrap().bond, FactionBond::Rivalry, "末筆應為敵對");
    }

    #[test]
    fn bond_wire_key_stable() {
        assert_eq!(FactionBond::Alliance.wire_key(), "alliance");
        assert_eq!(FactionBond::Rivalry.wire_key(), "rivalry");
        assert_eq!(FactionBond::Neutral.wire_key(), "neutral");
    }

    #[test]
    fn cooldown_prevents_repeat_announcement() {
        let mut r = NpcRelationsState::new();
        r.apply_world_event(crate::npc_relations::RelationsEvent::HordeRepelled);
        let mut fs = NpcFactionState::new();
        let first = fs.detect_changes(&r);
        assert!(!first.is_empty(), "第一次應觸發事件");
        let second = fs.detect_changes(&r);
        assert!(second.is_empty(), "冷卻期內不應重複宣告同一派系事件");
    }

    #[test]
    fn alliance_announce_text_has_emoji() {
        let event = FactionEvent {
            npc_a: "bounty_npc".to_string(),
            npc_b: "expedition_npc".to_string(),
            bond: FactionBond::Alliance,
        };
        let text = event.announce_text();
        assert!(!text.is_empty());
        assert!(text.contains("🤝"), "結盟文字應含握手 emoji");
        assert!(text.contains("蘭卡") || text.contains("獵手"), "應提及獵手");
        assert!(text.contains("芙利亞") || text.contains("探勘"), "應提及探勘員");
    }

    #[test]
    fn rivalry_announce_text_has_emoji() {
        let event = FactionEvent {
            npc_a: "bounty_npc".to_string(),
            npc_b: "workshop_npc".to_string(),
            bond: FactionBond::Rivalry,
        };
        let text = event.announce_text();
        assert!(!text.is_empty());
        assert!(text.contains("⚡"), "競爭文字應含閃電 emoji");
    }

    #[test]
    fn neutral_announce_text_is_empty() {
        let event = FactionEvent {
            npc_a: "merchant".to_string(),
            npc_b: "workshop_npc".to_string(),
            bond: FactionBond::Neutral,
        };
        assert!(event.announce_text().is_empty(), "中性事件不應廣播");
    }

    #[test]
    fn prompt_section_lists_alliance_for_involved_npc() {
        let mut r = NpcRelationsState::new();
        r.apply_world_event(crate::npc_relations::RelationsEvent::HordeRepelled);
        let mut fs = NpcFactionState::new();
        let _ = fs.detect_changes(&r);

        let section = fs.to_prompt_section("bounty_npc");
        assert!(!section.is_empty(), "獵手應有派系關係段落");
        assert!(section.contains("芙利亞"), "應提及探勘員芙利亞");
        assert!(section.contains("盟友") || section.contains("夥伴"), "應標示為盟友");
    }

    #[test]
    fn prompt_section_empty_for_uninvolved_npc() {
        let mut r = NpcRelationsState::new();
        r.apply_world_event(crate::npc_relations::RelationsEvent::HordeRepelled);
        let mut fs = NpcFactionState::new();
        let _ = fs.detect_changes(&r);

        // merchant 未捲入獵手↔探勘員的結盟，其段落不應提及探勘員
        let section = fs.to_prompt_section("merchant");
        assert!(
            !section.contains("芙利亞"),
            "商人段落不應提及非其夥伴的探勘員"
        );
    }

    #[test]
    fn prompt_section_empty_when_no_events() {
        let r = NpcRelationsState::new();
        let mut fs = NpcFactionState::new();
        let _ = fs.detect_changes(&r); // 初始無事件
        assert!(
            fs.to_prompt_section("village_chief").is_empty(),
            "無派系事件時應回空段落"
        );
    }

    #[test]
    fn threshold_constants_sane() {
        assert!(
            ALLIANCE_THRESHOLD > 50,
            "結盟門檻應高於中性（50）"
        );
        assert!(
            RIVALRY_THRESHOLD < 50,
            "競爭門檻應低於中性（50）"
        );
        assert!(
            ALLIANCE_THRESHOLD > RIVALRY_THRESHOLD,
            "結盟門檻應高於競爭門檻"
        );
    }
}
