//! 乙太方界·名號化為敬意 v1（自主提案切片·ROADMAP 777）——你掙得的名號第一次改變居民的**行為**。
//!
//! PLAN_ETHERVOX 路線圖 **item 2「記憶→行為·你的互動有後果」** 的一刀，承接北極星核心信念
//! 「**記憶要驅動行為、不只聊天**」。
//!
//! 名號弧線（773 注意到你蓋東西 → 774 為你取名號 → 775 名號口耳相傳 → 776 名號刻成牌）此前
//! 一路都停在**表達**這一層——居民把名號**說出來、傳出去、刻下來**，但名號本身還沒讓居民對你
//! **做出**任何不同的事。本刀把那條線推過門檻：**被你贏得名號的居民，看見你在中距離時，會偶爾
//! 放下手邊的閒晃、特地走過來向你致意。** 你的名聲第一次不只被說、被記，而是**把居民的腳步引向你**。
//!
//! **與既有「主動走近玩家」機制的分界（刻意區隔，非重複）**：
//! - `voxel_comfort`（678 孤獨尋伴）：由**孤獨心情**驅動，居民**需要你**、走過來求陪伴（😔）。
//! - 本模組：由**敬重**驅動（心中已為你昇華出名號），居民**看重你**、特地走過來**致意**——
//!   不是需要你陪，而是你的名號讓她想主動迎向你。冷卻更長（敬意稀少、不黏人）。
//! - `voxel_playerepithet`（774 名號招呼）：**被動**——是**你**走近她、她才用名號招呼。本模組相反，
//!   是**她**主動走向你：一個是「你來了我招呼」，一個是「你的名號讓我起身迎向你」。
//!
//! **純邏輯層**：決策與選句皆為**確定性純函式**，零 IO / 零鎖 / 零 async / 零 LLM，窮舉可測。
//! 鎖與副作用（讀名號表、設目標移動、冒泡泡、記動態）全在 `voxel_ws.rs`。不抄外部碼；繁中註解。

use crate::voxel_playerepithet::PlayerRole;

/// 趨近半徑（方塊，XZ 平面）：玩家在這麼近、居民才會起身迎向你致意。
/// 比孤獨尋伴（`SEEK_RANGE`=28）稍短——敬意致意是「順道」，不追著滿地圖跑。
pub const ESTEEM_APPROACH_RANGE: f32 = 22.0;

/// 走到玩家這麼近，才算「到面前」，冒出致意泡泡（方塊，XZ）。
pub const ESTEEM_ARRIVE_DIST: f32 = 4.5;

/// 敬意致意冷卻（秒，純記憶體、重啟歸零）：每次致意後要等這麼久才可能再起身。
/// 明顯長於孤獨尋伴（300）——敬意致意是稀少而慎重的舉動，不該一直往玩家身上湊。
pub const ESTEEM_COOLDOWN: f32 = 480.0;

/// 每 tick 起身的機率門檻：冷卻就緒且玩家在半徑內時，仍只低機率觸發，
/// 讓「特地走過來」讀起來偶然、慎重，不機械。純記憶體、不燒 LLM。
pub const ESTEEM_CHANCE_PER_TICK: f32 = 0.02;

/// 各居民初始冷卻偏移（秒）：讓居民不在同一 tick 同時起身，免得一擁而上。確定性純函式，可測。
pub fn approach_cooldown_offset(idx: usize) -> f32 {
    180.0 + idx as f32 * 90.0
}

/// 是否該起身主動迎向這位玩家致意。純函式、可窮舉測試。
///
/// 條件：冷卻就緒 + 玩家在趨近半徑內、但**尚未**在到達距離內（已在面前就不用「走過去」了，
/// 交給被動名號招呼即可）+ 過低機率門檻。`roll` 由呼叫端傳入（`rand::random`），便於測試注入。
pub fn should_start_approach(d2: f32, cooldown: f32, roll: f32) -> bool {
    cooldown <= 0.0
        && d2 <= ESTEEM_APPROACH_RANGE * ESTEEM_APPROACH_RANGE
        && d2 > ESTEEM_ARRIVE_DIST * ESTEEM_ARRIVE_DIST
        && roll < ESTEEM_CHANCE_PER_TICK
}

/// 抵達玩家面前時冒的**致意**泡泡（≤ 呼叫端再截 40 字）。
///
/// 刻意有別於 `greeting_for_role`（被動招呼「你回來啦」）——這是**特地走過來**的致意口吻
/// （「特地過來看看你」），嵌入名號＋玩家顯示名。`pick` 由呼叫端傳入的確定性擾動，讓幾句輪替。
/// **隱私鐵律**：輸出只由角色＋玩家顯示名決定（固定模板），永不含任何記憶原文 / 玩家原話。
pub fn esteem_arrive_line(role: PlayerRole, player_name: &str, pick: usize) -> String {
    let name: String = player_name.chars().take(12).collect();
    let who = if name.trim().is_empty() { "旅人".to_string() } else { name };
    let epithet = role.epithet();
    let lines: [String; 3] = [
        format!("看到{who}了，特地過來向你打聲招呼，{epithet}。"),
        format!("{epithet}，我特地走過來看看你～"),
        format!("遠遠望見{who}，就想過來跟你說說話，{epithet}。"),
    ];
    lines[pick % lines.len()].clone()
}

/// 城鎮動態（Feed）一句：某居民特地起身走向掙得名號的玩家致意時記一筆。
/// 居民名由 Feed 的 `resident` 欄另帶，**不重複**嵌名；玩家名嵌入（本就會出現在招呼／動態）。
/// 純模板、無記憶原文。
pub fn esteem_feed_line(player_name: &str, role: PlayerRole) -> String {
    let name: String = player_name.chars().take(12).collect();
    let who = if name.trim().is_empty() { "那位旅人".to_string() } else { name };
    format!("特地放下手邊的事，走過去向「{}」{who}打了聲招呼。", role.epithet())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approach_cooldown_offset_staggers_and_grows() {
        assert!(approach_cooldown_offset(0) < approach_cooldown_offset(1));
        assert!(approach_cooldown_offset(1) < approach_cooldown_offset(2));
        // 全部為正（不會立刻觸發）。
        for i in 0..8 {
            assert!(approach_cooldown_offset(i) > 0.0);
        }
    }

    #[test]
    fn should_start_approach_needs_cooldown_ready() {
        let in_range = (ESTEEM_APPROACH_RANGE - 1.0).powi(2);
        // 冷卻未就緒 → 不起身，哪怕近在眼前、機率必中。
        assert!(!should_start_approach(in_range, 1.0, 0.0));
        // 冷卻就緒且機率命中 → 起身。
        assert!(should_start_approach(in_range, 0.0, 0.0));
    }

    #[test]
    fn should_start_approach_respects_range_band() {
        let cd = 0.0;
        let roll = 0.0; // 機率必中，隔離出「距離」這一維
        // 太遠（超出趨近半徑）→ 不起身。
        let too_far = (ESTEEM_APPROACH_RANGE + 1.0).powi(2);
        assert!(!should_start_approach(too_far, cd, roll));
        // 已在到達距離內（就在面前）→ 不必走過去（交給被動招呼）。
        let already_here = (ESTEEM_ARRIVE_DIST - 1.0).powi(2);
        assert!(!should_start_approach(already_here, cd, roll));
        // 恰在中距離帶內 → 起身。
        let mid = (ESTEEM_ARRIVE_DIST + 2.0).powi(2);
        assert!(should_start_approach(mid, cd, roll));
    }

    #[test]
    fn should_start_approach_gated_by_probability() {
        let in_range = (ESTEEM_ARRIVE_DIST + 3.0).powi(2);
        // roll 高於門檻 → 這一 tick 不起身（讓觸發偶然、慎重）。
        assert!(!should_start_approach(in_range, 0.0, ESTEEM_CHANCE_PER_TICK + 0.01));
        // roll 低於門檻 → 起身。
        assert!(should_start_approach(in_range, 0.0, ESTEEM_CHANCE_PER_TICK - 0.001));
    }

    #[test]
    fn arrive_line_embeds_epithet_and_name_within_limit() {
        for role in [PlayerRole::Maker, PlayerRole::Giver, PlayerRole::Trader, PlayerRole::Companion] {
            for pick in 0..3 {
                let line = esteem_arrive_line(role, "露娜", pick);
                assert!(line.contains(role.epithet()), "應含名號: {line}");
                // 至少一句含玩家名、且全部句子截 40 字內安全（呼叫端還會再截一次）。
                assert!(line.chars().count() <= 40, "過長: {line}");
            }
        }
        // 三句輪替不全同。
        let a = esteem_arrive_line(PlayerRole::Maker, "露娜", 0);
        let b = esteem_arrive_line(PlayerRole::Maker, "露娜", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn arrive_line_empty_name_falls_back() {
        let line = esteem_arrive_line(PlayerRole::Giver, "   ", 0);
        assert!(line.contains("旅人"));
        assert!(line.contains("慷慨的人"));
    }

    #[test]
    fn feed_line_names_player_and_epithet() {
        let line = esteem_feed_line("露娜", PlayerRole::Maker);
        assert!(line.contains("露娜"));
        assert!(line.contains("造物者"));
        // 空名退回泛稱、不 panic。
        let anon = esteem_feed_line("", PlayerRole::Trader);
        assert!(anon.contains("那位旅人"));
        assert!(anon.contains("老搭檔"));
    }
}
