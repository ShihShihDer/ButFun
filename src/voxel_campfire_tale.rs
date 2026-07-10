//! 乙太方界·圍著營火說故事 v1（campfire tale）——夜裡兩位以上醒著的居民聚到同一座營火邊時，
//! 其中一位會把心裡的一段往事（從她自己的記憶庫挑一則，零 LLM）講給火邊的夥伴聽，夥伴聽了
//! 應和一句、把「在火邊聽某人講起往事」記進自己的社交記憶，兩人心情都亮一格。
//!
//! **這一刀補的缺口**：營火（791）讓入夜路過的居民各自駐足取暖，但他們圍著同一堆火卻**彼此無交流**
//! ——每人獨自念句暖語就完事。營火最動人的畫面，是「大家圍著火、聽一個人說故事」。本刀補上這一環：
//! 讓營火第一次成為**社交舞台**，居民的長期記憶不再只留在各自腦中，而是在火邊被講述、被聆聽、
//! 讓交情加溫。這是路線圖「小社會湧現」× 營火場景的內聚接續。
//!
//! **換維度（非同軸重複）**：791 是「居民 vs 火（各自取暖）」；本刀是「居民 vs 居民（圍火分享往事）」，
//! 全新動詞（講故事＋聆聽），且第一次讓玩家蓋的營火成為居民之間的社交場所。與口耳相傳（694·gossip）
//! 的分界：gossip 是**老朋友登門到訪**時主人轉述最近見聞、訪客記進**episodic 記憶**；本刀是**夜裡任兩位
//! 醒著的居民恰好聚在同一座營火邊**時分享**任一則往事**、聆聽者記進**社交記憶**（social store）——
//! 一個管「登門轉述近事」、一個管「圍火講起往事」，場景、觸發、落點都不同。
//!
//! **純函式層**：挑往事、三閘、講述／聆聽台詞、社交摘要、Feed 皆為確定性純函式，零 LLM、零鎖、
//! 零 IO、可單元測試。配對／鎖／擲骰／持久化觸發全留在 `voxel_ws.rs`（沿用既有居民配對快照與
//! 鎖外事件佇列慣例，守 prod 死鎖鐵律）。

use crate::voxel_memory::MemoryEntry;

/// 講述冷卻（秒）：一位居民講完一段往事後隔這麼久才會再開講，防同一人在火邊連珠炮洗版。
pub const TALE_COOLDOWN_SECS: f32 = 200.0;
/// 每次符合條件（夜晚＋兩人同在一座火邊＋講述冷卻到期）時真的開講的機率——其餘時候只是靜靜圍火。
pub const TALE_CHANCE: f32 = 0.5;
/// 聆聽者延遲幾秒後才應和（沿用社交回應的自然節奏，別讓兩人同一 tick 齊聲）。
pub const TALE_REPLY_DELAY_SECS: f32 = 3.0;
/// 泡泡字元上限（與既有社交泡泡同框，超長截斷不破框）。
pub const TALE_SAY_CHARS: usize = 40;
/// 社交摘要裡保留往事原文的字元數（遠低於記憶摘要上限，避免爆長）。
const TALE_SNIPPET_CHARS: usize = 26;

/// 從講述者的長期記憶挑一則可講的往事。
///
/// 排除規則（比照 gossip，避免無意義／無窮遞迴／洩漏內部標記）：
/// - 摘要為空。
/// - 已是「轉述」本身（以「聽」開頭）——只講第一手往事，不接力別人的八卦鏈。
/// - 帶了內部識別前綴（`voxel_diary::is_internal_tagged`，如 `🏘️鄰里`／`🪧讀到告示牌`）——這些前綴
///   只給日記端分類用、不是給玩家看的文字，若被挑中會直接顯在居民頭上的說故事泡泡裡洩漏出去。
///
/// 純函式、確定性：多筆候選時取 `seq` 最大（最新）者。`memories` 由呼叫端以 `all_memories_for`
/// 取得（已按 seq 由新到舊排序，但此處不倚賴傳入順序，自行取最新）。
pub fn pick_tale(memories: &[MemoryEntry]) -> Option<&MemoryEntry> {
    memories
        .iter()
        .filter(|e| {
            !e.summary.trim().is_empty()
                && !e.summary.starts_with('聽')
                && !crate::voxel_diary::is_internal_tagged(&e.summary)
        })
        .max_by_key(|e| e.seq)
}

/// 兩閘判定：講述冷卻到期（`cooldown <= 0`）＋過機率門檻（`roll < chance`）→ 這一 tick 開講。
/// 「兩人同在一座火邊」由呼叫端配對時判定，不進本函式（保持純粹好窮舉測）。
pub fn should_tell(cooldown: f32, roll: f32, chance: f32) -> bool {
    cooldown <= 0.0 && roll < chance
}

/// 講述者的說故事泡泡：開場白＋往事摘要，整句控制在 [`TALE_SAY_CHARS`] 內（開場白先佔位、
/// 往事原文截到剩餘額度），超長不破泡泡框。`pick` 由呼叫端用座標 bits 合成，讓開場自然分散。
pub fn tale_bubble(summary: &str, pick: usize) -> String {
    const OPENERS: [&str; 4] = [
        "跟你說個往事——",
        "我想起一件事……",
        "說起來啊，",
        "讓我跟你說說——",
    ];
    let opener = OPENERS[pick % OPENERS.len()];
    let budget = TALE_SAY_CHARS.saturating_sub(opener.chars().count());
    let body: String = summary.trim().chars().take(budget).collect();
    format!("{opener}{body}")
}

/// 聆聽者的應和泡泡（通用、四句輪替、≤ [`TALE_SAY_CHARS`]，不破框）。
pub fn listener_bubble(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "真好呢……後來呢？",
        "聽你這麼一說，我也想起些往事了。",
        "在火邊聽你講故事，真舒服。",
        "原來你也有這樣的往事啊。",
    ];
    LINES[pick % LINES.len()]
}

/// 聆聽者記進社交記憶的摘要（「在營火邊聽X講起往事：…」，走既有 `SocialStore`／`append_social`，
/// 零新持久化格式）。往事原文截斷、去換行，避免爆長或破壞 jsonl 一行一筆。
pub fn listen_social_summary(teller_name: &str, tale_summary: &str) -> String {
    let snippet: String = tale_summary.trim().chars().take(TALE_SNIPPET_CHARS).collect();
    format!("在營火邊聽{teller_name}講起往事：{snippet}")
        .replace('\n', " ")
}

/// 動態牆播報（訪客回來能讀到誰在火邊向誰講了故事）。
pub fn tale_feed_line(teller_name: &str, listener_name: &str) -> String {
    format!("{teller_name}在營火邊向{listener_name}講起一段往事，火光裡都是故事。")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(resident: &str, player: &str, summary: &str, seq: u64) -> MemoryEntry {
        MemoryEntry {
            resident: resident.to_string(),
            player: player.to_string(),
            summary: summary.to_string(),
            seq,
        }
    }

    #[test]
    fn pick_tale_takes_newest_valid() {
        let mems = vec![
            mem("vox_res_0", "諾娃", "第一次見到旅人諾娃", 1),
            mem("vox_res_0", "露娜", "和露娜一起蓋了口井", 5),
            mem("vox_res_0", "諾娃", "", 6),                 // 空摘要排除
            mem("vox_res_0", "諾娃", "聽露娜說她去了邊陲", 7), // 轉述（聽開頭）排除
        ];
        let got = pick_tale(&mems).expect("應挑到一則往事");
        assert_eq!(got.seq, 5, "應取最新的有效往事（seq=5），跳過空與轉述");
    }

    #[test]
    fn pick_tale_skips_internal_tagged_memories() {
        // 帶內部識別前綴的記憶（鄰里生活 NEIGHBORLY_TAG／讀牌 SIGN_MEMORY_TAG）雖是最新，
        // 也不該被挑進頭上說故事泡泡（否則「🏘️鄰里…」原始標記會直接顯給玩家看）。
        let mems = vec![
            mem("vox_res_0", "露娜", "和露娜一起蓋了口井", 5),
            mem("vox_res_0", "諾娃", &crate::voxel_diary::tag_neighborly("跟諾娃分了一顆果子"), 8),
            mem(
                "vox_res_0",
                "旅人",
                &format!("{}廣場的告示牌", crate::voxel_readsign::SIGN_MEMORY_TAG),
                9,
            ),
        ];
        let got = pick_tale(&mems).expect("應退回乾淨往事");
        assert_eq!(got.seq, 5, "應跳過帶內部標記的較新記憶，取乾淨的 seq=5");
    }

    #[test]
    fn pick_tale_none_when_all_excluded() {
        // 全空 / 全轉述 / 空清單 → None。
        assert!(pick_tale(&[]).is_none());
        let only_relay = vec![mem("vox_res_1", "x", "聽人說過的事", 3), mem("vox_res_1", "x", "  ", 4)];
        assert!(pick_tale(&only_relay).is_none());
    }

    #[test]
    fn should_tell_needs_both_gates() {
        assert!(should_tell(0.0, 0.1, TALE_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_tell(5.0, 0.1, TALE_CHANCE));
        // 骰過門檻 → 否；邊界 roll == chance 不觸發（嚴格小於）。
        assert!(!should_tell(0.0, TALE_CHANCE, TALE_CHANCE));
        assert!(!should_tell(0.0, 0.99, TALE_CHANCE));
    }

    #[test]
    fn tale_bubble_rotates_and_fits_frame() {
        let short = tale_bubble("和露娜一起蓋了口井", 0);
        assert!(short.contains("和露娜"));
        assert_ne!(tale_bubble("x", 0), tale_bubble("x", 1), "開場白應輪替");
        // 超長往事應被截到泡泡框內（開場白 + 往事截斷 ≤ 上限）。
        let long_tale: String = "很久很久以前有一段非常非常非常漫長說也說不完的往事".repeat(3);
        let b = tale_bubble(&long_tale, 2);
        assert!(b.chars().count() <= TALE_SAY_CHARS, "應在泡泡上限內：{}（{}字）", b, b.chars().count());
    }

    #[test]
    fn listener_bubble_rotates_and_fits_frame() {
        for p in 0..8 {
            let l = listener_bubble(p);
            assert!(!l.is_empty());
            assert!(l.chars().count() <= TALE_SAY_CHARS, "聆聽泡泡應在上限內：{l}");
        }
        assert_ne!(listener_bubble(0), listener_bubble(1));
    }

    #[test]
    fn listen_summary_embeds_name_no_newline() {
        let s = listen_social_summary("露娜", "和諾娃一起圍著營火\n取暖，暖進了心裡");
        assert!(s.contains("露娜"));
        assert!(s.starts_with("在營火邊聽"));
        assert!(!s.contains('\n'), "社交摘要不得含換行（jsonl 一行一筆）：{s}");
    }

    #[test]
    fn feed_line_embeds_both_names() {
        let f = tale_feed_line("露娜", "諾娃");
        assert!(f.contains("露娜") && f.contains("諾娃"));
    }
}
