//! 乙太煙火 v1（ROADMAP 785·自主提案切片）——玩家親手合成的「慶祝」道具。
//!
//! 乙太方界剛掛上繁星夜空（783），但玩家在這片天地裡，始終只能「看」天象自己
//! 發生（下雨 700、雨後彩虹 780、繁星 783 都由伺服器自發演變）——玩家自己**沒有
//! 任何一個能主動點亮這片天空的動作**。本切片補上那一拍：把最深處挖回的乙太礦與
//! 煤礦，在工作台裡做成一束**乙太煙火**，朝夜空一放，一朵彩色火花在星空中綻放開來，
//! 附近醒著的居民抬頭望見、一起歡呼。這是玩家第一個「主動施放、與居民共享」的
//! 慶祝動作——採集→合成→施放的全新「人類也玩得爽」動詞，也讓小村第一次為一場
//! 你放的煙火一起抬頭。
//!
//! **與既有天象的分界**：700/780/783 都是伺服器自發的天氣/晝夜事件，玩家只能旁觀；
//! 本切片是**玩家主動施放**、由玩家的動作觸發、火花位置就在玩家頭頂夜空——主動 vs 被動、
//! 玩家驅動 vs 伺服器驅動，維度全新。居民的歡呼比照彩虹（780）「天象→齊聲反應」的
//! 既有範式，但這回抬頭的緣由是「你放的煙火」，是玩家與居民**共享的一刻慶祝**。
//!
//! **成本紀律**：零 LLM（火花配色/歡呼/記憶/Feed 全確定性純函式）、零 migration
//! （煙火是既有背包物品、施放即消耗、火花純視覺不落地）、零新美術（火花＝程序生成點雲）。
//! **濫用防護**：施放會廣播給全場（人人都看得見火花），故 ① 每連線 [`FIREWORK_COOLDOWN_SECS`]
//! 冷卻擋連放洗版；② 每放消耗一份需先合成的煙火＝天然經濟節流（放不了白嫖）；
//! ③ 歡呼/記憶/Feed 全走固定模板、只嵌居民顯示名與玩家顯示名，永不回放玩家自由輸入。

/// 乙太煙火純物品 id（不可放置，住背包、施放即消耗）。承接 STEW_ID=67 之後的空號。
pub const FIREWORK_ID: u8 = 68;

/// 每連線施放冷卻（秒）：擋玩家連按洗爆全場畫面（濫用防護①）。
pub const FIREWORK_COOLDOWN_SECS: f32 = 3.0;

/// 附近居民抬頭歡呼的半徑（方塊）：施放點水平距離內、醒著有空的居民才會歡呼。
pub const CHEER_RADIUS: f32 = 26.0;

/// 歡呼泡泡字元上限（防溢出泡泡框）。
pub const CHEER_SAY_MAX_CHARS: usize = 40;

/// 火花配色盤數量：前端據此把伺服器選定的 palette 索引映射成一組火花顏色。
pub const PALETTE_COUNT: u32 = 6;

/// 從一枚隨機數選一個火花配色盤索引（確定性、恆落在 `0..PALETTE_COUNT`）。
/// 走真隨機讓每次施放的火花顏色有變化；前端收到索引後映射成實際 RGB。
pub fn firework_palette(r: u64) -> u32 {
    (r % PALETTE_COUNT as u64) as u32
}

/// 施放者自己看到的煙火綻放回饋（單播給施放者），4 句確定性輪替。
pub fn launch_self_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "咻——砰！一束乙太煙火在夜空中綻放成一朵光花。",
        "你朝天空一放，火花灑滿星幕，映亮了腳下的方塊世界。",
        "乙太煙火升空炸開，星空下一片絢爛的青金色光雨。",
        "砰！煙火在頭頂盛開，餘光在夜色裡慢慢飄落。",
    ];
    LINES[pick % LINES.len()]
}

/// 附近居民抬頭望見煙火的歡呼台詞，5 句確定性輪替。
pub fn cheer_line(pick: usize) -> &'static str {
    const LINES: [&str; 5] = [
        "哇——你看那煙火！好美啊！",
        "煙火！好久沒看到這麼漂亮的夜空了！",
        "快看快看，天上開出一朵花來了！",
        "這束煙火真教人開心，謝謝你放給大家看～",
        "抬頭一看正好趕上，這一刻真幸福呀！",
    ];
    LINES[pick % LINES.len()]
}

/// 世界動態牆一則：某玩家朝夜空施放了煙火（不在場的人回來也讀得到這份熱鬧）。
pub fn launch_feed_line(player: &str) -> String {
    let who = if player.is_empty() { "有位旅人" } else { player };
    let who: String = who.chars().take(24).collect();
    format!("{who}朝夜空施放了一束乙太煙火，火花灑滿星空，居民都抬頭歡呼起來。")
}

/// 居民把「和你一起看了煙火」記進交情（第一人稱情節記憶，只嵌玩家顯示名、不夾帶玩家原話）。
pub fn cheer_memory_line(player: &str) -> String {
    let who = if player.is_empty() { "一位旅人" } else { player };
    let who: String = who.chars().take(24).collect();
    format!("那晚{who}放了一束煙火，我抬頭看著它在夜空中綻放，心裡暖暖的。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_always_in_range() {
        for r in 0u64..1000 {
            assert!(firework_palette(r) < PALETTE_COUNT);
        }
        // 大隨機數也不越界。
        assert!(firework_palette(u64::MAX) < PALETTE_COUNT);
        assert_eq!(firework_palette(0), 0);
        assert_eq!(firework_palette(PALETTE_COUNT as u64), 0); // 環繞
    }

    #[test]
    fn cheer_line_nonempty_and_deterministic() {
        // 每一句都非空、且在泡泡上限內（防溢框）。
        for p in 0..20 {
            let s = cheer_line(p);
            assert!(!s.is_empty());
            assert!(s.chars().count() <= CHEER_SAY_MAX_CHARS);
        }
        // 同 pick 恆同句（確定性）。
        assert_eq!(cheer_line(3), cheer_line(3 + 5));
        // 覆蓋到全部 5 句（至少頭尾不同）。
        assert_ne!(cheer_line(0), cheer_line(1));
    }

    #[test]
    fn launch_self_line_nonempty_and_rotates() {
        for p in 0..12 {
            assert!(!launch_self_line(p).is_empty());
        }
        assert_eq!(launch_self_line(2), launch_self_line(2 + 4));
        assert_ne!(launch_self_line(0), launch_self_line(1));
    }

    #[test]
    fn feed_line_contains_player_and_falls_back() {
        let s = launch_feed_line("阿光");
        assert!(s.contains("阿光"));
        assert!(s.contains("煙火"));
        // 空名落回通用稱呼、不留空洞。
        let s2 = launch_feed_line("");
        assert!(!s2.is_empty());
        assert!(s2.contains("旅人"));
    }

    #[test]
    fn memory_line_contains_player_no_overflow() {
        let s = cheer_memory_line("諾娃");
        assert!(s.contains("諾娃"));
        // 超長玩家名被截、整句仍非空且不無限膨脹。
        let long = "名".repeat(200);
        let s2 = cheer_memory_line(&long);
        assert!(!s2.is_empty());
        assert!(s2.chars().count() < 60);
        // 空名落回通用稱呼。
        assert!(cheer_memory_line("").contains("旅人"));
    }
}
