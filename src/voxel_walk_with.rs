//! 乙太方界·居民邀你散步同行 v1（自主提案切片，ROADMAP 926）——「結伴同行」第一次把玩家接進來。
//!
//! **真缺口**：摯友結伴同行（925）讓兩位互為摯友的**居民**第一次並肩走過村子，補上了小社會裡
//! 「陪你走一段路，什麼都不為」那一拍——但那份最日常、最有溫度的一幕，至今**只發生在居民之間**，
//! 玩家從來不在其中。玩家與居民的「一起」一路疊了記憶（記得你）、招呼、贈禮、道謝、關心你挨餓
//! （845）、你久別歸來的迎接（921）、離開時替你留一盞燈（919）——但這些全是**你缺席或路過的
//! 一瞬**，或是**居民對你做一件事**；從沒有一刀，讓一位對你有交情的居民，就只是**在你身邊，
//! 陪你在村裡走走**。本刀補上那一拍：**白天好天氣裡，一位對你記憶夠厚（好感達摯友門檻）的居民，
//! 見你正閒著、就近晃過來邀你「陪我走走」，然後像個伴一樣貼著你此刻的位置，並肩在村裡漫步一段**——
//! 你往哪走，牠就順著你走，有一搭沒一搭地陪你晃過村子，走完各自記進心裡。這是玩家↔居民的情誼
//! 第一次不只寫在數字或一瞬的招呼裡，而是**走在了村子的路上**。
//!
//! **與既有元素 razor-sharp 區隔（非同軸換皮）**：
//! - **摯友結伴同行（925）**＝**居民↔居民**、follower 貼著另一位**居民 leader** 走；本刀＝
//!   **居民↔玩家**、居民貼著**玩家此刻的座標**走（你才是被陪著的那位）——對象全然不同。
//! - **居民惦記/守候/迎接你（915/919/921）**＝觸發於你**缺席或歸來的一瞬**，是「你不在時」的牽掛；
//!   本刀＝你**正在場**，居民就在你身邊陪你走一段路的**過程本身**。
//! - **跟隨你走的馴服動物（851）**＝**動物**當小跟班；本刀＝**具名 AI 居民**主動邀約、有交情門檻、
//!   有台詞與記憶落地——是社交行為不是寵物行為。
//! - **居民關心你挨餓（845）／道謝（888）**＝居民**對你做一件事**（遞麵包／道謝）後就結束；本刀＝
//!   不涉物品也不為報答，重點是**並肩走這一段路**的陪伴，不以任何動作為終點。
//!
//! **純邏輯層**：確定性、零 LLM、零鎖、零 IO、零 async、可單元測試（資格判定／台詞／記憶＋Feed
//! 台詞）。真正的鎖、快照、閒晃中心覆寫（貼著玩家座標）、廣播、記憶/Feed 落地全在 `voxel_ws.rs`
//! （沿用摯友結伴同行 925 的短鎖循序＋鎖外落地慣例，守 prod 死鎖鐵律）。
//!
//! **成本鐵律**：零 LLM（配對＋台詞全確定性）、零 migration（記憶走既有 episodic 層、重啟歸零零
//! 風險）、零新協議欄位（居民貼著玩家走靠既有廣播的居民座標同步，不動任何 WS/HTTP 欄位）、零新
//! 美術、零前端改動、FPS 零影響（配對只在既有低頻社交掃描的一個 tick 掃少數幾位居民×少數在線玩家一次）。
//! **濫用防護**：不開任何新對外端點、不觸發 LLM、不動帳號權限；玩家**無從自報或催發**——邀約完全由
//! 伺服器權威的「居民對你的好感（既有 memory 好感度）＋你此刻的座標」驅動。唯一嵌入的玩家可控字串是
//! **玩家顯示名**（早在加入時就過清洗），且只落進單行模板的台詞/記憶/動態牆（`.chars().take` 收斂
//! 長度、無換行、無 LLM），與既有守夜燈（919）/迎接（921）embed 玩家名同一手法，無新增注入面。

/// 一位對你有交情的居民在某個 tick 起意邀你同行的機率（低頻、稀有感，比照摯友結伴同行
/// `STROLL_CHANCE` 同量級——散步是偶爾為之的小確幸，不是每隔幾秒就上演）。
pub const WALK_CHANCE: f32 = 0.05;

/// 陪你同行的持續秒數：走一小段路就好，過了就各自回到平常的一天（同時兼作逾時保險，
/// 不會有居民被永遠黏在玩家身邊）。
pub const WALK_DURATION_SECS: f32 = 30.0;

/// 一段同行落幕後的靜置冷卻秒數：到期前這位居民不會又立刻邀你，避免同一位反覆黏著同行。
pub const WALK_COOLDOWN_SECS: f32 = 240.0;

/// 居民要離你夠近（水平距離 ≤ 此值，方塊）才會起意邀你走——是「就近邀身邊的你」，
/// 不是把遠在村子另一頭的居民硬拉過來。
pub const WALK_START_DIST: f32 = 6.0;

/// 居民陪你走時的閒晃半徑（方塊）：刻意小，讓牠看起來就在你身邊並肩、不散開
/// （比照摯友結伴同行 `STROLL_WANDER_RADIUS` 的「貼著走」手法）。
pub const WALK_WANDER_RADIUS: f32 = 1.8;

/// 居民要對你有多厚的交情（既有 memory 好感度 `affinity_count`）才會邀你同行——摯友門檻。
/// 陌生人不會沒來由地拉你散步；得是你和牠之間已經累積出情誼（送禮/道謝/相處），牠才會就近邀你。
/// 與招呼「友人級」門檻（`FRIEND_AFFINITY_THRESHOLD = 3`）同量級，語義一致：邀你同行＝把你當朋友。
pub const WALK_AFFINITY: usize = 3;

/// 動態牆分類（與摯友結伴同行「結伴同行」分開，讓玩家一眼看出這是**居民邀你**的同行）。
pub const FEED_KIND: &str = "邀你同行";

/// 判定這位居民此刻是否適合邀某位玩家同行：居民閒著（由呼叫端把各種「正忙」狀態收斂成一個 bool）、
/// **不在同行冷卻中**（上一段同行落幕後 `WALK_COOLDOWN_SECS` 內不再邀你、防同一位反覆黏著）、
/// 對這位玩家的好感達摯友門檻、且水平距離夠近。純函式、確定性、可測。
pub fn eligible(resident_free: bool, on_cooldown: bool, bond: usize, dist_sq: f32) -> bool {
    resident_free
        && !on_cooldown
        && bond >= WALK_AFFINITY
        && dist_sq <= WALK_START_DIST * WALK_START_DIST
}

/// 居民起意邀你時冒的一句（確定性輪替，`pick` 由呼叫端給隨機源；點名邀約的玩家更有溫度）。
pub fn invite_line(player: &str, pick: usize) -> String {
    let variants: [&str; 3] = [
        "{player}，走，陪我在村裡走走～",
        "難得都閒著，{player}，一起晃晃吧？",
        "{player}，這天氣真好，我們散散步？",
    ];
    variants[pick % variants.len()].replace("{player}", player)
}

/// 同行落幕後、居民記進心裡的一筆 episodic 記憶（點名同行的玩家；累積對你的好感）。
/// 純文字、確定性、單行防注入。
pub fn walk_memory_line(player: &str) -> String {
    format!("和{player}並肩在村裡散了會兒步，什麼都不為，就是舒服。")
}

/// 全村動態牆的一行（點名居民與玩家；讓不在場的玩家也讀得到今天誰邀了誰一起散步）。單行、確定性。
pub fn walk_feed_line(resident: &str, player: &str) -> String {
    format!("{resident}邀{player}並肩在村裡散步，有一搭沒一搭地聊著，走了好一段路。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_needs_free_bond_and_near() {
        let near = (WALK_START_DIST - 1.0).powi(2);
        // 閒著＋不在冷卻＋交情夠厚＋夠近 → 成立。
        assert!(eligible(true, false, WALK_AFFINITY, near));
        assert!(eligible(true, false, WALK_AFFINITY + 2, near));
        // 在忙 → 不邀。
        assert!(!eligible(false, false, WALK_AFFINITY, near));
        // 交情不到門檻 → 不硬邀陌生人。
        assert!(!eligible(true, false, WALK_AFFINITY - 1, near));
        assert!(!eligible(true, false, 0, near));
        // 交情夠但離太遠 → 不硬拉。
        let far = (WALK_START_DIST + 1.0).powi(2);
        assert!(!eligible(true, false, WALK_AFFINITY, far));
    }

    #[test]
    fn eligible_respects_walk_cooldown() {
        // 冷卻回歸釘死：上一段同行剛落幕、還在 240 秒冷卻中的居民，即使閒著＋交情夠厚
        //＋就在你身邊，也**不會**又立刻邀你——冷卻不是形同虛設（防同一位反覆黏著同行）。
        let near = (WALK_START_DIST - 1.0).powi(2);
        assert!(!eligible(true, true, WALK_AFFINITY, near));
        assert!(!eligible(true, true, WALK_AFFINITY + 5, near));
        // 冷卻退掉後同樣條件才重新成立（對照組）。
        assert!(eligible(true, false, WALK_AFFINITY, near));
    }

    #[test]
    fn eligible_exact_boundary_counts_as_near() {
        // 恰好等於門檻距離：視為夠近（<= 邊界含端點，確定性）。
        let exact = WALK_START_DIST * WALK_START_DIST;
        assert!(eligible(true, false, WALK_AFFINITY, exact));
    }

    #[test]
    fn invite_names_the_player_and_is_bounded() {
        // 台詞點名玩家、越界 pick 安全取模、永不空、單行。
        for pick in [0usize, 1, 2, 3, 7, 100] {
            let inv = invite_line("旅人", pick);
            assert!(inv.contains("旅人"));
            assert!(!inv.is_empty());
            assert!(!inv.contains('\n'));
        }
    }

    #[test]
    fn invite_variants_rotate() {
        // 不同 pick 至少能取到不只一種台詞（不是永遠同一句）。
        assert_ne!(invite_line("旅人", 0), invite_line("旅人", 1));
    }

    #[test]
    fn memory_and_feed_name_participants_and_single_line() {
        let mem = walk_memory_line("旅人");
        assert!(mem.contains("旅人"));
        assert!(!mem.contains('\n'));
        assert!(!mem.is_empty());
        let feed = walk_feed_line("露娜", "旅人");
        assert!(feed.contains("露娜") && feed.contains("旅人"));
        assert!(!feed.contains('\n'));
    }

    #[test]
    fn feed_does_not_falsely_claim_first_ever() {
        // 措辭不謊稱「史上第一次」——同行是日常小事，不是里程碑。
        let feed = walk_feed_line("露娜", "旅人");
        assert!(!feed.contains("第一次") && !feed.contains("史上"));
    }
}
