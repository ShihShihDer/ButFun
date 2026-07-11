//! 乙太方界·相思成行——跨村探親 v1（自主提案切片 ROADMAP 947，
//! PLAN_ETHERVOX §2「記憶→行為」的字面兌現 × §7「居民散佈世界各處住」殖民地生活閉環）。
//!
//! **真缺口 / 為誰做**：兩村相思（945，`voxel_farbond`）讓分住兩村的老朋友第一次隔村惦記彼此——
//! 露娜會望向遠方念叨搬去風禾屯的諾娃、諾娃也會在她那頭念叨主村的露娜。但那份相思至今**只停在
//! 念叨**：無論她念了多少次、想了多深，她的腳從沒有因此動過一步。「記憶要驅動行為，不只聊天」
//! 是這個世界的核心信念（`docs/PLAN_ETHERVOX.md`），而相思正是最該推人上路的那種記憶——
//! **想念到了深處，人是會真的走一趟的。**
//!
//! 本切片把這一步補上：945 的相思冷卻鐘記下「這位居民已經念叨了幾次」（[`crate::voxel_farbond::FarBondClock`]
//! 的念叨計數）；當她對隔村摯友的念叨累積滿 [`EMBARK_AFTER_MISSES`] 次，下一次相思時刻來臨、
//! 而她正好手邊無事，她就**不再只是念叨——她收拾心情、真的動身**，沿著大地走向那座幾百格外的村子。
//! 半路上玩家可能在荒野撞見一位趕路的居民（「要去風禾屯看諾娃」）；抵達後兩位老朋友在村口重逢，
//! 各自冒出重逢的話、雙方都把這一天記進長期記憶、情誼因這趟遠路真正加溫一格（`record_visit`）；
//! 小聚片刻後她道別、踏上歸途，思念計數歸零——**下一輪相思再從頭累積，聚散有時，往復不息**。
//! 世界的兩座村，第一次被一條「因想念而走出來」的路真正連了起來。
//!
//! **與既有系統 razor-sharp 區隔（非同軸換皮）**：
//! - **邊陲探友（821，`voxel_frontier_visit`）**＝留守居民**碰運氣遇合**（機率門檻）去荒野**營地**
//!   找一位**暫時遠行**的朋友（朋友幾天後自己就回來了）；本刀＝由**945 相思記憶累積**驅動（念滿
//!   N 次才成行、非隨機遇合）、目的地是**另一座村莊**、對象是**永久遷居**的定居摯友（她不動身就
//!   永遠見不到）。一個是「順路去看看」、一個是「積思成行」——觸發源（記憶 vs 骰子）、目的地
//!   （村 vs 荒野）、對象（定居者 vs 暫留者）三者皆不同。
//! - **兩村相思（945，`voxel_farbond`）**＝原地念叨、零移動；本刀＝念叨的**下一步**——腳真的動了。
//!   兩者共用同一座冷卻鐘與念叨計數，是同一條情感線的前後兩幕，不是並行的兩套系統。
//! - **遠行探野（756~762，`voxel_expedition`）**＝散居者**獨自**去**無人的荒野**，不為找人；
//!   本刀＝專程去**另一座村**找**特定一位老朋友**。
//! - **跨域探訪（671，`voxel_visit`）／登門串門子（751）**＝主村**街坊之間**的日常串門；本刀＝
//!   跨越幾百格、走向**另一座聚落**的鄭重探親，觸發、距離、敘事份量皆不同。
//!
//! **成本 / 濫用防護鐵律**：
//! - **零 LLM**：成行判定、台詞、記憶、Feed 全是確定性純函式；觸發由 945 既有的相思節拍驅動
//!   （FARBOND_MIN_INTERVAL_SECS＝90 分鐘一次念叨 × 滿 [`EMBARK_AFTER_MISSES`] 次才成行），
//!   天然稀有——一趟探親至少隔好幾個現實小時，不洗版、不佔用任何 API 額度。
//! - **玩家無從觸發或催發**：聚落歸屬、情誼層級、念叨計數全是伺服器內部狀態，不收任何玩家輸入、
//!   不開對外端點；台詞只嵌居民顯示名與村落名（皆伺服器策展字串），無注入／NSFW 面。
//! - **純邏輯層**：本檔零鎖、零 IO、零 async；狀態機推進、鎖序、Feed／記憶落地全在 `voxel_ws.rs`
//!  （比照 821 邊陲探友的短鎖慣例：事件鎖內收集、鎖外統一落地，守 prod 死鎖鐵律）。
//! - **零 migration、零協議破壞**：探親是純記憶體暫態（重啟＝這趟旅程自然作罷、居民照常回家域），
//!   不新增任何持久化格式；快照不新增欄位。

/// 念叨滿幾次才成行：945 的相思每 90 分鐘至多一次，滿 3 次（約 4.5 個現實小時的思念）後、
/// 第 4 次相思時刻來臨時她不再念叨，而是真的動身——積思成行是稀有而鄭重的大事。
pub const EMBARK_AFTER_MISSES: u32 = 3;

/// 抵達目的地（動身時摯友所在點）的判定距離（世界座標）：村子是一片開闊聚落、不必走到腳尖碰腳尖。
pub const ARRIVE_DIST: f32 = 5.0;

/// 抵達後判定「摯友真的在村裡」的距離：摯友平日在自己村的小半徑內活動（家域/地塊），走到動身時
/// 她所在的點附近、她本人在這個範圍內就算重逢；她若恰好也出遠門了（遠行/探親），就算撲空。
pub const REUNION_NEAR_DIST: f32 = 28.0;

/// 重逢後在摯友村裡小聚的秒數：夠長讓玩家有機會撞見兩位老朋友聚在一塊，又不至於賴著不走
/// （她自己的家在另一座村，天黑前想趕回去）。
pub const STAY_SECS: f32 = 75.0;

/// 去程逾時秒數：兩村相距幾百格（`voxel_colony::SITE_BASE_DIST`＝520 起），居民步速約 2.6 格/秒，
/// 單程理論值約 200～300 秒；給足餘裕容地形繞行，仍有上限——路實在走不通就折返，不無限跋涉。
pub const TIMEOUT_SECS: f32 = 480.0;

/// 一趟探親（重逢／撲空／逾時折返）後的冷卻秒數：探親本身已由「念叨滿 N 次」天然稀有，
/// 這裡再兜一層底，防交情/聚落狀態異動下的極端連發。
pub const COOLDOWN_SECS: f32 = 1800.0;

/// 重逢小聚期間在摯友身邊的閒晃半徑（世界座標）：與 821 邊陲探友逗留同量級，
/// 讓兩位老朋友看起來聚在村口一小片範圍裡。
pub const WANDER_RADIUS: f32 = 8.0;

/// 泡泡台詞字元上限（與本專案其他社交泡泡一致）。
pub const SAY_MAX_CHARS: usize = 50;

/// Feed 事件種類名稱（面向玩家、集中可 i18n）。
pub const FEED_KIND: &str = "跨村探親";

/// 擷取字串前 [`SAY_MAX_CHARS`] 個字元（安全截斷、不破多位元組中文）。
fn cap(s: String) -> String {
    s.chars().take(SAY_MAX_CHARS).collect()
}

/// 是否該把這一次「相思時刻」升級成「動身探親」：念叨已滿 [`EMBARK_AFTER_MISSES`] 次 +
/// 探親冷卻到期 + 閒置自由（沒接任何既定任務）+ 醒著。純函式、確定性、無 IO——
/// 相思滿了就成行，不再擲骰（成行的稀有度已由 945 的念叨節拍天然保證）。
pub fn should_embark(miss_count: u32, cooldown: f32, idle_free: bool, asleep: bool) -> bool {
    miss_count >= EMBARK_AFTER_MISSES && cooldown <= 0.0 && idle_free && !asleep
}

/// 動身時冒的泡泡（依 `pick` 輪替，不機械）：點名摯友與她現居的村落。
pub fn depart_bubble(friend: &str, place: &str, pick: usize) -> String {
    let lines = [
        format!("想{friend}想了這麼多回……這次我要親自走一趟「{place}」！"),
        format!("光是念叨不夠了，我這就動身去「{place}」看{friend}。"),
        format!("收拾好心情，去「{place}」！{friend}，等我啊。"),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 動身的 Feed 播報詳情（接在動身者名後、第三人稱）：讓不在場的玩家也讀得到「思念推人上路了」。
pub fn depart_feed_line(friend: &str, place: &str) -> String {
    format!("念叨了一回又一回，終於動身踏上前往「{place}」的遠路，要去看看老朋友{friend}")
}

/// 抵達重逢時訪客冒的泡泡。
pub fn arrive_bubble(friend: &str, pick: usize) -> String {
    let lines = [
        format!("{friend}！我真的走來了——這一路好遠，見到你就值得！"),
        format!("好久不見！{friend}，你在這村裡過得好不好？"),
        format!("想了你那麼多回，終於又見到{friend}了……"),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 被探望的摯友的驚喜回應泡泡（由 `voxel_ws.rs` 經 `say_updates` 套用，她原本沒在說話才冒出）。
pub fn host_reply_bubble(visitor: &str, pick: usize) -> String {
    let lines = [
        format!("{visitor}？！你竟然走了這麼遠的路來看我……"),
        format!("是{visitor}！隔著兩座村還特地跑一趟，太讓人感動了。"),
        format!("{visitor}來了！快，讓我好好看看你——一路辛苦了。"),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 重逢的 Feed 播報詳情：兩座村第一次被一趟「因想念而走出來」的路連起來。
pub fn reunion_feed_line(visitor: &str, host: &str, place: &str) -> String {
    format!("{visitor}走完幾百格的遠路抵達「{place}」，與久別的老朋友{host}在村口重逢了")
}

/// 訪客這趟探親昇華成的記憶摘要（掛摯友名下）。
pub fn visitor_memory_line(host: &str, place: &str) -> String {
    format!("想{host}想到坐不住，我真的走了一趟「{place}」——重逢那一刻，這一路的遠都值得了。")
}

/// 被探望的摯友這端昇華成的記憶摘要（掛訪客名下）。
pub fn host_memory_line(visitor: &str, place: &str) -> String {
    format!("{visitor}因為想念，從另一座村一路走到「{place}」來看我——這份情誼我一輩子記得。")
}

/// 小聚結束、道別踏上歸途的 Feed 播報詳情。
pub fn depart_home_feed_line(host: &str, place: &str) -> String {
    format!("在「{place}」與{host}道別，帶著滿滿的暖意踏上歸途")
}

/// 抵達卻發現摯友不在村裡（她恰好也出遠門了）的撲空泡泡。
pub fn not_home_bubble(friend: &str, pick: usize) -> String {
    let lines = [
        format!("咦……{friend}不在村裡？偏偏挑了她出門的日子來。"),
        format!("走了這麼遠，{friend}卻不在家……緣分真會捉弄人。"),
        format!("{friend}不在啊……那我在村口坐一會兒就回吧。"),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 撲空的 Feed 播報詳情。
pub fn not_home_feed_line(friend: &str, place: &str) -> String {
    format!("遠路走到「{place}」卻撲了個空——{friend}恰好不在村裡，只好悵然折返")
}

/// 路途逾時（地形擋路等）折返的 Feed 播報詳情。
pub fn giveup_feed_line(friend: &str, place: &str) -> String {
    format!("往「{place}」的路太遠太難走，這趟沒能見到{friend}，半途折返了")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_embark_requires_every_gate() {
        // 念滿 + 冷卻到期 + 閒置 + 醒著 → 成行。
        assert!(should_embark(EMBARK_AFTER_MISSES, 0.0, true, false));
        // 念叨次數再多也成行（>= 門檻）。
        assert!(should_embark(EMBARK_AFTER_MISSES + 5, 0.0, true, false));
        // 念叨還沒滿 → 這次仍只是念叨。
        assert!(!should_embark(EMBARK_AFTER_MISSES - 1, 0.0, true, false));
        assert!(!should_embark(0, 0.0, true, false));
        // 探親冷卻未到期 → 不成行。
        assert!(!should_embark(EMBARK_AFTER_MISSES, 1.0, true, false));
        // 手邊有事（非閒置）→ 不成行（照常念叨、計數續累）。
        assert!(!should_embark(EMBARK_AFTER_MISSES, 0.0, false, false));
        // 在睡 → 不成行。
        assert!(!should_embark(EMBARK_AFTER_MISSES, 0.0, true, true));
    }

    #[test]
    fn cap_truncates_on_char_boundary() {
        let long = "思".repeat(SAY_MAX_CHARS + 20);
        assert_eq!(cap(long).chars().count(), SAY_MAX_CHARS);
    }

    #[test]
    fn bubbles_cycle_by_pick_and_stay_within_cap() {
        for pick in 0..6 {
            for s in [
                depart_bubble("諾娃", "風禾屯", pick),
                arrive_bubble("諾娃", pick),
                host_reply_bubble("露娜", pick),
                not_home_bubble("諾娃", pick),
            ] {
                assert!(s.chars().count() <= SAY_MAX_CHARS);
                assert!(!s.is_empty());
                assert!(!s.contains('\n'), "泡泡須單行");
            }
        }
        // 三句輪替：pick 0 與 3 同句、0 與 1 不同句。
        assert_eq!(depart_bubble("諾娃", "風禾屯", 0), depart_bubble("諾娃", "風禾屯", 3));
        assert_ne!(depart_bubble("諾娃", "風禾屯", 0), depart_bubble("諾娃", "風禾屯", 1));
    }

    #[test]
    fn feed_lines_embed_names_and_place_single_line() {
        for s in [
            depart_feed_line("諾娃", "風禾屯"),
            reunion_feed_line("露娜", "諾娃", "風禾屯"),
            depart_home_feed_line("諾娃", "風禾屯"),
            not_home_feed_line("諾娃", "風禾屯"),
            giveup_feed_line("諾娃", "風禾屯"),
        ] {
            assert!(s.contains("諾娃"));
            assert!(s.contains("風禾屯"));
            assert!(!s.contains('\n'), "Feed 明細須單行、防洗版");
        }
        assert!(reunion_feed_line("露娜", "諾娃", "風禾屯").contains("露娜"));
    }

    #[test]
    fn memory_lines_embed_the_other_party_and_place() {
        let v = visitor_memory_line("諾娃", "風禾屯");
        assert!(v.contains("諾娃") && v.contains("風禾屯"));
        let h = host_memory_line("露娜", "風禾屯");
        assert!(h.contains("露娜") && h.contains("風禾屯"));
    }

    #[test]
    fn long_names_do_not_panic_any_line() {
        let long_name = "遠".repeat(200);
        let _ = depart_bubble(&long_name, &long_name, 1);
        let _ = arrive_bubble(&long_name, 2);
        let _ = host_reply_bubble(&long_name, 0);
        let _ = not_home_bubble(&long_name, 1);
        let _ = depart_feed_line(&long_name, &long_name);
        let _ = reunion_feed_line(&long_name, &long_name, &long_name);
        let _ = visitor_memory_line(&long_name, &long_name);
        let _ = host_memory_line(&long_name, &long_name);
        let _ = depart_home_feed_line(&long_name, &long_name);
        let _ = not_home_feed_line(&long_name, &long_name);
        let _ = giveup_feed_line(&long_name, &long_name);
    }

    #[test]
    fn timeout_covers_colony_distance_at_resident_speed() {
        // 誠實自檢：逾時上限必須真的走得完第一座殖民地的距離（520 格 ÷ 2.6 格/秒 ＝ 200 秒），
        // 且留有繞行餘裕；防未來有人把 TIMEOUT 調小到根本走不到。
        let one_way_secs = crate::voxel_colony::SITE_BASE_DIST as f32 / 2.6;
        assert!(TIMEOUT_SECS > one_way_secs * 1.5, "逾時須留足地形繞行餘裕");
    }
}
