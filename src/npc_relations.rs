//! NPC 人際關係網（ROADMAP 70：湧現派系第二塊）。
//!
//! 每對 NPC 之間有一個好惡值（0~100），50 = 中性，>70 = 溫暖親近，<30 = 緊張疏遠。
//! 世界事件影響關係（共患難加深信任；壓力事件帶來摩擦），
//! 對話時注入 system prompt，讓 NPC 談到彼此時語氣自然反映關係遠近。
//!
//! 設計鐵律：
//! - 純記憶體模式，重啟清零。
//! - 零 DB migration，純邏輯可獨立測試。
//! - LLM 仍只生成文字，關係值只影響 system prompt 語境，碰不到遊戲狀態。

use std::collections::HashMap;

const NEUTRAL: i32 = 50;
/// 每次 `tick_decay_all` 向中性靠近的步長（比需求衰減慢）。
const DECAY_STEP: i32 = 1;
/// game.rs 呼叫 `tick_decay_all` 的週期（秒）。關係比情緒持久，衰減更慢。
pub const DECAY_INTERVAL_SECS: u64 = 300; // 每 5 分鐘
/// prompt 只列入好惡差距超過此閾值的關係（避免無意義的 50/100 汙染 prompt）。
const NOTABLE_THRESHOLD: i32 = 12;

/// 七大 NPC ID（確保初始化一致性）。
const ALL_NPCS: &[&str] = &[
    "merchant",
    "workshop_npc",
    "bounty_npc",
    "expedition_npc",
    "procurement_npc",
    "farm_fair_npc",
    "village_chief",
];

/// 觸發關係調整的世界事件（與 npc_needs::NeedsEvent 同源，但獨立定義避免耦合）。
#[derive(Debug, Clone, Copy)]
pub enum RelationsEvent {
    RiftOpened,
    HordeArriving,
    HordeRepelled,
    QuestCompleted,
    VillageFestival,
    EliteSlain,
}

/// 所有 NPC 對之間的好惡狀態（記憶體模式，重啟清零）。
#[derive(Default)]
pub struct NpcRelationsState {
    /// `affinities[A][B]` = A 對 B 的好惡值（0~100）。關係不一定對稱。
    affinities: HashMap<String, HashMap<String, i32>>,
}

impl NpcRelationsState {
    /// 初始化七大 NPC 的人際關係，反映各自的個性與職務。
    pub fn new() -> Self {
        let mut s = Self::default();

        // 全部對先設為中性，再覆蓋有特色的
        for &a in ALL_NPCS {
            for &b in ALL_NPCS {
                if a != b {
                    s.set(a, b, NEUTRAL);
                }
            }
        }

        // 村落骨幹：凱爾長老 ↔ 卡特（共同守護鄉土，情誼深厚）
        s.set("village_chief", "farm_fair_npc", 75);
        s.set("farm_fair_npc", "village_chief", 75);

        // 里長寬厚，對所有居民略高於中性
        s.set("village_chief", "merchant", 65);
        s.set("village_chief", "workshop_npc", 65);
        s.set("village_chief", "bounty_npc", 65);
        s.set("village_chief", "expedition_npc", 65);
        s.set("village_chief", "procurement_npc", 60);

        // 商人敬重里長，但私下擔心金庫花費——關係略複雜（略低中性以上）
        s.set("merchant", "village_chief", 60);
        // 商人與採購代理人：互為同業，相互賞識
        s.set("merchant", "procurement_npc", 68);
        s.set("procurement_npc", "merchant", 68);

        // 工匠鐸恩 ↔ 卡特：踏實的土地人，惺惺相惜
        s.set("workshop_npc", "farm_fair_npc", 68);
        s.set("farm_fair_npc", "workshop_npc", 68);

        // 獵手 ↔ 探勘員：同樣在外闖蕩，自然親近
        s.set("bounty_npc", "expedition_npc", 72);
        s.set("expedition_npc", "bounty_npc", 72);

        // 獵手與工匠：行事風格迥異，略感距離
        s.set("bounty_npc", "workshop_npc", 43);
        s.set("workshop_npc", "bounty_npc", 43);

        s
    }

    fn set(&mut self, from: &str, to: &str, val: i32) {
        self.affinities
            .entry(from.to_string())
            .or_default()
            .insert(to.to_string(), val.clamp(0, 100));
    }

    fn adjust(&mut self, from: &str, to: &str, delta: i32) {
        let v = self.affinities
            .entry(from.to_string())
            .or_default()
            .entry(to.to_string())
            .or_insert(NEUTRAL);
        *v = (*v + delta).clamp(0, 100);
    }

    /// 取得 A 對 B 的好惡值；未知 NPC 對回 `None`。
    pub fn get(&self, from: &str, to: &str) -> Option<i32> {
        self.affinities.get(from)?.get(to).copied()
    }

    /// 世界事件發生，調整相關 NPC 對之間的好惡值。
    pub fn apply_world_event(&mut self, event: RelationsEvent) {
        match event {
            RelationsEvent::HordeRepelled => {
                // 共同打退獸潮——共患難加深信任，全體略感親近
                self.adjust("bounty_npc", "village_chief", 12);
                self.adjust("village_chief", "bounty_npc", 12);
                self.adjust("bounty_npc", "expedition_npc", 8);
                self.adjust("expedition_npc", "bounty_npc", 8);
                for &a in ALL_NPCS {
                    for &b in ALL_NPCS {
                        if a != b {
                            self.adjust(a, b, 2);
                        }
                    }
                }
            }
            RelationsEvent::HordeArriving => {
                // 危機壓力帶出小摩擦——商人擔心防禦支出
                self.adjust("merchant", "village_chief", -5);
                self.adjust("merchant", "workshop_npc", -3);
            }
            RelationsEvent::VillageFestival => {
                // 節慶共享喜悅，里長與卡特形象上升
                for &a in ALL_NPCS {
                    if a != "village_chief" {
                        self.adjust(a, "village_chief", 5);
                    }
                    if a != "farm_fair_npc" {
                        self.adjust(a, "farm_fair_npc", 3);
                    }
                }
                self.adjust("village_chief", "farm_fair_npc", 8);
            }
            RelationsEvent::QuestCompleted => {
                // 任務完成，里長與卡特對執行任務的居民印象提升
                self.adjust("village_chief", "bounty_npc", 5);
                self.adjust("farm_fair_npc", "bounty_npc", 5);
                self.adjust("village_chief", "expedition_npc", 3);
            }
            RelationsEvent::EliteSlain => {
                // 精英被討伐——獵手蘭卡聲望上升，各 NPC 對獵手好感提升
                for &a in ALL_NPCS {
                    if a != "bounty_npc" {
                        self.adjust(a, "bounty_npc", 6);
                    }
                }
            }
            RelationsEvent::RiftOpened => {
                // 裂縫開啟——商人擔心里長沒準備好，但對獵手好感提升（覺得他能派上用場）
                self.adjust("merchant", "village_chief", -4);
                self.adjust("merchant", "bounty_npc", 5);
                self.adjust("expedition_npc", "bounty_npc", 4); // 探勘員也覺得獵手重要
            }
        }
    }

    /// 所有關係值緩慢向中性（50）靠近（由 game.rs 每 DECAY_INTERVAL_SECS 呼叫）。
    pub fn tick_decay_all(&mut self) {
        for inner in self.affinities.values_mut() {
            for v in inner.values_mut() {
                if *v > NEUTRAL {
                    *v -= DECAY_STEP;
                } else if *v < NEUTRAL {
                    *v += DECAY_STEP;
                }
            }
        }
    }

    /// 組 system prompt 段落：列出此 NPC 最值得注意的關係（只列顯著非中性的）。
    /// 回空字串表示關係皆接近中性，不注入 prompt，不汙染。
    pub fn to_prompt_section(&self, speaker_id: &str) -> String {
        let inner = match self.affinities.get(speaker_id) {
            Some(m) => m,
            None => return String::new(),
        };

        // 篩選差距超過閾值的關係
        let mut notable: Vec<(&str, i32)> = inner
            .iter()
            .filter(|(_, &v)| (v - NEUTRAL).abs() > NOTABLE_THRESHOLD)
            .map(|(k, &v)| (k.as_str(), v))
            .collect();

        if notable.is_empty() {
            return String::new();
        }

        // 按差距降序，最多取 3 個
        notable.sort_by_key(|(_, v)| -((v - NEUTRAL).abs()));
        notable.truncate(3);

        let parts: Vec<String> = notable
            .iter()
            .map(|(other_id, affinity)| {
                let name = npc_display_name(other_id);
                let desc = affinity_desc(*affinity);
                format!("與{}：{}（{}/100）", name, desc, affinity)
            })
            .collect();

        format!(
            "\n\n【你與其他居民的關係（自然流露在提及他們時的口吻中，無需直說）】{}",
            parts.join("・")
        )
    }
}

/// NPC ID → 簡短顯示名（供 prompt 識別）。
fn npc_display_name(id: &str) -> &'static str {
    match id {
        "merchant"        => "商人薇拉",
        "workshop_npc"    => "工匠鐸恩",
        "bounty_npc"      => "獵手蘭卡",
        "expedition_npc"  => "探勘員芙利亞",
        "procurement_npc" => "採購代理人諾亞",
        "farm_fair_npc"   => "評審卡特",
        "village_chief"   => "凱爾長老",
        _                 => "某位居民",
    }
}

/// 好惡值 → 描述文字。
fn affinity_desc(v: i32) -> &'static str {
    if v >= 85 {
        "深厚信任"
    } else if v >= 70 {
        "溫暖親近"
    } else if v >= 58 {
        "互惠合作"
    } else if v <= 15 {
        "難以掩飾的緊張"
    } else if v <= 30 {
        "存在明顯隔閡"
    } else if v <= 42 {
        "略有摩擦"
    } else {
        "尚可"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_values_in_range() {
        let s = NpcRelationsState::new();
        for (a, inner) in &s.affinities {
            for (b, &v) in inner {
                assert!(
                    (0..=100).contains(&v),
                    "{a} 對 {b} 好惡值 {v} 超出範圍"
                );
            }
        }
    }

    #[test]
    fn seven_npcs_all_have_relations() {
        let s = NpcRelationsState::new();
        for &a in ALL_NPCS {
            assert!(s.affinities.contains_key(a), "{a} 應有關係資料");
            let inner = &s.affinities[a];
            for &b in ALL_NPCS {
                if a != b {
                    assert!(inner.contains_key(b), "{a} 應有對 {b} 的好惡值");
                }
            }
        }
    }

    #[test]
    fn chief_farmer_start_warm() {
        let s = NpcRelationsState::new();
        assert!(s.get("village_chief", "farm_fair_npc").unwrap() > 70);
        assert!(s.get("farm_fair_npc", "village_chief").unwrap() > 70);
    }

    #[test]
    fn hunter_explorer_start_warm() {
        let s = NpcRelationsState::new();
        assert!(s.get("bounty_npc", "expedition_npc").unwrap() > 70);
        assert!(s.get("expedition_npc", "bounty_npc").unwrap() > 70);
    }

    #[test]
    fn horde_repelled_raises_chief_hunter_affinity() {
        let mut s = NpcRelationsState::new();
        let prev = s.get("bounty_npc", "village_chief").unwrap();
        s.apply_world_event(RelationsEvent::HordeRepelled);
        assert!(s.get("bounty_npc", "village_chief").unwrap() > prev);
        assert!(s.get("village_chief", "bounty_npc").unwrap() > 65);
    }

    #[test]
    fn horde_arriving_lowers_merchant_chief_affinity() {
        let mut s = NpcRelationsState::new();
        let prev = s.get("merchant", "village_chief").unwrap();
        s.apply_world_event(RelationsEvent::HordeArriving);
        assert!(s.get("merchant", "village_chief").unwrap() < prev);
    }

    #[test]
    fn festival_raises_everyone_toward_chief() {
        let mut s = NpcRelationsState::new();
        let prev_bounty = s.get("bounty_npc", "village_chief").unwrap();
        s.apply_world_event(RelationsEvent::VillageFestival);
        assert!(s.get("bounty_npc", "village_chief").unwrap() > prev_bounty);
    }

    #[test]
    fn elite_slain_raises_everyone_toward_hunter() {
        let mut s = NpcRelationsState::new();
        let prev = s.get("merchant", "bounty_npc").unwrap();
        s.apply_world_event(RelationsEvent::EliteSlain);
        assert!(s.get("merchant", "bounty_npc").unwrap() > prev);
        assert!(s.get("village_chief", "bounty_npc").unwrap() > prev);
    }

    #[test]
    fn decay_moves_toward_neutral() {
        let mut s = NpcRelationsState::new();
        // 里長對卡特初始 75 → 應向 50 衰減
        let before = s.get("village_chief", "farm_fair_npc").unwrap();
        s.tick_decay_all();
        let after = s.get("village_chief", "farm_fair_npc").unwrap();
        assert_eq!(after, before - 1, "應往中性靠近 1");
    }

    #[test]
    fn decay_does_not_cross_neutral() {
        let mut s = NpcRelationsState::new();
        // 反覆衰減，不應越過 50
        for _ in 0..100 {
            s.tick_decay_all();
        }
        for inner in s.affinities.values() {
            for &v in inner.values() {
                assert!(v >= 40, "衰減不應過頭，值 = {v}");
            }
        }
    }

    #[test]
    fn clamping_prevents_out_of_range() {
        let mut s = NpcRelationsState::new();
        for _ in 0..20 {
            s.apply_world_event(RelationsEvent::HordeRepelled);
        }
        for inner in s.affinities.values() {
            for &v in inner.values() {
                assert!(v <= 100, "好惡值不應超過 100，值 = {v}");
                assert!(v >= 0, "好惡值不應低於 0，值 = {v}");
            }
        }
    }

    #[test]
    fn prompt_section_non_empty_for_notable_relation() {
        let s = NpcRelationsState::new();
        // 里長對卡特 75，應列入 prompt
        let section = s.to_prompt_section("village_chief");
        assert!(!section.is_empty(), "里長應有值得注意的關係");
        assert!(section.contains("卡特") || section.contains("凱爾"), "應提及相關 NPC");
    }

    #[test]
    fn prompt_section_empty_for_unknown_npc() {
        let s = NpcRelationsState::new();
        assert!(s.to_prompt_section("unknown_npc").is_empty());
    }
}
