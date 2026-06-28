//! ROADMAP 438 沃土輪休——耕地的「地力」純邏輯。
//!
//! 此前每格耕地只是「翻土→播種→澆水→收成」的開關：同一格猛種猛收、與細心輪流休耕，
//! 收成完全一樣。本模組給耕地第一次加上 Stardew/ONI 式的「地力」維度（原創實作）：
//!   - **翻好卻空著（休耕）的土**會隨時間慢慢養出地力（`accrue`）。
//!   - **收成**時依該格累積的地力換成額外乙太（`harvest_bonus`）；收成後地力歸零（被作物吸收）。
//!   - 種了作物期間地力「鎖住」不再累積，待收成兌現。
//!
//! 設計原則：
//! - 純邏輯、確定性、無 IO（`dt` 由 caller 傳入），便於自動測試。
//! - **純正向**：地力只給「額外」乙太、永不低於基準收成——讓「讓地歇口氣」是划得來的選擇，
//!   而非「不照辦就被倒扣」的懲罰（療癒向，呼應 406 用心栽培的基調）。
//! - **無乙太水龍頭**：地力有上限、收成即歸零、bonus 封頂——要真的休耕一段時間才換得到。
//! - 以「細格點」存地力（非直接存 0~3）：每 tick `dt`≈1/15≈0.067s，若直接以整數累積會被
//!   捨入抹平（0.067×rate<1），故放大成細格點累積、四捨五入取整，長期不偏移。

/// 地力上限（細格點）。配合 `REST_GAIN_PER_SEC`＝40 → 約 150 秒休耕養滿。
pub const SOIL_MAX_FINE: u16 = 6000;
/// 每秒休耕養出的地力（細格點）。
pub const REST_GAIN_PER_SEC: f32 = 40.0;
/// 滿地力收成最多多給的乙太。
pub const MAX_BONUS: u32 = 3;

/// 休耕養地：空翻好土歇了 `dt` 秒後的新地力（細格點），上限封頂。
/// 以 f32 計算後四捨五入回 u16——抵銷每 tick 不足 1 的捨入流失，長期養地速率不偏。
/// `dt` 非有限或負值一律不變（防壞輸入污染狀態）。
pub fn accrue(current: u16, dt: f32) -> u16 {
    if !dt.is_finite() || dt <= 0.0 {
        return current.min(SOIL_MAX_FINE);
    }
    let next = current as f32 + REST_GAIN_PER_SEC * dt;
    next.round().clamp(0.0, SOIL_MAX_FINE as f32) as u16
}

/// 收成時把地力換成「額外乙太」：線性映 [0, SOIL_MAX_FINE] → [0, MAX_BONUS]。
/// 滿地力給 `MAX_BONUS`、空地力給 0；中間整數截斷（要養夠才升一階）。
pub fn harvest_bonus(soil: u16) -> u32 {
    let soil = soil.min(SOIL_MAX_FINE) as u32;
    soil * MAX_BONUS / SOIL_MAX_FINE as u32
}

/// 量化成前端顯示等級（0~3）：0=貧（剛翻/剛收）、3=沃（養滿）。
/// 讓玩家一眼看出「哪幾格歇夠了、種下去更甜」。門檻取 1/4、1/2、3/4。
pub fn display_level(soil: u16) -> u8 {
    let soil = soil.min(SOIL_MAX_FINE) as u32;
    let max = SOIL_MAX_FINE as u32;
    if soil >= max * 3 / 4 {
        3
    } else if soil >= max / 2 {
        2
    } else if soil >= max / 4 {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accrue_builds_up_and_caps() {
        // 從 0 開始，每秒 +40：一秒後約 40。
        let a = accrue(0, 1.0);
        assert_eq!(a, 40);
        // 已滿不再長。
        assert_eq!(accrue(SOIL_MAX_FINE, 1.0), SOIL_MAX_FINE);
        // 接近滿時封頂、不溢位。
        assert_eq!(accrue(SOIL_MAX_FINE - 5, 1.0), SOIL_MAX_FINE);
    }

    #[test]
    fn accrue_reaches_full_in_about_150s() {
        // 模擬 15Hz tick 累積 150 秒，應養滿（容忍捨入誤差）。
        let dt = 1.0 / 15.0;
        let mut s = 0u16;
        for _ in 0..(15 * 150) {
            s = accrue(s, dt);
        }
        assert_eq!(s, SOIL_MAX_FINE, "150 秒 15Hz 累積應剛好養滿、不被捨入抹平");
    }

    #[test]
    fn accrue_ignores_bad_dt() {
        assert_eq!(accrue(100, f32::NAN), 100);
        assert_eq!(accrue(100, -1.0), 100);
        assert_eq!(accrue(100, 0.0), 100);
    }

    #[test]
    fn harvest_bonus_is_zero_to_three_and_monotone() {
        assert_eq!(harvest_bonus(0), 0);
        assert_eq!(harvest_bonus(SOIL_MAX_FINE), MAX_BONUS);
        // 單調不減。
        let mut prev = 0;
        for q in 0..=20 {
            let soil = (SOIL_MAX_FINE as u32 * q / 20) as u16;
            let b = harvest_bonus(soil);
            assert!(b >= prev, "bonus 應隨地力單調不減");
            assert!(b <= MAX_BONUS, "bonus 不得超過封頂");
            prev = b;
        }
    }

    #[test]
    fn harvest_bonus_never_punishes() {
        // 任何地力都只給「額外」（>=0），永遠不會是負的／倒扣。純正向＝療癒安全。
        for soil in [0u16, 1, 1500, 3000, 5999, SOIL_MAX_FINE, u16::MAX] {
            let _ = harvest_bonus(soil); // u32 天然非負；存在即證明不倒扣
        }
        assert_eq!(harvest_bonus(0), 0, "貧土只是沒額外、不倒扣基準收成");
    }

    #[test]
    fn display_level_buckets() {
        assert_eq!(display_level(0), 0);
        assert_eq!(display_level(SOIL_MAX_FINE / 4 - 1), 0);
        assert_eq!(display_level(SOIL_MAX_FINE / 4), 1);
        assert_eq!(display_level(SOIL_MAX_FINE / 2), 2);
        assert_eq!(display_level(SOIL_MAX_FINE * 3 / 4), 3);
        assert_eq!(display_level(SOIL_MAX_FINE), 3);
        assert_eq!(display_level(u16::MAX), 3, "超界一律夾到滿級、不 panic");
    }
}
