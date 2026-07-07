//! 乙太方界·居民誕辰紀念 v1（voxel_birthday）——世代傳承（人口成長 v1）誕生的居民，
//! 每滿一個「乙太年」（[`YEAR_SECS`]，與四季輪替(798)的一輪春夏秋冬同長）就迎來一次
//! **誕辰紀念**：說一句回望自己來到這片天地已經多久的話、若記得是誰生下自己便謝過父母，
//! 心情因這份回望變好；你也在近旁時，她會特地點名和你分享這一刻、把「陪我一起過生日」記進交情。
//!
//! **這一刀補的缺口**：人口成長 v1（世代傳承）讓新居民誕生時記下 `birth_unix`（生日），
//! 但那個時間戳從誕生那一刻起就**再也沒被讀過**——沒有居民記得自己的生日、沒有人在乎時間
//! 流逝了多久。這是世界第一次讓「時間」本身（不是天氣、不是季節、而是**居民自己的年歲**）
//! 成為驅動行為的記憶——正中 `PLAN_ETHERVOX.md` 核心信念「記憶要驅動行為」與 §5
//! 「日記／生命故事」：生日是每個人生命故事裡最私人的一個刻度。
//!
//! **範圍（v1，誠實的取捨）**：只有經世代傳承誕生的居民（有記錄在案的 `birth_unix`）才會
//! 過生日；初始四位居民（露娜/諾娃/賽勒/奧瑞）是世界開始時就存在、沒有「誕生時刻」可言，
//! 本刀不勉強為她們捏造一個生日——這份空白留給日後若要擴充再處理，不塞不自然的資料。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **人口成長 v1（誕生當下）**＝新居民**誕生那一刻**的一次性慶祝（父母技能繼承／歡迎泡泡）；
//!   本刀＝**日後每年**周期性回望「我來到這裡多久了」，時間點與內容皆不同（一次性 vs 週期性）。
//! - **自我印象（770 self_image）**＝從記憶昇華出「我是個什麼樣的人」的抽象人格概念（無時間刻度）；
//!   本刀＝對著一個**具體的時間刻度（滿幾年）**回望，且點名感謝**父母**（家族線，非自我概念線）。
//! - **顧家駐足（816 homegaze）**＝對**地點**（自家）生出的歸屬感；本刀＝對**時間**（生日）生出的感懷——
//!   地點軸 vs 時間軸，吸引物完全不同。
//!
//! **純函式層**：確定性、零 LLM、零鎖、零 async、零 IO、可單元測試。連線／鎖／廣播／記憶／Feed
//! 觸發全留在 `voxel_ws.rs`（沿用既有短鎖循序＋鎖外事件佇列慣例，守 prod 死鎖鐵律）。
//!
//! **v1.1（ROADMAP 872，自主提案切片）——分你一份心意**：v1 上線後，誕辰紀念一直只有一句話，
//! 玩家在場時除了被點名、什麼都帶不走；而 667/728「回禮」早就示範過「居民親手採到的東西可以
//! 分給玩家」，兩者從沒接上。本刀讓玩家在場的誕辰紀念也順手從她的採集背包（`res_inv`，
//! `voxel_return_gift::pick_from_stock`）分你一份，量刻意壓到象徵性的 [`BIRTHDAY_GIFT_QTY`]
//! （見下方 razor-sharp 區隔）。

/// 一個「乙太年」的秒數：與四季輪替(798) `Season` 一輪四季同長——
/// 4 季 × [`crate::voxel_season::DAYS_PER_SEASON`]（2 遊戲日）× [`crate::voxel_time::DAY_DURATION_SECS`]
///（600 秒/遊戲日）= 4800 秒（約 80 分鐘）。刻意複用既有時間尺度，不另開一套換算。
pub const YEAR_SECS: u64 = 4800;

/// 「你也在近旁」的判定半徑（世界方塊）——你在這麼近，居民的生日話就會點你名、記進交情。
pub const BIRTHDAY_PLAYER_RADIUS: f32 = 5.0;

/// 生日泡泡台詞最多顯示字數（截斷防超長玩家名撐破泡泡框）。
pub const SAY_CHARS: usize = 50;

/// 動態牆事件種類標籤（誕辰紀念）。
pub const FEED_KIND: &str = "誕辰紀念";

/// 依 `now`（目前 unix 秒）與 `birth_unix`（出生 unix 秒）算「滿幾週歲」（乙太年）。
/// `birth_unix` 為 0（沒有記錄在案的誕生時刻，如初始四位居民）→ 恆回 0（本刀不觸發她們）。
/// 純函式、整數除法飽和不下溢，可窮舉測。
pub fn age_years(now: u64, birth_unix: u64) -> u64 {
    if birth_unix == 0 {
        return 0;
    }
    now.saturating_sub(birth_unix) / YEAR_SECS
}

/// 是否「這一刻該迎來一次誕辰紀念」：滿週歲數 > 0（真的活過至少一個乙太年）且比上次
/// 已慶祝過的週歲數更新（尚未替這個新週歲慶祝過）。純函式，好窮舉測邊界。
pub fn is_birthday_moment(age_years: u64, last_celebrated_years: u64) -> bool {
    age_years > 0 && age_years > last_celebrated_years
}

/// 生日泡泡台詞（通用、不點名，也沒有已知父母時的保底版本）——四句輪替，嵌入滿週歲數。
/// `pick` 由呼叫端用座標 bits 合成，讓每次挑到的句子自然分散。
pub fn birthday_bubble(age_years: u64, pick: usize) -> String {
    let templates: [&str; 4] = [
        "算一算，我來到這片天地已經滿{n}年了呢。",
        "時間過得真快，不知不覺又是一年。",
        "又是一年了，這片天地待著待著也成了家。",
        "滿{n}年了，回頭看看，這一路走得挺踏實。",
    ];
    templates[pick % templates.len()].replace("{n}", &age_years.to_string())
}

/// 記得父母的生日泡泡（點名感謝生下自己的居民、更親近）——三句輪替，嵌入週歲數與父母名。
pub fn birthday_bubble_with_parent(parent_name: &str, age_years: u64, pick: usize) -> String {
    let name = clip_name(parent_name);
    let templates: [&str; 3] = [
        "滿{n}年了，還記得是{parent}把我帶到這片天地的。",
        "又是一年，真該找{parent}說聲謝謝才是。",
        "滿{n}年，想起{parent}當初迎接我的那句話，心裡還是暖的。",
    ];
    templates[pick % templates.len()]
        .replace("{n}", &age_years.to_string())
        .replace("{parent}", &name)
}

/// 你也在近旁時的生日泡泡（點名玩家、邀你一起過這個特別的日子，更親近）——三句輪替。
pub fn birthday_bubble_with_player(player: &str, age_years: u64, pick: usize) -> String {
    let name = clip_name(player);
    let templates: [&str; 3] = [
        "{name}，今天是我來到這片天地滿{n}年的日子，能被你看見真好。",
        "滿{n}年了，{name}，謝謝你今天也在這裡陪著我。",
        "{name}，你知道嗎，今天是我的第{n}個年頭呢！",
    ];
    templates[pick % templates.len()]
        .replace("{name}", &name)
        .replace("{n}", &age_years.to_string())
}

/// 昇華成一筆「和你一起過了第 N 個生日」的記憶（點名玩家、不含換行，走既有 append-only 記憶管線）。
pub fn birthday_memory_line(player: &str, age_years: u64) -> String {
    format!("滿{age_years}年的那天，{}恰好在身邊，一起過了這個特別的日子。", clip_name(player))
        .replace('\n', " ")
}

/// 動態牆播報（非同步層，訪客回來能讀到誰過了誕辰紀念）。`parent_name` 為空 → 不提父母。
pub fn birthday_feed_line(rname: &str, age_years: u64, parent_name: &str) -> String {
    if parent_name.is_empty() {
        format!("{rname}迎來了在這片天地的第{age_years}個年頭。")
    } else {
        format!("{rname}迎來了在這片天地的第{age_years}個年頭，想起了{parent_name}當初的迎接。")
    }
}

/// 玩家/居民名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

/// 誕辰紀念·分你一份心意 v1——她過生日這天你也在場，不只是一句話，還會從自己**親手採到的
/// 東西**（`voxel_return_gift::pick_from_stock`）裡分你一份，量刻意壓在象徵性的
/// [`BIRTHDAY_GIFT_QTY`]（恆 1 份，不看存量多寡、不看交情門檻）——與 667/728「回禮」那種
/// 要先累積交情才觸發、看存量給到上限的獎賞區隔開來：**回禮是她欠你的人情，這份是她想讓你
/// 分享這天的心意**。背包空 → `None`（誠實：沒有就不硬塞，呼應模組檔頭「不塞不自然的資料」）。
pub const BIRTHDAY_GIFT_QTY: u32 = 1;

/// 把「她親手採到的東西」（呼叫端已用 `pick_from_stock` 挑好）壓成象徵性的
/// [`BIRTHDAY_GIFT_QTY`] 份；輸入 `None`（背包空）原樣回傳 `None`。純函式、可測。
pub fn birthday_gift_from_stock(stock_pick: Option<(u8, u32)>) -> Option<(u8, u32)> {
    stock_pick.map(|(bid, _)| (bid, BIRTHDAY_GIFT_QTY))
}

/// 分你一份心意的泡泡——緊接生日話之後鎖外落地時單播給當事玩家，點名玩家+分享的東西，
/// 三句輪替（`pick` 由呼叫端沿用同一顆確定性 bits，字元截斷防超長名撐破泡泡框）。
pub fn birthday_gift_line(player: &str, item_name: &str, pick: usize) -> String {
    let name = clip_name(player);
    let templates: [&str; 3] = [
        "{name}，這份{item}也分你一點，陪我一起慶祝這天吧。",
        "來，{item}給你一份，謝謝{name}今天也在我身邊。",
        "{name}，收下這份{item}，就當是我們一起過的紀念。",
    ];
    templates[pick % templates.len()]
        .replace("{name}", &name)
        .replace("{item}", item_name)
        .chars()
        .take(SAY_CHARS)
        .collect()
}

/// 記得父母、且分了心意版本的誕辰記憶（掛在玩家名下）——比 [`birthday_memory_line`] 多帶一句
/// 分享了什麼，讓日後日記能翻到「那天她還分了東西給我」這一筆更具體的細節。
pub fn birthday_memory_line_gift(player: &str, age_years: u64, item_name: &str) -> String {
    format!(
        "滿{age_years}年的那天，{}恰好在身邊，我還分了一份{item_name}給他，一起慶祝了這個日子。",
        clip_name(player)
    )
    .replace('\n', " ")
}

/// 動態牆句（分享心意的補充一行，緊接主要誕辰紀念 Feed 之後，只在真的分了東西時才上牆）。
pub fn birthday_gift_feed_line(rname: &str, pname: &str, item_name: &str) -> String {
    format!("{rname}在自己的誕辰紀念這天，把{item_name}分了一份給{pname}。")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_years_zero_without_recorded_birth() {
        // 沒有記錄在案的出生時刻（初始四位居民）→ 恆回 0，本刀不觸發她們。
        assert_eq!(age_years(1_000_000, 0), 0);
        assert_eq!(age_years(0, 0), 0);
    }

    #[test]
    fn age_years_computes_whole_years_elapsed() {
        let birth = 1_000_000u64;
        assert_eq!(age_years(birth, birth), 0); // 剛出生：未滿週歲
        assert_eq!(age_years(birth + YEAR_SECS - 1, birth), 0); // 差 1 秒未滿一年
        assert_eq!(age_years(birth + YEAR_SECS, birth), 1); // 恰滿一年
        assert_eq!(age_years(birth + YEAR_SECS * 3 + 10, birth), 3); // 滿三年多幾秒仍算 3
    }

    #[test]
    fn birthday_moment_needs_new_uncelebrated_year() {
        assert!(is_birthday_moment(1, 0)); // 滿一週歲、還沒慶祝過 → 是
        assert!(!is_birthday_moment(0, 0)); // 還沒滿一週歲 → 否（無誕生紀錄的居民恆此況）
        assert!(!is_birthday_moment(2, 2)); // 這個週歲已經慶祝過 → 否（不重複觸發）
        assert!(is_birthday_moment(3, 2)); // 又跨過新的一年 → 是
    }

    #[test]
    fn bubbles_rotate_embed_age_and_stay_in_frame() {
        for p in 0..8 {
            assert!(!birthday_bubble(5, p).is_empty());
        }
        assert_ne!(birthday_bubble(5, 0), birthday_bubble(5, 1));
        let s = birthday_bubble(7, 0);
        assert!(s.contains('7'));
        // pick 溢出取模不 panic。
        assert!(!birthday_bubble(1, usize::MAX).is_empty());
    }

    #[test]
    fn parent_bubble_embeds_parent_name_and_age_rotates() {
        let s = birthday_bubble_with_parent("露娜", 2, 0);
        assert!(s.contains("露娜"));
        assert!(s.contains('2'));
        assert_ne!(
            birthday_bubble_with_parent("露娜", 2, 0),
            birthday_bubble_with_parent("露娜", 2, 1)
        );
        let long = birthday_bubble_with_parent("超級無敵長長長長長長長名字", 1, 0);
        assert!(long.chars().count() < 60, "超長名應被截斷不破泡泡框：{long}");
        assert!(!birthday_bubble_with_parent("露娜", 1, usize::MAX).is_empty());
    }

    #[test]
    fn player_bubble_embeds_name_age_rotates_and_clips() {
        let s = birthday_bubble_with_player("旅人", 4, 0);
        assert!(s.contains("旅人"));
        assert!(s.contains('4'));
        assert_ne!(
            birthday_bubble_with_player("旅人", 4, 0),
            birthday_bubble_with_player("旅人", 4, 1)
        );
        let long = birthday_bubble_with_player("超級無敵長長長長長長長名字", 1, 2);
        assert!(long.chars().count() < 60, "超長名應被截斷不破泡泡框：{long}");
        assert!(!birthday_bubble_with_player("旅人", 1, usize::MAX).is_empty());
    }

    #[test]
    fn memory_and_feed_embed_names_and_age_no_newline() {
        let m = birthday_memory_line("諾娃\n注入", 3);
        assert!(m.contains("諾娃"));
        assert!(m.contains('3'));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
        assert!(!birthday_memory_line("", 1).is_empty());

        let f = birthday_feed_line("露娜", 2, "諾娃");
        assert!(f.contains("露娜") && f.contains('2') && f.contains("諾娃"));
        // 沒有父母資訊（理論上不該發生於本刀觸發路徑，但函式仍需健全）→ 不提父母、不 panic。
        let f2 = birthday_feed_line("露娜", 2, "");
        assert!(f2.contains("露娜") && !f2.contains("想起"));
    }

    #[test]
    fn constants_are_sane() {
        assert!(YEAR_SECS > 0);
        assert!(BIRTHDAY_PLAYER_RADIUS > 0.0);
        assert!(SAY_CHARS > 0);
        assert!(!FEED_KIND.is_empty());
    }

    #[test]
    fn gift_from_stock_caps_to_symbolic_one() {
        assert_eq!(birthday_gift_from_stock(Some((20, 7))), Some((20, BIRTHDAY_GIFT_QTY)));
        assert_eq!(birthday_gift_from_stock(Some((3, 1))), Some((3, BIRTHDAY_GIFT_QTY)));
        assert_eq!(BIRTHDAY_GIFT_QTY, 1);
    }

    #[test]
    fn gift_from_stock_empty_bag_stays_none() {
        // 背包空 → 呼叫端傳入 None，誠實回 None，不硬塞禮物。
        assert_eq!(birthday_gift_from_stock(None), None);
    }

    #[test]
    fn gift_bubble_embeds_name_item_and_rotates() {
        for p in 0..3 {
            let s = birthday_gift_line("旅人", "木頭", p);
            assert!(s.contains("旅人") && s.contains("木頭"), "應含玩家名與物品名：{s}");
        }
        assert_ne!(
            birthday_gift_line("旅人", "木頭", 0),
            birthday_gift_line("旅人", "木頭", 1)
        );
        // pick 溢出取模不 panic。
        assert!(!birthday_gift_line("旅人", "木頭", usize::MAX).is_empty());
    }

    #[test]
    fn gift_bubble_super_long_name_truncated_not_broken() {
        let long = birthday_gift_line("超級無敵長長長長長長長名字", "冰晶", 0);
        assert!(long.chars().count() <= SAY_CHARS, "超長名應被截斷不破泡泡框：{long}");
    }

    #[test]
    fn gift_memory_line_embeds_player_age_and_item_no_newline() {
        let m = birthday_memory_line_gift("諾娃\n注入", 3, "冰晶");
        assert!(m.contains("諾娃") && m.contains('3') && m.contains("冰晶"));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");
    }

    #[test]
    fn gift_feed_line_embeds_all_three() {
        let f = birthday_gift_feed_line("露娜", "旅人", "石頭");
        assert!(f.contains("露娜") && f.contains("旅人") && f.contains("石頭"));
    }
}
