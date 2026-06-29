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
}
