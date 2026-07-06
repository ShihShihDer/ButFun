//! 乙太方界·居民認得「你」的家、會登門拜訪你 v1（自主提案切片，ROADMAP 830）。
//!
//! 749~763 讓居民之間長出一整條完整的鄰里網——立牌署名（749）→認出鄰居的家（750）→
//! 登門串門子（751）→在家迎客（752）→撲空留心意（763）：居民彼此會登門找對方、會在意
//! 有沒有碰上本人、撲空也留得住心意。但這整條鏈子從頭到尾都只發生在**居民彼此之間**，
//! 玩家從來不是這張鄰里網的一份子——你能去找居民，居民卻從沒有「專程去找你」這回事
//! （746 晨間思念是居民想著你、追著你**當下的位置**跑，跟這裡「記得你家在哪、有空繞去看看」
//! 是兩回事：一個追人、一個認地方）。
//!
//! 本切片把玩家也接進這張網：你在自家門前立一塊「{你的名字}的家」告示牌（740，比照居民
//! 自己的署名慣例），路過讀到的居民第一次認出「這是你的家」；日後牠朝聖重返這塊牌子時（743），
//! 不再是對牌自言自語，而是一趟真正的「登門拜訪你」——你在家就走上前暖暖打招呼、記進與你的
//! 交情；你不在，牠會在城鎮動態留一句「今天繞去找過你」，讓你回來就讀得到。
//!
//! **owner 是伺服器權威判定、不是猜牌面文字**：`voxel_sign.rs` 的 `SignEntry.owner` 在你
//! 送出 `SignSet` 那刻，由伺服器記下你已登入的帳號顯示名（訪客不記名，訪客的牌永遠 `owner=None`、
//! 行為與今日完全一致）——玩家的家不必靠居民「猜對牌面上的名字」，立牌那刻就確立了歸屬。
//! 只有牌面語氣被既有 [`crate::voxel_readsign::classify`] 判成 [`crate::voxel_readsign::SignTone::Home`]
//! （含「家/屋/窩/居/巢」等字）的牌才會被認成「家」——你隨手寫的路標／留言不會被誤當成家。
//!
//! **與既有元素的分界（換維度，非同軸重複）**：
//! - 746（晨間思念）：記憶讓居民追著**此刻在線的你**跑，隨你移動而移動；本刀讓居民記得
//!   **一個地點**，不論你在不在線、在不在附近，牌子的位置永遠在那裡。
//! - 751/752/763（居民↔居民鄰里網）：目標永遠是另一位**居民**；本刀把同一套「認家→登門→
//!   碰面或撲空」的機制，第一次伸向**玩家**——你不再只是這張網的旁觀者。
//!
//! **成本紀律**：零 LLM（純規則判定＋確定性選句）、零新協議欄位、零新美術、零前端改動
//! （沿用既有 say 泡泡／記憶／Feed 管線）；`cherished_player`/`pilgrimage_player` 純記憶體、
//! 重啟歸零（比照 743/751 的既有慣例，牌子本身仍由 `voxel_sign` 持久化，重啟後居民只是要
//! 重新路過讀一次才會再認得，優雅退化不影響玩家資料）。
//!
//! 純邏輯層：零 async、零鎖、零 IO；鎖 / 距離掃描 / 記憶 / Feed 全在 `voxel_ws.rs`。

/// 你「在家」的半徑（世界座標，方塊）：站在自家牌子這半徑內，居民登門時才算真的碰上你。
/// 沿用 [`crate::voxel_hosted_visit::HOST_HOME_DIST`] 同一個距離慣例（鄰里往來一致的「在家」定義）。
pub const PLAYER_HOME_DIST: f32 = 5.0;

/// 「認出這是你的家」記憶前綴：日記／回想端可據此把這筆歸為「認得你的家」主題。
pub const HOME_RECOGNIZED_TAG: &str = "🏠認得你的家";
/// 「登門拜訪你、碰上本人」記憶前綴。
pub const HOME_VISIT_TAG: &str = "🏠登門拜訪你";
/// 「登門拜訪你、撲了個空」記憶前綴。
pub const HOME_VISIT_MISS_TAG: &str = "🏠撲空拜訪你";

/// 你此刻是否「在家」（站在自家牌子附近）。任一座標非有限值（壞資料）時保守回 `false`——
/// 寧可當成你不在、走撲空路徑，也不誤判碰面。
pub fn player_is_home(player_x: f32, player_z: f32, plate_x: f32, plate_z: f32) -> bool {
    if !(player_x.is_finite() && player_z.is_finite() && plate_x.is_finite() && plate_z.is_finite())
    {
        return false;
    }
    let dx = player_x - plate_x;
    let dz = player_z - plate_z;
    dx * dx + dz * dz <= PLAYER_HOME_DIST * PLAYER_HOME_DIST
}

/// 居民第一次認出「這是你的家」時的招呼泡泡（比通用讀牌語更親暱、點名你）。
/// `pick` 取居民座標／時機雜湊，讓不同居民／不同時機說不同句（確定性、零 LLM）。
pub fn recognized_line(player: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "哦——原來{p}住在這兒呀，記下來了。",
        "這是{p}的家吧？我記住這個地方了。",
        "路過{p}立的牌子，原來你住這一帶呢。",
    ];
    let s = TEMPLATES[pick % TEMPLATES.len()].replace("{p}", player);
    s.chars().take(40).collect()
}

/// 掛在「你」名下的記憶：居民記住了你的家在哪。
pub fn recognized_memory(player: &str) -> String {
    format!("{HOME_RECOGNIZED_TAG}：路過{player}立的牌子，記住了{player}的家在哪一帶。")
}

/// 城鎮動態 Feed 一行：某居民認出了你的家。
pub fn recognized_feed(reader: &str, player: &str) -> String {
    format!("{reader}路過{player}立的牌子，記住了{player}住在哪。")
}

/// 居民登門抵達你家、你正好在（附近）時冒的雀躍暖句。
pub fn visit_present_line(player: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "{p}你在家呀！特地繞來看看你，太好了。",
        "可算碰上{p}本人了，來串個門子！",
        "哎呀{p}你在呀，難得順路來坐坐～",
    ];
    let s = TEMPLATES[pick % TEMPLATES.len()].replace("{p}", player);
    s.chars().take(40).collect()
}

/// 掛在「你」名下的記憶：居民特地登門找你，正好碰上本人。
pub fn visit_present_memory(player: &str) -> String {
    format!("{HOME_VISIT_TAG}：特地繞到{player}家門口，正好碰上本人，聊得暖暖的。")
}

/// 城鎮動態 Feed 一行：某居民登門找你，正好碰上你。
pub fn visit_present_feed(reader: &str, player: &str) -> String {
    format!("{reader}特地繞到{player}家門口，正好碰上{player}本人。")
}

/// 居民登門抵達你家、你不在（離線或不在附近）時撲空的獨白。
pub fn visit_missed_line(player: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "{p}不在家呀…那我下次再來看看。",
        "撲了個空呢，{p}出門啦？就當我來看過你了。",
        "唔，{p}不在，沒關係，下回換個時間再繞來。",
    ];
    let s = TEMPLATES[pick % TEMPLATES.len()].replace("{p}", player);
    s.chars().take(40).collect()
}

/// 掛在「你」名下的記憶：居民特地登門找你，卻撲了個空。
pub fn visit_missed_memory(player: &str) -> String {
    format!("{HOME_VISIT_MISS_TAG}：特地繞到{player}家門口找人，可惜{player}不在，撲了個空。")
}

/// 城鎮動態 Feed 一行：某居民登門找你，卻撲了個空。
pub fn visit_missed_feed(reader: &str, player: &str) -> String {
    format!("{reader}特地繞到{player}家門口找人，可惜撲了個空。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_home_within_and_outside_radius() {
        assert!(player_is_home(10.0, 10.0, 10.0, 10.0));
        // 邊界（3,4→距離5）含邊界算在家。
        assert!(player_is_home(13.0, 14.0, 10.0, 10.0));
        assert!(!player_is_home(20.0, 20.0, 10.0, 10.0));
    }

    #[test]
    fn player_home_rejects_bad_coords() {
        assert!(!player_is_home(f32::NAN, 0.0, 0.0, 0.0));
        assert!(!player_is_home(0.0, 0.0, f32::INFINITY, 0.0));
    }

    #[test]
    fn recognized_line_names_player_and_fits_bubble() {
        for pick in 0..6usize {
            let s = recognized_line("小明", pick);
            assert!(s.contains("小明"), "泡泡應點名玩家: {s}");
            assert!(s.chars().count() <= 40, "泡泡應 ≤40 字元: {s}");
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn recognized_line_varies_with_pick() {
        assert_ne!(recognized_line("露娜", 0), recognized_line("露娜", 1));
    }

    #[test]
    fn recognized_memory_carries_tag_and_player() {
        let m = recognized_memory("小明");
        assert!(m.starts_with(HOME_RECOGNIZED_TAG), "應以認得你的家前綴開頭: {m}");
        assert!(m.contains("小明"), "記憶應點名玩家: {m}");
    }

    #[test]
    fn recognized_feed_names_both() {
        let f = recognized_feed("露娜", "小明");
        assert!(f.contains("露娜") && f.contains("小明"));
    }

    #[test]
    fn visit_present_line_names_player_and_fits_bubble() {
        for pick in 0..6usize {
            let s = visit_present_line("阿宅", pick);
            assert!(s.contains("阿宅"), "泡泡應點名玩家: {s}");
            assert!(s.chars().count() <= 40, "泡泡應 ≤40 字元: {s}");
        }
    }

    #[test]
    fn visit_present_line_varies_with_pick() {
        assert_ne!(visit_present_line("諾娃", 0), visit_present_line("諾娃", 1));
    }

    #[test]
    fn visit_present_memory_carries_tag_and_player() {
        let m = visit_present_memory("阿宅");
        assert!(m.starts_with(HOME_VISIT_TAG), "應以登門拜訪你前綴開頭: {m}");
        assert!(m.contains("阿宅"));
    }

    #[test]
    fn visit_present_feed_names_both() {
        let f = visit_present_feed("賽勒", "阿宅");
        assert!(f.contains("賽勒") && f.contains("阿宅"));
    }

    #[test]
    fn visit_missed_line_names_player_and_fits_bubble() {
        for pick in 0..6usize {
            let s = visit_missed_line("阿宅", pick);
            assert!(s.contains("阿宅"), "泡泡應點名玩家: {s}");
            assert!(s.chars().count() <= 40, "泡泡應 ≤40 字元: {s}");
        }
    }

    #[test]
    fn visit_missed_line_varies_with_pick() {
        assert_ne!(visit_missed_line("奧瑞", 0), visit_missed_line("奧瑞", 1));
    }

    #[test]
    fn visit_missed_memory_carries_tag_and_player_and_differs_from_present() {
        let m = visit_missed_memory("阿宅");
        assert!(m.starts_with(HOME_VISIT_MISS_TAG), "應以撲空拜訪你前綴開頭: {m}");
        assert!(m.contains("阿宅"));
        assert_ne!(m, visit_present_memory("阿宅"), "碰面與撲空的記憶應不同");
    }

    #[test]
    fn visit_missed_feed_names_both() {
        let f = visit_missed_feed("露娜", "阿宅");
        assert!(f.contains("露娜") && f.contains("阿宅"));
    }
}
