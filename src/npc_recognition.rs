//! 街坊相認（ROADMAP 331）：午餐桌上攢起的交情，第一次走出午休、灑到整日的城鎮裡。
//!
//! 329 讓玩家能在午休席間向就座的 NPC 舉杯，330 把那一次次舉杯「累積」成相熟度——
//! 可那份交情至今只活在那一小段正午午休裡：午休一散、NPC 各自回崗位，就再不認得你了。
//! 這個切片把「被記得」兌現到午休之外：白天各司其職的七大 NPC，只要玩家走近，就會
//! 認出那張在飯桌上混熟的臉、停下手邊活計點名招呼你一句——點頭之交給漸熟的寒暄、
//! 餐桌熟客給熱絡的老友招呼。城鎮第一次「整日都記得你是誰」，而不只在飯桌上。
//!
//! 設計鐵律（與 `lunch_regular`〔相熟度帳本〕、`lunch_chatter`〔席間／舉杯回敬〕同一條線，
//! 刻意分工——那兩者管午休席間，這裡管午休之外整日的崗位相認）：
//! - 純記憶體模式，重啟清零（與相熟度帳本同調，零 DB／零 migration）。
//! - 零 LLM、純查表；相認只挑「招呼哪一句」，碰不到任何遊戲狀態（乙太／背包／戰力）。
//! - **只認得熟人**：生面孔（`Stranger`）一律不點名招呼——被叫出來打招呼，本身就是「我記得你」的證明。
//! - 純邏輯可獨立測試。面向玩家字串集中於本檔、含 `{name}` 佔位，作為 i18n 替換點。

use std::collections::HashMap;
use crate::lunch_regular::Familiarity;

/// 相認搆得著的半徑（像素）：玩家要走到 NPC 跟前這麼近，NPC 才認得出、出聲招呼。
/// 比午休舉杯的 `LUNCH_TOAST_REACH`(140) 收一點，貼到崗位前才觸發，避免遠遠路過就被喊。
pub const RECOGNIZE_RADIUS: f32 = 110.0;

/// 同一對「玩家×NPC」兩次相認招呼之間的冷卻（秒）：玩家在攤前逗留時不會被同一人連珠炮招呼。
/// 取得比午休舉杯冷卻長得多——崗位招呼是「久久碰面點個頭」，不是一直寒暄。
pub const GREET_COOLDOWN_SECS: f32 = 50.0;

/// 玩家是否走進某 NPC 的相認範圍。純函式、可測。
/// 用距離平方比較，免去開平方；非有限座標保守回 `false`。
pub fn within_reach(npc_x: f32, npc_y: f32, px: f32, py: f32) -> bool {
    let dx = npc_x - px;
    let dy = npc_y - py;
    let d2 = dx * dx + dy * dy;
    d2.is_finite() && d2 <= RECOGNIZE_RADIUS * RECOGNIZE_RADIUS
}

/// 街坊相認冷卻帳本（純記憶體，重啟清零）。
///
/// 鍵 =「玩家鍵 × NPC id」；值 = 距下次可再招呼的剩餘秒數。
/// 玩家鍵與相熟度帳本一致（即玩家在 `players` map 裡的鍵：登入玩家＝帳號 uid，訪客＝連線 id）。
#[derive(Debug, Default)]
pub struct RecognitionBook {
    cooldowns: HashMap<(String, String), f32>,
}

impl RecognitionBook {
    pub fn new() -> Self {
        Self {
            cooldowns: HashMap::new(),
        }
    }

    /// 推進冷卻倒數（每 tick 呼叫，`dt` 秒）：遞減各對的剩餘秒數，歸零者剔除以免帳本無限長大。
    pub fn tick(&mut self, dt: f32) {
        if self.cooldowns.is_empty() {
            return;
        }
        self.cooldowns.retain(|_, remaining| {
            *remaining -= dt;
            *remaining > 0.0
        });
    }

    /// 這一對玩家×NPC 此刻是否可再招呼（無紀錄或冷卻已過）。純查詢。
    pub fn ready(&self, player_key: &str, npc_id: &str) -> bool {
        self.cooldowns
            .get(&(player_key.to_string(), npc_id.to_string()))
            .map(|&remaining| remaining <= 0.0)
            .unwrap_or(true)
    }

    /// 記下「剛招呼過」：把這一對的冷卻設滿，`GREET_COOLDOWN_SECS` 秒內不再招呼。
    pub fn mark(&mut self, player_key: &str, npc_id: &str) {
        self.cooldowns
            .insert((player_key.to_string(), npc_id.to_string()), GREET_COOLDOWN_SECS);
    }
}

/// 「點頭之交」階段的崗位相認招呼池（語氣是漸熟的寒暄、認得你了）。含 `{name}` 佔位點名。
static ACQUAINTANCE_GREETS: &[(&str, &[&str])] = &[
    ("merchant", &[
        "喲，{name}！又上街啦，攤上有看對眼的儘管拿。",
        "{name}來啦，今日的果脯新鮮，路過嚐一口？",
    ]),
    ("workshop_npc", &[
        "嘿，{name}！爐火正旺，要不要看看新打的傢伙？",
        "是{name}啊，慢走——改天得空教你兩手鍛打。",
    ]),
    ("bounty_npc", &[
        "{name}！正想找個痛快人說話，野地又出新鮮事了。",
        "喲，{name}逛到這兒，懸賞榜剛換了一批，瞧瞧？",
    ]),
    ("expedition_npc", &[
        "{name}，走慣野路的，這趟可有往遠處去的打算？",
        "是{name}啊，地圖上又添了塊新景，回頭講你聽。",
    ]),
    ("procurement_npc", &[
        "{name}來得巧，新到的好料正清點著呢。",
        "喲，{name}！備的貨齊了，缺什麼跟我說一聲。",
    ]),
    ("farm_fair_npc", &[
        "{name}！自家菜熟了一畦，路過拿兩把回去？",
        "是{name}啊，今早的瓜頂甜，給你留著了。",
    ]),
    ("village_chief", &[
        "{name}，在鎮上轉悠呢？一切都還順心吧。",
        "喲，{name}！鎮務我巡著呢，有事儘管招呼。",
    ]),
];

/// 「餐桌熟客」階段的崗位相認招呼池（最熱絡的老友語氣）。含 `{name}` 佔位點名。
static REGULAR_GREETS: &[(&str, &[&str])] = &[
    ("merchant", &[
        "哎，是{name}！老主顧上門，最好的那批先給你留著呢！",
        "{name}來啦！別客氣，自家攤一樣翻——看上啥只管拿！",
    ]),
    ("workshop_npc", &[
        "老搭子{name}！爐邊永遠有你的位子，來歇歇腳！",
        "嘿，{name}！正想著你呢——那把好刀我替你磨上了！",
    ]),
    ("bounty_npc", &[
        "好兄弟{name}！並肩過命的交情，這榜上最肥的差我先給你！",
        "{name}！就盼你這張臉，野地的硬仗咱倆一道去！",
    ]),
    ("expedition_npc", &[
        "老夥計{name}！走遍星海最想同行的就是你，這就出發？",
        "{name}！我新測的那條路就等你來，帶你看真奇景！",
    ]),
    ("procurement_npc", &[
        "貴客{name}！壓箱的好料全為你備著，要啥開口！",
        "{name}！你那帳我早不記了——老朋友的，隨意拿！",
    ]),
    ("farm_fair_npc", &[
        "自家人{name}回來啦！最甜的瓜、最好的菜都給你留著！",
        "{name}！地裡好東西管夠，往後你隨便挑、隨便拿！",
    ]),
    ("village_chief", &[
        "鎮上的老朋友{name}！這一帶有你照應，我放心得很！",
        "{name}！你早是咱鎮上的自家人了，有難處只管來找我！",
    ]),
];

/// 取某 NPC 對某相熟層級玩家的崗位相認招呼（第 `slot` 句，在對應池內循環）。純函式、可測。
///
/// - `Stranger`：回 `None`——生面孔不會被點名招呼（被認得，本身就是相熟的證明）。
/// - `Acquaintance` / `Regular`：取本檔對應的漸熟／熱絡招呼池。
/// 非村落七大 NPC（不在池內）一律回 `None`。回傳字串含 `{name}` 佔位，由呼叫端替換為玩家名。
pub fn recognize_line(npc_id: &str, tier: Familiarity, slot: usize) -> Option<&'static str> {
    let pool = match tier {
        Familiarity::Stranger => return None,
        Familiarity::Acquaintance => ACQUAINTANCE_GREETS,
        Familiarity::Regular => REGULAR_GREETS,
    };
    pool.iter()
        .find(|(id, _)| *id == npc_id)
        .map(|(_, lines)| lines[slot % lines.len()])
}

/// 把招呼模板裡的 `{name}` 佔位替換成玩家名，產出最終要廣播的招呼。純函式、可測。
/// 集中於此，讓「點名」這件事只有一個替換點，方便日後 i18n。
pub fn fill_name(template: &str, player_name: &str) -> String {
    template.replace("{name}", player_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lunch_regular::Familiarity;

    #[test]
    fn within_reach_respects_radius() {
        // 同點當然搆得著。
        assert!(within_reach(100.0, 100.0, 100.0, 100.0));
        // 半徑內（差 100px < 110）搆得著。
        assert!(within_reach(100.0, 100.0, 200.0, 100.0));
        // 半徑外（差 200px > 110）搆不著。
        assert!(!within_reach(100.0, 100.0, 300.0, 100.0));
        // 非有限座標保守回 false。
        assert!(!within_reach(f32::NAN, 0.0, 0.0, 0.0));
    }

    #[test]
    fn stranger_never_greeted() {
        // 生面孔一律不點名招呼——無論哪個 NPC、哪一句。
        for npc in ["merchant", "village_chief", "bounty_npc"] {
            for slot in 0..5 {
                assert_eq!(recognize_line(npc, Familiarity::Stranger, slot), None);
            }
        }
    }

    #[test]
    fn acquaintance_and_regular_get_lines() {
        // 七大 NPC 在兩個相熟層級都有招呼可說，且含 {name} 佔位。
        for npc in [
            "merchant", "workshop_npc", "bounty_npc", "expedition_npc",
            "procurement_npc", "farm_fair_npc", "village_chief",
        ] {
            let acq = recognize_line(npc, Familiarity::Acquaintance, 0);
            let reg = recognize_line(npc, Familiarity::Regular, 0);
            assert!(acq.is_some(), "{npc} 應有點頭之交招呼");
            assert!(reg.is_some(), "{npc} 應有餐桌熟客招呼");
            assert!(acq.unwrap().contains("{name}"), "{npc} 招呼應含 {{name}} 佔位");
            assert!(reg.unwrap().contains("{name}"), "{npc} 招呼應含 {{name}} 佔位");
        }
    }

    #[test]
    fn non_village_npc_gets_nothing() {
        // 非村落七大 NPC（旅人／居民／其他星球商人）一律不相認。
        for tier in [Familiarity::Acquaintance, Familiarity::Regular] {
            assert_eq!(recognize_line("traveler_42", tier, 0), None);
            assert_eq!(recognize_line("resident_7", tier, 0), None);
        }
    }

    #[test]
    fn line_slot_cycles_within_pool() {
        // slot 在池內環繞，不越界（連取多個 slot 都能取到句子）。
        for slot in 0..10 {
            assert!(recognize_line("merchant", Familiarity::Regular, slot).is_some());
        }
    }

    #[test]
    fn fill_name_replaces_placeholder() {
        assert_eq!(fill_name("喲，{name}！", "阿星"), "喲，阿星！");
        // 沒有佔位也安全（原樣回傳）。
        assert_eq!(fill_name("早安", "阿星"), "早安");
    }

    #[test]
    fn book_ready_then_cooldown_then_recovers() {
        let mut book = RecognitionBook::new();
        // 從未招呼過 → 可招呼。
        assert!(book.ready("p1", "merchant"));
        // 招呼後進入冷卻 → 不可再招呼。
        book.mark("p1", "merchant");
        assert!(!book.ready("p1", "merchant"));
        // 冷卻未過完 → 仍不可。
        book.tick(GREET_COOLDOWN_SECS - 1.0);
        assert!(!book.ready("p1", "merchant"));
        // 冷卻過完 → 條目被剔除、恢復可招呼。
        book.tick(2.0);
        assert!(book.ready("p1", "merchant"));
    }

    #[test]
    fn book_cooldown_is_per_pair() {
        let mut book = RecognitionBook::new();
        book.mark("p1", "merchant");
        // 同玩家對「不同 NPC」互不影響。
        assert!(book.ready("p1", "village_chief"));
        // 不同玩家對「同一 NPC」也互不影響。
        assert!(book.ready("p2", "merchant"));
        // 被標記的那一對才在冷卻。
        assert!(!book.ready("p1", "merchant"));
    }

    #[test]
    fn empty_book_tick_is_noop() {
        // 空帳本 tick 不 panic、保持空。
        let mut book = RecognitionBook::new();
        book.tick(1.0);
        assert!(book.ready("anyone", "merchant"));
    }
}
