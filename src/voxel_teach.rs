//! 乙太方界·居民互相傳授技能 v1（ROADMAP 717）。
//!
//! **設計依據**：技能發明（716／PR #944）讓居民第一次能自己組合原語、發明出屬於
//! 自己的技能——但那個模組留了一句伏筆：「教別人是之後的事」。本模組把這件事做出來：
//! 老朋友到訪時，若其中一位已經發明了對方還不會的技能，偶爾會順手教一手，讓技能
//! 不再只鎖在發明者自己身上，第一次能在朋友網絡裡流通。
//!
//! 呼應 694（口耳相傳）把「見聞」傳開、696（互助蓋家）把「幫忙」傳開，本模組
//! 把「本事」也傳開——小社會不只互相問候、幫忙、拌嘴，還會互相**成長**。
//!
//! **全鏈**：老朋友到訪 →（沒觸發互助蓋家/拌嘴，同一次到訪只演一齣戲）→ 機率門檻 →
//! 查雙方技能庫（`InventedSkillStore::teachable`，host 教 visitor 優先，host 沒有
//! 可教的才看 visitor 教不教得了 host）→ 有可教的才真的教（技能原封不動複製一份、
//! 落地存檔）→ 教學者/受教者各自留一筆記憶、世界 Feed 播報、訪客頭頂冒台詞。
//!
//! 純邏輯層：零 IO、零鎖、零 LLM、零 async；確定性純函式。鎖／技能庫存取／持久化
//! 在 `voxel_ws.rs`（`bond_arrive_events` 迴圈內，短鎖即釋、不巢狀，同既有手法）。

use crate::voxel_bonds::BondTier;

/// Feed 播報種類名稱。
pub const FEED_KIND: &str = "技能傳授";

/// 每次「老朋友」到訪、且這次到訪沒有觸發互助蓋家（696）或拌嘴（715）時，
/// 觸發技能傳授的機率——同樣是偶爾出現的生活情趣，不是常態。是否真的有技能
/// 可教（雙方技能庫是否有落差）由呼叫端另外查表決定，本函式只把機率門檻。
pub const TEACH_CHANCE: f32 = 0.15;

/// 判斷這次到訪是否要嘗試傳授技能。
///
/// 只有「老朋友」夠熟才會傳授；且要求這次到訪沒有觸發互助蓋家或拌嘴——
/// 同一次到訪只演一齣戲，避免「剛拌完嘴/幫完忙又立刻教學」讀起來精神分裂。
pub fn should_teach(tier: BondTier, help_happened: bool, quarrel_happened: bool, roll: f32) -> bool {
    tier == BondTier::Friend && !help_happened && !quarrel_happened && roll < TEACH_CHANCE
}

// ── 相遇互教（技能互教·北極星第四刀）───────────────────────────────────────────
//
// 717 教在「登門到訪」、就地指導教在「有人卡關待救」；本節補上最日常的一種：
// 交情夠的兩位居民**平常相處時剛好走到近旁、雙方都閒**，偶爾把自己會、對方不會的
// 手藝就地教一手——知識在活人之間口耳相傳，與血脈繼承（#998「承自XX」）互補。
// 頻率由三道閘共同守住（不無限擴散）：機率門檻（本節）＋每居民教/學帳本冷卻
// （一遊戲天最多一次，見 [`TEACH_LEDGER_SECS`]）＋交情要到老朋友——一個技能
// 傳遍全村自然要花上好幾遊戲天，像真的村子。

/// 相遇互教的每輪掃描觸發機率（掃描與就地指導同一 15 秒節拍）：刻意設得比 717 到訪
/// 教學（0.15）更低——相遇比到訪頻繁得多，機率再高就洗版了。
pub const ENCOUNTER_TEACH_CHANCE: f32 = 0.04;

/// 每居民「教或學」帳本冷卻（秒）＝一個遊戲天（[`crate::daynight::DAY_LENGTH_SECS`]）：
/// 教過或學過一次後，這一遊戲天內不再參與相遇互教——手藝要在村裡慢慢傳開，
/// 不是一個下午全村都會了。QA 可用環境變數縮短（見 voxel_ws `teach_ledger_secs`）。
pub const TEACH_LEDGER_SECS: u64 = 600;

/// 教學那一刻兩人停留面對面的秒數（老師講、學生聽——路過的玩家看得見這一幕）。
pub const TEACH_PAUSE_SECS: f32 = 24.0;

/// 學生「原來如此！」應和泡泡的延遲秒數（老師先開口，學生聽了一會兒才點頭）。
pub const TEACH_REPLY_DELAY_SECS: f32 = 8.0;

/// 帳本冷卻是否已過（純函式）：`last`＝上次教/學的 unix 秒（None＝從沒教/學過）。
pub fn ledger_ready(now: u64, last: Option<u64>, cd: u64) -> bool {
    match last {
        Some(prev) => now.saturating_sub(prev) >= cd,
        None => true,
    }
}

/// 老師開講的台詞（相遇互教）——帶學生名與技能名，像真的在傳授訣竅。
pub fn teach_open_line(student: &str, skill_name: &str, pick: usize) -> String {
    const T: [&str; 3] = [
        "來，{student}，我教你「{skill}」——跟著我做一遍就會了",
        "{student}，「{skill}」這招很好用，我一步步拆給你看",
        "趁現在有空，{student}，把「{skill}」的訣竅教給你",
    ];
    T[pick % T.len()]
        .replace("{student}", student)
        .replace("{skill}", skill_name)
}

/// 學生聽完的應和台詞（相遇互教）——延遲幾秒才冒，一來一往像真的對話。
pub fn teach_reply_line(skill_name: &str, pick: usize) -> String {
    const T: [&str; 3] = [
        "原來如此！「{skill}」我也會了！",
        "懂了懂了——下次「{skill}」我自己來！",
        "學會了！「{skill}」原來是這麼做的～",
    ];
    T[pick % T.len()].replace("{skill}", skill_name)
}

/// 受教者（訪客）頭頂冒的台詞——這次是「host 教了 visitor」，visitor 剛學會、很開心。
pub fn teach_say_line_as_student(teacher: &str, skill_name: &str, pick: usize) -> String {
    const T: [&str; 3] = [
        "{teacher}教了我「{skill}」這招，我也會了！",
        "跟{teacher}學了一手「{skill}」，太感謝了！",
        "{teacher}把「{skill}」的訣竅教給我了，這下我也會了～",
    ];
    T[pick % T.len()]
        .replace("{teacher}", teacher)
        .replace("{skill}", skill_name)
}

/// 施教者（訪客）頭頂冒的台詞——這次是「visitor 教了 host」，visitor 剛教完，有點小驕傲。
pub fn teach_say_line_as_teacher(student: &str, skill_name: &str, pick: usize) -> String {
    const T: [&str; 3] = [
        "我把「{skill}」這招教給{student}了！",
        "教了{student}「{skill}」的訣竅，她一下就學會了！",
        "順手把「{skill}」教給{student}，以後她也會了～",
    ];
    T[pick % T.len()]
        .replace("{student}", student)
        .replace("{skill}", skill_name)
}

/// Feed 播報文字（第三人稱，讓沒在場的玩家也知道世界有這麼一幕）。
pub fn teach_feed_line(teacher: &str, student: &str, skill_name: &str, pick: usize) -> String {
    const T: [&str; 3] = [
        "{teacher}把自己發明的「{skill}」教給了{student}，本事又多流傳了一個人！",
        "{student}跟{teacher}學會了「{skill}」——技能第一次在朋友之間傳開！",
        "{teacher}和{student}湊在一起，「{skill}」這招就這麼教會了！",
    ];
    T[pick % T.len()]
        .replace("{teacher}", teacher)
        .replace("{student}", student)
        .replace("{skill}", skill_name)
}

/// 傳授進教學者記憶的摘要文字（教學者視角）。
pub fn teach_memory_line_teacher(student: &str, skill_name: &str) -> String {
    format!("我把自己發明的「{skill_name}」教給了{student}，這份本事現在她也會了")
}

/// 傳授進受教者記憶的摘要文字（受教者視角）。
pub fn teach_memory_line_student(teacher: &str, skill_name: &str) -> String {
    format!("{teacher}教了我「{skill_name}」這招，現在我也會了——謝謝她")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_teach_only_for_friend_tier() {
        assert!(!should_teach(BondTier::Stranger, false, false, 0.0));
        assert!(!should_teach(BondTier::Acquaintance, false, false, 0.0));
        assert!(should_teach(BondTier::Friend, false, false, 0.0));
    }

    #[test]
    fn should_teach_never_when_help_or_quarrel_already_happened() {
        assert!(!should_teach(BondTier::Friend, true, false, 0.0), "已互助蓋家就不再教學");
        assert!(!should_teach(BondTier::Friend, false, true, 0.0), "已拌嘴就不再教學");
        assert!(!should_teach(BondTier::Friend, true, true, 0.0));
    }

    #[test]
    fn should_teach_respects_chance_threshold() {
        assert!(should_teach(BondTier::Friend, false, false, TEACH_CHANCE - 0.001));
        assert!(!should_teach(BondTier::Friend, false, false, TEACH_CHANCE));
        assert!(!should_teach(BondTier::Friend, false, false, 0.99));
    }

    #[test]
    fn teach_say_line_as_student_mentions_teacher_and_skill() {
        let line = teach_say_line_as_student("諾娃", "燒玻璃", 0);
        assert!(line.contains('諾'));
        assert!(line.contains("燒玻璃"));
    }

    #[test]
    fn teach_say_line_as_teacher_mentions_student_and_skill() {
        let line = teach_say_line_as_teacher("諾娃", "燒玻璃", 0);
        assert!(line.contains('諾'));
        assert!(line.contains("燒玻璃"));
    }

    #[test]
    fn say_lines_vary_with_pick() {
        let a = teach_say_line_as_student("諾娃", "燒玻璃", 0);
        let b = teach_say_line_as_student("諾娃", "燒玻璃", 1);
        assert_ne!(a, b);
        let c = teach_say_line_as_teacher("諾娃", "燒玻璃", 0);
        let d = teach_say_line_as_teacher("諾娃", "燒玻璃", 1);
        assert_ne!(c, d);
    }

    #[test]
    fn feed_line_mentions_both_names_and_skill_and_varies() {
        let a = teach_feed_line("露娜", "諾娃", "燒玻璃", 0);
        assert!(a.contains('露') && a.contains('諾') && a.contains("燒玻璃"));
        let b = teach_feed_line("露娜", "諾娃", "燒玻璃", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn memory_lines_mention_the_other_party_and_skill() {
        let t = teach_memory_line_teacher("諾娃", "燒玻璃");
        assert!(t.contains('諾') && t.contains("燒玻璃"));
        let s = teach_memory_line_student("露娜", "燒玻璃");
        assert!(s.contains('露') && s.contains("燒玻璃"));
    }

    // ── 相遇互教（技能互教·北極星第四刀）────────────────────────────────────────

    #[test]
    fn ledger_ready_when_never_taught_before() {
        assert!(ledger_ready(1000, None, TEACH_LEDGER_SECS), "從沒教/學過 → 隨時可教");
    }

    #[test]
    fn ledger_ready_respects_cooldown_boundary() {
        let cd = TEACH_LEDGER_SECS;
        assert!(!ledger_ready(1000, Some(1000), cd), "剛教完不可再教");
        assert!(!ledger_ready(1000 + cd - 1, Some(1000), cd), "差一秒仍在冷卻");
        assert!(ledger_ready(1000 + cd, Some(1000), cd), "冷卻恰滿即可再教");
    }

    #[test]
    fn ledger_ready_survives_clock_skew() {
        // now 比 last 還小（時鐘回撥）→ saturating_sub=0，不 panic、老實回「還在冷卻」。
        assert!(!ledger_ready(10, Some(999), TEACH_LEDGER_SECS));
        // 冷卻設 0（QA 縮短）→ 永遠可教。
        assert!(ledger_ready(10, Some(999), 0));
    }

    #[test]
    fn teach_ledger_equals_one_game_day() {
        assert_eq!(
            TEACH_LEDGER_SECS,
            crate::daynight::DAY_LENGTH_SECS as u64,
            "帳本冷卻應恰為一個遊戲天（daynight 若改天長，這裡要跟著改）"
        );
    }

    #[test]
    fn teach_pause_is_a_natural_moment() {
        assert!((20.0..=30.0).contains(&TEACH_PAUSE_SECS), "停留一會（20~30 秒）才像真的在教");
        assert!(TEACH_REPLY_DELAY_SECS < TEACH_PAUSE_SECS, "學生要在兩人還站在一起時應和");
    }

    #[test]
    fn teach_open_line_mentions_student_and_skill_and_varies() {
        let a = teach_open_line("諾娃", "燒玻璃", 0);
        assert!(a.contains('諾') && a.contains("燒玻璃"));
        let b = teach_open_line("諾娃", "燒玻璃", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn teach_reply_line_mentions_skill_and_varies() {
        let a = teach_reply_line("燒玻璃", 0);
        assert!(a.contains("燒玻璃"));
        let b = teach_reply_line("燒玻璃", 1);
        assert_ne!(a, b);
    }
}
