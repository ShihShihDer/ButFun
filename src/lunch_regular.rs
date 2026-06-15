//! 餐桌熟客（ROADMAP 330）：玩家↔NPC 的第一筆「相熟度」。
//!
//! 329 讓玩家能在午休席間向就座的 NPC 舉杯，NPC 回敬一句——可那份互動是「打過就忘」的：
//! 不管你是頭一回入席、還是天天來同一桌乾杯，NPC 的回敬都一個樣。這個切片把那份互動
//! 「累積」起來：每次你向某個 NPC 舉杯都記一筆，舉得多了，這位 NPC 就跟你愈來愈熟——
//! 從客套的招呼（生面孔）→ 漸熟的寒暄（點頭之交）→ 熱絡的老友乾杯（餐桌熟客），
//! 跨進新一層交情時還會冒出一句專屬的「混熟了」台詞。這是世界第一次「記得你做過什麼、
//! 並因此待你不同」——玩家的社交行為頭一回有了看得見的累積回報。
//!
//! 設計鐵律（與 `npc_relations`〔NPC↔NPC 派系好惡〕刻意區隔——那是 NPC 彼此間的關係，
//! 這是玩家↔NPC 的相熟度，兩條獨立的線）：
//! - 純記憶體模式，重啟清零（與 `npc_relations` / `lunch_chatter` 同調，零 DB／零 migration）。
//! - 零 LLM、純查表；相熟度只挑「回敬哪一句」，碰不到任何遊戲狀態（乙太／背包／戰力）。
//! - 純邏輯可獨立測試。面向玩家字串集中於本檔作為 i18n 替換點。

use std::collections::HashMap;

/// 累積到此次數即升為「點頭之交」（回敬語氣轉為漸熟的寒暄）。
pub const ACQUAINTANCE_AT: u32 = 3;
/// 累積到此次數即升為「餐桌熟客」（回敬語氣轉為熱絡的老友乾杯）。
pub const REGULAR_AT: u32 = 8;

/// 玩家在某 NPC 眼中的相熟程度。順序遞增（`Stranger < Acquaintance < Regular`），
/// 故可直接用比較判斷「是否跨進更高一層」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Familiarity {
    /// 生面孔：客套有禮的招呼。
    Stranger,
    /// 點頭之交：漸熟的寒暄。
    Acquaintance,
    /// 餐桌熟客：熱絡的老友乾杯。
    Regular,
}

/// 由「累積舉杯次數」判定相熟層級。純函式、可測。
pub fn tier_of(count: u32) -> Familiarity {
    if count >= REGULAR_AT {
        Familiarity::Regular
    } else if count >= ACQUAINTANCE_AT {
        Familiarity::Acquaintance
    } else {
        Familiarity::Stranger
    }
}

/// 記一筆舉杯後的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToastRecord {
    /// 累積到目前（含這一筆）的舉杯次數。
    pub count: u32,
    /// 若這一筆剛好讓玩家「跨進」更高一層交情，回傳新層級；否則 `None`。
    pub crossed: Option<Familiarity>,
}

/// 玩家↔NPC 相熟度帳本（純記憶體，重啟清零）。
///
/// 鍵 = `(玩家鍵, NPC id)`。玩家鍵由呼叫端決定：登入玩家用帳號 uid（跨連線延續本場），
/// 訪客用連線 id（一斷線即失去這份交情，與訪客本就無存檔一致）。
#[derive(Debug, Default)]
pub struct RegularBook {
    counts: HashMap<(String, String), u32>,
}

impl RegularBook {
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
        }
    }

    /// 記一筆「玩家向某 NPC 舉杯」，回傳累積次數與是否跨層。
    pub fn record(&mut self, player_key: &str, npc_id: &str) -> ToastRecord {
        let key = (player_key.to_string(), npc_id.to_string());
        let slot = self.counts.entry(key).or_insert(0);
        let before = *slot;
        let after = before.saturating_add(1);
        *slot = after;
        let crossed = {
            let old_tier = tier_of(before);
            let new_tier = tier_of(after);
            if new_tier > old_tier {
                Some(new_tier)
            } else {
                None
            }
        };
        ToastRecord {
            count: after,
            crossed,
        }
    }

    /// 查詢玩家對某 NPC 目前的累積舉杯次數（測試／除錯用）。
    pub fn count(&self, player_key: &str, npc_id: &str) -> u32 {
        self.counts
            .get(&(player_key.to_string(), npc_id.to_string()))
            .copied()
            .unwrap_or(0)
    }
}

/// 「點頭之交」階段的回敬模板池（語氣比生面孔的客套更鬆、認得你了）。
static ACQUAINTANCE_LINES: &[(&str, &[&str])] = &[
    ("merchant", &[
        "又是你！坐坐坐，這碗算我請的老主顧價（笑）。",
        "面熟面熟，今日這果脯給你多抓一把。",
    ]),
    ("workshop_npc", &[
        "嘿，常來的好漢！這碗敬你，爐子隨時為你燒著。",
        "又見你入席，乾了——改天真教你打把好刀。",
    ]),
    ("bounty_npc", &[
        "哈，又碰上你這痛快人！這碗照舊敬你。",
        "坐這兒，野地的新鮮事我頭一個說給你聽。",
    ]),
    ("expedition_npc", &[
        "走慣野路的老相識，敬你這一碗熱飯！",
        "又同席了，山那頭的新景我正想找人講。",
    ]),
    ("procurement_npc", &[
        "你這位常客來得巧，新到的好料先給你嚐！",
        "舉杯——你這頓嘛，老樣子記我帳上（笑）。",
    ]),
    ("farm_fair_npc", &[
        "又是你來陪我吃飯，這頂甜的瓜留著就等你！",
        "常來的朋友，這碗敬你，自家菜管夠！",
    ]),
    ("village_chief", &[
        "你又入席啦，這桌有你才熱鬧，多吃點！",
        "常見的面孔了，鎮上有事真儘管來找我。",
    ]),
];

/// 「餐桌熟客」階段的回敬模板池（最熱絡的老友語氣）。
static REGULAR_LINES: &[(&str, &[&str])] = &[
    ("merchant", &[
        "老朋友來啦！別客氣，自家桌一樣坐——乾一個！",
        "就等你這張熟臉，最好的那壇我給你溫上了！",
    ]),
    ("workshop_npc", &[
        "老搭子！這碗不敬都不行，乾——爐邊永遠有你的位子。",
        "你來我這桌就齊了，喝！打鐵的事咱倆慢慢聊。",
    ]),
    ("bounty_npc", &[
        "好兄弟入席！別人敬一碗，你這得敬三碗，乾！",
        "就缺你了！野地裡並肩過的交情，乾杯！",
    ]),
    ("expedition_npc", &[
        "老夥計！走遍星海最想同席的就是你，滿上！",
        "你一坐下我就踏實，乾——回頭帶你看真正的奇景。",
    ]),
    ("procurement_npc", &[
        "貴客中的貴客！壓箱的好料今日全為你開，請！",
        "你這帳啊我早不記了——老朋友的，乾杯！",
    ]),
    ("farm_fair_npc", &[
        "自家人來啦！最甜的瓜、最好的湯，都給你留著呢！",
        "就盼你這張熟臉，坐下坐下，敬咱倆的老交情！",
    ]),
    ("village_chief", &[
        "鎮上的老朋友回來了！這桌你坐主位，乾杯！",
        "有你在這桌才算真齊，往後你就是自家人——乾！",
    ]),
];

/// 跨進新一層交情時，那一次專屬的「混熟了」台詞（每層每 NPC 一句，點明關係升溫）。
static TIERUP_LINES: &[(&str, &str, &str)] = &[
    // (npc_id, 升「點頭之交」, 升「餐桌熟客」)
    ("merchant",
        "（這位客官我可記住了）哎，往後就是熟人啦，常來！",
        "（重重一舉杯）夠交情！從今天起，你就是我攤子的老主顧了！"),
    ("workshop_npc",
        "嘿，臉熟了——往後叫我老哥便是。",
        "（猛地拍你肩）成！打今兒起咱倆是過命的老搭子，乾！"),
    ("bounty_npc",
        "你這人對我胃口，算交上了！",
        "（仰頭一飲）夠兄弟！這碗下肚，往後生死有我一份！"),
    ("expedition_npc",
        "同席幾回，也算路上的相識了。",
        "（鄭重舉碗）走遍星海的交情，從今往後你就是我的老夥計！"),
    ("procurement_npc",
        "面熟了，往後給你留好料。",
        "（推來整壇好酒）貴客！我這壓箱的，從今天起為你常備！"),
    ("farm_fair_npc",
        "常來的朋友，瓜我給你記著挑。",
        "（塞給你一大袋）自家人啦！這地裡的好東西，往後你隨便拿！"),
    ("village_chief",
        "你這面孔，鎮上認得了。",
        "（朗聲）諸位，這位往後就是咱鎮上的自家人——滿上，敬他！"),
];

/// 取得某 NPC 在某相熟層級對玩家的回敬。純函式、可測。
///
/// - `Stranger`：沿用 329 的客套回敬池（`lunch_chatter::toast_line`，不重複造輪子）。
/// - `Acquaintance` / `Regular`：取本檔對應的漸熟／熱絡模板池，依 `slot` 循環。
/// 非村落七大 NPC 一律回 `None`（與 `toast_line` 一致）。
pub fn tier_reply(npc_id: &str, tier: Familiarity, slot: usize) -> Option<&'static str> {
    let pool = match tier {
        Familiarity::Stranger => return crate::lunch_chatter::toast_line(npc_id, slot),
        Familiarity::Acquaintance => ACQUAINTANCE_LINES,
        Familiarity::Regular => REGULAR_LINES,
    };
    pool.iter()
        .find(|(id, _)| *id == npc_id)
        .map(|(_, lines)| lines[slot % lines.len()])
}

/// 跨進新一層交情時的專屬台詞。純函式、可測；`Stranger` 無「升入」台詞回 `None`。
pub fn tierup_line(npc_id: &str, tier: Familiarity) -> Option<&'static str> {
    TIERUP_LINES
        .iter()
        .find(|(id, _, _)| *id == npc_id)
        .map(|(_, ac, reg)| match tier {
            Familiarity::Acquaintance => *ac,
            Familiarity::Regular => *reg,
            Familiarity::Stranger => "",
        })
        .filter(|s| !s.is_empty())
}

/// 一次舉杯該播哪句回敬：剛跨層就播專屬「混熟了」台詞，否則播當前層級的常態回敬。
/// 把 329 的「挑句」邏輯升級為相熟度感知，純函式、可測，讓 ws.rs 端只要呼叫一次。
pub fn toast_response(npc_id: &str, rec: ToastRecord, slot: usize) -> Option<&'static str> {
    if let Some(tier) = rec.crossed {
        if let Some(line) = tierup_line(npc_id, tier) {
            return Some(line);
        }
    }
    tier_reply(npc_id, tier_of(rec.count), slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_thresholds_are_monotonic() {
        assert_eq!(tier_of(0), Familiarity::Stranger);
        assert_eq!(tier_of(ACQUAINTANCE_AT - 1), Familiarity::Stranger);
        assert_eq!(tier_of(ACQUAINTANCE_AT), Familiarity::Acquaintance);
        assert_eq!(tier_of(REGULAR_AT - 1), Familiarity::Acquaintance);
        assert_eq!(tier_of(REGULAR_AT), Familiarity::Regular);
        assert_eq!(tier_of(REGULAR_AT + 50), Familiarity::Regular);
    }

    #[test]
    fn familiarity_orders_ascending() {
        assert!(Familiarity::Stranger < Familiarity::Acquaintance);
        assert!(Familiarity::Acquaintance < Familiarity::Regular);
    }

    #[test]
    fn record_accumulates_per_player_npc_pair() {
        let mut book = RegularBook::new();
        assert_eq!(book.count("alice", "merchant"), 0);
        let r = book.record("alice", "merchant");
        assert_eq!(r.count, 1);
        assert_eq!(book.count("alice", "merchant"), 1);
        // 不同 NPC、不同玩家各自獨立計數，互不沾染。
        assert_eq!(book.count("alice", "workshop_npc"), 0);
        assert_eq!(book.count("bob", "merchant"), 0);
        book.record("alice", "merchant");
        assert_eq!(book.count("alice", "merchant"), 2);
    }

    #[test]
    fn record_reports_crossing_into_each_tier_once() {
        let mut book = RegularBook::new();
        let mut crossings = Vec::new();
        for _ in 0..(REGULAR_AT + 2) {
            let r = book.record("alice", "merchant");
            if let Some(t) = r.crossed {
                crossings.push((r.count, t));
            }
        }
        // 恰好兩次跨層：第 ACQUAINTANCE_AT 筆升點頭之交、第 REGULAR_AT 筆升熟客。
        assert_eq!(
            crossings,
            vec![
                (ACQUAINTANCE_AT, Familiarity::Acquaintance),
                (REGULAR_AT, Familiarity::Regular),
            ]
        );
    }

    #[test]
    fn stranger_reply_falls_back_to_existing_toast_pool() {
        // 生面孔沿用 329 的回敬池，確保不是空的、且與 lunch_chatter 同步。
        for slot in 0..3 {
            let a = tier_reply("merchant", Familiarity::Stranger, slot);
            let b = crate::lunch_chatter::toast_line("merchant", slot);
            assert_eq!(a, b);
            assert!(a.is_some());
        }
    }

    #[test]
    fn warmer_tiers_have_lines_for_every_village_npc() {
        // 七大 NPC 在點頭之交／熟客層都得有回敬，免得熟了反而沒詞可說。
        let npcs = [
            "merchant",
            "workshop_npc",
            "bounty_npc",
            "expedition_npc",
            "procurement_npc",
            "farm_fair_npc",
            "village_chief",
        ];
        for id in npcs {
            assert!(tier_reply(id, Familiarity::Acquaintance, 0).is_some(), "{id} 缺點頭之交回敬");
            assert!(tier_reply(id, Familiarity::Regular, 0).is_some(), "{id} 缺熟客回敬");
            assert!(tierup_line(id, Familiarity::Acquaintance).is_some(), "{id} 缺升點頭之交台詞");
            assert!(tierup_line(id, Familiarity::Regular).is_some(), "{id} 缺升熟客台詞");
        }
    }

    #[test]
    fn non_village_npc_has_no_lines() {
        assert!(tier_reply("stranger_npc", Familiarity::Acquaintance, 0).is_none());
        assert!(tier_reply("stranger_npc", Familiarity::Regular, 0).is_none());
        assert!(tierup_line("stranger_npc", Familiarity::Regular).is_none());
    }

    #[test]
    fn stranger_tier_has_no_tierup_line() {
        assert!(tierup_line("merchant", Familiarity::Stranger).is_none());
    }

    #[test]
    fn toast_response_prefers_tierup_then_falls_to_tier_reply() {
        // 剛跨層那筆：播專屬「混熟了」台詞。
        let crossing = ToastRecord {
            count: REGULAR_AT,
            crossed: Some(Familiarity::Regular),
        };
        assert_eq!(
            toast_response("merchant", crossing, 0),
            tierup_line("merchant", Familiarity::Regular)
        );
        // 同層的後續舉杯（未跨層）：播該層常態回敬。
        let steady = ToastRecord {
            count: REGULAR_AT + 1,
            crossed: None,
        };
        assert_eq!(
            toast_response("merchant", steady, 0),
            tier_reply("merchant", Familiarity::Regular, 0)
        );
    }

    #[test]
    fn reply_slot_cycles_within_pool() {
        // slot 超出池長度時環繞，不會 panic / 越界。
        let n = 100usize;
        assert!(tier_reply("merchant", Familiarity::Regular, n).is_some());
        assert!(tier_reply("village_chief", Familiarity::Acquaintance, n).is_some());
    }
}
