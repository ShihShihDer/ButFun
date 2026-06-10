//! 里長 NPC 純邏輯：位置、村落金庫、村落節慶活動。
//!
//! 設計鐵律（第二個「有手」的 AI NPC）：
//! - **腦子自由、手有界**：LLM 自主決定講什麼、要不要辦活動；引擎只認金庫裡真實有的乙太。
//! - **約束 = 真實稀缺**：金庫見底就做不了——辦活動要花金庫餘額，花完就沒了。
//! - **沒有寫死門檻**：辦不辦、何時辦，由里長看著金庫與往來自己決定（統計只當資料）。
//! - **非同步**：LLM 呼叫在 tokio::spawn 裡，永不阻塞 15Hz 迴圈。
//! - **降級**：ollama 連不到 / 關閉 → 回罐頭句，遊戲不壞。

use crate::npc_chat::NpcRel;

/// 里長 NPC 位置（主城 NPC 列最右側，與農展評審距 120px）。
pub const CHIEF_WX: f32 = 2720.0;
pub const CHIEF_WY: f32 = 2080.0;
/// 互動觸及距離（同其他工職 NPC）。
pub const CHIEF_REACH: f32 = 80.0;

/// 村落金庫初始餘額（乙太）。玩家自願捐獻後會增加；里長辦活動扣減。
pub const INITIAL_TREASURY: u32 = 80;
/// 觸發「村落節慶加成」所需金庫成本（乙太）。
pub const EVENT_COST: u32 = 30;
/// 村落節慶加成持續時間（秒）。
pub const EVENT_DURATION_SECS: u64 = 600; // 10 分鐘
/// 村落節慶加成：全服玩家殺怪/採集所得 EXP +這個百分比。
pub const EVENT_EXP_BONUS_PCT: u32 = 30;
/// 里長自主發動村落活動的暗號（玩家看不到；引擎攔截後抽掉）。
pub const EVENT_TOKEN: &str = "[VILLAGE_EVENT]";
/// 玩家每次捐獻的固定金額（乙太）。
pub const DONATE_AMOUNT: u32 = 10;
/// 村落金庫上限（乙太）。
pub const MAX_TREASURY: u32 = 200;

/// 確認玩家是否在里長互動範圍內。
pub fn is_within_reach(px: f32, py: f32) -> bool {
    let dx = px - CHIEF_WX;
    let dy = py - CHIEF_WY;
    (dx * dx + dy * dy).sqrt() <= CHIEF_REACH
}

/// 村落金庫是否足夠辦一次活動。
pub fn can_afford_event(treasury: u32) -> bool {
    treasury >= EVENT_COST
}

/// 從金庫扣除活動成本。若不足回傳 `None`（拒絕執行）。
pub fn spend_on_event(treasury: u32) -> Option<u32> {
    treasury.checked_sub(EVENT_COST)
}

/// 玩家捐獻後的新金庫餘額（不超過 `MAX_TREASURY` 上限）。
pub fn donate_to_treasury(current: u32, amount: u32) -> u32 {
    (current + amount).min(MAX_TREASURY)
}

/// 偵測 LLM 回話是否決定辦村落活動（夾了暗號），回傳（決定辦?, 乾淨回話）。
pub fn extract_event_decision(raw: &str) -> (bool, String) {
    if raw.contains(EVENT_TOKEN) {
        (true, raw.replace(EVENT_TOKEN, "").trim().to_string())
    } else {
        (false, raw.to_string())
    }
}

/// 組里長專屬 system prompt：世界觀 + 人設 + 玩家印象 + 金庫現況 + 世界近況。
/// 金庫充足時告訴里長有「村落節慶」選項（回話夾暗號）；不足時改鼓勵捐獻。
/// `world_news`：引擎世界事件段落（ROADMAP 65），空字串表示無近況。
pub fn system_prompt(rel: &NpcRel, treasury: u32, world_news: &str, player_activity: &str) -> String {
    const WORLD_LORE: &str = "這是 ButFun，一個蒸汽龐克交織太空歌劇的療癒世界。「大靜默」之後，乙太能量緩緩回流，拓荒者們回到邊境星，在文明的廢墟上重建家園。新手村主城有黃銅城牆、怪物進不來；城外有危險也有資源。";
    const PERSONA: &str = "你是新手村的里長，大家都叫你凱爾長老。你德高望重、溫暖而威嚴，見過大靜默前後的興衰，把守護這個村落視為畢生使命。你平時說話緩慢而有力，喜歡引用老格言，對每一位拓荒者都像看待自己的後輩。村落金庫是全體居民共同積累的信任，你花每一枚乙太都非常謹慎。";

    let imp = if rel.impression.trim().is_empty() {
        "你還不認識這位拓荒者，這是第一次見面。".to_string()
    } else {
        format!("【你對這位拓荒者的印象】{}", rel.impression)
    };

    let stats = format!(
        "【你和這位拓荒者的往來】聊過大約 {} 次。",
        rel.talks
    );

    let treasury_info = format!(
        "【村落金庫現況】目前金庫有 {current}/{max} 乙太。金庫來自拓荒者自願捐獻，你動用的每一枚都代表全村的信任。",
        current = treasury,
        max = MAX_TREASURY
    );

    let event_hint = if can_afford_event(treasury) {
        format!(
            "\n\n若你真心覺得現在是好時機，你可以**自主決定**動用金庫的 {cost} 乙太，舉辦一場「村落節慶」——為期 10 分鐘，全村拓荒者殺怪採集所得經驗加成 {bonus}%。只需在回話自然地提到辦慶典的意願，並在句末加上暗號 {tok}。但這完全看你：金庫是全村的信任，請謹慎使用；若覺得時機未到，就不要加那個暗號。",
            cost = EVENT_COST,
            bonus = EVENT_EXP_BONUS_PCT,
            tok = EVENT_TOKEN
        )
    } else {
        format!(
            "\n\n金庫目前只有 {current} 乙太，辦村落節慶至少需要 {cost} 乙太——請鼓勵拓荒者們多多捐獻，讓金庫充實一些。",
            current = treasury,
            cost = EVENT_COST
        )
    };

    format!(
        "{lore}\n\n{persona}\n\n{imp}\n{stats}\n{treasury}{hint}{player_activity}{world_news}\n\n用繁體中文回話，2 到 3 句，口吻慈祥威嚴、符合世界觀，絕不跳出角色、不要提到你是 AI 或語言模型。",
        lore = WORLD_LORE,
        persona = PERSONA,
        treasury = treasury_info,
        hint = event_hint,
        player_activity = player_activity,
        world_news = world_news,
    )
}

/// 降級罐頭回話（LLM 沒啟用或連不到時）。
pub fn canned_reply() -> String {
    "老朽很高興你來拜訪。村落的興盛，需要每一位拓荒者的努力與信任。".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc_chat::NpcRel;

    #[test]
    fn is_within_reach_detects_nearby() {
        assert!(is_within_reach(CHIEF_WX, CHIEF_WY));
        assert!(is_within_reach(CHIEF_WX + 50.0, CHIEF_WY));
    }

    #[test]
    fn is_within_reach_rejects_far() {
        assert!(!is_within_reach(CHIEF_WX + 100.0, CHIEF_WY));
        assert!(!is_within_reach(CHIEF_WX, CHIEF_WY + 100.0));
    }

    #[test]
    fn can_afford_event_boundary() {
        assert!(can_afford_event(EVENT_COST));
        assert!(can_afford_event(EVENT_COST + 10));
        assert!(!can_afford_event(EVENT_COST - 1));
        assert!(!can_afford_event(0));
    }

    #[test]
    fn spend_on_event_deducts_correctly() {
        assert_eq!(spend_on_event(50), Some(20));
        assert_eq!(spend_on_event(EVENT_COST), Some(0));
        assert_eq!(spend_on_event(EVENT_COST - 1), None);
        assert_eq!(spend_on_event(0), None);
    }

    #[test]
    fn donate_to_treasury_caps_at_max() {
        assert_eq!(donate_to_treasury(195, DONATE_AMOUNT), 200);
        assert_eq!(donate_to_treasury(MAX_TREASURY, DONATE_AMOUNT), MAX_TREASURY);
        assert_eq!(donate_to_treasury(0, DONATE_AMOUNT), DONATE_AMOUNT);
    }

    #[test]
    fn extract_event_decision_detects_token() {
        let raw = format!("讓我們辦個慶典吧！{}", EVENT_TOKEN);
        let (got, clean) = extract_event_decision(&raw);
        assert!(got, "含暗號時應偵測到辦活動意圖");
        assert!(!clean.contains(EVENT_TOKEN), "乾淨回話不應含暗號");
        assert!(clean.contains("讓我們辦個慶典"), "乾淨回話應保留對話文字");
    }

    #[test]
    fn extract_event_decision_no_token() {
        let raw = "感謝你的造訪，孩子。";
        let (got, clean) = extract_event_decision(raw);
        assert!(!got, "無暗號時不應偵測到活動意圖");
        assert_eq!(clean, raw);
    }

    #[test]
    fn system_prompt_shows_treasury_and_event_hint_when_affordable() {
        let s = system_prompt(&NpcRel::default(), 50, "", "");
        assert!(s.contains("50"), "prompt 應包含金庫餘額 50");
        assert!(s.contains(EVENT_TOKEN), "金庫充足時 prompt 應包含活動暗號說明");
    }

    #[test]
    fn system_prompt_no_event_hint_when_treasury_insufficient() {
        let s = system_prompt(&NpcRel::default(), EVENT_COST - 1, "", "");
        assert!(!s.contains(EVENT_TOKEN), "金庫不足時 prompt 不應含活動暗號");
        assert!(s.contains("捐獻"), "金庫不足時應鼓勵捐獻");
    }

    #[test]
    fn system_prompt_includes_world_news_when_provided() {
        let news = "\n\n【近期世界大事（引擎紀錄・純事實，你可自然提及）】\n・村落節慶剛剛結束\n";
        let s = system_prompt(&NpcRel::default(), 50, news, "");
        assert!(s.contains("近期世界大事"), "世界近況應注入里長 prompt：{s}");
        assert!(s.contains("節慶剛剛結束"), "事件文字應在 prompt 中：{s}");
    }

    #[test]
    fn system_prompt_no_world_news_when_empty() {
        let s = system_prompt(&NpcRel::default(), 50, "", "");
        assert!(!s.contains("近期世界大事"), "無世界近況時 prompt 不應含此段落：{s}");
    }

    #[test]
    fn canned_reply_is_not_empty() {
        assert!(!canned_reply().is_empty());
    }

    #[test]
    fn constants_are_sane() {
        assert!(EVENT_COST > 0, "活動成本必須大於 0");
        assert!(INITIAL_TREASURY >= EVENT_COST, "初始金庫應能辦至少一次活動");
        assert!(MAX_TREASURY > INITIAL_TREASURY, "上限應大於初始值");
        assert!(EVENT_EXP_BONUS_PCT > 0 && EVENT_EXP_BONUS_PCT <= 100);
        assert!(EVENT_DURATION_SECS >= 60, "活動至少持續 1 分鐘");
        assert!(DONATE_AMOUNT > 0);
    }

    #[test]
    fn event_token_distinct_from_npc_chat_tokens() {
        use crate::npc_chat::{DISCOUNT_TOKEN, GIFT_TOKEN};
        assert_ne!(EVENT_TOKEN, GIFT_TOKEN, "活動暗號不能與送禮暗號相同");
        assert_ne!(EVENT_TOKEN, DISCOUNT_TOKEN, "活動暗號不能與折扣暗號相同");
    }
}
