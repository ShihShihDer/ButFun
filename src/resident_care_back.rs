//! 居民回禮·交情深的居民會反過來關心你（ROADMAP 556）。
//!
//! 553 讓居民「有在想事情」、554 讓你能上前「關心」流露煩惱的居民、555 把關心累積成
//! 會長大的交情——可那份關係到目前為止仍是**單向**的：園丁付出，居民領情，如此而已。
//! 這個切片把它補成**雙向**：當你照料的對象已是與你深交的「餐桌熟客」、而你自己正帶著傷，
//! 居民被「你自己都傷成這樣，還惦記著來看我」打動，會**反過來關心你**——道一句暖語、
//! 順手替你療一小段傷。這是世界第一次「住著的人也在乎你」：你三番五次的照料，終於有了回禮。
//!
//! 對齊北極星「由 AI 棲居的世界·人類當園丁」——讓 AI 居民不只是被照料的對象，而是
//! 有能動性、會回應、會回報你善意的鄰居；關係從「園丁→居民」長成「彼此照拂」。
//!
//! 設計鐵律：
//! - 純邏輯、純函式、可獨立測試；無 IO、不碰 WebSocket / 遊戲迴圈。
//! - 零 LLM、零持久化、零 migration；面向玩家字串集中於本檔作為 i18n 替換點。
//! - **零經濟水龍頭**：回禮只還玩家一小段「生命值」（非乙太／非物品），且受「深交門檻 ×
//!   只在帶傷時 × 獨立長冷卻」三重節流，回血速率遠低於脫戰自然回血，純為暖意點綴、不擾平衡。

/// 觸發回禮所需的最低交情層級序（對齊 [`crate::lunch_regular::Familiarity`] 的順序：
/// 0 生面孔／1 點頭之交／2 餐桌熟客）。刻意設在最深一層——只有真正交心的老友，
/// 見你帶傷還來看顧，才會反過來疼惜你。
pub const CARE_BACK_TIER: u8 = 2;

/// 玩家算「帶著傷、值得被疼惜」的生命值門檻（佔上限的百分比）。
/// 低於此比例才觸發回禮；血量還算飽滿時居民只道尋常領情、不多此一舉地療傷。
pub const CARE_BACK_HP_PCT: u32 = 60;

/// 兩次「領受回禮療傷」之間的冷卻（秒）。比「關心」本身的 5 秒冷卻長得多——
/// 關心（撫平居民需求）可常做，但「反過來被療傷」嚴格節流，免得深交居民變成貼身回血泉。
pub const CARE_BACK_COOLDOWN_SECS: f32 = 60.0;

/// 此刻是否該觸發「居民反過來關心你」。純函式、確定性、可測。
///
/// 三個條件**同時成立**才回 `true`：
/// 1. 與這位居民的交情已達 [`CARE_BACK_TIER`]（餐桌熟客）；
/// 2. 玩家確實帶著傷——生命值低於上限的 [`CARE_BACK_HP_PCT`]%；
/// 3. 玩家還活著（`hp > 0`）——倒地時療傷本就是 no-op，這裡先擋掉、語意更清楚。
///
/// `max_hp == 0`（理論上不該發生）保守回 `false`，不除零、不誤判。
pub fn reciprocates(bond_tier_ord: u8, hp: u32, max_hp: u32) -> bool {
    if max_hp == 0 || hp == 0 {
        return false;
    }
    if bond_tier_ord < CARE_BACK_TIER {
        return false;
    }
    // hp / max_hp < PCT/100  ⇔  hp*100 < max_hp*PCT（純整數、無浮點誤差、無除零）。
    (hp as u64) * 100 < (max_hp as u64) * (CARE_BACK_HP_PCT as u64)
}

/// 回禮療傷的幅度：依生命上限取約三分之一（向上取整、至少 1）。
/// 隨 `max_hp` 縮放（玩家加點提升上限後仍是「約三分之一」的暖意），刻意不滿血——
/// 是「歇口氣」的疼惜，不是一鍵回滿，受傷探索的張力仍在。純函式、可測。
pub fn care_heal_amount(max_hp: u32) -> u32 {
    max_hp.div_ceil(3).max(1)
}

/// 居民反過來關心你時道出的一句暖語（就地 NpcSpeech 泡泡）。只有故鄉七大居民有詞，
/// 其餘 NPC 回 `None`（與 `lunch_regular` / `npc_agent` 的村落 NPC 慣例一致）。
/// 以 🍵 起頭（有別於 554/555 撫慰領情的 💚），讓「換我來疼你」的回禮一眼可辨。
/// `seed` 用來在該居民的兩句之間輪替、不老是同一句；越界自動環繞、不會 panic。
/// 純查表、確定性；面向玩家字串集中於此作為 i18n 替換點。
pub fn care_back_line(npc_id: &str, seed: usize) -> Option<&'static str> {
    CARE_BACK_LINES
        .iter()
        .find(|(id, _)| *id == npc_id)
        .map(|(_, lines)| lines[seed % lines.len()])
}

/// 故鄉七大居民各自的「回禮關心」台詞池（每位兩句，語氣貼合其行當與性格）。
static CARE_BACK_LINES: &[(&str, &[&str])] = &[
    ("merchant", &[
        "🍵 哎喲你這傷！快坐下，這碗熱茶我請——身子要緊，生意改日再說。",
        "🍵 自己傷成這樣還來看我？喏，敷上這帖，老主顧的命我可賠不起。",
    ]),
    ("workshop_npc", &[
        "🍵 帶著傷還惦記我這把老骨頭——爐邊暖著，過來歇歇，我替你看看。",
        "🍵 傻小子，先養好身子再逞強！這碗下去，氣力就回來幾分。",
    ]),
    ("bounty_npc", &[
        "🍵 好兄弟掛了彩還來尋我？並肩過命的交情，這點傷我替你扛一半！",
        "🍵 坐這兒緩緩，野地的兇險我見多了——你這身子骨，得先顧住。",
    ]),
    ("expedition_npc", &[
        "🍵 走遍星海的老夥計，你這傷可不能拖！靠著我歇會兒，路還長。",
        "🍵 別硬撐，星圖再美也得有命看——來，這口氣力你拿著。",
    ]),
    ("procurement_npc", &[
        "🍵 貴客傷成這樣怎麼行！壓箱的好藥我這就取，先給你敷上。",
        "🍵 你這帳我早不記了——倒是這傷，得我親自替你料理才安心。",
    ]),
    ("farm_fair_npc", &[
        "🍵 帶著傷還來陪我？快坐田埂上，自家熬的湯，喝下去暖一暖。",
        "🍵 自家人受了傷哪能不管，這把草藥靈得很，敷上就好大半。",
    ]),
    ("village_chief", &[
        "🍵 鎮上的自家人傷著了，這還得了——來，靠著我，鎮上有我照看著。",
        "🍵 你為大夥兒費這麼多心，自己卻傷著……歇歇，這份心鎮上記著。",
    ]),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// 七大居民完整名單（與 `lunch_regular` 測試同一份，確保不漏人）。
    const VILLAGE: [&str; 7] = [
        "merchant",
        "workshop_npc",
        "bounty_npc",
        "expedition_npc",
        "procurement_npc",
        "farm_fair_npc",
        "village_chief",
    ];

    #[test]
    fn reciprocates_needs_deep_bond_and_being_hurt() {
        // 餐桌熟客（2）+ 帶傷（10/20 = 50% < 60%）→ 觸發。
        assert!(reciprocates(2, 10, 20));
        // 交情不夠（生面孔 0 / 點頭之交 1）→ 不觸發，哪怕傷得重。
        assert!(!reciprocates(0, 1, 20));
        assert!(!reciprocates(1, 1, 20));
        // 深交但血量飽滿（13/20 = 65% ≥ 60%）→ 不多此一舉。
        assert!(!reciprocates(2, 13, 20));
    }

    #[test]
    fn reciprocates_hp_threshold_is_strict_below_pct() {
        // 恰好 60%（12/20）→ 不算「帶傷」，不觸發（門檻為「嚴格低於」）。
        assert!(!reciprocates(2, 12, 20));
        // 59.x%（11/20 = 55%）→ 觸發。
        assert!(reciprocates(2, 11, 20));
    }

    #[test]
    fn reciprocates_over_max_tier_still_triggers() {
        // 交情序越界（>2）仍視為已達深交門檻，照樣觸發。
        assert!(reciprocates(9, 5, 20));
    }

    #[test]
    fn reciprocates_guards_downed_and_zero_max() {
        // 倒地（hp == 0）→ 不觸發（療傷本就 no-op，這裡先擋）。
        assert!(!reciprocates(2, 0, 20));
        // max_hp == 0 → 保守不觸發、不除零。
        assert!(!reciprocates(2, 0, 0));
    }

    #[test]
    fn care_heal_amount_is_about_a_third_and_scales() {
        assert_eq!(care_heal_amount(20), 7); // ceil(20/3)
        assert_eq!(care_heal_amount(30), 10);
        assert_eq!(care_heal_amount(6), 2); // ceil(6/3)
        // 極小上限仍至少回 1（不會回 0 變成「假療傷」）。
        assert_eq!(care_heal_amount(1), 1);
        assert_eq!(care_heal_amount(2), 1);
    }

    #[test]
    fn every_village_npc_has_two_care_back_lines() {
        for id in VILLAGE {
            let l0 = care_back_line(id, 0);
            let l1 = care_back_line(id, 1);
            assert!(l0.is_some(), "{id} 缺回禮台詞");
            assert!(l1.is_some(), "{id} 缺第二句回禮台詞");
            // 兩句不同（真有輪替，不是同一句重覆）。
            assert_ne!(l0, l1, "{id} 兩句回禮台詞應不同");
            // 以 🍵 起頭，與 💚 撫慰領情一眼可辨。
            assert!(l0.unwrap().starts_with("🍵"), "{id} 回禮台詞應以 🍵 起頭");
        }
    }

    #[test]
    fn care_back_line_cycles_and_never_panics() {
        // seed 超出池長度時環繞、不越界 panic。
        assert!(care_back_line("merchant", 100).is_some());
        assert_eq!(care_back_line("merchant", 0), care_back_line("merchant", 2));
        assert_eq!(care_back_line("merchant", 1), care_back_line("merchant", 3));
    }

    #[test]
    fn non_village_npc_has_no_care_back_line() {
        assert!(care_back_line("traveler_npc", 0).is_none());
        assert!(care_back_line("wandering_merchant", 0).is_none());
        assert!(care_back_line("", 0).is_none());
    }
}
