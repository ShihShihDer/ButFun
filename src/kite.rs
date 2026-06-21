//! ROADMAP 470 放風箏——玩家第一次能跟世界風（430）互動。
//!
//! 430 給世界吹起一陣伺服器權威、全服共享、隨時間緩緩轉向的風，但風至今只會吹**樹／作物**
//! （搖曳）與**天氣粒子**（432 雨絲斜飛）——玩家始終只能旁觀這陣風，碰不到它。本切片給玩家
//! 第一個「握住這陣風」的動作：拿出風箏，它順著全服共享的世界風飄揚、起風時越飛越高、隨陣風
//! 左右擺盪；鄰近玩家都看得見你的風箏，且大家的風箏都朝**同一個風向**傾斜——「全服同享一片天」
//! 的血脈（彩虹 361）第一次變成玩家能一起玩的東西。
//!
//! ## 設計鐵律
//! - **純邏輯可測**：`can_fly_kite`／`kite_soar`／`kite_sway_amp` 皆純函式、確定可重現、無副作用。
//!   風箏飛行幾何（高度、傾斜、擺幅）的「源頭數值」定在這裡，前端 `kiteFlightSpec` 鏡像同一組
//!   常數做渲染——一份契約、兩邊一致。
//! - **記憶體前置、零持久化、零 migration**：放不放風箏是 `Player` 上的暫態 bool（鏡像
//!   `busking`／`guard_shield`／`dodging`），斷線／重啟清零、不存檔。
//! - **玩家一眼有感且社交**：放風箏（`flying_kite: bool`）放進 `PlayerView` 快照廣播，前端對
//!   放風箏的玩家頭頂畫一只順風飄揚的風箏，旁觀者一眼看見「有人在放風箏」、整片天的風箏同向斜飛。
//! - **療癒向、零平衡風險**：放風箏純粹是「跟風玩」的療癒小動作——**不送物品／乙太／戰力／經驗、
//!   不改任何冷卻或機制**（誠實比照植樹 370 上線時的純景物定位）。零 LLM、零經濟擾動。

/// 風箏在無風（晴天 strength≈0）時仍維持的最低飛行高度因子 [0,1]——
/// 線一拉直，風箏在微風（430 晴天保底一縷微風）裡也飄得起來、不會癱在地上。
pub const SOAR_FLOOR: f32 = 0.35;

/// 風力對飛行高度的增益：高度因子＝`SOAR_FLOOR + SOAR_GAIN × 風強`，夾在 [0,1]。
/// 取 0.65，使滿風（strength=1）時高度因子剛好抵達 1.0（飛到最高）。
pub const SOAR_GAIN: f32 = 0.65;

/// 風箏左右擺盪的基礎擺幅因子——即使近乎無風，風箏也會極輕微地晃。
pub const SWAY_BASE: f32 = 0.20;

/// 風力對擺幅的增益：擺幅因子＝`SWAY_BASE + SWAY_GAIN × 風強`。風越大、風箏擺得越兇。
pub const SWAY_GAIN: f32 = 0.80;

/// 能不能放風箏：倒地（休息復原中）時放不了，其餘皆可。純查表、可測。
pub fn can_fly_kite(downed: bool) -> bool {
    !downed
}

/// 由世界風強度算風箏飛行高度因子 [0,1]。
/// 風越強飛越高（單調遞增）；無風時回 `SOAR_FLOOR`（微風裡仍飄得起）；
/// 風強壞值（NaN／±∞）保守回 `SOAR_FLOOR`、永不產生 NaN。
pub fn kite_soar(wind_strength: f32) -> f32 {
    if !wind_strength.is_finite() {
        return SOAR_FLOOR;
    }
    let s = wind_strength.clamp(0.0, 1.0);
    (SOAR_FLOOR + SOAR_GAIN * s).clamp(0.0, 1.0)
}

/// 由世界風強度算風箏擺幅因子（≥0）。風越大擺幅越大；壞值保守回 `SWAY_BASE`。
pub fn kite_sway_amp(wind_strength: f32) -> f32 {
    if !wind_strength.is_finite() {
        return SWAY_BASE;
    }
    let s = wind_strength.clamp(0.0, 1.0);
    SWAY_BASE + SWAY_GAIN * s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 倒地放不了風箏其餘皆可() {
        assert!(can_fly_kite(false));
        assert!(!can_fly_kite(true));
    }

    #[test]
    fn 無風時回最低飛行高度() {
        assert!((kite_soar(0.0) - SOAR_FLOOR).abs() < 1e-6);
    }

    #[test]
    fn 滿風時飛到最高() {
        assert!((kite_soar(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn 飛行高度隨風單調遞增() {
        let mut prev = kite_soar(0.0);
        for i in 1..=10 {
            let cur = kite_soar(i as f32 / 10.0);
            assert!(cur >= prev, "高度應隨風不減：{} → {}", prev, cur);
            prev = cur;
        }
    }

    #[test]
    fn 飛行高度恆在區間內() {
        for &s in &[-5.0_f32, -0.1, 0.0, 0.3, 0.7, 1.0, 2.0, 100.0] {
            let h = kite_soar(s);
            assert!((0.0..=1.0).contains(&h), "高度越界：strength={} → {}", s, h);
        }
    }

    #[test]
    fn 飛行高度壞值保守回最低() {
        assert!((kite_soar(f32::NAN) - SOAR_FLOOR).abs() < 1e-6);
        assert!((kite_soar(f32::INFINITY) - SOAR_FLOOR).abs() < 1e-6);
        assert!((kite_soar(f32::NEG_INFINITY) - SOAR_FLOOR).abs() < 1e-6);
    }

    #[test]
    fn 擺幅隨風變大() {
        assert!(kite_sway_amp(1.0) > kite_sway_amp(0.0));
        assert!((kite_sway_amp(0.0) - SWAY_BASE).abs() < 1e-6);
    }

    #[test]
    fn 擺幅壞值保守回基礎() {
        assert!((kite_sway_amp(f32::NAN) - SWAY_BASE).abs() < 1e-6);
        assert!((kite_sway_amp(f32::INFINITY) - SWAY_BASE).abs() < 1e-6);
    }

    #[test]
    fn 同輸入同輸出可重現() {
        assert_eq!(kite_soar(0.42), kite_soar(0.42));
        assert_eq!(kite_sway_amp(0.42), kite_sway_amp(0.42));
    }
}
