//! 乙太方界·居民為你取一個名號 v1（自主提案切片·PLAN_ETHERVOX 核心信念「記憶要驅動**行為**、
//! 不只聊天」＋路線圖②「你的互動有後果」）。
//!
//! 這是 `voxel_self_image`（居民昇華出「**我**是個怎樣的人」）的**對外鏡像**：居民把牠累積的
//! **關於某位玩家**的記憶昇華成「**你**在牠心中是個怎樣的人」——一個你用作為掙來的**名號**。
//! 熟識之後，牠打招呼時不再只喊你的名字，而改用這個名號喊你：
//!
//!   「造物者，你回來啦！」  「慷慨的人，好久不見～」
//!
//! 你的每一次建造／餽贈／交易／相伴，都不再只是散落的單筆記憶——它們**聚合**成世界對你的一個
//! 稱呼。這是「記憶→行為」的一種：居民累積的印象，形塑了牠**怎麼稱呼你**。
//!
//! **與 `voxel_fond_greeting` 的分界（刻意區隔，非重複）**：
//! - `fond_greeting` 是**單次互動的回憶**——偵測最近最有感的**那一件事**，說出來（「你上次幫我
//!   蓋家真好」）；著眼「我們做過**什麼**」。
//! - 本模組是**聚合印象的昇華**——要某類作為**持續且明顯主導**（門檻同自我印象：≥
//!   [`MIN_ROLE_MEMORIES`] 筆且領先次多 ≥ [`LEAD_MARGIN`] 筆）才昇華出一個**名詞式稱號**，並用它
//!   **稱呼你**；著眼「你**是**誰」。兩者輸入訊號相近、機制與輸出維度不同（事件回憶 vs 身分名號）。
//!
//! **隱私 / 濫用防護鐵律**（沿用 `voxel_self_image` 同守則）：名號輸出**永遠是固定模板**
//! （角色→確定性稱呼），**絕不回放任何記憶原文或玩家原話**；只有玩家的**顯示名**（本就會出現在
//! 招呼／動態牆）會嵌入。零新輸入面、零新端點、零 LLM。
//!
//! 純邏輯層：分類、統計、選句皆為**確定性純函式**，零 IO / 零鎖 / 零 async / 零 LLM，窮舉可測。
//! 鎖與副作用（讀記憶、冒泡泡、記動態）全在 `voxel_ws.rs`。不抄外部碼；繁中註解。

use crate::voxel_memory::MemoryEntry;

/// 居民眼中「這位玩家是個怎樣的人」——由玩家在牠面前的作為聚合而來。
///
/// 分類刻意聚焦「**玩家對我 / 在我身邊做了什麼**」：蓋東西、送東西、換東西、常相伴。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerRole {
    /// 造物者——玩家在附近親手蓋東西、或幫這位居民蓋家（建造的作為）。
    Maker,
    /// 慷慨的人——玩家送禮／送種子（餽贈的作為）。
    Giver,
    /// 老搭檔——雙方有以物易物往來（交易的作為）。
    Trader,
    /// 常來的老友——沒有特別突出的作為，但總來聊天相伴（相處的積累）。
    Companion,
}

impl PlayerRole {
    /// 這個角色的**名號**（居民用來稱呼玩家；i18n 預留點：集中此一處替換）。
    pub fn epithet(self) -> &'static str {
        match self {
            PlayerRole::Maker => "造物者",
            PlayerRole::Giver => "慷慨的人",
            PlayerRole::Trader => "老搭檔",
            PlayerRole::Companion => "常來的老友",
        }
    }
}

/// 至少累積這麼多筆同角色記憶，才可能昇華出名號（太少＝還看不出你是個怎樣的人）。
/// 高於 `fond_greeting` 的單次回憶——名號要的是**持續**的模式，不是偶一為之。
pub const MIN_ROLE_MEMORIES: usize = 4;

/// 主導角色必須比第二名多出這麼多筆，才算「明顯」（防兩類作為勢均力敵時搖擺）。
pub const LEAD_MARGIN: usize = 2;

/// 把一筆玩家名下記憶摘要歸類到某個玩家角色；歸不了類（純寒暄／無關）→ `None`。
///
/// 命中優先序由「最具體的作為」往「最泛的相伴」排：建造→餽贈→交易→（其餘落 `None`，
/// 由 [`dominant_role`] 的相伴兜底另計）。關鍵詞對齊既有玩家名下記憶模板：
/// - 造物者：773 讚賞「親手蓋起了…」、769 協助「幫我蓋…」
/// - 慷慨的人：660 贈禮「送來／送我」、754 種子禮「送的…種子」
/// - 老搭檔：交易「以物易物」
///
/// 確定性、可窮舉測試。
pub fn classify_role(summary: &str) -> Option<PlayerRole> {
    const MAKER: &[&str] = &["親手蓋", "幫我蓋", "蓋起", "蓋了", "堆起", "砌", "築"];
    const GIVER: &[&str] = &["送來", "送我", "送的", "餽贈", "贈", "分了我", "捎來"];
    const TRADER: &[&str] = &["以物易物", "換給我", "交易", "跟我換"];

    let table: &[(&[&str], PlayerRole)] = &[
        (MAKER, PlayerRole::Maker),
        (GIVER, PlayerRole::Giver),
        (TRADER, PlayerRole::Trader),
    ];
    for (kws, role) in table {
        if kws.iter().any(|k| summary.contains(k)) {
            return Some(*role);
        }
    }
    None
}

/// 統計某玩家名下記憶落在各角色的筆數，回傳「明顯主導」的角色；沒有夠明顯的主導 → `None`。
///
/// 規則（比照 `voxel_self_image::dominant_domain`）：
/// - 有具體作為（造物／餽贈／交易）：最多筆者 ≥ [`MIN_ROLE_MEMORIES`] 且領先次多 ≥ [`LEAD_MARGIN`]
///   才昇華；平手或差距不足 → `None`（寧可暫時「還看不出你是誰」也不亂貼標籤）。
/// - 沒有任何具體作為、但相處記憶（未歸類的寒暄相伴）夠多（≥ [`MIN_ROLE_MEMORIES`]）：昇華成
///   [`PlayerRole::Companion`]「常來的老友」——你沒為牠做什麼特別的事，但總來相伴，也是一種名號。
///
/// `memories`：某居民**關於這位玩家**的全部記憶（呼叫端已用 `all_player_memories` 篩過）。純函式。
pub fn dominant_role(memories: &[MemoryEntry]) -> Option<PlayerRole> {
    // 三個「具體作為」角色的計數（順序對齊 classify_role，供同分時確定性 tie-break：取表中較前者）。
    const ROLES: [PlayerRole; 3] = [PlayerRole::Maker, PlayerRole::Giver, PlayerRole::Trader];
    let mut counts = [0usize; 3];
    let mut classified = 0usize; // 有歸到具體作為的筆數
    for e in memories {
        if let Some(role) = classify_role(&e.summary) {
            // 窮舉映射到 ROLES 索引：日後加變體時編譯器會在此報錯（漏配 match 臂），
            // 而非留到執行期 position().unwrap() 落空 panic 掉遊戲迴圈（守 #1019 硬安全閘教訓）。
            let idx = match role {
                PlayerRole::Maker => 0,
                PlayerRole::Giver => 1,
                PlayerRole::Trader => 2,
                PlayerRole::Companion => unreachable!("Companion 不由 classify_role 產生"),
            };
            counts[idx] += 1;
            classified += 1;
        }
    }

    // 找最高分（同分取索引較小＝表中較前，確定性）與次高分。
    let mut best_idx = 0usize;
    for i in 1..counts.len() {
        if counts[i] > counts[best_idx] {
            best_idx = i;
        }
    }
    let best = counts[best_idx];
    if best >= MIN_ROLE_MEMORIES {
        let second = counts
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != best_idx)
            .map(|(_, c)| *c)
            .max()
            .unwrap_or(0);
        if best >= second + LEAD_MARGIN {
            return Some(ROLES[best_idx]);
        }
        return None; // 具體作為主導不夠明顯，先不貼標籤（不退回相伴，避免蓋掉勢均力敵的作為）。
    }

    // 沒有夠份量的具體作為主導：看相處（未歸類的相伴記憶）是否夠多 → 常來的老友。
    let companionship = memories.len().saturating_sub(classified);
    if companionship >= MIN_ROLE_MEMORIES {
        return Some(PlayerRole::Companion);
    }
    None
}

/// 居民打招呼時用**名號**稱呼玩家的一句（≤ 呼叫端再截 40 字）。無明顯主導角色 → `None`。
///
/// `pick`：呼叫端傳入的確定性擾動（如座標 bits），讓同一角色的幾句招呼輪替、不機械重複。
/// **隱私鐵律**：輸出只由角色決定（固定模板），永不含任何記憶原文 / 玩家原話。
pub fn epithet_greeting(memories: &[MemoryEntry], pick: usize) -> Option<String> {
    Some(greeting_for_role(dominant_role(memories)?, pick))
}

/// 依已昇華出的角色選一句名號招呼（呼叫端已算好 [`dominant_role`]，此處不重算）。純模板、輪替、可測。
pub fn greeting_for_role(role: PlayerRole, pick: usize) -> String {
    let lines: &[&str] = match role {
        PlayerRole::Maker => &["造物者，你回來啦！", "哦，造物者，今天想蓋點什麼？", "造物者來了，這一帶又要多點新東西了～"],
        PlayerRole::Giver => &["慷慨的人，你來了～", "是你呀，慷慨的人！", "慷慨的人回來啦，快進來坐。"],
        PlayerRole::Trader => &["老搭檔，來看看有什麼好貨？", "嘿，老搭檔！", "老搭檔，今天想換點什麼？"],
        PlayerRole::Companion => &["常來的老友，又見面啦！", "是你呀，我的老友～", "老友，你來啦，我正想著你呢。"],
    };
    lines[pick % lines.len()].to_string()
}

/// 動態牆（Feed）一句：某居民**第一次**（或改換）在心裡為某玩家安下名號時記一筆。
/// 居民名由 Feed 的 `resident` 欄另帶，**不重複**嵌名；玩家名嵌入（本就會出現在招呼／動態）。
/// 無明顯主導 → 呼叫端不會走到這裡（先 `dominant_role` 判定）。純模板、無記憶原文。
pub fn coined_feed_line(player_name: &str, role: PlayerRole) -> String {
    let name: String = player_name.chars().take(12).collect();
    let who = if name.trim().is_empty() { "那位旅人".to_string() } else { name };
    format!("在心裡漸漸把{who}看作這一帶的「{}」了。", role.epithet())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(summary: &str) -> MemoryEntry {
        MemoryEntry {
            resident: "vox_res_0".to_string(),
            player: "露娜".to_string(),
            summary: summary.to_string(),
            seq: 0,
        }
    }

    #[test]
    fn classify_role_covers_real_memory_templates() {
        // 773 讚賞、769 協助 → 造物者
        assert_eq!(classify_role("我看著露娜在附近親手蓋起了一片方塊，真了不起。"), Some(PlayerRole::Maker));
        assert_eq!(classify_role("露娜幫我蓋小屋，出了一份力，我記著這份情。"), Some(PlayerRole::Maker));
        // 660 贈禮、754 種子禮 → 慷慨的人
        assert_eq!(classify_role("露娜送來一些木頭"), Some(PlayerRole::Giver));
        assert_eq!(classify_role("🌱把露娜送的小麥種子種在家旁，盼它長成一畦菜園"), Some(PlayerRole::Giver));
        // 交易 → 老搭檔
        assert_eq!(classify_role("和露娜以物易物，換到石頭"), Some(PlayerRole::Trader));
    }

    #[test]
    fn classify_role_none_for_smalltalk() {
        assert_eq!(classify_role("和露娜聊過，對方提到「今天天氣真好」"), None);
        assert_eq!(classify_role(""), None);
    }

    #[test]
    fn classify_role_never_yields_companion() {
        // Companion 只由 dominant_role 的相伴兜底產生，不由 classify_role 直接產出。
        for s in ["親手蓋", "送來", "以物易物", "隨便聊聊"] {
            assert_ne!(classify_role(s), Some(PlayerRole::Companion));
        }
    }

    #[test]
    fn maker_priority_over_giver_when_both_present() {
        // 一句同時含「蓋」與「送」：先判具體作為表順序，造物者優先。
        assert_eq!(classify_role("露娜送來材料還幫我蓋牆"), Some(PlayerRole::Maker));
    }

    #[test]
    fn dominant_role_needs_min_and_margin() {
        // 3 筆造物 < MIN(4) → 還昇華不出
        let few: Vec<_> = (0..3).map(|_| mem("親手蓋起了一片方塊")).collect();
        assert_eq!(dominant_role(&few), None);
        // 4 筆造物、0 筆其他 → 造物者
        let four: Vec<_> = (0..4).map(|_| mem("親手蓋起了一片方塊")).collect();
        assert_eq!(dominant_role(&four), Some(PlayerRole::Maker));
    }

    #[test]
    fn dominant_role_none_when_tie_between_deeds() {
        // 4 筆造物 vs 3 筆餽贈：領先 1 < LEAD_MARGIN(2) → 不貼標籤
        let mut ms: Vec<_> = (0..4).map(|_| mem("親手蓋起了一片方塊")).collect();
        ms.extend((0..3).map(|_| mem("送來一些木頭")));
        assert_eq!(dominant_role(&ms), None);
        // 拉開到領先 2（5 vs 3）→ 造物者
        ms.push(mem("親手蓋起了一片方塊"));
        assert_eq!(dominant_role(&ms), Some(PlayerRole::Maker));
    }

    #[test]
    fn dominant_role_giver_and_trader() {
        let givers: Vec<_> = (0..5).map(|_| mem("送來一些木頭")).collect();
        assert_eq!(dominant_role(&givers), Some(PlayerRole::Giver));
        let traders: Vec<_> = (0..4).map(|_| mem("以物易物換到石頭")).collect();
        assert_eq!(dominant_role(&traders), Some(PlayerRole::Trader));
    }

    #[test]
    fn dominant_role_companion_from_pure_company() {
        // 全是寒暄相伴、無具體作為，但夠多（≥4）→ 常來的老友
        let chats: Vec<_> = (0..5).map(|_| mem("和露娜聊過，對方提到「今天天氣真好」")).collect();
        assert_eq!(dominant_role(&chats), Some(PlayerRole::Companion));
        // 太少 → 還看不出
        let few: Vec<_> = (0..3).map(|_| mem("隨便聊聊")).collect();
        assert_eq!(dominant_role(&few), None);
    }

    #[test]
    fn companion_not_returned_when_a_deed_leads_but_short_of_margin() {
        // 4 造物 vs 3 餽贈（具體作為存在但主導不明顯）→ None，不因此退回相伴（避免蓋掉勢均力敵作為）。
        let mut ms: Vec<_> = (0..4).map(|_| mem("親手蓋起了一片方塊")).collect();
        ms.extend((0..3).map(|_| mem("送來一些木頭")));
        // 再加一堆寒暄也不該讓它變 Companion（因為 best=4 ≥ MIN，走的是「作為不明顯」分支回 None）
        ms.extend((0..10).map(|_| mem("隨便聊聊")));
        assert_eq!(dominant_role(&ms), None);
    }

    #[test]
    fn epithet_greeting_none_below_threshold_some_above() {
        let few: Vec<_> = (0..2).map(|_| mem("親手蓋起了一片方塊")).collect();
        assert_eq!(epithet_greeting(&few, 0), None);
        let four: Vec<_> = (0..4).map(|_| mem("親手蓋起了一片方塊")).collect();
        let g = epithet_greeting(&four, 0).unwrap();
        assert!(g.contains("造物者"));
        assert!(g.chars().count() <= 40);
    }

    #[test]
    fn epithet_greeting_rotates_by_pick() {
        let four: Vec<_> = (0..4).map(|_| mem("送來一些木頭")).collect();
        let a = epithet_greeting(&four, 0).unwrap();
        let b = epithet_greeting(&four, 1).unwrap();
        assert_ne!(a, b); // 不同 pick 輪替不同句
    }

    #[test]
    fn coined_feed_line_names_player_and_role_no_leak() {
        let line = coined_feed_line("露娜", PlayerRole::Maker);
        assert!(line.contains("露娜"));
        assert!(line.contains("造物者"));
        // 空名安全退回泛稱
        let anon = coined_feed_line("  ", PlayerRole::Giver);
        assert!(anon.contains("那位旅人"));
        assert!(anon.contains("慷慨的人"));
    }

    #[test]
    fn epithet_names_are_stable() {
        assert_eq!(PlayerRole::Maker.epithet(), "造物者");
        assert_eq!(PlayerRole::Giver.epithet(), "慷慨的人");
        assert_eq!(PlayerRole::Trader.epithet(), "老搭檔");
        assert_eq!(PlayerRole::Companion.epithet(), "常來的老友");
    }
}
