//! 乙太方界·居民登門撲空、留下心意；主人回家後感應到「有人來找過我」v1（ROADMAP 763）
//!
//! 751（居民登門找鄰居串門子）讓居民朝聖抵達的其實是某位鄰居親手立的家牌時，把這趟走過去
//! 當成一次登門拜訪；752（登門遇主人在家）補上另一半——若那位鄰居**正好在家**，訪客見到本人、
//! 主人也回一句迎客暖招呼。但至此為止，若主人**不在家**（撲空），訪客照樣念一句對空門口的話就
//! 走了，那位鄰居**永遠不會知道曾有人特地來找過牠**——這趟心意就這樣蒸發了。
//!
//! 本切片把「撲空」這條一直被忽略的分支接成閉環：訪客撲空時不再只是對空門口說句話，而是**在門口
//! 留下一份心意**（一張「我來找過你」的心意）；日後那位主人**回到自家附近、閒著沒事時**，會
//! **感應到**這份留下的心意——冒一句「咦，某某趁我不在時來找過我呢」的暖泡泡、把「某某特地來找過我」
//! 記成一筆掛在那位訪客名下的記憶（日後回想／日記可引用）、並在城鎮動態留一則「回家發現有人來找過」。
//! 撲空第一次不再是白跑一趟——你來找過我，就算沒碰上，我回家終究會知道、會記得。
//!
//! 這是路線圖④「居民↔居民關係→小社會湧現」把 752 的「撲空」補上溫度的一刀：鄰里往來從
//!「當面碰上才算數」長成了「就算錯過，心意也留得住、傳得到」。
//!
//! **情誼不重複記帳**：751 抵達時已 `record_visit(訪客, 主人)` 記了這對的一次往來（撲空亦然），
//! 本切片的「主人回家感應到」只補上主人側的**記憶／泡泡／Feed**，**不再加記情誼**，避免情誼被灌爆
//!（守成本紀律，比照 752 的作法）。
//!
//! **純邏輯層**：撲空留心意台詞／主人感應台詞／記憶／Feed／「回到自家附近」判定與心意佇列上限，
//! 皆確定性純函式、可單元測試；居民狀態機、位置快照、記憶／Feed IO 全留在 `voxel_ws.rs`
//!（沿用既有短鎖不巢狀的鎖序）。零新協議、零 migration（心意佇列與冷卻皆純記憶體欄位、重啟歸零，
//! 比照 743 讀牌記憶與 751 朝聖狀態的慣例）、零 LLM、零新美術（沿用既有 Feed 列與泡泡框）。

/// 主人算「回到自家附近」的半徑（世界座標，方塊）：主人當前座標在家域中心這半徑內、又閒著沒事時，
/// 才會感應到門口留下的心意。比 752 的 `HOST_HOME_DIST`(=5) 稍寬——居民本來就常在家域周邊晃盪，
/// 「回到家這一帶」就該感應得到，不必剛好站在牌子腳下。
pub const NOTICE_HOME_DIST: f32 = 10.0;

/// 主人感應到門口心意之間的冷卻（秒）：多張心意留在門口時，一張一張慢慢感應、逐張念一句，
/// 不在同一瞬間一次倒完（避免泡泡／Feed 洗版，守成本紀律）。
pub const NOTICE_COOLDOWN: f32 = 30.0;

/// 每位居民門口最多堆積幾份「有人來找過」的心意：撲空頻繁時上限保護，超過就不再堆
///（最舊的先感應完才騰出空間），避免佇列無限長。
pub const MAX_PENDING_CALLERS: usize = 4;

/// 主人側記憶前綴：日記／回想端可據此把這筆歸為「鄰里往來」主題（沿用 751 的登門串門子主題）。
pub const CALLINGCARD_TAG: &str = "🏡登門串門子";

/// 訪客登門撲空、在門口留下心意時冒的暖句（比 751 對空門口的 `visit_line` 更明確帶「留個心意」意味）。
/// `pick` 取居民座標／時機雜湊，讓不同居民／不同時機說不同句（確定性、零 LLM）。
pub fn miss_line(neighbor: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "{n}不在家呀…那我在門口留個心意，改天再來。",
        "撲了個空呢，{n}。就當我來看過你了，留點心意在這兒。",
        "唔，{n}出門啦？沒關係，留張心意讓你知道我來過。",
    ];
    let s = TEMPLATES[pick % TEMPLATES.len()].replace("{n}", neighbor);
    // 泡泡框保險：控制在 40 字元內（與 741/750/751/752 一致）。
    s.chars().take(40).collect()
}

/// 主人回到自家附近、感應到門口留下的心意時冒的暖泡泡（點名那位訪客）。
/// `pick` 取居民座標／時機雜湊，確定性、零 LLM。
pub fn notice_line(guest: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "咦？{n}趁我不在時來找過我呢，真窩心。",
        "門口有{n}留下的心意…{n}特地來過一趟啊。",
        "原來{n}來找過我，可惜錯過了，下回換我去找{n}。",
    ];
    let s = TEMPLATES[pick % TEMPLATES.len()].replace("{n}", guest);
    s.chars().take(40).collect()
}

/// 掛在「那位訪客」名下、主人側的記憶：某某趁我不在時特地來找過我。
/// 主人的記憶第一次記下「就算錯過，也有人特地來找過我」——鄰里往來的溫度從撲空這一側也留下了痕跡。
pub fn notice_memory(guest: &str) -> String {
    format!("{CALLINGCARD_TAG}：{guest}趁我不在時特地登門來找過我，回家才發現，心裡暖暖的。")
}

/// 城鎮動態 Feed 一行：主人回到家，發現某訪客趁自己不在時來找過。
pub fn notice_feed(host: &str, guest: &str) -> String {
    format!("{host}回到家，發現{guest}趁自己不在時特地來找過。")
}

/// 主人是否「回到自家附近」（家域中心 `(home_x, home_z)` 的 `NOTICE_HOME_DIST` 內）。
/// 任一座標非有限值（壞資料）時保守回 `false`——寧可先不感應，也不誤判。
pub fn noticed_at_home(body_x: f32, body_z: f32, home_x: f32, home_z: f32) -> bool {
    if !(body_x.is_finite() && body_z.is_finite() && home_x.is_finite() && home_z.is_finite()) {
        return false;
    }
    let dx = body_x - home_x;
    let dz = body_z - home_z;
    dx * dx + dz * dz <= NOTICE_HOME_DIST * NOTICE_HOME_DIST
}

/// 把一位訪客加進主人門口的「心意佇列」：已在佇列裡就不重複塞（同一位鄰居連來幾趟只留一份心意，
/// 避免同一人洗爆佇列）；佇列已達上限（`MAX_PENDING_CALLERS`）就不再堆（最舊的先感應完才騰空間）。
/// 回傳是否真的塞進去了（供呼叫端決定要不要記一筆）。
pub fn enqueue_caller(pending: &mut Vec<String>, guest: &str) -> bool {
    if guest.is_empty() {
        return false;
    }
    if pending.iter().any(|g| g == guest) {
        return false;
    }
    if pending.len() >= MAX_PENDING_CALLERS {
        return false;
    }
    pending.push(guest.to_string());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miss_line_names_neighbor_and_fits_bubble() {
        for pick in 0..6usize {
            let s = miss_line("諾娃", pick);
            assert!(s.contains("諾娃"), "泡泡應點名鄰居: {s}");
            assert!(s.chars().count() <= 40, "泡泡應 ≤40 字元: {s}");
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn miss_line_varies_with_pick() {
        assert_ne!(miss_line("露娜", 0), miss_line("露娜", 1));
    }

    #[test]
    fn notice_line_names_guest_and_fits_bubble() {
        for pick in 0..6usize {
            let s = notice_line("賽勒", pick);
            assert!(s.contains("賽勒"), "泡泡應點名訪客: {s}");
            assert!(s.chars().count() <= 40, "泡泡應 ≤40 字元: {s}");
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn notice_line_varies_with_pick() {
        assert_ne!(notice_line("奧瑞", 0), notice_line("奧瑞", 1));
    }

    #[test]
    fn notice_memory_carries_tag_and_guest() {
        let m = notice_memory("諾娃");
        assert!(m.starts_with(CALLINGCARD_TAG), "應以登門串門子前綴開頭: {m}");
        assert!(m.contains("諾娃"), "記憶應點名訪客: {m}");
    }

    #[test]
    fn notice_feed_names_both() {
        let f = notice_feed("奧瑞", "露娜");
        assert!(f.contains("奧瑞") && f.contains("露娜"));
    }

    #[test]
    fn at_home_within_and_outside_radius() {
        // 剛好站在家域中心：算在家附近。
        assert!(noticed_at_home(10.0, 10.0, 10.0, 10.0));
        // 邊界（6,8 → 距離 10）：含邊界算在家附近。
        assert!(noticed_at_home(16.0, 18.0, 10.0, 10.0));
        // 半徑外：還沒回到家附近。
        assert!(!noticed_at_home(30.0, 30.0, 10.0, 10.0));
    }

    #[test]
    fn at_home_rejects_bad_coords() {
        assert!(!noticed_at_home(f32::NAN, 0.0, 0.0, 0.0));
        assert!(!noticed_at_home(0.0, 0.0, f32::INFINITY, 0.0));
    }

    #[test]
    fn enqueue_dedups_and_caps() {
        let mut q: Vec<String> = Vec::new();
        assert!(enqueue_caller(&mut q, "露娜"));
        // 同一人不重複。
        assert!(!enqueue_caller(&mut q, "露娜"));
        assert_eq!(q.len(), 1);
        // 塞到上限。
        assert!(enqueue_caller(&mut q, "諾娃"));
        assert!(enqueue_caller(&mut q, "賽勒"));
        assert!(enqueue_caller(&mut q, "奧瑞"));
        assert_eq!(q.len(), MAX_PENDING_CALLERS);
        // 已滿，新的塞不進。
        assert!(!enqueue_caller(&mut q, "旅人"));
        assert_eq!(q.len(), MAX_PENDING_CALLERS);
    }

    #[test]
    fn enqueue_rejects_empty() {
        let mut q: Vec<String> = Vec::new();
        assert!(!enqueue_caller(&mut q, ""));
        assert!(q.is_empty());
    }
}
