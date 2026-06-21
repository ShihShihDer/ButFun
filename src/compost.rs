//! 堆肥循環（ROADMAP 473）。
//!
//! 在此之前，種田的「收成」是這條療癒迴圈的終點：作物收完只換成乙太，那些一季季
//! 反覆收下的農產對「下一輪怎麼種得更好」毫無回饋——建議箱多位居民一再反映「種田
//! 後期缺乏後續用途／回收循環／用途展示，收穫後就斷在那裡」。本切片把收成接回起點：
//! 每收成幾株作物就攢成一份**堆肥**，下次播種自動把一份堆肥漚進土裡，那株作物從一
//! 開始就帶「滋養」加成——長得更快、收成多得一筆乙太。種田第一次有了「收成→漚肥→
//! 種得更壯」的循環，辛苦收下的每一季都回饋到下一季。
//!
//! **與既有土地維度刻意正交、不重疊**：
//!   - 438 沃土輪休＝獎勵「讓地**空著歇**」（時間休耕→收成多得乙太），是「靜養」的。
//!   - 367 連片沃土＝獎勵「把地種成**連片**」（空間鄰接→成長加速），是「佈局」的。
//!   - 454 輪作＝獎勵「**換著品種**種」（品種多樣→成長加速），是「換種」的。
//!   - 473 堆肥＝獎勵「把**收成回收**漚肥」（主動投入→成長加速＋多得乙太），是「循環」的。
//! 四種真實農法各走各的軸：靜養／佈局／換種／循環，互補而不互相取代。
//!
//! 這層是純資料 + 純函式，無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試；
//! 滋養加成的源頭數值定在這裡，`field.rs` 接線層呼叫並套用。

/// 收成幾株作物攢成一份堆肥。刻意設成「收幾株才換一份」——堆肥來自實際的收成回收，
/// 不是無限水龍頭；要先用心種、收得夠多，才漚得出堆肥。
pub const HARVESTS_PER_CHARGE: u8 = 3;

/// 一塊地能囤積的堆肥份數上限。攢滿就先停（避免囤積到天荒地老），鼓勵「收→漚→種」
/// 的流動而非死囤；用掉一份後又能繼續攢。
pub const COMPOST_MAX: u8 = 6;

/// 滋養（漚過堆肥）的作物成長加速倍率。與連片沃土／品種季節／輪作三條倍率正交、
/// 獨立疊乘，皆只放大成長、不多耗水（見 `Crop::grow_boosted`），維持公平。
pub const NOURISH_GROWTH_MULT: f32 = 1.35;

/// 滋養的作物收成時多得的乙太（純正向）。刻意小（+1）——堆肥的主要回饋在「長得快」，
/// 乙太只多一點點，避免把堆肥變成刷乙太的捷徑、擾動既有經濟。
pub const NOURISH_ETHER_BONUS: u32 = 1;

/// 收成一株後累積堆肥原料；攢滿 `HARVESTS_PER_CHARGE` 株、且未達囤積上限時，轉成一份堆肥。
/// 回傳 `(新原料計數, 新堆肥份數)`。純函式——伺服器收成結算與測試共用同一套門檻。
///
/// 規則：
///   - 每收一株，原料 +1（夾在 `[0, HARVESTS_PER_CHARGE]` 不爆 u8、不無限長）。
///   - 原料攢到門檻且堆肥未滿 → 轉成一份堆肥、原料歸零。
///   - 堆肥已滿 → 原料停在門檻（擱著等用掉一份再轉，不浪費也不溢出）。
pub fn accrue(matter: u8, charges: u8) -> (u8, u8) {
    let charges = charges.min(COMPOST_MAX);
    // 原料夾在門檻內：既不會 u8 溢位，也讓「攢滿但堆肥已滿」時原料停在門檻備用。
    let next_matter = matter.saturating_add(1).min(HARVESTS_PER_CHARGE);
    if next_matter >= HARVESTS_PER_CHARGE && charges < COMPOST_MAX {
        (0, charges + 1)
    } else {
        (next_matter, charges)
    }
}

/// 播種時若手上有堆肥，消耗一份、這株作物得「滋養」。回傳 `(剩餘堆肥份數, 這株是否滋養)`。
/// 純函式——沒堆肥（0 份）則不消耗、不滋養（向後相容：沒漚過肥就跟改動前一樣種）。
pub fn consume_for_planting(charges: u8) -> (u8, bool) {
    if charges > 0 {
        (charges - 1, true)
    } else {
        (0, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 收滿一份門檻才攢出一份堆肥，原料隨即歸零（收成回收，非無限水龍頭）。
    #[test]
    fn accrues_one_charge_per_threshold() {
        let (mut matter, mut charges) = (0u8, 0u8);
        // 收前 HARVESTS_PER_CHARGE-1 株：只攢原料、還沒到一份。
        for _ in 0..(HARVESTS_PER_CHARGE - 1) {
            let (m, c) = accrue(matter, charges);
            matter = m;
            charges = c;
            assert_eq!(charges, 0, "未滿門檻不該攢出堆肥");
        }
        // 收第 HARVESTS_PER_CHARGE 株：剛好攢出一份，原料歸零。
        let (m, c) = accrue(matter, charges);
        assert_eq!(c, 1, "滿門檻該攢出一份堆肥");
        assert_eq!(m, 0, "攢出堆肥後原料歸零");
    }

    /// 連收很多株，堆肥份數封頂在 COMPOST_MAX、不溢出、不無限長。
    #[test]
    fn charges_cap_at_max() {
        let (mut matter, mut charges) = (0u8, 0u8);
        // 收遠超過攢滿所需的株數。
        for _ in 0..(HARVESTS_PER_CHARGE as u32 * (COMPOST_MAX as u32 + 5)) {
            let (m, c) = accrue(matter, charges);
            matter = m;
            charges = c;
        }
        assert_eq!(charges, COMPOST_MAX, "堆肥份數該封頂在上限");
        assert!(matter <= HARVESTS_PER_CHARGE, "原料不該無限長");
    }

    /// 堆肥已滿時，原料停在門檻備用；一旦用掉一份，下次收成立刻補回一份（原料不浪費）。
    #[test]
    fn matter_waits_when_full_then_converts_after_spend() {
        // 先攢到滿。
        let (mut matter, mut charges) = (0u8, 0u8);
        for _ in 0..(HARVESTS_PER_CHARGE as u32 * COMPOST_MAX as u32 + HARVESTS_PER_CHARGE as u32) {
            let (m, c) = accrue(matter, charges);
            matter = m;
            charges = c;
        }
        assert_eq!(charges, COMPOST_MAX);
        assert_eq!(matter, HARVESTS_PER_CHARGE, "滿載時原料停在門檻備用");
        // 用掉一份堆肥（播種）。
        let (after, nourished) = consume_for_planting(charges);
        assert!(nourished);
        assert_eq!(after, COMPOST_MAX - 1);
        // 用掉後再收一株：備用原料立刻補回一份堆肥。
        let (_m, c) = accrue(matter, after);
        assert_eq!(c, COMPOST_MAX, "用掉一份後備用原料立刻補回");
    }

    /// 有堆肥才消耗、才滋養；沒堆肥則不消耗、不滋養（向後相容）。
    #[test]
    fn consume_only_when_available() {
        assert_eq!(consume_for_planting(0), (0, false));
        assert_eq!(consume_for_planting(1), (0, true));
        assert_eq!(consume_for_planting(COMPOST_MAX), (COMPOST_MAX - 1, true));
    }

    /// 滋養加成是純正向：成長倍率 >1、乙太加成 ≥1，永不懲罰。
    #[test]
    fn nourish_bonus_is_strictly_positive() {
        assert!(NOURISH_GROWTH_MULT > 1.0, "滋養該加速成長");
        assert!(NOURISH_GROWTH_MULT.is_finite());
        assert!(NOURISH_ETHER_BONUS >= 1, "滋養該多得乙太");
    }

    /// 同輸入同輸出（確定性，可重現）。
    #[test]
    fn accrue_is_deterministic() {
        assert_eq!(accrue(1, 2), accrue(1, 2));
        assert_eq!(consume_for_planting(3), consume_for_planting(3));
    }
}
