//! 乙太方界·居民顧家駐足 v1（voxel_homegaze）——白天，閒著、醒著、恰好走在自家附近的居民，
//! 偶爾**停下腳步、望著自己一手安頓下來的家，湧起一股踏實的歸屬感**：說句安穩的話、心情因
//! 這份「有個家」的踏實而亮一格；你也在近旁時，牠會把「這個能安身的家、有你這樣的旅人相伴」
//! 記進交情、日後浮進日記。
//!
//! **這一刀補的缺口**：居民和自己的「家」之間，至今只有兩種關係——**夜裡回家睡覺**（功能性的
//! 就寢）、或**把渴望蓋成家**（652 蓋造）。可一個真的活著的人，對自己安身的地方會有**情感**：
//! 白天路過自家門前，會不自覺放慢腳步、望一眼那片屬於自己的天地，心裡踏實。這是世界第一次讓
//! 居民對**一個地點**（自己的家域）生出**歸屬感**——記憶/身世驅動行為（PLAN_ETHERVOX 核心信念
//! 「記憶要驅動行為」與 §3「你的家是你安身之處」），而非又一個與地點無關的原地小動作。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **夜歸就寢（sleep/bedtime）**＝**夜裡**走回家**睡覺**（功能性、閉眼）；本刀＝**白天**在家門前
//!   **駐足端詳**（睜眼、有情感的凝望），時段相反、動作相反。
//! - **自我印象（770 self_image）**＝把記憶昇華成「我是個什麼樣的人」的**抽象自我概念**（無地點、
//!   不駐足）；本刀＝對**一個具體地點（自家）**的**當下情感**，且真的**停下腳步**（設 wait_timer）。
//! - **臨水垂釣（814）／長椅歇腳（810）**＝被**水／椅**這類**外物**吸引的悠閒駐足；本刀＝被**自己的家**
//!   這份**歸屬**牽動的駐足——吸引物是「屬於自己的地方」，情感內核（歸屬 vs 閒情）不同。
//!
//! **純函式層**：確定性、零 LLM、零鎖、零 async、零 IO、可單元測試。連線／鎖／廣播／記憶／Feed
//! 觸發全留在 `voxel_ws.rs`（沿用既有短鎖循序＋鎖外事件佇列慣例，守 prod 死鎖鐵律）。

/// 顧家駐足冷卻（秒）：一次駐足後隔這麼久才會再駐足，防同一居民整天賴在家門口狂刷歸屬泡泡。
/// 比避雨（150）／垂釣同量級偏長——顧家是難得的一份靜心，不是每 tick 都停。
pub const GAZE_COOLDOWN_SECS: f32 = 200.0;

/// 每次符合條件（在家附近＋白天＋冷卻到期）時的駐足觸發機率——其餘時候只是照常走過自家。
/// 略低於長椅歇腳（0.28）：不是每次路過都會停下望家，讓「顧家駐足」稀疏而自然。
pub const GAZE_CHANCE: f32 = 0.2;

/// 判定「在自家附近」的半徑（世界方塊）：離家域中心這麼近才算走到了自家門前。
/// 刻意小於居民歸巢半徑（HOME_RADIUS），確保是真的「在家門口」而非只是家域外緣路過。
pub const HOME_RADIUS: f32 = 6.0;

/// 顧家駐足時原地停留的秒數（設進 `wait_timer`）：居民真的停下腳步、望著自家凝望一會兒。
pub const GAZE_PAUSE_SECS: f32 = 5.0;

/// 「你也在近旁」的判定半徑（世界方塊）——你在這麼近，居民的顧家話就會點你名、把家的踏實記進交情。
pub const GAZE_PLAYER_RADIUS: f32 = 5.0;

/// 顧家泡泡台詞最多顯示字數（截斷防超長玩家名撐破泡泡框）。
pub const SAY_CHARS: usize = 50;

/// 動態牆事件種類標籤（顧家駐足）。
pub const FEED_KIND: &str = "顧家駐足";

/// 各居民首次顧家冷卻的錯開偏移（秒）：避免一到白天同一 tick 一群人齊聲說顧家話。
/// 依居民序 `i` 遞增，比照 `vrain::shelter_cd_offset` 慣例。
pub fn gaze_cd_offset(i: usize) -> f32 {
    90.0 + i as f32 * 30.0
}

/// 是否「在自家附近」：居民當前座標與家域中心的水平距離在 [`HOME_RADIUS`] 內。
/// 純函式（平方比較免開根號），好窮舉測邊界。
pub fn near_home(rx: f32, rz: f32, home_x: f32, home_z: f32) -> bool {
    let dx = rx - home_x;
    let dz = rz - home_z;
    dx * dx + dz * dz <= HOME_RADIUS * HOME_RADIUS
}

/// 三閘判定：在家附近（`near`）＋冷卻到期（`cooldown <= 0`）＋過機率門檻（`roll < chance`）
/// → 這一 tick 停下顧家駐足。純函式，好窮舉測邊界。
pub fn should_gaze(near: bool, cooldown: f32, roll: f32, chance: f32) -> bool {
    near && cooldown <= 0.0 && roll < chance
}

/// 顧家泡泡台詞（通用、不點名）——五句輪替，字數短不破泡泡框。`pick` 由呼叫端用座標 bits
/// 合成，讓每次挑到的句子自然分散。
pub fn gaze_bubble(pick: usize) -> &'static str {
    const LINES: [&str; 5] = [
        "這片小天地，是我親手安頓下來的家。",
        "有個能回的地方，心裡真踏實。",
        "站在自家門前，怎麼看都看不膩。",
        "一磚一瓦都是我的，這裡就是家了。",
        "走過千百回，還是最愛自家這一角。",
    ];
    LINES[pick % LINES.len()]
}

/// 你也在近旁時的顧家泡泡（點名玩家、把家的踏實與你分享，更親近）——四句輪替，玩家名截斷不破泡泡框。
pub fn gaze_bubble_with_player(player: &str, pick: usize) -> String {
    let name = clip_name(player);
    const TEMPLATES: [&str; 4] = [
        "{name}，你看，這就是我安下來的家。",
        "有{name}來看看我的家，心裡更暖了。",
        "{name}，這片天地能安頓，也多虧有你們常來。",
        "來{name}，陪我在自家門前站一會兒。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 昇華成一筆「在自家門前和你一起感受歸屬」的記憶（點名玩家、不含換行，走既有 append-only 記憶管線）。
pub fn gaze_memory_line(player: &str) -> String {
    format!("站在自家門前，和{}一起望著這片我安頓下來的家，心裡格外踏實。", clip_name(player))
        .replace('\n', " ")
}

/// 動態牆播報（非同步層，訪客回來能讀到誰在自家門前駐足過）。
pub fn gaze_feed_line(rname: &str) -> String {
    format!("{rname}在自家門前停下腳步，望著這片安頓下來的家出了會兒神。")
}

/// 玩家名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_home_respects_radius() {
        // 正中家域中心 → 在家。
        assert!(near_home(10.0, 10.0, 10.0, 10.0));
        // 半徑內（3,4 直角三角形斜邊 5 < 6）→ 在家。
        assert!(near_home(13.0, 14.0, 10.0, 10.0));
        // 恰在半徑上（距離 = HOME_RADIUS）→ 仍算在家（<=）。
        assert!(near_home(10.0 + HOME_RADIUS, 10.0, 10.0, 10.0));
        // 半徑外 → 不在家。
        assert!(!near_home(10.0 + HOME_RADIUS + 0.1, 10.0, 10.0, 10.0));
    }

    #[test]
    fn should_gaze_needs_all_three_gates() {
        // 三閘齊備才觸發。
        assert!(should_gaze(true, 0.0, 0.1, GAZE_CHANCE));
        // 不在家附近 → 否（在外頭閒晃不會顧家）。
        assert!(!should_gaze(false, 0.0, 0.1, GAZE_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_gaze(true, 5.0, 0.1, GAZE_CHANCE));
        // 骰過門檻 → 否；邊界 roll == chance 不觸發（嚴格小於）。
        assert!(!should_gaze(true, 0.0, GAZE_CHANCE, GAZE_CHANCE));
        assert!(!should_gaze(true, 0.0, 0.99, GAZE_CHANCE));
    }

    #[test]
    fn cd_offset_staggers_by_index_and_is_positive() {
        // 各居民錯開、遞增、皆為正（避免一到白天同一 tick 齊駐足）。
        assert!(gaze_cd_offset(0) > 0.0);
        assert!(gaze_cd_offset(1) > gaze_cd_offset(0));
        assert!(gaze_cd_offset(3) > gaze_cd_offset(2));
    }

    #[test]
    fn bubbles_rotate_and_stay_in_frame() {
        // 通用顧家語輪替、非空。
        for p in 0..10 {
            assert!(!gaze_bubble(p).is_empty());
        }
        assert_ne!(gaze_bubble(0), gaze_bubble(1));
        // pick 溢出取模不 panic、仍回合法句。
        assert!(!gaze_bubble(usize::MAX).is_empty());
    }

    #[test]
    fn player_bubble_embeds_name_rotates_and_clips() {
        // 點名版含玩家名、輪替、超長名截斷不破框。
        let s = gaze_bubble_with_player("旅人", 0);
        assert!(s.contains("旅人"));
        assert_ne!(
            gaze_bubble_with_player("旅人", 0),
            gaze_bubble_with_player("旅人", 1)
        );
        let long = gaze_bubble_with_player("超級無敵長長長長長長長名字", 2);
        assert!(long.chars().count() < 40, "超長名應被截斷不破泡泡框：{long}");
        // pick 溢出取模不 panic。
        assert!(!gaze_bubble_with_player("旅人", usize::MAX).is_empty());
    }

    #[test]
    fn memory_and_feed_embed_names_no_newline() {
        // 記憶點名、去換行（防注入撐破 jsonl 行）。
        let m = gaze_memory_line("諾娃\n注入");
        assert!(m.contains("諾娃"));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
        // 空名安全不 panic、仍成句。
        assert!(!gaze_memory_line("").is_empty());
        let f = gaze_feed_line("露娜");
        assert!(f.contains("露娜"));
    }

    #[test]
    fn constants_are_sane() {
        // 機率在 (0,1) 開區間、冷卻/停留/半徑為正。
        assert!(GAZE_CHANCE > 0.0 && GAZE_CHANCE < 1.0);
        assert!(GAZE_COOLDOWN_SECS > 0.0);
        assert!(GAZE_PAUSE_SECS > 0.0);
        assert!(HOME_RADIUS > 0.0);
        assert!(GAZE_PLAYER_RADIUS > 0.0);
        assert!(!FEED_KIND.is_empty());
    }
}
