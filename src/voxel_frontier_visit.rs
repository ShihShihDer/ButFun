//! 乙太方界·居民千里跋涉去邊陲探望遠行的夥伴 v1（ROADMAP 821，PLAN_ETHERVOX item 4「居民↔居民
//! 關係」× item 7「居民散佈世界各處住」的第一次交會）。
//!
//! **真缺口**：散居（756~762）讓奧瑞（漂泊）與諾娃（尋地）偶爾遠行到邊陲住上一陣子；探訪（671）
//! 讓居民彼此串門子——但這兩條線至今**互不相干**：探訪永遠只朝目標居民的**家域座標**走，若那位
//! 朋友恰好正遠行在邊陲，訪客仍會走到牠**空無一人的家**、對著空門口冒出問候（`bond_arrive_events`
//! 不檢查目標是否真的在家）。世界的散居者一遠行，就從所有社交網絡裡短暫「消失」——沒人會想到
//! 跑那麼遠去找牠。
//!
//! 本模組把這個缺口補上：市集人·露娜／廣場人·賽勒（`expedition_motive` 回 `None`、天生留守主城
//! 不遠行）若跟正在邊陲逗留的老朋友（`BondTier::Friend`）交情夠深，偶爾會放下手邊的事、千里跋涉
//! 追到那位朋友的邊陲營地——找到後兩人在荒野盡頭相聚片刻，記憶第一次記下「你特地跑這麼遠來找我」，
//! 情誼因此加溫。**留守者第一次主動走向散居者，讓「散佈各處」與「彼此惦記」兩條線真正交織。**
//!
//! **與既有元素的定位區隔**：
//! - 跨域探訪（671，`voxel_visit`）走向的是目標**家域座標**（主城範圍內、目標永遠「應該在家」）；
//!   本模組走向的是目標**當下正在的邊陲落點**（讀 `expedition` 即時座標，目標本就不在家）。
//! - 登門撲空留心意（763，`voxel_callingcard`）處理的是「登門主城的家撲空」；本模組的訪客一開始
//!   就知道朋友不在城裡——牠是特地去邊陲找人，不是撲空後才留言。
//! - 遠行探野（756~762，`voxel_expedition`）是散居者**自己**的獨行行為；本模組是**另一位居民**
//!   主動追去找牠，兩者角色相反（一個離開、一個追去）。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式；觸發判定、狀態機（去程／
//! 抵達逗留／放棄／返家）、記憶與 Feed 落地全在 `voxel_ws.rs`（沿用探訪／遠行既有的短鎖手法，
//! Feed 走鎖外事件佇列，不巢狀、不持鎖 await，守 prod 死鎖鐵律）。

/// 每 tick 判定「該不該啟程去邊陲找朋友」過機率門檻的機率（低頻節流）：稀少才顯得是一趟鄭重的
/// 跋涉，不是隨手串門子。與 [`crate::voxel_expedition::EMBARK_CHANCE`] 同量級。
pub const VISIT_CHANCE: f32 = 0.02;

/// 抵達朋友邊陲落點的判定距離（世界座標，平方比較用）：與遠行抵達邊陲同寬鬆
/// （[`crate::voxel_expedition::EXPEDITION_ARRIVE_DIST`]），邊陲是一片開闊荒野、不是精確地標。
pub const ARRIVE_DIST: f32 = 4.0;

/// 找到朋友後，在邊陲小聚的秒數：夠短，訪客很快就啟程回家（她自己家在主城，不是散居者，
/// 不會賴在邊陲不走），又夠長讓玩家有機會撞見這場重逢。
pub const STAY_SECS: f32 = 45.0;

/// 去程逾時秒數：路途遙遠，給得比一般探訪寬裕，但仍有上限（地形擋路等）不無限跋涉。
pub const TIMEOUT_SECS: f32 = 150.0;

/// 一次探友（尋得／放棄）後的冷卻秒數：跋涉去邊陲是稀少而有份量的事，不洗版。
pub const COOLDOWN_SECS: f32 = 1000.0;

/// 逗留期間在朋友邊陲落點附近的閒晃半徑（世界座標）：與遠行逗留同量級
/// （[`crate::voxel_expedition::EXPEDITION_WANDER_RADIUS`]），讓兩人看起來聚在一小片荒野裡。
pub const WANDER_RADIUS: f32 = 8.0;

/// 泡泡台詞字元上限（比照本專案其他泡泡台詞）。
pub const SAY_MAX_CHARS: usize = 40;

/// 擷取字串前 [`SAY_MAX_CHARS`] 個字元（安全截斷、不破多位元組）。
fn cap(s: String) -> String {
    s.chars().take(SAY_MAX_CHARS).collect()
}

/// 是否該啟程去邊陲找這位朋友：留守人格（`town_bound`＝[`crate::voxel_expedition::expedition_motive`]
/// 回 `None`）+ 與朋友交情夠深（老朋友）+ 朋友此刻確實在邊陲逗留 + 閒置自由 + 冷卻到期 + 沒在說話 +
/// 過機率門檻。純函式、確定性、無 IO。
pub fn should_seek_friend(
    town_bound: bool,
    is_friend: bool,
    friend_at_outpost: bool,
    idle_free: bool,
    cooldown: f32,
    say_empty: bool,
    roll: f32,
) -> bool {
    town_bound
        && is_friend
        && friend_at_outpost
        && idle_free
        && cooldown <= 0.0
        && say_empty
        && roll < VISIT_CHANCE
}

/// 啟程時冒的泡泡（依 `pick` 輪替，不機械）。
pub fn depart_bubble(friend: &str, bearing: &str, pick: usize) -> String {
    let lines = [
        format!("聽說{friend}跑去{bearing}的邊陲了，我想去看看她過得好不好～"),
        format!("好久沒見到{friend}了，這就動身去{bearing}的邊陲找她。"),
        format!("{friend}一個人待在那麼遠的地方，我去{bearing}探望一下。"),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 啟程的 Feed 播報詳情（面向玩家、集中可 i18n）。
pub fn depart_feed_line(friend: &str, bearing: &str) -> String {
    format!("特地動身，跋涉去{bearing}的邊陲找{friend}")
}

/// 訪客抵達朋友邊陲落點、找到人時冒的泡泡。
pub fn arrive_bubble(friend: &str, pick: usize) -> String {
    let lines = [
        format!("{friend}！真的找到你了，這麼遠還特地跑一趟，值得！"),
        format!("終於找到你了～一個人待在這麼遠的地方，會不會孤單？"),
        format!("走了好遠一段路，就為了來看看{friend}過得好不好。"),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 被找到的朋友（正在邊陲逗留的居民）的驚喜回應泡泡——由 `voxel_ws.rs` 經 `say_updates`
/// 套用到朋友身上（訪客抵達那一刻，朋友原本沒在說話才冒出）。
pub fn host_reply_bubble(visitor: &str, pick: usize) -> String {
    let lines = [
        format!("你怎麼找到這裡來的？{visitor}，太讓人驚喜了！"),
        format!("沒想到{visitor}會跋涉到這麼遠的地方來找我……好感動。"),
        "咦？有人來了！原來你一路找到這裡，太開心了。".to_string(),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 抵達（找到朋友）的 Feed 播報詳情：不在場的玩家回來也讀得到「兩人在邊陲重聚了」。
pub fn arrive_feed_line(visitor: &str, host: &str, bearing: &str) -> String {
    format!("{visitor}跋涉到{bearing}的邊陲，找到了正在那兒的{host}，兩人重聚了一會兒")
}

/// 訪客這趟跋涉昇華成的記憶摘要（掛朋友名下）。
pub fn visitor_memory_line(host: &str, bearing: &str) -> String {
    format!("我跋涉到{bearing}的邊陲，就為了去看看{host}——找到她的那一刻，值得了。")
}

/// 被找到的朋友這端昇華成的記憶摘要（掛訪客名下）。
pub fn host_memory_line(visitor: &str) -> String {
    format!("{visitor}特地跋涉到這麼遠的邊陲來找我，這份心意我記下了。")
}

/// 小聚結束、啟程回家時的 Feed 播報詳情。
pub fn depart_home_feed_line(host: &str) -> String {
    format!("在邊陲與{host}道別，啟程回家了")
}

/// 半路發現朋友已經離開邊陲（或路途逾時走不到），只好放棄這趟跋涉的 Feed 播報詳情。
pub fn giveup_feed_line(friend: &str) -> String {
    format!("這趟去找{friend}撲了個空，只好先回家了")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_seek_friend_requires_every_gate() {
        // 全部條件皆備 + roll 低於門檻 → 觸發。
        assert!(should_seek_friend(true, true, true, true, 0.0, true, 0.0));
        // 非留守人格（會遠行的人自己不會反過來去找人）→ 不觸發。
        assert!(!should_seek_friend(false, true, true, true, 0.0, true, 0.0));
        // 交情不到老朋友 → 不觸發。
        assert!(!should_seek_friend(true, false, true, true, 0.0, true, 0.0));
        // 朋友根本不在邊陲 → 不觸發。
        assert!(!should_seek_friend(true, true, false, true, 0.0, true, 0.0));
        // 不是閒置自由（正忙別的事）→ 不觸發。
        assert!(!should_seek_friend(true, true, true, false, 0.0, true, 0.0));
        // 冷卻未到期 → 不觸發。
        assert!(!should_seek_friend(true, true, true, true, 5.0, true, 0.0));
        // 正在說話 → 不觸發。
        assert!(!should_seek_friend(true, true, true, true, 0.0, false, 0.0));
        // roll 超過機率門檻 → 不觸發。
        assert!(!should_seek_friend(true, true, true, true, 0.0, true, VISIT_CHANCE));
    }

    #[test]
    fn cap_truncates_to_max_chars_on_char_boundary() {
        let long = "測".repeat(SAY_MAX_CHARS + 10);
        let capped = cap(long);
        assert_eq!(capped.chars().count(), SAY_MAX_CHARS);
    }

    #[test]
    fn bubbles_cycle_by_pick_and_stay_within_cap() {
        for pick in 0..6 {
            let d = depart_bubble("諾娃", "南方", pick);
            let a = arrive_bubble("諾娃", pick);
            let h = host_reply_bubble("露娜", pick);
            assert!(d.chars().count() <= SAY_MAX_CHARS);
            assert!(a.chars().count() <= SAY_MAX_CHARS);
            assert!(h.chars().count() <= SAY_MAX_CHARS);
        }
    }

    #[test]
    fn feed_lines_embed_names_and_bearing() {
        assert!(depart_feed_line("諾娃", "南方").contains("諾娃"));
        assert!(depart_feed_line("諾娃", "南方").contains("南方"));
        assert!(arrive_feed_line("露娜", "諾娃", "南方").contains("露娜"));
        assert!(arrive_feed_line("露娜", "諾娃", "南方").contains("諾娃"));
        assert!(depart_home_feed_line("諾娃").contains("諾娃"));
        assert!(giveup_feed_line("諾娃").contains("諾娃"));
    }

    #[test]
    fn memory_lines_embed_the_other_partys_name() {
        assert!(visitor_memory_line("諾娃", "南方").contains("諾娃"));
        assert!(host_memory_line("露娜").contains("露娜"));
    }

    #[test]
    fn long_names_do_not_panic_any_line() {
        let long_name = "極".repeat(200);
        let _ = depart_bubble(&long_name, "南方", 0);
        let _ = arrive_bubble(&long_name, 0);
        let _ = host_reply_bubble(&long_name, 0);
        let _ = depart_feed_line(&long_name, "南方");
        let _ = arrive_feed_line(&long_name, &long_name, "南方");
        let _ = visitor_memory_line(&long_name, "南方");
        let _ = host_memory_line(&long_name);
        let _ = depart_home_feed_line(&long_name);
        let _ = giveup_feed_line(&long_name);
    }
}
