//! 乙太方界·戀人成婚 v1（自主提案切片，ROADMAP 927）。
//!
//! **缺口 / 為誰做**：居民↔居民的浪漫軸線至今走了三步——心動締結成戀人（846
//! `voxel_romance`）、把渴望蓋成兩人的愛巢（`voxel_lovenest`）、分開得夠遠會放下手邊事奔去
//! 相見（852 `voxel_lover_seek`）。但這條全庫最深的羈絆，**從沒有一個「終成眷屬」的高潮**：
//! 一對戀人不管相守多久、把交情疊得多深，關係面板上永遠只是一枚安靜的 ❤️，世界裡也從沒有
//! 任何一處看得出「這兩個人，是在這裡結為連理的」。小社會有戀愛、有奔赴、有愛巢，卻從沒有
//! 過一場**婚禮**。
//!
//! **做法**：白天好天氣，一對早已是戀人、且締結之後又把交情持續疊得夠深（累積拜訪數
//! ≥ [`WED_MIN_VISITS`]，象徵「戀人之後仍不斷相守相伴」）、此刻都閒著、又正相鄰的居民，偶爾
//! **就地結為連理**——兩人互許終身（各冒一句誓言泡泡），身旁醒著的鄰居們一起道賀（歡呼泡泡），
//! 並在世界裡**永久立起一座小小的花拱門**當作見證（兩根木柱＋頂上一排紅黃藍野花，玩家日後
//! 路過還看得到）。這場婚事寫進城鎮動態牆、也各自成為兩人心裡一筆最重的記憶（誓言含「一定會」
//! → 被記憶系統判為永久精華事實）。**一對戀人一生只辦一次**（冪等、持久化，永不重辦）。
//!
//! **與既有元素 razor-sharp 區隔（非同軸換皮）**：
//! - 與 846 心動締結：那是「陌路→戀人」的**起點**、觸發於長椅並坐擦出火花；本刀是戀人關係的
//!   **終點高潮**、觸發於「戀人之後交情又疊得夠深」的狀態，且**在世界裡放置永久方塊**（心動只寫
//!   資料與泡泡，不動世界）。
//! - 與 `voxel_lovenest` 愛巢：愛巢是戀人**蓋房子安身**（功能性居所）；花拱門是**婚禮的紀念物**
//!   （儀式性見證），兩者物件、觸發、語意都不同。
//! - 與 852 戀人牽掛：牽掛是分開時**奔去相見**；本刀是相聚時**結為連理**，觸發點相反。
//! - 與 918 堆雪人：雪人是季節限定、冬末即融的**短暫**物件；花拱門是**永久**紀念（`append_world_block`
//!   落地持久化），且由「一對戀人的關係里程碑」而非天氣驅動。
//!
//! **成本紀律（鐵律）**：零 LLM（觸發、選點、誓言、道賀全是確定性純函式）、零既有格式 migration
//! （新增獨立的 `data/voxel_weddings.jsonl`，只 append、向後相容、不動任何既有欄位）、零新美術
//! （沿用既有木頭 5／野花 94/95/96，前端本就會渲染）、FPS 零影響（婚事稀有、掛低頻節拍、一次
//! 至多辦一場、花拱門僅 7 塊）。
//!
//! **濫用防護**：本切片**不收任何玩家自由輸入、不觸發 LLM、不開對外端點、不動帳號權限**——
//! 誓言／道賀／動態牆台詞全為確定性模板、只嵌居民系統顯示名（本就出現在關係面板／動態牆），
//! 永不回放玩家原話或記憶原文（無注入／NSFW 面）；觸發純伺服器 tick 內部狀態（戀人身分＋交情
//! 深度＋冷卻＋低機率），玩家無從自報、無法從外部催發；全村冷卻天然防洗版。
//!
//! **純邏輯層**：本檔全是零鎖、零 async 的確定性純函式／常數（唯一 IO 是與 `voxel_romance`
//! 同款的婚書 jsonl 讀寫函式，鎖仍在 `voxel_ws.rs`）；放置方塊、廣播、寫記憶、Feed 的副作用都
//! 在 `voxel_ws.rs`（短鎖循序即釋、不巢狀，守 prod 死鎖鐵律，比照 `maybe_build_snowman`）。

use crate::voxel::Block;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 成婚門檻：一對戀人「締結之後」交情仍要疊到累積拜訪數 ≥ 此值才辦婚禮。刻意遠高於
/// `voxel_bonds::FRIEND_VISITS`(8，戀人締結前就已達到)，象徵「成了戀人之後又相守相伴了好一陣」，
/// 讓婚事是水到渠成的深情，而非剛心動就閃婚。
pub const WED_MIN_VISITS: u32 = 14;

/// 全村辦婚禮冷卻（秒）：至多每這麼久辦一場——婚事稀有而有份量，不洗版。
pub const WED_COOLDOWN_SECS: u64 = 300;

/// 通過所有前置閘（戀人＋未婚＋交情夠深＋都閒＋相鄰＋白天好天氣＋冷卻到期）後仍要擲骰的觸發
/// 機率：成婚是可遇不可求的水到渠成，不是條件一到就必成。
pub const WED_CHANCE: f32 = 0.25;

/// 一對戀人要成婚，此刻兩人水平距離平方需 ≤ 此值（相鄰才好並肩成婚，比照散步的相鄰判定）。
pub const PAIR_NEAR_SQ: f32 = 36.0;

/// 道賀半徑：離婚禮這麼近、醒著、閒著的鄰居會一起道賀（水平距離）。
pub const CHEER_RADIUS: f32 = 12.0;

/// 一場婚禮至多幾位鄰居道賀（防洗版、只挑最近幾位）。
pub const MAX_CHEER: usize = 4;

/// 花拱門立在兩人中點旁幾格外（不立在腳下擋路）。
pub const ANCHOR_DIST: i32 = 2;

/// 婚禮記憶掛的偽玩家標籤：婚事是關於「另一位戀人」而非某玩家，但為與一般玩家好感記帳區隔，
/// 記憶實際掛在**對方的顯示名**下（見 `voxel_ws.rs`），此常數僅作 Feed／識別用途保留。
pub const WEDDING_FEED_KIND: &str = "結為連理";

// ── 婚書持久化格式（新增獨立檔，不動任何既有格式）─────────────────────────────

/// 一筆婚書：一對已成婚的戀人（以居民**顯示名**記帳，比照 `voxel_romance::RomanceEntry`）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WeddingEntry {
    pub id_a: String,
    pub id_b: String,
}

/// 正規化一對名字的鍵順序，讓 (a,b) 與 (b,a) 落在同一個鍵。
fn norm(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// 婚書帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct ResidentWeddings {
    pairs: HashSet<(String, String)>,
}

impl ResidentWeddings {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。
    pub fn from_entries(entries: impl IntoIterator<Item = WeddingEntry>) -> Self {
        let mut w = Self::new();
        for e in entries {
            w.pairs.insert(norm(&e.id_a, &e.id_b));
        }
        w
    }

    /// 這兩位是否已成婚。
    pub fn is_wed(&self, a: &str, b: &str) -> bool {
        self.pairs.contains(&norm(a, b))
    }

    /// 記一次成婚（冪等）：真正新成婚才回傳 `true`——呼叫端只在回傳 `true` 時才落地
    /// 持久化 / 放拱門 / 廣播 / 寫記憶，避免重複觸發洗版。
    pub fn record_wedding(&mut self, a: &str, b: &str) -> bool {
        let key = norm(a, b);
        if self.pairs.contains(&key) {
            return false;
        }
        self.pairs.insert(key);
        true
    }

    /// 快照所有已成婚的配對（以顯示名，`(a, b)` 已正規化排序）。供出生系統查「村裡有沒有
    /// 一對夫妻可以一起迎來孩子」（愛的結晶 v1，ROADMAP 928）。
    pub fn all_pairs(&self) -> Vec<(String, String)> {
        self.pairs.iter().cloned().collect()
    }

    /// 快照成持久化記錄清單（供 `save_weddings` 整份 append 一行）。
    pub fn to_entries(&self) -> Vec<WeddingEntry> {
        self.pairs
            .iter()
            .map(|(a, b)| WeddingEntry { id_a: a.clone(), id_b: b.clone() })
            .collect()
    }
}

// ── 觸發判定（純函式）─────────────────────────────────────────────────────────

/// 前置閘：這一對此刻是否有資格成婚（純判定，實際選點／放置在呼叫端）。
/// - `is_sweetheart`：兩人已是戀人（`voxel_romance`）。
/// - `already_wed`：兩人是否已成婚（成婚一生一次，已婚直接否決）。
/// - `visits`：兩人累積拜訪數（交情深度信號）。
/// - `both_free`：兩人此刻都醒著、閒著、沒在別的事裡。
/// - `dist_sq`：兩人此刻水平距離平方（要相鄰）。
/// - `good_day`：白天且沒下雨（喜事要好天氣）。
/// - `cooldown_ready`：全村辦婚禮冷卻已到期。
/// - `roll`：擲骰（0.0..1.0）。
#[allow(clippy::too_many_arguments)]
pub fn should_wed(
    is_sweetheart: bool,
    already_wed: bool,
    visits: u32,
    both_free: bool,
    dist_sq: f32,
    good_day: bool,
    cooldown_ready: bool,
    roll: f32,
) -> bool {
    is_sweetheart
        && !already_wed
        && visits >= WED_MIN_VISITS
        && both_free
        && dist_sq <= PAIR_NEAR_SQ
        && good_day
        && cooldown_ready
        && roll < WED_CHANCE
}

// ── 花拱門（永久紀念物）───────────────────────────────────────────────────────

/// 花拱門由哪些方塊組成：給定拱門左柱底的地面正上方一格 `(x, y, z)`，回傳 7 塊——
/// 左右兩根兩格高的木柱、頂上一排三朵野花（紅黃藍）當花冠；中間留一格空當拱門可穿過。
///
/// ```text
///   花 花 花   ← y+2：紅 黃 藍 野花花冠
///   柱 . 柱   ← y+1：木柱（中間空）
///   柱 . 柱   ← y  ：木柱（中間空，可站/穿過）
/// ```
pub fn arch_blocks(x: i32, y: i32, z: i32) -> [(i32, i32, i32, Block); 7] {
    [
        (x, y, z, Block::Wood),                    // 左柱底
        (x, y + 1, z, Block::Wood),                // 左柱上
        (x + 2, y, z, Block::Wood),                // 右柱底
        (x + 2, y + 1, z, Block::Wood),            // 右柱上
        (x, y + 2, z, Block::WildflowerRed),       // 花冠左
        (x + 1, y + 2, z, Block::WildflowerYellow),// 花冠中
        (x + 2, y + 2, z, Block::WildflowerBlue),  // 花冠右
    ]
}

/// 給定婚禮中點水平座標，確定性選一個立花拱門的左柱錨點（中點旁四方向之一，距 [`ANCHOR_DIST`] 格）。
/// 用 `pick` 取模選方向，讓不同婚禮的拱門立在不同側、不會全擠同一格。
pub fn pick_anchor(mx: f32, mz: f32, pick: usize) -> (i32, i32) {
    let bx = mx.floor() as i32;
    let bz = mz.floor() as i32;
    match pick % 4 {
        0 => (bx + ANCHOR_DIST, bz),
        1 => (bx - ANCHOR_DIST - 2, bz), // 往左：左柱再退兩格，讓三格寬拱門整個落在中點左側
        2 => (bx, bz + ANCHOR_DIST),
        _ => (bx, bz - ANCHOR_DIST),
    }
}

// ── 台詞／記憶／Feed（確定性模板）────────────────────────────────────────────

/// 成婚當下，其中一位戀人對另一位許下的誓言泡泡（確定性三選一）。
pub fn vow_say_line(partner: &str, pick: usize) -> String {
    const LINES: [&str; 3] = [
        "{p}，從今天起我們就是一家人了，我願意。💍",
        "在這座花拱下，我把餘生都許給你，{p}。",
        "{p}，往後的每個晴天雨天，我都想和你一起走。",
    ];
    LINES[pick % LINES.len()].replace("{p}", partner)
}

/// 成婚記進戀人心裡的一筆記憶（第一人稱、含「一定會」→ 被記憶系統判為永久精華 Promise 事實，
/// 成為這輩子最重的一筆回憶）。
pub fn wed_memory_line(partner: &str) -> String {
    format!("今天，我和{partner}在乙太方界的花拱下結為連理——我一定會永遠守著這份情。")
}

/// 身旁鄰居道賀的歡呼泡泡（確定性三選一，嵌兩位新人名）。
pub fn cheer_say_line(a: &str, b: &str, pick: usize) -> String {
    const LINES: [&str; 3] = [
        "{a}和{b}成婚啦，恭喜恭喜！🎉",
        "祝{a}和{b}白頭偕老～",
        "好美的一場婚禮，{a}、{b}要幸福喔！",
    ];
    LINES[pick % LINES.len()].replace("{a}", a).replace("{b}", b)
}

/// 成婚上城鎮動態牆的一行。
pub fn wed_feed_line(a: &str, b: &str) -> String {
    format!("💍 {a}和{b}在花拱下結為連理，全村都來道賀了！")
}

// ── 婚書持久化 IO（只有函式，鎖在 voxel_ws.rs；比照 voxel_romance）─────────────

const WEDDING_FILE: &str = "data/voxel_weddings.jsonl";

/// 從 `data/voxel_weddings.jsonl` 讀取所有婚書（檔案不存在回空 Vec）。
pub fn load_weddings() -> Vec<WeddingEntry> {
    let content = match std::fs::read_to_string(WEDDING_FILE) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// 把整份婚書帳本快照 append 一行到 `data/voxel_weddings.jsonl`（比照 `voxel_romance::save_romance`：
/// 婚書對數極少，每次成婚整份快照重寫也不會無限長大；載入時 `from_entries` 以 HashSet 去重）。
pub fn save_weddings(weddings: &ResidentWeddings) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(WEDDING_FILE)
    {
        for entry in weddings.to_entries() {
            if let Ok(line) = serde_json::to_string(&entry) {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 婚書帳本 ──
    #[test]
    fn record_wedding_is_idempotent() {
        let mut w = ResidentWeddings::new();
        assert!(w.record_wedding("露娜", "諾娃"), "首次成婚應回 true");
        assert!(!w.record_wedding("露娜", "諾娃"), "重辦應回 false");
        assert!(!w.record_wedding("諾娃", "露娜"), "反序也算同一對、應回 false");
    }

    #[test]
    fn is_wed_order_independent() {
        let mut w = ResidentWeddings::new();
        w.record_wedding("諾娃", "露娜");
        assert!(w.is_wed("露娜", "諾娃"));
        assert!(w.is_wed("諾娃", "露娜"));
        assert!(!w.is_wed("露娜", "凱依"));
    }

    #[test]
    fn roundtrip_entries() {
        let mut w = ResidentWeddings::new();
        w.record_wedding("露娜", "諾娃");
        w.record_wedding("凱依", "阿川");
        let restored = ResidentWeddings::from_entries(w.to_entries());
        assert!(restored.is_wed("露娜", "諾娃"));
        assert!(restored.is_wed("阿川", "凱依"));
        assert_eq!(restored.to_entries().len(), 2);
    }

    // ── should_wed 各道閘 ──
    fn all_pass() -> (bool, bool, u32, bool, f32, bool, bool, f32) {
        // is_sweetheart, already_wed, visits, both_free, dist_sq, good_day, cooldown_ready, roll
        (true, false, WED_MIN_VISITS, true, PAIR_NEAR_SQ, true, true, 0.0)
    }

    #[test]
    fn wed_all_conditions_pass() {
        let (a, b, c, d, e, f, g, h) = all_pass();
        assert!(should_wed(a, b, c, d, e, f, g, h));
    }

    #[test]
    fn wed_rejects_non_sweetheart() {
        let (_, b, c, d, e, f, g, h) = all_pass();
        assert!(!should_wed(false, b, c, d, e, f, g, h));
    }

    #[test]
    fn wed_rejects_already_wed() {
        let (a, _, c, d, e, f, g, h) = all_pass();
        assert!(!should_wed(a, true, c, d, e, f, g, h), "已婚一生一次、不重辦");
    }

    #[test]
    fn wed_rejects_shallow_bond() {
        let (a, b, _, d, e, f, g, h) = all_pass();
        assert!(!should_wed(a, b, WED_MIN_VISITS - 1, d, e, f, g, h), "交情未疊夠深不成婚");
        assert!(should_wed(a, b, WED_MIN_VISITS, d, e, f, g, h), "剛好達門檻應成");
    }

    #[test]
    fn wed_rejects_busy_pair() {
        let (a, b, c, _, e, f, g, h) = all_pass();
        assert!(!should_wed(a, b, c, false, e, f, g, h));
    }

    #[test]
    fn wed_rejects_far_apart() {
        let (a, b, c, d, _, f, g, h) = all_pass();
        assert!(!should_wed(a, b, c, d, PAIR_NEAR_SQ + 0.1, f, g, h), "隔太遠不成婚");
        assert!(should_wed(a, b, c, d, PAIR_NEAR_SQ, f, g, h), "剛好在半徑上應成");
    }

    #[test]
    fn wed_rejects_bad_day() {
        let (a, b, c, d, e, _, g, h) = all_pass();
        assert!(!should_wed(a, b, c, d, e, false, g, h), "夜裡或下雨不辦喜事");
    }

    #[test]
    fn wed_rejects_on_cooldown() {
        let (a, b, c, d, e, f, _, h) = all_pass();
        assert!(!should_wed(a, b, c, d, e, f, false, h));
    }

    #[test]
    fn wed_respects_roll() {
        let (a, b, c, d, e, f, g, _) = all_pass();
        assert!(should_wed(a, b, c, d, e, f, g, WED_CHANCE - 0.001));
        assert!(!should_wed(a, b, c, d, e, f, g, WED_CHANCE), "roll 達機率上界不成");
        assert!(!should_wed(a, b, c, d, e, f, g, 0.99));
    }

    // ── 花拱門 ──
    #[test]
    fn arch_has_two_posts_and_flower_crown() {
        let blocks = arch_blocks(10, 5, 20);
        assert_eq!(blocks.len(), 7);
        // 木柱四塊
        let woods = blocks.iter().filter(|(_, _, _, b)| *b == Block::Wood).count();
        assert_eq!(woods, 4, "兩根兩格高木柱＝4 塊");
        // 花冠三色各一
        assert!(blocks.iter().any(|&(_, _, _, b)| b == Block::WildflowerRed));
        assert!(blocks.iter().any(|&(_, _, _, b)| b == Block::WildflowerYellow));
        assert!(blocks.iter().any(|&(_, _, _, b)| b == Block::WildflowerBlue));
        // 中柱底那格 (x+1, y) 刻意留空、可穿過——不在方塊清單裡
        assert!(!blocks.iter().any(|&(x, y, _, _)| x == 11 && y == 5), "拱門中間底部留空可穿過");
    }

    #[test]
    fn arch_all_distinct_cells() {
        let blocks = arch_blocks(0, 0, 0);
        let mut cells: Vec<(i32, i32, i32)> = blocks.iter().map(|&(x, y, z, _)| (x, y, z)).collect();
        cells.sort();
        cells.dedup();
        assert_eq!(cells.len(), 7, "7 塊座標互不重疊");
    }

    #[test]
    fn pick_anchor_varies_by_direction() {
        let mut seen = HashSet::new();
        for p in 0..4 {
            seen.insert(pick_anchor(50.0, 50.0, p));
        }
        assert_eq!(seen.len(), 4, "四個方向錨點互異");
    }

    // ── 台詞／記憶／Feed ──
    #[test]
    fn vow_embeds_partner_and_cycles() {
        for p in 0..6 {
            let line = vow_say_line("諾娃", p);
            assert!(line.contains("諾娃"), "誓言要嵌對方名");
            assert!(!line.contains("{p}"), "模板佔位要被替換乾淨");
        }
        assert_ne!(vow_say_line("諾娃", 0), vow_say_line("諾娃", 1), "不同 pick 應輪替");
    }

    #[test]
    fn wed_memory_is_persistent_promise() {
        let line = wed_memory_line("諾娃");
        assert!(line.contains("諾娃"));
        // 含「一定會」→ 被 classify_importance 判為 Promise 永久精華事實
        assert!(line.contains("一定會"), "婚禮記憶要能升為永久精華");
        assert!(!line.contains('\n'), "記憶單行");
    }

    #[test]
    fn cheer_embeds_both_names() {
        for p in 0..6 {
            let line = cheer_say_line("露娜", "諾娃", p);
            assert!(line.contains("露娜") && line.contains("諾娃"));
            assert!(!line.contains("{a}") && !line.contains("{b}"));
        }
    }

    #[test]
    fn feed_line_non_empty_with_names() {
        let line = wed_feed_line("露娜", "諾娃");
        assert!(line.contains("露娜") && line.contains("諾娃") && !line.is_empty());
    }
}
