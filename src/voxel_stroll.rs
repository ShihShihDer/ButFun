//! 乙太方界·摯友結伴同行 v1（自主提案切片，ROADMAP 925）——小社會湧現第一次寫進「移動」。
//!
//! **真缺口**：居民↔居民關係一路疊了小圈子聚會（711，三人以上「定點」聚在某位家域打轉）、
//! 戀人牽掛（852，戀人「單向奔赴」、抵達即相見結束）、暮聚（村莊自發習俗，一群人晃到村碑
//! 廣場「定點」聚著）——但這些「一起」全是**聚到某個點**，或是**戀愛**專屬的奔赴；至今從沒有
//! 一刀讓兩位單純的摯友，就只是**並肩在村裡走走**。世界裡的居民要嘛各自閒晃、要嘛湊到一個定點，
//! 卻從不曾兩兩結伴、有一搭沒一搭地一起穿過村子——而「陪你走一段路，什麼都不為」正是柏拉圖式
//! 摯友之間最日常、最有溫度的一幕。本刀補上那一拍：**白天好天氣裡，兩位互為摯友、都閒著且正相鄰
//! 的居民，偶爾一位會邀另一位一起在村裡散步一段**，follower 把閒晃中心貼著 leader 此刻的位置，
//! 兩人並肩緩緩漫步過村子，走一小段路，各自記進心裡。你路過會撞見露娜和諾娃肩並肩晃著，這是小
//! 社會的情誼第一次不只寫在數字或定點聚會裡，而是走在了村子的路上。
//!
//! **與既有元素 razor-sharp 區隔（非同軸換皮）**：
//! - **小圈子聚會（711）**＝**三人以上**相約**聚到某位家域這個定點**、到齊後**站著**說笑；本刀＝
//!   **恰好兩位**摯友、**沒有定點**、重點是**移動中並肩同行的過程本身**（follower 貼著 leader 的
//!   「移動座標」漫步，不是奔向一個固定聚會點）。
//! - **戀人牽掛（852）**＝**戀愛**關係（需先締結戀人）、**單向奔赴**、**抵達即相見結束**；本刀＝
//!   **柏拉圖式摯友**（BondTier::Friend）、**結伴同行的整段過程**才是重點、不以抵達為終點。
//! - **暮聚（村莊自發習俗）**＝黃昏、全村級、晃到**村碑廣場定點**聚著；本刀＝白天、**一對**、
//!   **沒有廣場定點**，是兩個人的散步不是全村的集會。
//!
//! **純邏輯層**：確定性、零 LLM、零鎖、零 IO、零 async、可單元測試（資格判定／挑 leader／台詞／
//! 記憶＋Feed 台詞）。真正的鎖、快照、閒晃中心覆寫、廣播、記憶/Feed 落地全在 `voxel_ws.rs`
//! （沿用小圈子聚會 711、戀人牽掛 852 的短鎖循序＋鎖外落地慣例，守 prod 死鎖鐵律）。
//!
//! **成本鐵律**：零 LLM（配對＋台詞全確定性）、零 migration（記憶走既有 episodic 層、重啟歸零零
//! 風險）、零新協議欄位（follower 貼著 leader 走靠既有廣播的居民座標同步，不動任何 WS/HTTP 欄位）、
//! 零新美術、零前端改動、FPS 零影響（配對只在既有低頻社交掃描的一個 tick 掃少數幾位居民兩兩一次）。
//! **濫用防護**：不收玩家輸入、不觸發 LLM、不開對外端點、不動帳號權限——結伴完全由伺服器權威的
//! 居民交情與座標驅動，玩家無從自報或催發、也無從洗版；台詞/記憶皆內建常數、無玩家可注入內容。

/// 一對摯友在某個 tick 被選中一起散步的機率（低頻、稀有感，比照小圈子聚會 `GATHER_CHANCE`
/// 同量級——散步是偶爾為之的小確幸，不是每隔幾秒就上演）。
pub const STROLL_CHANCE: f32 = 0.06;

/// 結伴同行的持續秒數：走一小段路就好，過了就各自回到平常的一天（同時兼作逾時保險，
/// 不會有人被永遠黏在別人身邊）。
pub const STROLL_DURATION_SECS: f32 = 30.0;

/// 一段散步落幕後的靜置冷卻秒數：到期前這兩位不會又立刻被選中，避免同一對反覆黏著散步。
pub const STROLL_COOLDOWN_SECS: f32 = 240.0;

/// 兩位摯友要湊得夠近（水平距離 ≤ 此值，方塊）才會起意一起走——散步是「就近邀身邊的朋友」，
/// 不是把遠在村子另一頭的人硬拉過來。
pub const STROLL_START_DIST: f32 = 6.0;

/// follower 貼著 leader 漫步時的閒晃半徑（方塊）：刻意小，讓兩人看起來並肩同行、不散開
/// （比照小圈子聚會 `GATHER_WANDER_RADIUS` 的「湊在一塊」手法）。
pub const STROLL_WANDER_RADIUS: f32 = 1.8;

/// 動態牆分類（與小圈子聚會「小圈子」、暮聚「村莊習俗」分開，讓玩家一眼看出這是一對摯友的散步）。
pub const FEED_KIND: &str = "結伴同行";

/// 判定兩位居民此刻是否適合結伴散步：兩人都閒著（由呼叫端把各種「正忙」狀態收斂成一個 bool）、
/// 且水平距離夠近。純函式、確定性、可測；情誼層級（摯友）由呼叫端另外把關（本函式不重複判 tier）。
pub fn eligible_pair(a_free: bool, b_free: bool, dist_sq: f32) -> bool {
    a_free && b_free && dist_sq <= STROLL_START_DIST * STROLL_START_DIST
}

/// 從一對居民 id 決定誰當 leader（被貼著走的那位）：確定性取 id 字典序較小者當 leader，
/// 另一位當 follower（比照小圈子聚會「host＝排序後 group[0]」的確定性選法，重跑結果一致、可測）。
/// 回傳 `(leader_id, follower_id)`。
pub fn pick_leader<'a>(id_a: &'a str, id_b: &'a str) -> (&'a str, &'a str) {
    if id_a <= id_b {
        (id_a, id_b)
    } else {
        (id_b, id_a)
    }
}

/// leader 起意邀約時冒的一句（確定性輪替，`pick` 由呼叫端給隨機源；點名同行的朋友更有溫度）。
pub fn invite_line(friend: &str, pick: usize) -> String {
    let variants: [&str; 3] = [
        "{friend}，走，陪我在村裡走走～",
        "難得都閒著，{friend}，一起晃晃吧？",
        "{friend}，這天氣真好，我們散散步？",
    ];
    variants[pick % variants.len()].replace("{friend}", friend)
}

/// follower 應邀時冒的一句（確定性輪替；點名邀約的朋友）。
pub fn join_line(friend: &str, pick: usize) -> String {
    let variants: [&str; 3] = [
        "好啊{friend}，一起～",
        "正想找人走走呢，走吧{friend}！",
        "陪{friend}散個步，最好不過了。",
    ];
    variants[pick % variants.len()].replace("{friend}", friend)
}

/// 散步落幕後、雙方各記進心裡的一筆 episodic 記憶（點名同行的朋友）。純文字、確定性、單行防注入。
pub fn stroll_memory_line(friend: &str) -> String {
    format!("和{friend}並肩在村裡散了會兒步，什麼都不為，就是舒服。")
}

/// 全村動態牆的一行（點名兩位摯友；讓不在場的玩家也讀得到今天誰和誰一起散了步）。單行、確定性。
pub fn stroll_feed_line(a: &str, b: &str) -> String {
    format!("{a}和{b}並肩在村裡散步，有一搭沒一搭地聊著，走了好一段路。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_needs_both_free_and_near() {
        let near = (STROLL_START_DIST - 1.0).powi(2);
        assert!(eligible_pair(true, true, near));
        // 任一位在忙 → 不成對。
        assert!(!eligible_pair(false, true, near));
        assert!(!eligible_pair(true, false, near));
        // 都閒但離太遠 → 不硬拉。
        let far = (STROLL_START_DIST + 1.0).powi(2);
        assert!(!eligible_pair(true, true, far));
    }

    #[test]
    fn eligible_exact_boundary_counts_as_near() {
        // 恰好等於門檻距離：視為夠近（<= 邊界含端點，確定性）。
        let exact = STROLL_START_DIST * STROLL_START_DIST;
        assert!(eligible_pair(true, true, exact));
    }

    #[test]
    fn leader_is_deterministic_smaller_id() {
        // id 較小者當 leader，與傳入順序無關（對稱、確定性）。
        assert_eq!(pick_leader("vox_res_0", "vox_res_3"), ("vox_res_0", "vox_res_3"));
        assert_eq!(pick_leader("vox_res_3", "vox_res_0"), ("vox_res_0", "vox_res_3"));
    }

    #[test]
    fn leader_stable_when_equal() {
        // 理論上不會同 id，但邊界仍不 panic、給確定結果。
        assert_eq!(pick_leader("vox_res_1", "vox_res_1"), ("vox_res_1", "vox_res_1"));
    }

    #[test]
    fn invite_and_join_name_the_friend_and_are_bounded() {
        // 台詞點名朋友、越界 pick 安全取模、永不空。
        for pick in [0usize, 1, 2, 3, 7, 100] {
            let inv = invite_line("諾娃", pick);
            assert!(inv.contains("諾娃"));
            assert!(!inv.is_empty());
            let join = join_line("露娜", pick);
            assert!(join.contains("露娜"));
            assert!(!join.is_empty());
        }
    }

    #[test]
    fn invite_variants_rotate() {
        // 不同 pick 至少能取到不只一種台詞（不是永遠同一句）。
        let a = invite_line("諾娃", 0);
        let b = invite_line("諾娃", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn memory_and_feed_name_participants_and_single_line() {
        let mem = stroll_memory_line("賽勒");
        assert!(mem.contains("賽勒"));
        assert!(!mem.contains('\n'));
        assert!(!mem.is_empty());
        let feed = stroll_feed_line("露娜", "奧瑞");
        assert!(feed.contains("露娜") && feed.contains("奧瑞"));
        assert!(!feed.contains('\n'));
    }

    #[test]
    fn feed_does_not_falsely_claim_first_ever() {
        // 措辭不謊稱「史上第一次」——散步是日常小事，不是里程碑。
        let feed = stroll_feed_line("露娜", "奧瑞");
        assert!(!feed.contains("第一次") && !feed.contains("史上"));
    }
}
