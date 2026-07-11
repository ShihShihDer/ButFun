//! 乙太方界·居民互取親暱綽號 v1（voxel-nickname，自主提案切片）。
//!
//! **真缺口 / 為誰做**：PLAN_ETHERVOX 路線圖④「居民↔居民關係 → 小社會湧現」＋核心信念
//! 「記憶要驅動**行為**、不只聊天」。居民之間早已會累積情誼（`voxel_bonds`：相識→老朋友）、
//! 會拌嘴和好（`voxel_bench_tiff`）、會圍火講古（792）、會傳八卦（`voxel_gossip`），但彼此
//! 之間**始終只以本名相稱**——交情再深，露娜喊凜也還是「凜」，村子裡感受不到那種「熟到會給
//! 對方取個只屬於你們之間的暱稱」的親密。這正是一個真的活著的小村最生活化的一筆：好朋友之間
//! 有只有彼此才用的親暱稱呼。
//!
//! **本刀**：當兩位居民的情誼到了**老朋友（Friend）**、又恰好走得夠近時，交情較主動的一方
//! 偶爾為對方取一個親暱綽號（依對方名字第一個字＋樣式，確定性生成，如「露露」「小凜」），當面
//! 宣告、並記進城鎮動態牆；此後這位居民**記得**這個綽號，日後再遇見對方時，會偶爾直接用綽號
//! 招呼——綽號一旦取下，就在世界裡**被真的使用**（記憶→行為的閉環）。玩家路過村子，第一次
//! 會聽見居民互相以只屬於彼此的暱稱相稱，村子的關係網第一次有了它自己的親密紋理。
//!
//! **與既有稱呼類切片 razor-sharp 區隔**：
//! - `voxel_playerepithet`／`voxel_epithet_spread`／`voxel_nameplate`：那些是**居民為「玩家」**
//!   昇華的**名號**（榮譽感、口耳相傳給的是你的稱號）；本刀是**居民為「彼此」**取的**親暱綽號**
//!   （生活化的親密，與玩家無關）。
//! - `voxel_petname`：那是**玩家為「寵物」**取名；本刀是**居民↔居民**之間。
//! - `voxel_fond_greeting`：那是熟人相遇時的溫暖招呼；本刀先「取下」一個綽號、再讓它在日後招呼中
//!   **被使用**，多的是「命名 → 記住 → 沿用」這條記憶→行為線。
//!
//! **成本紀律**：零 LLM（綽號與台詞皆確定性純函式）、零 migration（綽號存居民記憶體 map，比照
//! 其他世界暫態、重啟歸零）、零協議破壞（綽號只化為既有 `say` 泡泡＋城鎮動態，無新廣播欄位）、
//! 零前端改動、零新美術、FPS 零影響（純後端低頻 tick，長冷卻＋低機率保持稀疏）。
//!
//! **濫用防護**：不收玩家輸入、不觸發 LLM、不開對外端點、不動帳號權限——觸發全由伺服器內部狀態
//! （情誼層級／距離／冷卻／機率）驅動，玩家無從自報或催發；綽號只由**居民顯示名**（既有安全字串）
//! 衍生、長度天然有界，無注入／NSFW 面。
//!
//! 本模組全是零 IO／零鎖／零 async 的純邏輯，可單元測試。

/// 取暱稱／用暱稱共用冷卻（秒）：一位居民對某對象做過一次「取名或招呼」後，隔這麼久才會再來一次，
/// 讓親暱互動稀疏、不洗版。
pub const NICKNAME_COOLDOWN_SECS: f32 = 240.0;

/// 每次符合條件的相遇「取下新綽號」的機率（低機率＝罕見而有份量）。
pub const COIN_CHANCE: f32 = 0.05;

/// 每次符合條件的相遇「用綽號招呼」的機率（略高於取名，讓取下的綽號日後真的常被用到）。
pub const GREET_CHANCE: f32 = 0.06;

/// 城鎮動態牆分類標籤。
pub const FEED_KIND: &str = "暱稱";

/// 泡泡台詞字數上限（防溢框；綽號本身天然極短，此處保守截斷整句）。
pub const SAY_CHARS: usize = 40;

/// 依對方顯示名的第一個字 + 樣式索引，生成一個親暱綽號（確定性、可測、天然有界）。
///
/// 取名字第一個字當種字，套四種常見的中文親暱樣式：疊字（露露）／小＋（小露）／阿＋（阿露）／
/// ＋兒（露兒）。`seed` 由呼叫端傳（如居民座標雜湊），保證同一次事件確定性、跨事件有變化。
pub fn coin_nickname(target_name: &str, seed: usize) -> String {
    let first: String = target_name
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();
    if first.is_empty() {
        // 居民顯示名理論上皆非空；極端防禦回一個通用暱稱。
        return "小夥伴".to_string();
    }
    match seed % 4 {
        0 => format!("{first}{first}"), // 露露
        1 => format!("小{first}"),      // 小露
        2 => format!("阿{first}"),      // 阿露
        _ => format!("{first}兒"),      // 露兒
    }
}

/// 是否在這次相遇「取下一個新綽號」：冷卻到期 ∧ 已是老朋友 ∧ 尚未為對方取過綽號 ∧ 擲骰命中。
pub fn should_coin(
    cooldown_ready: bool,
    is_close_friend: bool,
    already_named: bool,
    roll: f32,
    chance: f32,
) -> bool {
    cooldown_ready && is_close_friend && !already_named && roll < chance
}

/// 是否在這次相遇「用既有綽號招呼對方」：冷卻到期 ∧ 已為對方取過綽號 ∧ 擲骰命中。
pub fn should_greet(cooldown_ready: bool, has_nickname: bool, roll: f32, chance: f32) -> bool {
    cooldown_ready && has_nickname && roll < chance
}

/// 取下綽號當面宣告的泡泡台詞（確定性選句，截斷防溢框）。
pub fn coin_announce_bubble(nickname: &str, seed: usize) -> String {
    let pool = [
        format!("從今天起，我就叫你「{nickname}」吧！"),
        format!("嘿，「{nickname}」，這是只屬於你的暱稱喔～"),
        format!("我想給你取個親暱的名字——「{nickname}」，喜歡嗎？"),
        format!("「{nickname}」，這樣叫你，是不是更親了呢？"),
    ];
    pool[seed % pool.len()].chars().take(SAY_CHARS).collect()
}

/// 城鎮動態牆的一句：「A 開始親暱地喚 B 為「暱稱」。」
pub fn coin_feed_line(coiner: &str, target: &str, nickname: &str) -> String {
    format!("{coiner}開始親暱地喚{target}為「{nickname}」。")
}

/// 日後再遇見、用既有綽號招呼對方的泡泡台詞（確定性選句，截斷防溢框）。
pub fn greet_bubble(nickname: &str, seed: usize) -> String {
    let pool = [
        format!("「{nickname}」，今天也一起加油喔～"),
        format!("早啊，「{nickname}」！"),
        format!("「{nickname}」，過得還好嗎？"),
        format!("看到你真開心，「{nickname}」。"),
    ];
    pool[seed % pool.len()].chars().take(SAY_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nickname_uses_first_char_and_four_styles() {
        assert_eq!(coin_nickname("露娜", 0), "露露");
        assert_eq!(coin_nickname("露娜", 1), "小露");
        assert_eq!(coin_nickname("露娜", 2), "阿露");
        assert_eq!(coin_nickname("露娜", 3), "露兒");
        // seed 循環（% 4）：4 回到疊字。
        assert_eq!(coin_nickname("露娜", 4), "露露");
    }

    #[test]
    fn nickname_deterministic_across_names() {
        assert_eq!(coin_nickname("凜", 1), "小凜");
        assert_eq!(coin_nickname("諾娃", 0), "諾諾");
        // 同輸入必得同輸出。
        assert_eq!(coin_nickname("蕾雅", 2), coin_nickname("蕾雅", 2));
    }

    #[test]
    fn nickname_empty_name_falls_back() {
        assert_eq!(coin_nickname("", 0), "小夥伴");
        assert_eq!(coin_nickname("", 3), "小夥伴");
    }

    #[test]
    fn nickname_is_short_and_bounded() {
        for seed in 0..8usize {
            let n = coin_nickname("露娜", seed);
            assert!(!n.is_empty());
            assert!(n.chars().count() <= 3, "綽號應天然極短：{n}");
        }
    }

    #[test]
    fn should_coin_requires_all_conditions() {
        // 全部滿足 → true。
        assert!(should_coin(true, true, false, 0.0, COIN_CHANCE));
        // 冷卻沒到 → false。
        assert!(!should_coin(false, true, false, 0.0, COIN_CHANCE));
        // 還不是老朋友 → false。
        assert!(!should_coin(true, false, false, 0.0, COIN_CHANCE));
        // 已經取過綽號 → false。
        assert!(!should_coin(true, true, true, 0.0, COIN_CHANCE));
        // 擲骰沒中 → false。
        assert!(!should_coin(true, true, false, 1.0, COIN_CHANCE));
    }

    #[test]
    fn should_coin_boundary_is_strict_less_than() {
        // roll 恰等於 chance → 不觸發（嚴格小於）。
        assert!(!should_coin(true, true, false, COIN_CHANCE, COIN_CHANCE));
        // 略小於 → 觸發。
        assert!(should_coin(true, true, false, COIN_CHANCE - 0.001, COIN_CHANCE));
    }

    #[test]
    fn should_greet_requires_nickname_and_cooldown() {
        assert!(should_greet(true, true, 0.0, GREET_CHANCE));
        assert!(!should_greet(false, true, 0.0, GREET_CHANCE)); // 冷卻沒到
        assert!(!should_greet(true, false, 0.0, GREET_CHANCE)); // 還沒取過綽號
        assert!(!should_greet(true, true, GREET_CHANCE, GREET_CHANCE)); // 邊界嚴格小於
    }

    #[test]
    fn announce_bubble_contains_nickname_and_cycles() {
        let a = coin_announce_bubble("小凜", 0);
        assert!(a.contains("小凜"));
        // 四種樣式各不相同。
        let set: std::collections::HashSet<_> =
            (0..4).map(|s| coin_announce_bubble("小凜", s)).collect();
        assert_eq!(set.len(), 4, "四句宣告台詞應各異");
        // seed 循環穩定。
        assert_eq!(
            coin_announce_bubble("小凜", 0),
            coin_announce_bubble("小凜", 4)
        );
    }

    #[test]
    fn greet_bubble_contains_nickname_and_bounded() {
        for seed in 0..6usize {
            let g = greet_bubble("露露", seed);
            assert!(g.contains("露露"));
            assert!(g.chars().count() <= SAY_CHARS);
        }
    }

    #[test]
    fn feed_line_names_both_and_nickname() {
        let line = coin_feed_line("露娜", "凜", "小凜");
        assert!(line.contains("露娜"));
        assert!(line.contains("凜"));
        assert!(line.contains("小凜"));
    }
}
