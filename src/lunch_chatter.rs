//! 午休席間閒話（ROADMAP 328）。
//!
//! 327 讓七大 NPC 每到正午都聚到鎮中廣場圍桌用餐，但聚在一起後只是靜靜站著。
//! 本模組補上「席間你一言我一語」的社交層：聚食期間，圍桌的 NPC 偶爾冒出一句
//! 符合其性格的家常閒話，由前端畫成頭頂對話泡泡（`NpcSpeech`），讓正午的廣場
//! 看起來真的有人在搭話、在過日子。
//!
//! 與既有 76 廣場夜談 / 81 白日工位對話**刻意區隔**：
//! - 那兩者走 **LLM**、廣播到**世界聊天頻道**、覆蓋整個夜晚 / 白天、5 分鐘一次。
//! - 本模組是**零 LLM、純模板**、**只在正午聚食時**、**只在就地泡泡**（不洗聊天頻道）、
//!   節奏更密（約 11 秒一句），專為「圍桌共食」這個畫面服務。
//!
//! 成本紀律：零 LLM、零網路、純查表，背景生活不燒任何額度。

use crate::npc_schedule::VILLAGE_NPCS;

/// 伺服器啟動 / 進入午休後，首句閒話的最短等待（秒）——讓 NPC 先走到座位坐定再開口。
const FIRST_CHATTER_WAIT_SECS: f32 = 6.0;

/// 兩句席間閒話之間的冷卻（秒）。約一輪 60 秒的午休窗內可聽到約 5 句你來我往。
const LUNCH_CHATTER_COOLDOWN_SECS: f32 = 11.0;

/// 一句席間閒話：說話者穩定 id 與內容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LunchUtterance {
    pub speaker_id: &'static str,
    pub text: &'static str,
}

/// 各 NPC 的席間閒話模板池（零 LLM）。
///
/// 內容皆為「圍桌用餐時的家常話」，語氣輕鬆、扣合各自職業性格。集中於此處單一表，
/// 作為面向玩家字串的集中替換點（i18n 空間）；與既有 NPC 對話一律後端繁中的慣例一致。
static LUNCH_LINES: &[(&str, &[&str])] = &[
    ("merchant", &[
        "趁熱吃啊，下午還有一車貨要點呢。",
        "這湯熬得真夠味，比集市那攤強多了。",
        "歇會兒也好，做生意也得顧著肚子。",
        "我帶了點曬乾的果脯，誰要嚐嚐？",
    ]),
    ("workshop_npc", &[
        "手停一停，吃飽了敲起鐵來才有勁。",
        "這饃啃著扎實，跟我打的鐵一個脾氣。",
        "爐火我封好了，安心吃飯。",
        "午飯不能省，下午還得趕一爐活。",
    ]),
    ("bounty_npc", &[
        "東邊那群怪也該歇晌了吧，哈哈。",
        "吃飽才有力氣追賞，這肉燉得正好。",
        "難得坐下來，平日都在野地裡啃乾糧。",
        "誰把那碟醃菜遞過來？",
    ]),
    ("expedition_npc", &[
        "走了一上午的圖，這頓飯救命。",
        "下回探勘，我給大夥帶點山裡的野味。",
        "坐這兒看天，比盯著地圖舒坦多了。",
        "這口水甜，像極了北谷那眼泉。",
    ]),
    ("procurement_npc", &[
        "這批米是我上週調來的，不賴吧？",
        "吃著順口的，我多進些貨。",
        "跨星的香料，給湯添了點不一樣。",
        "午歇歸午歇，帳我可記著呢。",
    ]),
    ("farm_fair_npc", &[
        "今季的菜就是水靈，自家種的最香。",
        "這瓜給我打高分，又脆又甜。",
        "吃的都是大夥的收成，踏實。",
        "慢點吃，飯後我還要評後園那畦呢。",
    ]),
    ("village_chief", &[
        "難得人都齊了，多吃點，別客氣。",
        "村子有你們撐著，這頓我請。",
        "吃飽喝足，下午各忙各的去。",
        "看大夥圍坐一桌，這日子才像話。",
    ]),
];

/// 取得某 NPC 第 `slot` 句席間閒話（`slot` 在其模板池內循環）。純函式、可測。
/// 非村落 NPC（不在 `LUNCH_LINES`）回 `None`——只有圍桌的七大 NPC 會閒話。
pub fn lunch_line(npc_id: &str, slot: usize) -> Option<&'static str> {
    LUNCH_LINES
        .iter()
        .find(|(id, _)| *id == npc_id)
        .map(|(_, lines)| lines[slot % lines.len()])
}

/// 取得 NPC 顯示名稱（從 npc_chat 共用資料；與 plaza_talk / daytime_talk 一致）。
pub fn display_name(id: &str) -> &'static str {
    crate::npc_chat::find_npc(id).map(|n| n.display).unwrap_or("村民")
}

/// 午休席間閒話狀態（純記憶體，重啟清零）。
pub struct LunchChatterState {
    /// 距下句閒話倒數（秒）；只在午休時遞減。
    cooldown: f32,
    /// 輪替索引：決定下一個說話者與其取用的句子，逐句遞增。
    turn: usize,
}

impl LunchChatterState {
    pub fn new() -> Self {
        Self {
            cooldown: FIRST_CHATTER_WAIT_SECS,
            turn: 0,
        }
    }

    /// 推進時間。只在午休聚食時倒數；冷卻歸零時回傳一句席間閒話並重置冷卻。
    ///
    /// 說話者依 `VILLAGE_NPCS` 權威次序輪替（每句換一人），句子隨 `turn` 在各自池內推進，
    /// 讓席間你一言我一語、不重複、不獨白。非午休時段一律不發話、並把冷卻復位到首句等待，
    /// 讓下一場午休從頭乾淨開始。
    pub fn tick(&mut self, dt: f32, lunching: bool) -> Option<LunchUtterance> {
        if !lunching {
            // 午休結束：復位，等下一場正午重新起算。
            self.cooldown = FIRST_CHATTER_WAIT_SECS;
            return None;
        }
        self.cooldown -= dt;
        if self.cooldown > 0.0 {
            return None;
        }
        self.cooldown = LUNCH_CHATTER_COOLDOWN_SECS;
        let n = VILLAGE_NPCS.len();
        let speaker_id = VILLAGE_NPCS[self.turn % n].id;
        // 該說話者此輪取用的句子序號：每繞完一圈說話者，句子往後推一句。
        let line_slot = self.turn / n;
        let utterance = lunch_line(speaker_id, line_slot)
            .map(|text| LunchUtterance { speaker_id, text });
        self.turn = self.turn.wrapping_add(1);
        utterance
    }
}

impl Default for LunchChatterState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lunch_line_returns_some_for_all_village_npcs() {
        for s in VILLAGE_NPCS {
            assert!(lunch_line(s.id, 0).is_some(), "{} 應有席間閒話", s.id);
        }
    }

    #[test]
    fn lunch_line_returns_none_for_unknown_npc() {
        assert_eq!(lunch_line("traveler", 0), None);
        assert_eq!(lunch_line("unknown", 3), None);
    }

    #[test]
    fn lunch_line_cycles_within_pool() {
        // slot 超出池長度時循環回頭，不會 panic、且回到第一句。
        let first = lunch_line("merchant", 0).unwrap();
        // merchant 池有 4 句，slot=4 應繞回第一句。
        assert_eq!(lunch_line("merchant", 4), Some(first));
        // 池內每一句都取得到。
        for slot in 0..4 {
            assert!(lunch_line("merchant", slot).is_some());
        }
    }

    #[test]
    fn no_chatter_when_not_lunching() {
        let mut st = LunchChatterState::new();
        // 非午休：連跑多幀都不發話。
        for _ in 0..100 {
            assert_eq!(st.tick(1.0, false), None);
        }
    }

    #[test]
    fn eventually_chatters_during_lunch() {
        let mut st = LunchChatterState::new();
        let mut spoke = false;
        for _ in 0..30 {
            if let Some(u) = st.tick(1.0, true) {
                // 說話者必為村落七大 NPC。
                assert!(VILLAGE_NPCS.iter().any(|s| s.id == u.speaker_id));
                assert!(!u.text.is_empty());
                spoke = true;
                break;
            }
        }
        assert!(spoke, "午休期間應在數秒內冒出席間閒話");
    }

    #[test]
    fn chatter_rotates_speakers() {
        // 連續觸發應輪替不同說話者（不獨白）。
        let mut st = LunchChatterState::new();
        let mut speakers = Vec::new();
        // 跑足夠久收集多句（首句等待 6s + 每句冷卻 11s）。
        for _ in 0..200 {
            if let Some(u) = st.tick(1.0, true) {
                speakers.push(u.speaker_id);
            }
            if speakers.len() >= 3 {
                break;
            }
        }
        assert!(speakers.len() >= 3, "應在期限內收集到至少三句");
        // 連續兩句說話者不同（輪替）。
        assert_ne!(speakers[0], speakers[1]);
        assert_ne!(speakers[1], speakers[2]);
    }

    #[test]
    fn cooldown_resets_after_lunch_ends() {
        // 午休中發了一句後，午休結束→復位；下一場午休仍需等首句等待才再發話。
        let mut st = LunchChatterState::new();
        // 跑到第一句出現。
        let mut fired = false;
        for _ in 0..30 {
            if st.tick(1.0, true).is_some() { fired = true; break; }
        }
        assert!(fired);
        // 午休結束復位。
        assert_eq!(st.tick(1.0, false), None);
        // 進入新一場午休，前幾幀（< 首句等待）不應立刻發話。
        assert_eq!(st.tick(1.0, true), None);
    }
}
