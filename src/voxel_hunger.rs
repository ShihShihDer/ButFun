//! 乙太方界·居民也會肚子餓 v1——居民第一個「生理需求」（自主提案切片）。
//!
//! **缺口 / 為誰做**：乙太方界的居民至今有一整套**情緒**內心（心情 677、孤獨尋伴 678、
//! 掏心 confide、渴望 desires…），會記得你、形成關係、蓋家、種田——但牠們從沒有過一個
//! **身體上的需求**。牠們不會餓、不會累到得吃點東西，「活著」少了最基本的一拍：肚子餓。
//! 這正對著 PLAN_ETHERVOX 的核心信念——**記憶／狀態要驅動行為，讓居民真的活著**。本刀給
//! 居民第一個生理需求：**餓意**。它隨時間默默累積，餓了居民會冒一句「肚子有點餓了…」的
//! 心聲、放下閒晃走回家找點存糧吃，吃飽了滿足地舒一口氣——一個由**內在需求驅動**的自理
//! 行為，玩家第一次看見居民為了照顧自己的身體而行動。
//!
//! **交織點（你的善意踩在對的時間點上）**：而如果就在牠正餓的時候，你剛好餵了牠一口吃的
//! （沿用既有送食物→細細享用管線 765），牠會**記得格外深**——不是普通的一句道謝，而是
//! 「你在我正餓的時候餵了我」這樣一筆掛在你名下的暖記憶，你的餽贈第一次不只被收下，還**正好
//! 落在牠最需要的時刻**。需求驅動行為，你的互動因此更有後果。
//!
//! **與既有系統的分界（換維度·非同軸重複）**：這不是 678「孤獨尋伴」（**情緒**缺口→走向
//! **玩家**求陪）——餓是**生理**缺口、居民走向**自己的家**吃存糧、自己就能滿足，不黏玩家；
//! 也不是 664「拜託你幫個小忙」（`open_request`：討**建材**、一次性的人情）——餓是會隨時間
//! **反覆累積**的持續狀態、由居民**自理**（回家吃）為主、玩家餵食只是錦上添花的加深記憶。
//! 這是居民的第一個**需求 (need)**，開「需求驅動行為」這條至今空白的維度。
//!
//! **這裡只放確定性純邏輯**（餓意累積、門檻判定、台詞／記憶／Feed 文案），零 LLM、零鎖、
//! 零 IO、零 async，可單元測試。連線 / 鎖 / 走動 / 廣播全留在 `voxel_ws.rs`（沿用尋伴／
//! 致意的短鎖循序 + 逐 tick 重設目標慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：不觸發 LLM、不開對外端點、不收玩家自由文字——餓／飽台詞與記憶全為
//! 確定性模板、只嵌玩家**顯示名**（本就出現在道謝／記憶），永不回放記憶原文或玩家原話（無
//! 注入 / NSFW 面）；餓意純伺服器 tick 累積、玩家無法自報；純記憶體、重啟歸零（餓意是數
//! 分鐘的過場狀態，重啟大不了少餓一次、零資料風險、零 migration），不碰玩家資料 / 帳號權限。

/// 飽足時餓意為 0.0、餓到極點為 [`HUNGER_MAX`]。
pub const HUNGER_MAX: f32 = 1.0;

/// 餓意累積速率（每秒）：約 15 分鐘從全飽累到餓極。刻意慢——餓是偶爾一次的生活節拍、
/// 不是每分鐘的騷擾，稀少才有份量。
pub const HUNGER_RATE_PER_SEC: f32 = 1.0 / 900.0;

/// 「餓了」門檻：餓意越過這條線（約 10.5 分鐘沒進食）居民才開始想找點吃的。
pub const HUNGRY_THRESHOLD: f32 = 0.70;

/// 走到家域中心多近，算「到家、吃得上存糧」（世界方塊）。
pub const EAT_ARRIVE_DIST: f32 = 3.0;

/// 冒餓／吃飽後的靜默冷卻（秒）：一位居民喊過餓或剛吃飽後，這段時間內不再喊餓，
/// 避免反覆碎念、讓「餓了」這件事稀少而有感。
pub const HUNGER_SAY_COOLDOWN: f32 = 120.0;

/// 餓意隨時間累積 `dt` 秒（clamp 到 `[0, HUNGER_MAX]`）。純函式、確定性、可測。
pub fn tick_hunger(cur: f32, dt: f32) -> f32 {
    (cur + HUNGER_RATE_PER_SEC * dt).clamp(0.0, HUNGER_MAX)
}

/// 餓意是否已越過門檻、居民會想找吃的。
pub fn is_hungry(h: f32) -> bool {
    h >= HUNGRY_THRESHOLD
}

/// 入場錯開初始靜默冷卻（秒），避免啟動後短時間內全員一起喊餓。
pub fn hunger_cd_offset(i: usize) -> f32 {
    60.0 + i as f32 * 45.0
}

/// 居民冒「肚子餓了」心聲的台詞（起身回家找吃的那一刻，四句輪替）。
pub fn hunger_say_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "肚子有點餓了…回家找點吃的吧",
        "唔，該吃點東西了",
        "肚子在叫了，回去墊墊肚子",
        "有點餓，去翻翻存糧",
    ];
    LINES[pick % LINES.len()]
}

/// 居民回到家、吃上存糧後滿足的暖泡泡（四句輪替）。
pub fn sated_say_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "吃飽了，舒服～",
        "嗯，這下有力氣了",
        "肚子暖暖的，滿足",
        "填飽了肚子，真好",
    ];
    LINES[pick % LINES.len()]
}

/// 「你在我正餓的時候餵了我」——玩家在居民餓時餵食，居民掛在玩家名下的深記憶。
/// `player` 空（訪客無顯示名）→ 退成不點名的泛稱，仍不回放任何原話。
pub fn fed_memory_line(player: &str) -> String {
    if player.is_empty() {
        "有人在我正餓的時候餵了我一口，這份好，我記得特別牢。".to_string()
    } else {
        format!("{player}在我正餓的時候餵了我一口，這份好，我記得特別牢。")
    }
}

/// 玩家餓時餵食的城鎮動態牆一行。`player` 空 → 泛稱「有人」。
pub fn fed_feed_line(rname: &str, player: &str) -> String {
    let who = if player.is_empty() { "有人" } else { player };
    format!("{rname}正餓著，{who}剛好餵了一口——這份暖，記得格外深。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunger_accumulates_and_clamps() {
        // 從全飽開始累積：正向增加。
        let h1 = tick_hunger(0.0, 60.0);
        assert!(h1 > 0.0 && h1 < HUNGER_MAX);
        // 大 dt 也不會超過上限。
        assert_eq!(tick_hunger(0.9, 100_000.0), HUNGER_MAX);
        // 已在上限：維持上限、不溢位。
        assert_eq!(tick_hunger(HUNGER_MAX, 10.0), HUNGER_MAX);
        // 負餓意（理論上不會發生）也夾回 0 以上。
        assert_eq!(tick_hunger(-5.0, 0.0), 0.0);
    }

    #[test]
    fn rate_reaches_threshold_in_expected_time() {
        // 約 10.5 分鐘（630 秒）應恰好越過門檻，不會太快也不會太慢。
        let h = tick_hunger(0.0, HUNGRY_THRESHOLD / HUNGER_RATE_PER_SEC);
        assert!((h - HUNGRY_THRESHOLD).abs() < 1e-4);
        // 略早於此則尚未餓。
        assert!(!is_hungry(tick_hunger(0.0, 600.0)));
    }

    #[test]
    fn is_hungry_threshold_boundary() {
        assert!(!is_hungry(HUNGRY_THRESHOLD - 0.01));
        assert!(is_hungry(HUNGRY_THRESHOLD)); // 恰好等於門檻算餓
        assert!(is_hungry(HUNGER_MAX));
        assert!(!is_hungry(0.0));
    }

    #[test]
    fn cd_offsets_are_staggered() {
        // 四位居民初始冷卻互不相同、遞增，錯開喊餓時機。
        let offs: Vec<f32> = (0..4).map(hunger_cd_offset).collect();
        for w in offs.windows(2) {
            assert!(w[1] > w[0], "初始冷卻應遞增錯開");
        }
        assert!(offs[0] > 0.0);
    }

    #[test]
    fn say_lines_rotate_and_bounded() {
        // 台詞輪替、非空、長度合理（前端泡泡 ≤ 50 字上限內）。
        for pick in 0..8 {
            let hl = hunger_say_line(pick);
            let sl = sated_say_line(pick);
            assert!(!hl.is_empty() && hl.chars().count() <= 40);
            assert!(!sl.is_empty() && sl.chars().count() <= 40);
        }
        // pick 溢出用取模包回，不 panic。
        assert_eq!(hunger_say_line(0), hunger_say_line(4));
        assert_eq!(sated_say_line(1), sated_say_line(5));
    }

    #[test]
    fn fed_memory_embeds_player_or_falls_back() {
        let m = fed_memory_line("諾娃");
        assert!(m.contains("諾娃"));
        assert!(m.contains("餓"));
        // 空名（訪客）退泛稱、不留空洞、不含原話。
        let g = fed_memory_line("");
        assert!(!g.contains("諾娃"));
        assert!(g.contains("餓"));
    }

    #[test]
    fn fed_feed_embeds_names() {
        let f = fed_feed_line("露娜", "旅人阿爾");
        assert!(f.contains("露娜") && f.contains("旅人阿爾"));
        // 空玩家名 → 泛稱「有人」。
        let g = fed_feed_line("露娜", "");
        assert!(g.contains("露娜") && g.contains("有人"));
    }
}
