//! 乙太方界·長椅並坐閒聊 v1（bench chat）——白天，兩位交情已到相識以上的居民恰好都走到
//! 玩家擺的**同一張長椅**邊時，其中一位招呼另一位一起坐下，並肩歇著、閒聊上幾句家常，
//! 兩人心情都亮一格、把「和某人並肩坐在長椅上聊了會兒」記進彼此的交情——你親手做的那張
//! 長椅，第一次成了村子**白天**的社交角落。
//!
//! **這一刀補的缺口**：木長椅（809）讓白天路過的居民停下腳步**獨自**坐下歇腳，補上了「走累了
//! 找張椅子坐」這一日常拍——但那始終是**一個人**的休息，兩位居民就算前後腳坐上同一張椅子，
//! 也各歇各的、彼此無交流。長椅最有生活氣息的畫面，是「兩個熟人並肩坐在椅上有一搭沒一搭地
//! 閒聊」。本刀補上這一環：讓長椅第一次成為居民之間的**白日社交舞台**，交情在並肩閒坐裡加溫。
//! 這正對著營火那條夜間社交線（圍火講故事 792）——營火＝夜裡圍**火**講**往事**，長椅＝白天並**坐**
//! 聊**家常**，把「白天／夜晚 × 火／椅 × 往事／家常」對成完整的一雙。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **木長椅（809）**＝居民 vs 椅（**獨自**坐下歇腳）；本刀＝居民 vs 居民（**並肩**坐下閒聊），
//!   全新社交動詞（招呼同坐＋閒聊應和），第一次讓玩家的長椅成為兩位居民之間的相聚點。
//! - **圍火講故事（792）**＝**夜間**、圍**營火**、講一段**長期記憶裡的往事**、聆聽者記進**社交記憶**；
//!   本刀＝**白天**、坐**長椅**、聊**當下的家常閒話**（不翻舊記憶）、雙方各記一筆**交情**（record_visit
//!   加溫、可能升級）——時段、吸引物、話題、落點皆不同。
//! - **一般社交攀談（rel）**＝任意位置擦身而過就攀談；本刀＝**恰好都在同一張長椅邊**才觸發、且
//!   **交情要到相識以上**（記憶驅動行為的閘：只有處出交情的兩人才會招呼彼此並肩坐下）。
//!
//! **純函式層**：三閘、情誼閘、招呼／應和台詞、交情記憶、Feed 皆為確定性純函式，零 LLM、零鎖、
//! 零 async、零 IO、可窮舉單元測試。配對／鎖／擲骰／持久化觸發全留在 `voxel_ws.rs`（沿用圍火講
//! 故事 792 那條已驗證的居民配對快照＋鎖外事件佇列＋短鎖循序慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護鐵律**：零 LLM（招呼／應和／記憶／Feed 全為固定模板、確定性選句）、零持久化
//! 格式新增（並坐冷卻／待應和皆純記憶體欄位、重啟歸零，比照 792；交情走既有 bonds jsonl、記憶走
//! 既有 append-only 社交管線）、零新協議欄位（只改居民 say／內部狀態，走既有泡泡廣播）、零新美術、
//! 零前端改動、FPS 零影響（配對掃描與既有 792/800 掃描同一份 `snaps` 快照、每 tick 最多一對、長冷卻
//! ＋機率門檻＝天然節流）。**永不回放記憶原文或玩家原話**——閒聊台詞只嵌居民**顯示名**（本就出現在
//! 問候／動態牆），無注入／NSFW 面。

use crate::voxel_bonds::BondTier;

/// 並坐閒聊冷卻（秒）：一位居民招呼別人並坐聊完後隔這麼久才會再起哄，防同一人在椅邊連珠炮洗版。
/// 設長（3 分半）：白日並坐是偶爾的悠閒一拍，不是每次路過都拉人坐下。
pub const CHAT_COOLDOWN_SECS: f32 = 210.0;

/// 每次符合條件（白天＋兩人同在一張長椅邊＋交情夠＋發起者冷卻到期）時真的招呼並坐的機率——
/// 其餘時候只是各自路過或獨自歇腳（809）。刻意偏低：並肩閒聊是可遇的悠閒時光，不是走近就觸發。
pub const CHAT_CHANCE: f32 = 0.45;

/// 被招呼者延遲幾秒後才應和（沿用社交回應的自然節奏，別讓兩人同一 tick 齊聲）。
pub const CHAT_REPLY_DELAY_SECS: f32 = 2.5;

/// 兩人要同時離**同一張**長椅這麼近（世界方塊，XZ 平面）才算「都在這張椅邊、坐得下並肩」。
/// 比獨自歇腳半徑（[`crate::voxel_bench::REST_RADIUS`] 3.0）稍寬——並坐是招呼近旁的熟人一起坐下，
/// 給彼此走到椅邊留點餘裕，但仍夠近才算同一張椅。
pub const CHAT_RADIUS: f32 = 4.5;

/// 泡泡字元上限（與既有社交泡泡同框，超長截斷不破框）。
pub const CHAT_SAY_CHARS: usize = 40;

/// 這對居民的交情夠不夠深、值不值得招呼彼此並肩坐下——相識（`Acquaintance`）以上才會，
/// 陌生人各歇各的（比照 800 分食的相識門檻＝記憶驅動行為的閘）。純函式。
pub fn tier_allows_chat(tier: BondTier) -> bool {
    tier >= BondTier::Acquaintance
}

/// 兩閘判定：並坐冷卻到期（`cooldown <= 0`）＋過機率門檻（`roll < chance`）→ 這一 tick 招呼並坐。
/// 「兩人同在一張椅邊」與「交情夠」由呼叫端配對時判定，不進本函式（保持純粹好窮舉測）。
pub fn should_chat(cooldown: f32, roll: f32, chance: f32) -> bool {
    cooldown <= 0.0 && roll < chance
}

/// 發起者招呼對方一起坐下、並肩閒聊的開場泡泡（點名對方，更親近）——五句輪替，
/// `pick` 由呼叫端用座標 bits 合成，讓每次挑到的句子自然分散。超長名截斷不破泡泡框。
pub fn opener_line(other: &str, pick: usize) -> String {
    let name = clip_name(other);
    const TEMPLATES: [&str; 5] = [
        "{name}，來坐這歇會兒，一起聊聊吧。",
        "{name}，難得都在，並肩坐著閒聊兩句。",
        "{name}，走累了吧？坐下來說說話。",
        "來，{name}，坐這兒陪我曬曬太陽。",
        "{name}，好久沒好好聊了，坐會兒吧。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 被招呼者並坐後應和的泡泡（點名發起者，一來一往）——五句輪替、字數短不破框。
pub fn reply_line(opener: &str, pick: usize) -> String {
    let name = clip_name(opener);
    const TEMPLATES: [&str; 5] = [
        "好啊{name}，正想歇歇腳呢。",
        "跟{name}並肩坐著，日子真愜意。",
        "{name}，這椅子坐著真舒坦。",
        "難得清閒，就陪{name}坐會兒。",
        "有{name}作伴，聊聊天真好。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 昇華成一筆「和某人並肩坐在長椅上閒聊」的交情記憶（點名對方、不含換行，走既有社交記憶管線）。
/// 空名退泛稱，不留突兀的「和，」。
pub fn chat_memory_line(other: &str) -> String {
    let name = clip_name(other);
    let who = if name.trim().is_empty() { "一位鄰居" } else { &name };
    format!("白天並肩坐在長椅上，和{who}有一搭沒一搭地閒聊了會兒，愜意得很。").replace('\n', " ")
}

/// 動態牆播報（訪客回來能讀到誰跟誰在長椅上並坐閒聊過）。任一空名退泛稱。
pub fn chat_feed_line(a: &str, b: &str) -> String {
    let na = if a.trim().is_empty() { "一位居民" } else { a };
    let nb = if b.trim().is_empty() { "一位鄰居" } else { b };
    format!("{na}和{nb}並肩坐在長椅上，曬著太陽閒聊了會兒。")
}

/// 居民名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_gate_needs_acquaintance_or_above() {
        // 陌生人不招呼並坐；相識以上才會（記憶驅動行為的閘）。
        assert!(!tier_allows_chat(BondTier::Stranger));
        assert!(tier_allows_chat(BondTier::Acquaintance));
        assert!(tier_allows_chat(BondTier::Friend));
    }

    #[test]
    fn should_chat_two_gates_and_boundary() {
        // 冷卻到期 + 骰過門檻才觸發。
        assert!(should_chat(0.0, 0.1, CHAT_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_chat(5.0, 0.1, CHAT_CHANCE));
        // 邊界：roll == chance 不觸發（嚴格小於）。
        assert!(!should_chat(0.0, CHAT_CHANCE, CHAT_CHANCE));
        // 骰過頭 → 否。
        assert!(!should_chat(0.0, 0.99, CHAT_CHANCE));
    }

    #[test]
    fn opener_and_reply_rotate_embed_name_in_frame() {
        // 招呼句含對方名、輪替、非空。
        let o0 = opener_line("諾娃", 0);
        let o1 = opener_line("諾娃", 1);
        assert!(o0.contains("諾娃"));
        assert_ne!(o0, o1);
        // 應和句含發起者名、輪替。
        let r0 = reply_line("露娜", 0);
        let r1 = reply_line("露娜", 1);
        assert!(r0.contains("露娜"));
        assert_ne!(r0, r1);
        // 超長名截斷不破泡泡框。
        let long = opener_line("超級無敵長長長長長長長名字", 2);
        assert!(long.chars().count() < 40, "超長名應被截斷：{long}");
    }

    #[test]
    fn pick_overflow_wraps_by_modulo() {
        // pick 遠超句數（座標 bits 合成可能很大）不 panic、循環取句。
        assert!(!opener_line("賽勒", usize::MAX).is_empty());
        assert!(!reply_line("賽勒", usize::MAX).is_empty());
    }

    #[test]
    fn memory_embeds_name_no_newline_and_empty_falls_back() {
        let m = chat_memory_line("諾娃\n注入");
        assert!(m.contains("諾娃"));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
        // 空名退泛稱，不留突兀的「和，」。
        let e = chat_memory_line("");
        assert!(e.contains("一位鄰居"));
        assert!(!e.contains("和，"));
    }

    #[test]
    fn feed_embeds_both_names_and_empty_falls_back() {
        let f = chat_feed_line("露娜", "諾娃");
        assert!(f.contains("露娜") && f.contains("諾娃"));
        // 任一空名退泛稱。
        let e = chat_feed_line("", "諾娃");
        assert!(e.contains("一位居民") && e.contains("諾娃"));
    }

    #[test]
    fn long_names_never_panic() {
        // 超長名在各台詞／記憶都不 panic（截斷保護）。
        let big = "字".repeat(200);
        let _ = opener_line(&big, 3);
        let _ = reply_line(&big, 3);
        let _ = chat_memory_line(&big);
        let _ = chat_feed_line(&big, &big);
    }
}
