//! 乙太方界·居民遠行探野 v1（PLAN_ETHERVOX item 7「居民散佈世界各處住」第一刀，ROADMAP 756）。
//!
//! **設計依據**：路線圖 item 7 是「居民散佈世界各處住（別擠主城，麥塊式散居）」——一個至今
//! 完全沒動過的**空間／聚落維度**。目前四位居民各有一個固定家域中心（露娜在世界原點、其餘三位
//! 在南／西／東 75 格），日常只在自家域半徑（[`crate::voxel_residents::HOME_RADIUS`]=20 格）內
//! 閒晃——世界的荒野遠處始終空無一人，玩家永遠只在主城一帶撞見居民。這一刀把「散佈各處」的第一步
//! 做出來：**讓天生愛四處走的 Wanderer 人格居民（奧瑞，東方「山林、遠足感」），偶爾放下手邊的事、
//! 獨自遠行到遠離主城的世界邊陲住上一陣子，再走回家。**
//!
//! 玩家第一次會在遠離主城的荒野撞見居民的身影——「奧瑞獨自往東方的邊陲遠行了」浮上動態牆，過一
//! 陣子再讀到「奧瑞遠行歸來」。世界不再只圍著主城打轉，居民的足跡第一次真的散進了荒野。這是把
//! 居民從「主城的固定住戶」推向「散佈世界各處的居民」的地基——日後可在此之上長出「在遠方紮營／
//! 蓋第二個家／真的搬過去住」（item 7 後續）。
//!
//! **與既有『離家』行為的定位區隔**：
//! - 探訪鄰居（671，`voxel_visit`）／登門串門子（751，`voxel_neighborvisit`）走向的是**另一位
//!   居民的家域**（主城範圍內、社交目的）；本模組走向的是**無人的荒野邊陲**（空間探索、不為找人）。
//! - 重返心中的牌子（743，`voxel_readsign`）走向的是一塊**玩家立過、讓牠印象深刻的告示牌**（記憶
//!   驅動、有具體地標）；本模組走向的是**由方位算出的遠方一點**（天性驅動、不倚賴任何既有地標）。
//! - 孤獨尋伴（678）／久別奔迎（747）走向的是**玩家**；本模組是獨自遠行，不朝任何人。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式；朝邊陲走的狀態機、逗留計時、
//! 記憶昇華與 Feed 廣播都在 `voxel_ws.rs`（沿用探訪／朝聖既有的短鎖手法與 wander 中心覆寫慣例）。

/// 遠行落點距家域中心的基準距離（世界座標）：家已在主城外圍（±75），再往外推這麼遠 → 落在
/// 離主城逾百格的荒野，玩家平常閒晃絕不會誤入，撞見居民在那才顯得「牠真的走遠了」。
pub const EXPEDITION_DIST: f32 = 95.0;

/// 抵達邊陲的判定距離（世界座標，平方比較用）：落在此半徑內即視為「到了遠方」。比探訪抵達距離
/// 寬鬆些——邊陲是一片開闊荒野、不是一個精確地標，走到大概位置就算到了。
pub const EXPEDITION_ARRIVE_DIST: f32 = 4.0;

/// 抵達邊陲後在遠方逗留（探索、四處走走）的秒數：夠久讓玩家有機會撞見牠獨自在荒野的身影，
/// 又不會久到牠整天不回家。逗留期間以邊陲為閒晃中心自由走動（見 `voxel_ws` wander 中心覆寫）。
pub const EXPEDITION_STAY_SECS: f32 = 120.0;

/// 去程逾時秒數：啟程時設此值、未抵達時每 tick 遞減；走太久（地形擋路、繞遠路等）還沒到就放棄
/// 這趟遠行、交回一般 wander 帶牠回家，不無限走。路遠故給得寬裕。
pub const EXPEDITION_TIMEOUT: f32 = 150.0;

/// 遠行冷卻秒數：一趟遠行（歸來或放棄）後至少隔這麼久才可能再啟程——遠行是稀少而有份量的事件，
/// 不洗版；各居民初始冷卻另行錯開。
pub const EXPEDITION_COOLDOWN: f32 = 900.0;

/// 逗留期間的閒晃半徑（世界座標）：比家域半徑略小，讓牠在邊陲一小片範圍內自然走動、不再散得更開。
pub const EXPEDITION_WANDER_RADIUS: f32 = 8.0;

/// 每次「是否啟程遠行」判定過機率門檻的機率（低頻節流）：實際還要層層過閘（Wanderer 人格、閒置
/// 自由、白天、冷卻到期），故有感頻率遠低於此。稀少才顯得是一趟鄭重的遠行。
pub const EMBARK_CHANCE: f32 = 0.02;

/// 遠行記憶掛的哨兵「玩家名」：與 `voxel_bedtime` / `voxel_readsign` 同慣例，用一個絕不與真實玩家
/// 撞名的內部鍵，讓「我到過遠方」這類記憶不汙染任何玩家的好感度／回想。
pub const EXPEDITION_MEMORY_PLAYER: &str = "__voxel_expedition__";

/// 泡泡台詞字元上限（比照其他泡泡台詞）。
pub const SAY_MAX_CHARS: usize = 40;

/// 是否啟程遠行：Wanderer 人格 + 閒置自由（沒在忙別的意圖）+ 白天 + 冷卻到期 + 此刻沒在說話
/// + 過機率門檻。`roll` 由呼叫端以 `rand::random::<f32>()` 餵入（與本專案其他機率骰同慣例）。
/// 純函式、確定性、無 IO。
pub fn should_embark(
    is_wanderer: bool,
    idle_free: bool,
    is_day: bool,
    cooldown: f32,
    say_empty: bool,
    roll: f32,
) -> bool {
    is_wanderer && idle_free && is_day && cooldown <= 0.0 && say_empty && roll < EMBARK_CHANCE
}

/// 由方位向量算出玩家看得懂的方位名（繁中）。本世界座標約定：+x = 東、+z = 南
/// （見 `voxel_residents::resident_home_base`：南方在 (0,75)、東方在 (75,0)）。
pub fn bearing_label(dx: f32, dz: f32) -> &'static str {
    if dx.abs() >= dz.abs() {
        if dx >= 0.0 {
            "東方"
        } else {
            "西方"
        }
    } else if dz >= 0.0 {
        "南方"
    } else {
        "北方"
    }
}

/// 算出這趟遠行的落點與方位：由主城（世界原點）朝這位居民家的方向再往外推 [`EXPEDITION_DIST`]
/// ——落在離主城更遠的荒野，實現「散往世界各處、別擠主城」。依 `seq` 給落點一點角度抖動與距離
/// 變化，讓每趟遠行不落在同一格、不機械。家恰在原點（罕見）時退回用 `seq` 選一個基本方位。
/// 回傳 (落點 x, 落點 z, 方位名)。純函式、確定性。
pub fn pick_frontier(home_x: f32, home_z: f32, seq: usize) -> (f32, f32, &'static str) {
    let (mut ux, mut uz) = (home_x, home_z);
    let len = (ux * ux + uz * uz).sqrt();
    if len < 1.0 {
        // 家就在世界原點：沒有「往外」的方向可依，用 seq 選一個基本方位當作出發朝向。
        let dirs = [(1.0_f32, 0.0_f32), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
        let d = dirs[seq % 4];
        ux = d.0;
        uz = d.1;
    } else {
        ux /= len;
        uz /= len;
    }
    // 角度抖動（約 ±0.45 rad ≈ ±26°）：讓遠行落點依 seq 散開，不每趟都同一點。
    let jitter = ((seq % 7) as f32 - 3.0) * 0.15;
    let (s, c) = jitter.sin_cos();
    let rx = ux * c - uz * s;
    let rz = ux * s + uz * c;
    // 距離也依 seq 微調（0~32 格），讓深淺不一。
    let dist = EXPEDITION_DIST + (seq % 5) as f32 * 8.0;
    let fx = home_x + rx * dist;
    let fz = home_z + rz * dist;
    (fx, fz, bearing_label(rx, rz))
}

/// 擷取字串前 [`SAY_MAX_CHARS`] 個字元（安全截斷、不破多位元組）。
fn cap(s: String) -> String {
    s.chars().take(SAY_MAX_CHARS).collect()
}

/// 啟程遠行時冒的泡泡（點出方位、依 `pick` 輪替不機械）。
pub fn embark_bubble(bearing: &str, pick: usize) -> String {
    let lines = [
        format!("今天想往{bearing}走遠一點，去世界的邊陲看看～"),
        format!("待在城裡太久了，我想一個人去{bearing}的荒野走走。"),
        format!("腳癢了！這就動身往{bearing}遠行一趟。"),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 抵達遠方邊陲時冒的泡泡。
pub fn arrive_bubble(bearing: &str, pick: usize) -> String {
    let lines = [
        format!("終於走到{bearing}的邊陲了，這裡好開闊啊……"),
        format!("原來{bearing}這麼遠的地方，是這副模樣。"),
        "遠離人聲的荒野，安安靜靜的，真好。".to_string(),
    ];
    cap(lines[pick % lines.len()].clone())
}

/// 遠行歸來（回到家域）時冒的泡泡。
pub fn return_bubble(pick: usize) -> String {
    let lines = [
        "遠行回來啦！世界真的比想像的還大。",
        "走了好遠一圈，還是家附近讓人安心～",
        "這趟遠行看了好多，回家真好。",
    ];
    cap(lines[pick % lines.len()].to_string())
}

/// 抵達邊陲時昇華成的記憶摘要（掛 [`EXPEDITION_MEMORY_PLAYER`] 哨兵，日記／內心可引用）。
pub fn arrive_memory_summary(bearing: &str) -> String {
    format!("我獨自遠行到{bearing}的邊陲，看見了主城以外那片開闊的荒野。")
}

/// 啟程遠行的 Feed 播報詳情（面向玩家、集中可 i18n）。
pub fn embark_feed_line(bearing: &str) -> String {
    format!("獨自往{bearing}的邊陲遠行了")
}

/// 遠行歸來的 Feed 播報詳情。
pub fn return_feed_line() -> String {
    "遠行歸來，帶回了荒野盡頭的見聞".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embark_needs_all_gates() {
        // 全條件滿足 + roll 過門檻 → true。
        assert!(should_embark(true, true, true, 0.0, true, 0.0));
        // 非 Wanderer 人格 → 永不遠行。
        assert!(!should_embark(false, true, true, 0.0, true, 0.0));
        // 正忙別的意圖（idle_free=false）→ 不遠行。
        assert!(!should_embark(true, false, true, 0.0, true, 0.0));
        // 夜裡 → 不往荒野跑。
        assert!(!should_embark(true, true, false, 0.0, true, 0.0));
        // 冷卻未到 → 不遠行。
        assert!(!should_embark(true, true, true, 5.0, true, 0.0));
        // 此刻正在說話 → 不遠行（不打斷冒泡）。
        assert!(!should_embark(true, true, true, 0.0, false, 0.0));
        // roll 沒過機率門檻 → 不遠行。
        assert!(!should_embark(true, true, true, 0.0, true, 0.99));
    }

    #[test]
    fn embark_chance_is_the_gate_boundary() {
        // 恰在門檻下 → true；恰在門檻（含以上）→ false。
        assert!(should_embark(true, true, true, 0.0, true, EMBARK_CHANCE - 0.001));
        assert!(!should_embark(true, true, true, 0.0, true, EMBARK_CHANCE));
    }

    #[test]
    fn bearing_label_four_quadrants() {
        assert_eq!(bearing_label(1.0, 0.0), "東方");
        assert_eq!(bearing_label(-1.0, 0.0), "西方");
        assert_eq!(bearing_label(0.0, 1.0), "南方");
        assert_eq!(bearing_label(0.0, -1.0), "北方");
        // 對角時取主導軸（x 佔優 → 東西）。
        assert_eq!(bearing_label(2.0, 1.0), "東方");
        assert_eq!(bearing_label(-2.0, 1.0), "西方");
        assert_eq!(bearing_label(1.0, 2.0), "南方");
        assert_eq!(bearing_label(1.0, -2.0), "北方");
    }

    #[test]
    fn frontier_pushes_farther_from_origin_than_home() {
        // 奧瑞家在東方 (75,0)：遠行落點應更往東、離原點更遠。
        let (fx, fz, bearing) = pick_frontier(75.0, 0.0, 0);
        let home_d = (75.0_f32 * 75.0).sqrt();
        let front_d = (fx * fx + fz * fz).sqrt();
        assert!(front_d > home_d, "遠行落點應比家離主城更遠");
        assert!(fx > 75.0, "應更往東推");
        assert_eq!(bearing, "東方");
        // z 幾乎沿東向（抖動有限）。
        assert!(fz.abs() < 60.0);
    }

    #[test]
    fn frontier_directions_match_home_direction() {
        // 南方家 (0,75) → 往南更遠。
        let (_, fz, bearing) = pick_frontier(0.0, 75.0, 0);
        assert!(fz > 75.0);
        assert_eq!(bearing, "南方");
        // 西方家 (-75,0) → 往西更遠。
        let (fx, _, bearing) = pick_frontier(-75.0, 0.0, 0);
        assert!(fx < -75.0);
        assert_eq!(bearing, "西方");
    }

    #[test]
    fn frontier_origin_home_falls_back_to_seq_cardinal() {
        // 家恰在原點：不 panic、用 seq 選基本方位，落點離原點約 EXPEDITION_DIST。
        let (fx, fz, _) = pick_frontier(0.0, 0.0, 0);
        let d = (fx * fx + fz * fz).sqrt();
        assert!(d >= EXPEDITION_DIST - 1.0 && d <= EXPEDITION_DIST + 40.0);
        // 不同 seq 落在不同基本方位。
        let (ax, _, _) = pick_frontier(0.0, 0.0, 0);
        let (bx, bz, _) = pick_frontier(0.0, 0.0, 1);
        assert!((ax - bx).abs() > 1.0 || bz.abs() > 1.0);
    }

    #[test]
    fn frontier_seq_varies_landing_spot() {
        // 同一個家、不同 seq → 落點不同（角度／距離抖動生效），不機械地永遠同一格。
        let (x0, z0, _) = pick_frontier(75.0, 0.0, 0);
        let (x1, z1, _) = pick_frontier(75.0, 0.0, 3);
        assert!((x0 - x1).abs() > 0.5 || (z0 - z1).abs() > 0.5);
    }

    #[test]
    fn bubbles_and_memory_nonempty_capped_and_mention_bearing() {
        for pick in 0..6 {
            let e = embark_bubble("東方", pick);
            let a = arrive_bubble("西方", pick);
            let r = return_bubble(pick);
            assert!(!e.is_empty() && e.chars().count() <= SAY_MAX_CHARS);
            assert!(!a.is_empty() && a.chars().count() <= SAY_MAX_CHARS);
            assert!(!r.is_empty() && r.chars().count() <= SAY_MAX_CHARS);
        }
        // 啟程泡泡與抵達記憶都點出方位。
        assert!(embark_bubble("南方", 0).contains("南方"));
        assert!(arrive_memory_summary("北方").contains("北方"));
        assert!(!arrive_memory_summary("東方").is_empty());
    }

    #[test]
    fn bubbles_rotate_by_pick() {
        // 不同 pick 至少有兩種不同的啟程泡泡（輪替、不永遠同一句）。
        let a = embark_bubble("東方", 0);
        let b = embark_bubble("東方", 1);
        let c = embark_bubble("東方", 2);
        assert!(a != b || b != c);
    }

    #[test]
    fn feed_lines_nonempty() {
        assert!(!embark_feed_line("東方").is_empty());
        assert!(embark_feed_line("南方").contains("南方"));
        assert!(!return_feed_line().is_empty());
    }
}
