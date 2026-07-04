//! 乙太方界·居民為友誼立下信物 v1（voxel-friendtoken，自主提案切片）。
//!
//! **北極星**：小社會湧現（`PLAN_ETHERVOX.md` §4 居民↔居民關係）。居民的情誼（672，
//! `voxel_bonds`）一路累積：陌生→相識→老朋友；升到老朋友時，世界至今只在動態牆記一行字、
//! 兩人腦中各留一筆記憶——這段關係**在世界裡沒有留下任何看得見、摸得著的痕跡**。玩家送的
//! 紀念物（keepsake 732）、居民蓋的家（652）、立的牌（749）都會在方塊天地裡留下實體，
//! 唯獨「兩位居民成了老朋友」這件小社會裡最動人的事，過眼即逝。這一刀補上那一拍：**當兩位
//! 居民第一次成為老朋友，作東的那位會在自家旁的空地上，親手點起一盞「友誼的燈」作為信物**——
//! 一盞會發光的小方塊，日夜亮著、持久留在世界裡（隨既有蓋家方塊管線落地、重啟後仍在）。
//! 玩家在方塊天地裡走著走著，會發現小村的各個角落漸漸亮起一盞盞小燈——每一盞，都是兩位
//! AI 居民之間真的萌生、又被鄭重紀念下來的一段友誼。小社會的關係網，第一次從「查表才知道」
//! 變成**世界裡看得見的實體地標**。
//!
//! **與既有社交／實體的定位區隔**：
//! - 居民互助蓋家（696，`voxel_building`）雖也在老朋友到訪時放方塊，但放的是**主人自己蓋家
//!   計畫裡的下一塊**（推進她的建物）；本刀放的是**專為這段友誼而立、獨立於任何蓋家計畫的
//!   信物**——一個推進工程、一個紀念關係，意義全然不同。
//! - 居民立牌命名（749，`voxel_nameplate`）是居民為**自己蓋好的家**署名；本刀是為**與另一位
//!   居民的友誼**立物——一個朝向「我的家」、一個朝向「我們的情誼」。
//! - 見證圓夢（782，`voxel_witness`）與此都在關係升溫的一刻反應，但那是**過眼的道賀泡泡**；
//!   本刀在世界裡留下**持久的實體信物**——一個是聲音、一個是地標。
//!
//! **純邏輯層**：信物方塊挑選（[`token_block`]，確定性依配對序輪替、皆為既有裝飾方塊、零新
//! 美術）、立信物泡泡（[`token_say_line`]）、雙方記憶摘要（[`token_memory_line`]）、動態牆句
//! （[`token_feed_line`]）全是確定性純函式，零 LLM、零鎖、零 IO。鎖 / 選址 / 方塊落地 /
//! 記憶寫入全在 `voxel_ws.rs`，沿用居民立牌（749）那條已驗證的短鎖循序 + 選址 + 持久化路徑。
//!
//! **成本 / 濫用防護**：只在情誼帳本（`voxel_bonds::record_visit`）**首次跨越老朋友門檻**
//! （`tier_changed && tier == Friend`）這個本就稀有、且冪等（同一對只會跨越一次）的事件上
//! 觸發——無每 tick 迴圈、無新對外端點、不觸發 LLM、不收玩家自由輸入，天然防洗版與白嫖。
//! 立信物泡泡／記憶／動態牆句全走固定模板，**只嵌居民自己的系統內建顯示名**（非玩家自由
//! 輸入），**永不夾帶玩家原話**（無注入 / NSFW 面）。零 migration（信物方塊走既有蓋家方塊
//! append-only 持久化、記憶走既有 append-only 管線）、零新協議欄位、零前端改動、零新美術
//! （信物＝既有發光方塊）、FPS 零影響（一盞燈一塊、稀有事件、無每幀開銷）。

use crate::voxel::Block;

/// 立信物泡泡／記憶／動態牆句的字元上限（與泡泡框上限一致，超出截斷不破框）。
pub const TOKEN_SAY_MAX_CHARS: usize = 40;

/// 動態牆分類（友誼信物）。
pub const FEED_KIND: &str = "友誼信物";

/// 友誼信物候選方塊——皆為**既有的發光方塊**（零新美術），讓每一段友誼在世界裡化作一盞
/// 日夜都亮著的小燈。確定性依配對序輪替，讓不同友誼的信物略有變化、不至於千篇一律。
const TOKEN_BLOCKS: [Block; 2] = [Block::Torch, Block::IceLantern];

/// 依配對序確定性挑一種信物方塊（恆落在 `TOKEN_BLOCKS` 範圍內，永不 panic）。
pub fn token_block(pick: usize) -> Block {
    TOKEN_BLOCKS[pick % TOKEN_BLOCKS.len()]
}

/// 把字串以「字元」截到上限內（不破多位元組字元、不破泡泡框）。
fn clamp(s: &str) -> String {
    s.chars().take(TOKEN_SAY_MAX_CHARS).collect()
}

/// 作東居民立信物時的泡泡（確定性輪替、只嵌兩位居民顯示名、截字防溢框）。
pub fn token_say_line(friend: &str, pick: usize) -> String {
    let f = if friend.trim().is_empty() { "老朋友" } else { friend };
    let lines = [
        format!("我和{f}成了老朋友，就在這兒點盞燈，紀念我們的情誼吧。"),
        format!("和{f}處成了老朋友，這盞小燈，就當作我們友誼的信物。"),
        format!("我想為和{f}的這段友誼留點什麼——就點一盞燈在這兒吧。"),
    ];
    clamp(&lines[pick % lines.len()])
}

/// 動態牆句（讓玩家一眼看到「兩位居民立了友誼信物」；只嵌顯示名）。
pub fn token_feed_line(host: &str, friend: &str) -> String {
    let h = if host.trim().is_empty() { "一位居民" } else { host };
    let f = if friend.trim().is_empty() { "另一位居民" } else { friend };
    clamp(&format!("{h}為和{f}的友誼，在家旁點起了一盞友誼的燈。"))
}

/// 立信物者的第一人稱記憶摘要（掛在對方名下、停在情節層、不夾帶玩家原話；只嵌對方顯示名）。
pub fn token_memory_line(friend: &str) -> String {
    let f = if friend.trim().is_empty() { "一位老朋友" } else { friend };
    clamp(&format!("我和{f}成了老朋友，在家旁點起一盞燈，紀念我們這段情誼。"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_block_always_in_range() {
        // 任意 pick（含大值、環繞）都落在候選內、永不 panic。
        for pick in [0usize, 1, 2, 3, 99, 1000, usize::MAX] {
            let b = token_block(pick);
            assert!(
                b == Block::Torch || b == Block::IceLantern,
                "信物方塊應恆為候選發光方塊，pick={pick} 得 {b:?}"
            );
        }
        // 相鄰 pick 會輪替（確定性、有變化）。
        assert_ne!(token_block(0), token_block(1), "相鄰 pick 應挑到不同信物");
        assert_eq!(token_block(0), token_block(2), "配對序循環應確定性");
    }

    #[test]
    fn say_line_embeds_name_and_fits_frame() {
        let s = token_say_line("諾娃", 0);
        assert!(s.contains("諾娃"), "立信物泡泡應嵌入對方名：{s}");
        assert!(!s.is_empty(), "泡泡非空");
        assert!(
            s.chars().count() <= TOKEN_SAY_MAX_CHARS,
            "泡泡不得超過框上限：{}",
            s.chars().count()
        );
        // 確定性輪替：同 pick 同名同句。
        assert_eq!(token_say_line("諾娃", 0), token_say_line("諾娃", 0));
    }

    #[test]
    fn say_line_super_long_name_truncated_not_broken() {
        let long = "諾".repeat(100);
        let s = token_say_line(&long, 1);
        assert!(!s.is_empty(), "超長名仍非空");
        assert!(
            s.chars().count() <= TOKEN_SAY_MAX_CHARS,
            "超長名截到框內：{}",
            s.chars().count()
        );
    }

    #[test]
    fn say_line_empty_name_falls_back() {
        let s = token_say_line("   ", 2);
        assert!(!s.is_empty(), "空白名落回通用稱呼、非空");
        assert!(s.contains("老朋友"), "空白名應落回『老朋友』通用稱呼：{s}");
    }

    #[test]
    fn feed_line_embeds_both_names() {
        let s = token_feed_line("露娜", "諾娃");
        assert!(s.contains("露娜") && s.contains("諾娃"), "動態牆句應含雙方名：{s}");
        assert!(s.chars().count() <= TOKEN_SAY_MAX_CHARS, "動態牆句不破框");
        // 空名雙側落回通用稱呼、仍非空不 panic。
        let s2 = token_feed_line("", "");
        assert!(!s2.is_empty() && s2.contains("居民"), "空名落回通用稱呼：{s2}");
    }

    #[test]
    fn memory_line_embeds_friend_only_no_break() {
        let s = token_memory_line("賽勒");
        assert!(s.contains("賽勒"), "記憶應嵌對方名：{s}");
        assert!(s.contains("老朋友"), "記憶停在『成了老朋友』情節層：{s}");
        assert!(s.chars().count() <= TOKEN_SAY_MAX_CHARS, "記憶不破框");
        // 超長名截斷不膨脹、空名落回。
        assert!(token_memory_line(&"賽".repeat(80)).chars().count() <= TOKEN_SAY_MAX_CHARS);
        assert!(!token_memory_line("").is_empty());
    }
}
