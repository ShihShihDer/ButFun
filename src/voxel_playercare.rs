//! 乙太方界·居民關心你挨餓 v1（voxel_playercare）——當你肚子餓得受不了（`is_starving`）時，
//! 恰好在近旁、閒著、醒著的居民會**注意到你**、走過來遞上一份麵包、說句關切的話，並把「那次你
//! 很餓，我給了你一份吃的」記進她心裡。
//!
//! **這一刀補的缺口**：乙太方界至今「居民→玩家」的所有互動——迎客（830）、贈禮回禮（667/728/731）、
//! 招呼閒聊、集會鐘應召——全都是**你先做了什麼**（走近、送禮、敲鐘）居民才回應；居民**主動注意到你
//! 此刻過得好不好**（你餓不餓、累不累），至今完全空白。全庫已有居民互相照顧的鏡像（生病陪伴
//! `voxel_illness`、分食守望相助 800/801），卻沒有一條線是「居民照顧真人玩家」——這是「你的互動有
//! 後果」（PLAN_ETHERVOX 核心信念）第一次反過來：**居民主動關心你的後果**，而非你先給居民才有回應。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **回禮（667/728/731）**＝你先送過禮、好感夠高才回贈，觸發物是「你送過東西」；本刀＝你此刻
//!   **正在挨餓**，觸發物是你的**生存狀態**，與是否送過禮無關（新玩家、從沒送過禮也會被照顧）。
//! - **分食守望相助（800）**＝**居民之間**互相分食；本刀＝**居民照顧玩家**，方向不同、對象不同。
//! - **迎客（830）／集會鐘（796）**＝你先登門/敲鐘，居民才回應；本刀＝居民**主動**察覺你的狀態，
//!   不必你先做任何動作。
//!
//! **純函式層**：確定性、零 LLM、零鎖、零 async、零 IO、可單元測試。連線／鎖／廣播／記憶／Feed／
//! 背包 io 全留在 `voxel_ws.rs`（沿用既有短鎖循序＋鎖外事件佇列慣例，守 prod 死鎖鐵律）。

/// 居民一次關心後的冷卻（秒）：同一位居民對你的關心不會連環轟炸——你若持續挨餓，過了這麼久
/// 才會再被同一位居民注意到一次。比回禮系列更頻繁一些（挨餓是即時狀態、不是稀有一次性事件）。
pub const CARE_COOLDOWN_SECS: f32 = 240.0;

/// 每次符合條件（你在附近挨餓＋冷卻到期）時的觸發機率——不是每一拍都會被注意到，讓「被關心」
/// 保有一絲自然的隨機感，而非機械式必中。
pub const CARE_CHANCE: f32 = 0.3;

/// 「你在附近」的判定半徑（世界方塊）——比照回禮（`RETURN_GIFT_REACH`）同量級，近到能遞東西給你。
pub const CARE_REACH: f32 = 6.0;

/// 關心你時遞出的麵包份數。
pub const CARE_GIFT_QTY: u32 = 1;

/// 關心泡泡台詞最多顯示字數（截斷防超長玩家名撐破泡泡框）。
pub const SAY_CHARS: usize = 50;

/// 動態牆事件種類標籤。
pub const FEED_KIND: &str = "居民關心";

/// 各居民初始冷卻的錯開偏移（秒）：避免伺服器剛啟動、你剛好挨餓時一群居民同時衝過來關心。
/// 依居民序 `i` 遞增，比照 `vrain::shelter_cd_offset` 慣例。
pub fn care_cd_offset(i: usize) -> f32 {
    40.0 + i as f32 * 20.0
}

/// 三閘判定：你正在挨餓（`starving`）＋冷卻到期（`cooldown <= 0`）＋過機率門檻
/// （`roll < chance`）→ 這一 tick 被這位居民注意到並上前關心。純函式，好窮舉測邊界。
pub fn should_notice_hunger(starving: bool, cooldown: f32, roll: f32, chance: f32) -> bool {
    starving && cooldown <= 0.0 && roll < chance
}

/// 關心泡泡台詞（點名玩家）——四句輪替，玩家名截斷不破泡泡框。`pick` 由呼叫端用座標 bits
/// 合成，讓每次挑到的句子自然分散。
pub fn care_bubble_with_player(player: &str, pick: usize) -> String {
    let name = clip_name(player);
    const TEMPLATES: [&str; 4] = [
        "{name}，你的臉色不太好，肚子餓了吧？先吃點這個。",
        "{name}，別硬撐著，這份麵包給你墊墊肚子。",
        "看你這麼虛弱，{name}，快吃點東西吧。",
        "{name}，餓著肚子可不行，拿去吃吧。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{name}", &name)
}

/// 昇華成一筆「你那次很餓，我給了你一份麵包」的記憶（點名玩家、去換行，走既有 append-only 記憶管線）。
pub fn care_memory_line(player: &str) -> String {
    format!(
        "{}那次餓得臉色發白，我遞了一份麵包給ta。",
        clip_name(player)
    )
    .replace('\n', " ")
}

/// 動態牆播報（非同步層，訪客回來能讀到誰關心了誰）。
pub fn care_feed_line(rname: &str, pname: &str) -> String {
    format!("{rname}見{pname}餓著肚子，遞了一份麵包過去。")
}

/// 玩家名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_notice_hunger_needs_all_three_gates() {
        assert!(should_notice_hunger(true, 0.0, 0.1, CARE_CHANCE));
        // 沒挨餓 → 否。
        assert!(!should_notice_hunger(false, 0.0, 0.1, CARE_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_notice_hunger(true, 5.0, 0.1, CARE_CHANCE));
        // 骰過門檻 → 否；邊界 roll == chance 不觸發（嚴格小於）。
        assert!(!should_notice_hunger(true, 0.0, CARE_CHANCE, CARE_CHANCE));
        assert!(!should_notice_hunger(true, 0.0, 0.99, CARE_CHANCE));
    }

    #[test]
    fn cd_offset_staggers_by_index_and_is_positive() {
        assert!(care_cd_offset(0) > 0.0);
        assert!(care_cd_offset(1) > care_cd_offset(0));
        assert!(care_cd_offset(3) > care_cd_offset(2));
    }

    #[test]
    fn bubble_embeds_name_rotates_and_clips() {
        let s = care_bubble_with_player("旅人", 0);
        assert!(s.contains("旅人"));
        assert_ne!(care_bubble_with_player("旅人", 0), care_bubble_with_player("旅人", 1));
        let long = care_bubble_with_player("超級無敵長長長長長長長名字", 2);
        assert!(long.chars().count() < 60, "超長名應被截斷不破泡泡框：{long}");
    }

    #[test]
    fn memory_and_feed_embed_names_no_newline() {
        let m = care_memory_line("諾娃\n注入");
        assert!(m.contains("諾娃"));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
        // 空名安全不 panic、仍成句。
        assert!(!care_memory_line("").is_empty());
        let f = care_feed_line("露娜", "旅人");
        assert!(f.contains("露娜") && f.contains("旅人"));
    }

    #[test]
    fn constants_are_sane() {
        assert!(CARE_CHANCE > 0.0 && CARE_CHANCE < 1.0);
        assert!(CARE_COOLDOWN_SECS > 0.0);
        assert!(CARE_REACH > 0.0);
        assert!(CARE_GIFT_QTY > 0);
        assert!(!FEED_KIND.is_empty());
    }
}
