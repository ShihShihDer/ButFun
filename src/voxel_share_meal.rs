//! 乙太方界·飢餓時的守望相助 v1——餓著的居民路遇交情好的鄰居，鄰居分她一口飯（自主提案切片）。
//!
//! **缺口 / 為誰做**：799 給了居民第一個生理需求「餓」——餓了會冒一句心聲、自己走回家吃存糧。
//! 但那條線至今全是**居民對自己**（自理回家吃）或**玩家對居民**（你餵她一口）——居民**彼此之間**
//! 對這個需求毫無反應：一位居民餓著肚子從老朋友身邊走過，朋友卻視若無睹。這正對著 PLAN_ETHERVOX
//! §4「居民↔居民關係 → 小社會湧現」——一個真的活著的小村，鄰里之間會在對方餓著時分一口飯。本刀
//! 把「餓」這個生理需求接上「情誼」這張關係網：**餓著找吃的（`seeking_food`）居民路過一位交情已到
//! 相識以上、此刻閒著又不餓的鄰居時，鄰居偶爾會喚住她、分一口飯**——餓意當場解除、雙方各記一筆
//! 暖記憶、情誼因這頓飯再加溫一格、上城鎮動態牆。守望相助第一次在方塊天地裡自己湧現。
//!
//! **記憶驅動行為（北極星）**：分不分這口飯，看的是**兩人的交情**——只有相識以上的鄰居才會分
//! （陌生人擦身而過不會），情誼越深越自然。你不會憑空看到路人施捨，只會看到**處出了交情的鄰居**
//! 在對方餓時伸手。關係網（記憶累積出的 bonds）真的改變了居民的行為。
//!
//! **與既有系統的分界（換維度·非同軸重複）**：
//! - 不是 799（餓 → 自己回家吃 / 玩家餵）——那是「對自己」「玩家對居民」，本刀是**居民對居民**的互助。
//! - 不是 679 打氣（走向**孤獨**的同伴送暖話）——那治的是**情緒**缺口、且是主動巡找 Lonely；本刀治的
//!   是**生理**缺口（餓）、且是**恰好路過**才觸發（零新巡路，鄰居本來就在附近）。
//! - 不是 792 圍火講古（夜裡同一座營火邊講往事）——那是**言語**分享、需營火場景；本刀是**食物**分享、
//!   任何時地只要一方餓著、一方閒著相遇即可。
//! - 不是 782 witness 賀喜（為鄰居**圓夢**道賀）——那回應的是成就；本刀回應的是**匱乏**（餓）。
//!
//! **這裡只放確定性純邏輯**（分食門檻、情誼資格、台詞/記憶/Feed 文案），零 LLM、零鎖、零 IO、
//! 零 async，可單元測試。配對掃描 / 鎖 / 走動 / 廣播全留在 `voxel_ws.rs`（沿用 792 圍火講古的
//! 「位置快照 → i≠j 循序掃描 → 每 tick 最多一對 → 記憶/Feed 鎖外落地」慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：不觸發 LLM、不開對外端點、不收玩家自由文字——分食/道謝台詞與記憶全為確定性
//! 模板、只嵌居民系統顯示名（本就出現在動態牆），永不回放記憶原文或玩家原話（無注入 / NSFW 面）；
//! 觸發純伺服器 tick 內部狀態（餓意 + 情誼 + 冷卻 + 低機率），玩家無法自報、無法從外部催發；分食冷卻
//! （每位分食者 `SHARE_COOLDOWN_SECS`）+ 每 tick 低機率 + 需相識以上 = 天然節流，不洗版泡泡/動態牆。
//! 餓意/冷卻純記憶體重啟歸零（過場狀態、零資料風險、零 migration），記憶/情誼走既有 append-only。

use crate::voxel_bonds::BondTier;

/// 分食者要多靠近餓著的鄰居，才會喚住她分一口（世界方塊距離）。刻意比社交半徑近一點——
/// 分食是「就在你旁邊」的舉動，不是隔著大半個村子喊。
pub const SHARE_RADIUS: f32 = 4.5;

/// 分食者一次分食後的靜默冷卻（秒）：讓「分一口飯」這件事稀少而有份量，不會同一位居民
/// 短時間內反覆對人施食、洗版泡泡與動態牆。
pub const SHARE_COOLDOWN_SECS: f32 = 200.0;

/// 條件都滿足後，這一 tick 真的開口分食的機率。刻意不設 1.0——就算旁邊有餓著的老友，
/// 也不是每一瞬間都會伸手，偶爾才觸發，像真的生活裡的不期而遇。
pub const SHARE_CHANCE: f32 = 0.5;

/// 被分食的居民延遲幾秒才道謝（讓「分食 → 道謝」有一來一往的呼吸感，非同一瞬間兩人齊聲）。
pub const THANKS_DELAY_SECS: f32 = 2.5;

/// 這對居民的交情夠不夠深、值不值得分一口飯——相識（`Acquaintance`）以上才會分，
/// 陌生人擦身而過不會。這是「記憶（情誼）驅動行為」的閘：只有處出了交情的鄰居才會伸手。
pub fn tier_allows_share(tier: BondTier) -> bool {
    tier >= BondTier::Acquaintance
}

/// 分食門檻：冷卻已過 + 過了機率骰。純函式、確定性、可測。
pub fn should_share(cooldown: f32, roll: f32, chance: f32) -> bool {
    cooldown <= 0.0 && roll < chance
}

/// 入場錯開初始分食冷卻（秒），避免啟動後短時間內全員一起搶著分食。
pub fn share_cd_offset(i: usize) -> f32 {
    80.0 + i as f32 * 40.0
}

/// 分食者喚住餓著的鄰居、遞上一口飯時的暖泡泡（四句輪替）。
pub fn sharer_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "來，分你一口，別餓著肚子",
        "餓了吧？這個拿去墊墊",
        "正好我有多的，一起吃點",
        "別客氣，先吃口東西暖暖",
    ];
    LINES[pick % LINES.len()]
}

/// 被分食的居民延遲後冒出的道謝泡泡（嵌分食者名，四句輪替，截字前先組好）。
/// `sharer` 空（理論上不會發生）→ 退成不點名的泛稱，仍不回放任何原話。
pub fn thanks_line(sharer: &str, pick: usize) -> String {
    const LINES: [&str; 4] = [
        "多虧有你，這頓解了餓，謝謝",
        "真是太謝謝你了，暖到心裡",
        "有你這樣的鄰居，真好",
        "這口飯的情，我記著了",
    ];
    let base = LINES[pick % LINES.len()];
    if sharer.is_empty() {
        base.to_string()
    } else {
        format!("{sharer}，{base}")
    }
}

/// 被分食者掛在分食者名下的暖記憶（episodic，累積情誼）。`sharer` 空 → 泛稱。
pub fn shared_memory_for_hungry(sharer: &str) -> String {
    if sharer.is_empty() {
        "我正餓著的時候，有位鄰居分了我一口飯，這份情我記著。".to_string()
    } else {
        format!("我正餓著的時候，{sharer}分了我一口飯，這份情我記著。")
    }
}

/// 分食者掛在被分食者名下的記憶（episodic，累積情誼）。`hungry` 空 → 泛稱。
pub fn shared_memory_for_sharer(hungry: &str) -> String {
    if hungry.is_empty() {
        "有位鄰居餓著肚子，我分了她一口飯——這點小忙，樂意幫。".to_string()
    } else {
        format!("{hungry}餓著肚子，我分了她一口飯——這點小忙，樂意幫。")
    }
}

/// 城鎮動態牆一行：讓不在場 / 回來的玩家也讀到「鄰里之間互相照應」。
pub fn share_feed_line(sharer: &str, hungry: &str) -> String {
    let s = if sharer.is_empty() { "有位鄰居" } else { sharer };
    let h = if hungry.is_empty() { "一位餓著的鄰居" } else { hungry };
    format!("{h}正餓著，{s}分了她一口飯——這份守望相助，暖了整座村子。")
}

/// Feed 分類標籤。
pub const FEED_KIND: &str = "守望相助";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_gate_requires_acquaintance() {
        // 陌生人不分；相識、老朋友才分。
        assert!(!tier_allows_share(BondTier::Stranger));
        assert!(tier_allows_share(BondTier::Acquaintance));
        assert!(tier_allows_share(BondTier::Friend));
    }

    #[test]
    fn should_share_needs_cooldown_and_roll() {
        // 冷卻未到：不分（就算骰贏）。
        assert!(!should_share(1.0, 0.0, SHARE_CHANCE));
        // 冷卻到、骰贏：分。
        assert!(should_share(0.0, 0.1, SHARE_CHANCE));
        // 冷卻到、骰輸（roll == chance 不觸發，嚴格小於）：不分。
        assert!(!should_share(0.0, SHARE_CHANCE, SHARE_CHANCE));
        assert!(!should_share(0.0, 0.9, SHARE_CHANCE));
        // 冷卻恰為 0 也算到期。
        assert!(should_share(0.0, 0.0, 1.0));
    }

    #[test]
    fn cd_offsets_are_staggered() {
        let offs: Vec<f32> = (0..4).map(share_cd_offset).collect();
        for w in offs.windows(2) {
            assert!(w[1] > w[0], "初始冷卻應遞增錯開");
        }
        assert!(offs[0] > 0.0);
    }

    #[test]
    fn lines_rotate_and_bounded() {
        // 分食/道謝台詞輪替、非空、長度在泡泡上限（≤40 字，含嵌名後）內。
        for pick in 0..8 {
            let sl = sharer_line(pick);
            assert!(!sl.is_empty() && sl.chars().count() <= 40);
            let tl = thanks_line("諾娃", pick);
            assert!(!tl.is_empty() && tl.chars().count() <= 40);
        }
        // pick 溢出取模包回、不 panic。
        assert_eq!(sharer_line(0), sharer_line(4));
        assert_eq!(thanks_line("露娜", 1), thanks_line("露娜", 5));
    }

    #[test]
    fn thanks_embeds_or_falls_back() {
        let t = thanks_line("賽勒", 0);
        assert!(t.contains("賽勒"));
        // 空名退泛稱、不留空洞。
        let g = thanks_line("", 0);
        assert!(!g.is_empty() && !g.contains("，，"));
    }

    #[test]
    fn memories_embed_names_or_fall_back() {
        let mh = shared_memory_for_hungry("露娜");
        assert!(mh.contains("露娜") && mh.contains("餓"));
        let ms = shared_memory_for_sharer("奧瑞");
        assert!(ms.contains("奧瑞"));
        // 空名皆退泛稱、仍提及分食情境、不含原話。
        assert!(shared_memory_for_hungry("").contains("餓"));
        assert!(!shared_memory_for_sharer("").is_empty());
    }

    #[test]
    fn feed_embeds_both_or_falls_back() {
        let f = share_feed_line("露娜", "諾娃");
        assert!(f.contains("露娜") && f.contains("諾娃"));
        // 任一空名皆退泛稱、不破句。
        let g = share_feed_line("", "");
        assert!(g.contains("鄰居") && !g.is_empty());
    }

    #[test]
    fn long_names_do_not_break_or_panic() {
        let long = "超".repeat(200);
        // 嵌超長名不 panic；泡泡層由呼叫端截字，這裡只保證不炸。
        let _ = thanks_line(&long, 0);
        let _ = shared_memory_for_hungry(&long);
        let _ = shared_memory_for_sharer(&long);
        let _ = share_feed_line(&long, &long);
    }
}
