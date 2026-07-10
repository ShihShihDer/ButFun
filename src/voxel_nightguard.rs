//! 乙太方界·守夜恩人 v1（voxel_nightguard）——當你揮劍（或徒手）為近旁受威脅的居民**驅散一團
//! 逼近的暗影**時，那位差點被嚇著的居民會注意到是**你**替她解了圍，冒一句道謝、心情亮一格，並把
//! 「那夜你為我驅散了暗影」記進她心裡。
//!
//! **這一刀補的缺口**：驅影之劍（887）讓戰鬥第一次成形，但至今驅散暗影只換來那一枚掉落的乙太礦——
//! 純資源回饋，**沒有任何社交後果**。居民雖有整套「被暗影嚇到」的害怕反應（`voxel_shadow` 的
//! `frightened_by`／`FEAR_LINES`，居民會冒害怕泡泡、加速奔回家），卻**從不知道是誰替她驅走了威脅**。
//! 本刀把這條線閉上：**你的守護第一次被居民看見、記住、道謝**——戰鬥（人對怪）第一次長出社交後果
//! （怪→人→居民），正是 PLAN_ETHERVOX 核心信念「你的互動有後果」在戰鬥軸的首次落地。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **居民關心你挨餓（845）**＝觸發物是**你的生存狀態**（你餓不餓），居民主動遞麵包；本刀＝觸發物是
//!   **你剛替她做的一個動作**（你當場驅散了逼近她的暗影），是「你先做了什麼→居民回應」的方向。
//! - **回禮（667/728/731）**＝你**先送禮**、好感夠才回贈；本刀不涉贈禮，觸發物是**戰鬥行為**、無關背包。
//! - **打氣／讚賞（679/773）**＝居民欣賞你的**建造成果**（放了方塊、蓋了東西）；本刀是居民感激你的
//!   **保護行為**（替她擋掉一個當下的威脅），對象與情境全然不同。
//! - **居民害怕暗影（`voxel_shadow` FEAR）**＝居民**單方面**怕、奔回家，玩家不在迴圈裡；本刀把玩家
//!   接進來——**你驅散了那團暗影**，害怕的居民才有得道謝。
//!
//! **純函式層**：確定性、零 LLM、零鎖、零 async、零 IO、可單元測試。連線／鎖／廣播／記憶／Feed 全留
//! 在 `voxel_ws.rs`（沿用既有短鎖循序＋鎖外處理慣例，守 prod 死鎖鐵律）。

/// 「你替她驅散了那團暗影」的判定半徑（世界方塊）：暗影散落時，這格內、醒著的居民視為**當時正被
/// 這團暗影威脅**（對齊 `voxel_shadow::FEAR_RADIUS = 9`——會怕的距離，正是會感激的距離）。
pub const RESCUE_RADIUS: f32 = 9.0;

/// 同一位居民對你的道謝冷卻（秒）：整夜你可能替她驅散好幾團暗影，但她不會每一團都上前道謝——
/// 過了這麼久才會再感激一次，保住「被守護」那份稀有的暖意，不淪為機械式洗版。
pub const GRATITUDE_COOLDOWN_SECS: f32 = 180.0;

/// 每次符合條件（近旁醒著的居民＋冷卻到期）時的道謝機率——不是每一次驅散都必然換來道謝，讓「被
/// 感激」保有一絲自然的隨機感（比照 `voxel_playercare::CARE_CHANCE` 的態度，但這是英勇一拍、稍高）。
pub const THANK_CHANCE: f32 = 0.7;

/// 道謝泡泡台詞最多顯示字數（截斷防超長玩家名撐破泡泡框，比照 `voxel_playercare::SAY_CHARS`）。
pub const SAY_CHARS: usize = 50;

/// 動態牆事件種類標籤。
pub const FEED_KIND: &str = "守夜恩情";

/// 兩點的水平距離平方（省一次開根號，呼叫端拿去和 `RESCUE_RADIUS²` 比）。
pub fn horiz_dist_sq(ax: f32, az: f32, bx: f32, bz: f32) -> f32 {
    let dx = ax - bx;
    let dz = az - bz;
    dx * dx + dz * dz
}

/// 這位居民是否在「被你救到」的範圍內（傳入水平距離平方，和 `RESCUE_RADIUS²` 比）。
pub fn within_rescue(dist_sq: f32) -> bool {
    dist_sq <= RESCUE_RADIUS * RESCUE_RADIUS
}

/// 三閘判定：居民在你驅散暗影的近旁（`within_rescue`）＋道謝冷卻到期（`cd_ok`）＋過機率門檻
/// （`roll < chance`）→ 這一次驅散換來這位居民的道謝。純函式，好窮舉測邊界。
pub fn should_thank(dist_sq: f32, cd_ok: bool, roll: f32, chance: f32) -> bool {
    within_rescue(dist_sq) && cd_ok && roll < chance
}

/// 道謝泡泡台詞（點名玩家）——四句輪替，玩家名截斷不破泡泡框。`pick` 由呼叫端用座標 bits 合成，
/// 讓每次挑到的句子自然分散。
pub fn thanks_bubble(player: &str, pick: usize) -> String {
    let name = clip_name(player);
    const TEMPLATES: [&str; 4] = [
        "{name}，多虧你把那團暗影趕走了…我又能安心了。",
        "剛才好險，謝謝你，{name}，你替我擋下了那片黑影。",
        "{name}，你來得正是時候，那暗影快貼上我了。",
        "有你在真好，{name}——這份守護我記著了。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 昇華成一筆「那夜你為我驅散了逼近的暗影」的記憶（點名玩家、去換行防注入，走既有 append-only
/// 記憶管線）。
pub fn guard_memory_line(player: &str) -> String {
    format!(
        "那夜暗影逼到我跟前，是{}揮手替我驅散了它，這份守護我記在心裡。",
        clip_name(player)
    )
    .replace('\n', " ")
}

/// 動態牆播報（訪客回來能讀到誰守護了誰）。去換行防注入。
pub fn guard_feed_line(rname: &str, pname: &str) -> String {
    format!("{rname}被暗影逼近，{pname}揮劍替ta驅散了危機。").replace('\n', " ")
}

/// 玩家名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_rescue_boundary() {
        // 圓內／圓上／圓外三態（半徑 9 → 半徑平方 81）。
        assert!(within_rescue(0.0));
        assert!(within_rescue(RESCUE_RADIUS * RESCUE_RADIUS)); // 邊界含（<=）
        assert!(!within_rescue(RESCUE_RADIUS * RESCUE_RADIUS + 0.01));
        // horiz_dist_sq 幾何自洽：對角 (3,4) → 25。
        assert_eq!(horiz_dist_sq(0.0, 0.0, 3.0, 4.0), 25.0);
        assert!(within_rescue(25.0));
    }

    #[test]
    fn should_thank_needs_all_three_gates() {
        let near = 4.0; // 在範圍內
        let far = 100.0; // 超出 81
        assert!(should_thank(near, true, 0.1, THANK_CHANCE));
        // 太遠 → 否。
        assert!(!should_thank(far, true, 0.1, THANK_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_thank(near, false, 0.1, THANK_CHANCE));
        // 骰過門檻 → 否；邊界 roll == chance 不觸發（嚴格小於）。
        assert!(!should_thank(near, true, THANK_CHANCE, THANK_CHANCE));
        assert!(!should_thank(near, true, 0.99, THANK_CHANCE));
    }

    #[test]
    fn bubble_embeds_name_rotates_and_clips() {
        let s = thanks_bubble("旅人", 0);
        assert!(s.contains("旅人"));
        // 四句輪替，相鄰 pick 不同句。
        assert_ne!(thanks_bubble("旅人", 0), thanks_bubble("旅人", 1));
        assert_ne!(thanks_bubble("旅人", 1), thanks_bubble("旅人", 2));
        // 超長名截斷不破泡泡框。
        let long = thanks_bubble("超級無敵長長長長長長長名字", 3);
        assert!(long.chars().count() < 60, "超長名應被截斷：{long}");
    }

    #[test]
    fn memory_and_feed_embed_names_no_newline() {
        // 記憶：嵌名、去換行防注入、空名不 panic 仍成句。
        let m = guard_memory_line("諾娃\n注入");
        assert!(m.contains("諾娃"));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
        assert!(!guard_memory_line("").is_empty());
        // Feed：嵌雙名、去換行。
        let f = guard_feed_line("露娜", "旅人\n洗版");
        assert!(f.contains("露娜") && f.contains("旅人"));
        assert!(!f.contains('\n'), "Feed 不得含換行：{f}");
    }

    #[test]
    fn constants_are_sane() {
        assert!(THANK_CHANCE > 0.0 && THANK_CHANCE < 1.0);
        assert!(GRATITUDE_COOLDOWN_SECS > 0.0);
        assert!(RESCUE_RADIUS > 0.0);
        assert!(SAY_CHARS > 0);
        assert!(!FEED_KIND.is_empty());
    }
}
