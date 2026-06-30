//! 念頭播種閉環——玩家說話的空間廣播純邏輯（零 LLM）。
//!
//! 玩家對某位居民說話時，附近其他居民「聽到」——若語句戳中個性，居民冒反應泡泡、
//! 甚至把玩家的話化為自己的心願（由建造系統實現）。全程零 LLM，確定性純函式。
//!
//! 設計依據：`docs/PLAN_ETHERVOX.md` §「對話 / 社交 / 內心系統設計」
//! - 聽到 ≠ 要回：聽到零 LLM，進記憶/起念；「要回」才花 LLM（此切片不觸發 LLM 回覆）。
//! - 念頭播種閉環：你的話被 overhear → 若戳中個性 → 起念 → 後由建造系統實現。

use crate::resident_npc::ResidentPersona;

/// 玩家說話的「廣播半徑」（方塊距離，二維 xz 平面）。
pub const OVERHEAR_RADIUS: f32 = 20.0;
/// 聽到後氣泡反應顯示秒數。
pub const REACTION_SAY_SECS: f32 = 4.0;

// ── embodied 靠近說話 v1：指名選擇 + 旁聽搭話閘（皆零 LLM、純函式）──────────────
//
// 設計依據 `docs/PLAN_ETHERVOX.md` §「對話 / 社交 / 內心系統設計」：
// - 範圍說話：說話有半徑、對誰講＝走近/面對誰（`pick_addressed`）。
// - 聽到 ≠ 要回：旁聽默默進記憶；「要不要回」走便宜閘（`should_chime_in`），多半不開口。
// - 防對話風暴三閘：①每居民搭話冷卻 ②機率×個性(外向度) ③戳中度。

/// 指名選擇的「面對加權」：在玩家正前方的居民，有效距離最多打這麼多折
/// （0=純最近、1=完全以面對取勝）。讓「面對誰＝對誰說」，又不至於忽略距離。
pub const ADDRESSED_FACING_WEIGHT: f32 = 0.4;
/// 旁聽搭話冷卻（秒）：某居民搭過一句話後要等這麼久才會再搭，防同一位連發洗版。
pub const OVERHEAR_CHIME_COOLDOWN_SECS: f32 = 30.0;
/// 沒戳中個性時的搭話基準機率（極低，偶爾因外向才開口）。
pub const CHIME_BASE_CHANCE: f32 = 0.05;
/// 戳中個性時的搭話機率（明顯升高，但仍乘上外向度與冷卻）。
pub const CHIME_FIT_CHANCE: f32 = 0.6;

/// 指名選擇：玩家「範圍說話」時，從半徑內的居民挑出「被指名」回話的那位。
/// 規則＝最近 ＋ 面對加權：在玩家正前方的居民「感覺更近」（有效距離打折），
/// 沒人在正前方時自然退化成純最近。`yaw` 與前端一致（前向 = (-sin, -cos)）。
/// `positions` 是各居民的 (x, z)；回傳被指名居民在切片中的索引，半徑內無人 → None。
/// 純函式、零鎖零 LLM，可測。
pub fn pick_addressed(px: f32, pz: f32, yaw: f32, positions: &[(f32, f32)], radius: f32) -> Option<usize> {
    let fwd_x = -yaw.sin();
    let fwd_z = -yaw.cos();
    let r2 = radius * radius;
    let mut best: Option<(usize, f32)> = None;
    for (i, &(rx, rz)) in positions.iter().enumerate() {
        let dx = rx - px;
        let dz = rz - pz;
        let d2 = dx * dx + dz * dz;
        if d2 > r2 {
            continue; // 半徑外聽不到、更不會被指名
        }
        let dist = d2.sqrt();
        // 面對度：方向單位向量與前向的內積取正值（0..1），越正＝越在正前方。
        let facing = if dist > 1e-4 {
            ((dx / dist) * fwd_x + (dz / dist) * fwd_z).max(0.0)
        } else {
            1.0 // 站在同格＝視為正對
        };
        let score = dist * (1.0 - ADDRESSED_FACING_WEIGHT * facing);
        if best.map_or(true, |(_, bs)| score < bs) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

/// 各個性的「外向度」乘數：話多/愛社交的個性較常搭話，內向的多半只聽不講。
/// 純函式、確定性。
pub fn persona_extroversion(persona: ResidentPersona) -> f32 {
    match persona {
        ResidentPersona::TownSquare => 1.2,    // 廣場社交咖：最愛搭話
        ResidentPersona::MarketBrowser => 1.0, // 市集人：常與人攀談
        ResidentPersona::Wanderer => 0.7,      // 漫遊者：偶爾插話
        ResidentPersona::FarmWorker => 0.5,    // 農夫：埋頭做事、話少
    }
}

/// 旁聽搭話便宜閘（零 LLM）：範圍內、非被指名的居民「要不要搭一句」。
/// 戳中度(個性關鍵詞) × 外向度 × 冷卻 → 機率 → 擲骰。**多數時候只聽不講**，防對話風暴。
/// `roll` 由呼叫端傳入 `rand::random::<f32>()`，方便測試釘住。
pub fn should_chime_in(fits_persona: bool, extroversion: f32, cooldown_ok: bool, roll: f32) -> bool {
    if !cooldown_ok {
        return false; // 冷卻未到：這位剛搭過，先安靜
    }
    let base = if fits_persona { CHIME_FIT_CHANCE } else { CHIME_BASE_CHANCE };
    let chance = (base * extroversion).clamp(0.0, 1.0);
    roll < chance
}

/// 各個性對應的關鍵詞——玩家說的話包含任一個 → 戳中這位居民。
fn persona_keywords(persona: ResidentPersona) -> &'static [&'static str] {
    match persona {
        ResidentPersona::MarketBrowser => {
            &["交易", "買", "賣", "市集", "熱鬧", "商品", "禮物", "攤"]
        }
        ResidentPersona::FarmWorker => {
            &["種", "農", "田", "植物", "收割", "澆", "作物", "土", "蔬菜", "果"]
        }
        ResidentPersona::TownSquare => {
            &["廣場", "鄰居", "大家", "節日", "聚", "社區", "人來人往"]
        }
        ResidentPersona::Wanderer => {
            &["探索", "冒險", "遠方", "發現", "好奇", "走走", "地方", "旅行"]
        }
    }
}

/// 玩家說的話「戳中」這位居民的個性關鍵詞嗎？零 LLM、純字串比對。
pub fn speech_fits_persona(speech: &str, persona: ResidentPersona) -> bool {
    persona_keywords(persona).iter().any(|kw| speech.contains(kw))
}

/// 居民「聽到了」後的 canned 反應台詞（泡泡顯示，不走 LLM，1-2 秒消失）。
pub fn canned_overhear_reaction(persona: ResidentPersona, resident_name: &'static str) -> String {
    let _ = resident_name; // 名字留空間給未來個性化
    let snippet = match persona {
        ResidentPersona::MarketBrowser => "*偷聽到了* 哦，有人在聊這個...",
        ResidentPersona::FarmWorker    => "*偷聽到了* 嗯，這讓我想去田裡看看。",
        ResidentPersona::TownSquare    => "*偷聽到了* 廣場上的朋友也這樣說呢...",
        ResidentPersona::Wanderer      => "*偷聽到了* 聽起來很有趣！想去找找看。",
    };
    snippet.to_string()
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn farm_worker_fits_agricultural_words() {
        assert!(speech_fits_persona("我想種一些作物", ResidentPersona::FarmWorker));
        assert!(speech_fits_persona("這裡可以種田嗎", ResidentPersona::FarmWorker));
        assert!(speech_fits_persona("澆水很重要", ResidentPersona::FarmWorker));
    }

    #[test]
    fn market_browser_fits_trade_words() {
        assert!(speech_fits_persona("我想買個東西", ResidentPersona::MarketBrowser));
        assert!(speech_fits_persona("這裡有交易嗎", ResidentPersona::MarketBrowser));
        assert!(speech_fits_persona("市集在哪裡", ResidentPersona::MarketBrowser));
    }

    #[test]
    fn wanderer_fits_exploration_words() {
        assert!(speech_fits_persona("我想去遠方探索", ResidentPersona::Wanderer));
        assert!(speech_fits_persona("好奇這裡有什麼", ResidentPersona::Wanderer));
        assert!(speech_fits_persona("冒險走走吧", ResidentPersona::Wanderer));
    }

    #[test]
    fn town_square_fits_social_words() {
        assert!(speech_fits_persona("廣場上有什麼活動嗎", ResidentPersona::TownSquare));
        assert!(speech_fits_persona("大家都在聚這裡", ResidentPersona::TownSquare));
    }

    #[test]
    fn unrelated_speech_does_not_fit_any() {
        assert!(!speech_fits_persona("天空好藍", ResidentPersona::FarmWorker));
        assert!(!speech_fits_persona("你好", ResidentPersona::Wanderer));
        assert!(!speech_fits_persona("今天天氣不錯", ResidentPersona::MarketBrowser));
        assert!(!speech_fits_persona("方塊真的很多", ResidentPersona::TownSquare));
    }

    #[test]
    fn canned_reaction_non_empty_for_all_personas() {
        for persona in [
            ResidentPersona::MarketBrowser,
            ResidentPersona::FarmWorker,
            ResidentPersona::TownSquare,
            ResidentPersona::Wanderer,
        ] {
            let r = canned_overhear_reaction(persona, "露娜");
            assert!(!r.is_empty(), "persona {persona:?} canned_reaction 不可為空");
        }
    }

    #[test]
    fn overhear_radius_and_secs_positive() {
        assert!(OVERHEAR_RADIUS > 0.0);
        assert!(REACTION_SAY_SECS > 0.0);
    }

    // ── embodied 靠近說話 v1：指名選擇 ──────────────────────────────────────────

    #[test]
    fn pick_addressed_none_when_all_out_of_radius() {
        // 兩位居民都在半徑外 → 沒人被指名（也就沒人會被 LLM 回話）。
        let positions = [(100.0, 0.0), (0.0, 100.0)];
        assert_eq!(pick_addressed(0.0, 0.0, 0.0, &positions, 20.0), None);
    }

    #[test]
    fn pick_addressed_prefers_nearest_when_no_facing_pref() {
        // yaw=0 → 前向 = (0, -1)（朝 -z）。兩位居民都在 +z（背後），面對度=0，
        // 退化成純最近：index 0 在 (0,5) 比 index 1 在 (0,9) 近。
        let positions = [(0.0, 5.0), (0.0, 9.0)];
        assert_eq!(pick_addressed(0.0, 0.0, 0.0, &positions, 20.0), Some(0));
    }

    #[test]
    fn pick_addressed_facing_can_beat_slightly_closer() {
        // yaw=0 → 朝 -z。正前方(-z)稍遠的居民，靠面對折扣勝過背後(+z)稍近的。
        // index 0：背後 (0, 4)，dist=4、facing=0 → score=4。
        // index 1：正前方 (0, -5)，dist=5、facing=1 → score=5*(1-0.4)=3 < 4 → 勝。
        let positions = [(0.0, 4.0), (0.0, -5.0)];
        assert_eq!(pick_addressed(0.0, 0.0, 0.0, &positions, 20.0), Some(1));
    }

    // ── embodied 靠近說話 v1：旁聽搭話閘 ────────────────────────────────────────

    #[test]
    fn chime_gate_blocked_by_cooldown() {
        // 冷卻未到：就算戳中、外向、骰子=0 也絕不搭話（防同一位連發）。
        assert!(!should_chime_in(true, 1.2, false, 0.0));
    }

    #[test]
    fn chime_gate_fit_more_likely_than_unfit() {
        // 戳中個性的搭話機率 > 沒戳中的：取一個落在兩者之間的骰值驗證。
        let extro = 1.0;
        let fit_chance = CHIME_FIT_CHANCE * extro;
        let unfit_chance = CHIME_BASE_CHANCE * extro;
        let mid = (fit_chance + unfit_chance) / 2.0;
        assert!(should_chime_in(true, extro, true, mid));   // 戳中 → 過
        assert!(!should_chime_in(false, extro, true, mid)); // 沒戳中 → 不過
    }

    #[test]
    fn chime_gate_high_roll_never_passes() {
        // 骰子=1.0（最大）永遠不過（機率上限 < 1）。
        assert!(!should_chime_in(true, 1.2, true, 1.0));
    }

    #[test]
    fn persona_extroversion_townsquare_most_outgoing() {
        // 廣場社交咖外向度最高、農夫最低（個性差異有意義）。
        assert!(persona_extroversion(ResidentPersona::TownSquare)
            > persona_extroversion(ResidentPersona::FarmWorker));
        for p in [
            ResidentPersona::TownSquare,
            ResidentPersona::MarketBrowser,
            ResidentPersona::Wanderer,
            ResidentPersona::FarmWorker,
        ] {
            assert!(persona_extroversion(p) > 0.0);
        }
    }
}
