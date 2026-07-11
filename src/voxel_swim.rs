//! 乙太方界·水體游泳深化 v1——下水游泳/潛水的手感純邏輯（自主提案切片，ROADMAP 930）。
//!
//! **真缺口**：世界早有水體（河/湖/海）、垂釣（794/841）、水中游魚（848）、溺水扣血
//! （[`crate::voxel_player_stats::tick_drown`]）。但「在水裡」本身的體驗單薄——下水只有
//! 「往下沉＋撐過緩衝就扣血」的負面感受，沒有游泳該有的浮力、水阻、可浮可潛的手感。
//! 本切片把下水從「純負面懲罰」轉成「有浮力、有水阻、可按跳浮起／潛下的療癒游泳」。
//!
//! **架構定位（重要）**：實際的玩家移動是**前端權威預測**（見 `web/voxel/main.js` 的
//! `update()`）——游泳物理在前端做，做成與陸地移動不衝突的「水中分支」，絕不動既有陸地手感。
//! 本 Rust 模組是把游泳的**純數學（浮力、水平水阻、憋氣曲線）抽成可單元測試的函式**，
//! 前端以相同常數鏡像實作（比照 `dawnMistStrength` 前端純函式的既有慣例）。這樣手感的
//! 數學有一份權威測試，改參數時有回歸保護。
//!
//! **療癒不懲罰鐵律**：憋氣曲線與既有溺水機制**相容且向後相容**——頭在水面上時完全不消耗
//! （沿用 [`tick_drown`] 離水即歸零的溫柔設計），只有「潛到頭沒頂水下」才慢慢累積；且潛水
//! 緩衝比溺水扣血緩衝**更寬**，絕不變成硬核憋氣死亡。浮力預設偏正（放開按鍵會慢慢浮上水面），
//! 讓「不小心沉下去」也能自然回到水面透氣。
//!
//! [`tick_drown`]: crate::voxel_player_stats::tick_drown

/// 水中重力倍率：只剩陸地重力的一小截——人在水裡靠浮力抵消大半重力，只會**緩緩**下沉。
/// 與陸地重力（前端 GRAVITY=24 格/秒²）相乘後得到水中的等效下沉加速度。
pub const WATER_GRAVITY_MULT: f32 = 0.18;

/// 浮力上浮加速度（格/秒²）：不按任何鍵時，人會被水**溫柔地往上托**，慢慢浮回水面。
/// 刻意略大於「水中重力」（GRAVITY*WATER_GRAVITY_MULT ≈ 4.32），使**中性略偏上浮**——
/// 只要頭沒頂水下、又沒主動下潛，人會緩緩上浮透氣（避免不小心沉下去出不來）。
pub const BUOYANCY_ACCEL: f32 = 4.8;

/// 主動上浮加速度（格/秒²）：按跳（Space/跳鈕）時往上游的力道，明顯大於被動浮力，
/// 讓「按著跳＝浮起來」一按就有感。
pub const SWIM_UP_ACCEL: f32 = 16.0;

/// 主動下潛加速度（格/秒²）：按下潛（Shift/潛鈕）時往下游的力道；配合水阻不會一路暴衝。
pub const SWIM_DOWN_ACCEL: f32 = 14.0;

/// 垂直速度的水阻夾限（格/秒）：水裡上下游的最大速度都被夾在此範圍，
/// 營造「在水裡動作變沉、變慢」的黏滯感，也避免潛水暴衝穿越地形。
pub const VERT_SPEED_CLAMP: f32 = 5.0;

/// 水平游泳速度倍率：水中水平移動比陸地慢（水阻），只剩陸地速度的六成。
/// 前端以 SPEED（5 格/秒）相乘 → 水中約 3 格/秒。
pub const SWIM_HORIZ_SPEED_MULT: f32 = 0.6;

/// 潛水憋氣緩衝（秒）：頭沒頂水下後，撐這麼久才開始消耗「氣」。
/// 刻意比溺水扣血緩衝（[`DROWN_GRACE_SECS`]=6）更寬鬆——潛下去先給充裕的探索時間，
/// 憋氣表都還沒開始掉，先玩夠了才慢慢有壓力。療癒不懲罰。
///
/// [`DROWN_GRACE_SECS`]: crate::voxel_player_stats::DROWN_GRACE_SECS
pub const BREATH_GRACE_SECS: f32 = 8.0;

/// 一口氣可憋的總時長（秒，含緩衝）：從頭沒頂到氣見底的總秒數。
/// 憋氣表（0..1）在 `[BREATH_GRACE_SECS, BREATH_FULL_SECS]` 這段線性從 1 掉到 0。
/// 注意：氣見底**不等於**死亡——扣血仍完全交給既有的溫柔 [`tick_drown`]（緩衝＋慢速扣血），
/// 憋氣表只是給玩家一個「該上去透氣了」的視覺提示，不是新的傷害來源。
///
/// [`tick_drown`]: crate::voxel_player_stats::tick_drown
pub const BREATH_FULL_SECS: f32 = 20.0;

/// 水中等效下沉加速度（格/秒²，正值＝往下）。純函式。
/// `land_gravity`：陸地重力加速度（前端 GRAVITY）。
pub fn water_sink_accel(land_gravity: f32) -> f32 {
    land_gravity * WATER_GRAVITY_MULT
}

/// 依這一幀的輸入意圖，算水中垂直加速度（格/秒²，**向上為正**）。純函式、確定性、可測。
///
/// - `land_gravity`：陸地重力加速度（前端 GRAVITY，正值）。
/// - `swim_up`：這幀是否按著「上浮」（Space/跳鈕）。
/// - `swim_down`：這幀是否按著「下潛」（Shift/潛鈕）。
/// - 三態：按上浮＝強力往上；按下潛＝往下（先抵消浮力再下沉）；都不按＝被動浮力（略偏上浮）。
pub fn swim_vertical_accel(land_gravity: f32, swim_up: bool, swim_down: bool) -> f32 {
    if swim_up {
        // 主動上浮：上浮力道扣掉水中重力（淨往上）。
        SWIM_UP_ACCEL - water_sink_accel(land_gravity)
    } else if swim_down {
        // 主動下潛：下潛力道扣掉被動浮力（淨往下 → 回傳負）。
        -(SWIM_DOWN_ACCEL - BUOYANCY_ACCEL)
    } else {
        // 被動：浮力往上抵消水中重力（淨值略偏上浮，見常數註解）。
        BUOYANCY_ACCEL - water_sink_accel(land_gravity)
    }
}

/// 對垂直速度施加水阻夾限（格/秒）。純函式：把 `vy` 夾進 `[-CLAMP, +CLAMP]`。
pub fn clamp_water_vy(vy: f32) -> f32 {
    vy.clamp(-VERT_SPEED_CLAMP, VERT_SPEED_CLAMP)
}

/// 水中水平移動速度（格/秒）。純函式：陸地速度乘水阻倍率。
pub fn swim_horiz_speed(land_speed: f32) -> f32 {
    land_speed * SWIM_HORIZ_SPEED_MULT
}

/// 憋氣表推進一 tick（純函式）：回傳新的「頭沒頂水下累計秒」。
///
/// - `head_underwater`：這幀頭是否沒頂在水下（比溺水判定嚴格：頭那格是水才算）。
/// - 頭在水面上（`false`）→ 立即歸零（沿用溺水的溫柔設計，一透氣就回滿）。
/// - 頭沒頂 → 累加 dt。
pub fn tick_breath(head_underwater: bool, submerged_acc: f32, dt: f32) -> f32 {
    if !head_underwater {
        0.0
    } else {
        submerged_acc + dt
    }
}

/// 憋氣表顯示值（0..1，1＝滿氣、0＝氣見底）。純函式、確定性、可測。
///
/// - `submerged_acc`：頭沒頂水下的累計秒（來自 [`tick_breath`]）。
/// - 緩衝期（<= [`BREATH_GRACE_SECS`]）內維持滿氣 1.0（潛下去先不掉，給探索時間）。
/// - 緩衝後到 [`BREATH_FULL_SECS`] 之間線性從 1 掉到 0。
/// - 壞值（NaN/負）一律保守回 1.0（滿氣、永不誤扣、永不爆）。
pub fn breath_fraction(submerged_acc: f32) -> f32 {
    if submerged_acc.is_nan() || submerged_acc <= BREATH_GRACE_SECS {
        return 1.0;
    }
    if submerged_acc >= BREATH_FULL_SECS {
        return 0.0;
    }
    let span = BREATH_FULL_SECS - BREATH_GRACE_SECS;
    (1.0 - (submerged_acc - BREATH_GRACE_SECS) / span).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: f32 = 24.0; // 前端 GRAVITY 鏡像值

    #[test]
    fn water_sink_is_gentle_fraction_of_land_gravity() {
        let s = water_sink_accel(G);
        assert!(s > 0.0, "水中仍有下沉傾向");
        assert!(s < G, "水中下沉遠慢於陸地自由落體");
        assert!((s - G * WATER_GRAVITY_MULT).abs() < 1e-4);
    }

    #[test]
    fn swim_up_pushes_net_upward() {
        let a = swim_vertical_accel(G, true, false);
        assert!(a > 0.0, "按上浮應淨往上");
        // 主動上浮遠強於被動浮力
        assert!(a > swim_vertical_accel(G, false, false));
    }

    #[test]
    fn swim_down_pushes_net_downward() {
        let a = swim_vertical_accel(G, false, true);
        assert!(a < 0.0, "按下潛應淨往下");
    }

    #[test]
    fn passive_is_slightly_buoyant_upward() {
        // 都不按時，被動浮力略勝水中重力 → 淨往上（會慢慢浮回水面透氣）。
        let a = swim_vertical_accel(G, false, false);
        assert!(
            a > 0.0,
            "被動應略偏上浮（浮力 {} > 水中重力 {:.2}）",
            BUOYANCY_ACCEL,
            water_sink_accel(G)
        );
        // 但淨上浮很溫柔，遠小於主動上浮。
        assert!(a < 1.5, "被動上浮應溫柔");
    }

    #[test]
    fn up_and_down_both_pressed_prefers_up() {
        // 同時按上下：上浮優先（跳鍵語意優先於下潛），不會互相抵消成 0。
        let a = swim_vertical_accel(G, true, true);
        assert!(a > 0.0);
    }

    #[test]
    fn vy_is_clamped_both_directions() {
        assert_eq!(clamp_water_vy(100.0), VERT_SPEED_CLAMP);
        assert_eq!(clamp_water_vy(-100.0), -VERT_SPEED_CLAMP);
        assert_eq!(clamp_water_vy(1.0), 1.0);
    }

    #[test]
    fn horiz_speed_is_slower_in_water() {
        let land = 5.0;
        let s = swim_horiz_speed(land);
        assert!(s < land, "水中水平比陸地慢");
        assert!(s > 0.0);
        assert!((s - land * SWIM_HORIZ_SPEED_MULT).abs() < 1e-4);
    }

    #[test]
    fn breath_resets_on_surfacing() {
        // 頭一出水面 → 累計立即歸零（一透氣就回滿）。
        assert_eq!(tick_breath(false, 15.0, 0.1), 0.0);
    }

    #[test]
    fn breath_accumulates_while_submerged() {
        let a = tick_breath(true, 1.0, 0.5);
        assert!((a - 1.5).abs() < 1e-6);
    }

    #[test]
    fn breath_fraction_full_during_grace() {
        // 緩衝期內維持滿氣（潛下去先不掉）。
        assert_eq!(breath_fraction(0.0), 1.0);
        assert_eq!(breath_fraction(BREATH_GRACE_SECS - 0.01), 1.0);
        assert_eq!(breath_fraction(BREATH_GRACE_SECS), 1.0);
    }

    #[test]
    fn breath_fraction_declines_after_grace() {
        let mid = breath_fraction((BREATH_GRACE_SECS + BREATH_FULL_SECS) / 2.0);
        assert!(mid > 0.0 && mid < 1.0, "緩衝後半途應在 0..1 之間，實得 {mid}");
    }

    #[test]
    fn breath_fraction_bottoms_out_at_full_secs() {
        assert_eq!(breath_fraction(BREATH_FULL_SECS), 0.0);
        assert_eq!(breath_fraction(BREATH_FULL_SECS + 5.0), 0.0);
    }

    #[test]
    fn breath_grace_more_generous_than_drown_grace() {
        // 憋氣表緩衝比溺水扣血緩衝更寬鬆（療癒：先給探索時間）。
        assert!(
            BREATH_GRACE_SECS > crate::voxel_player_stats::DROWN_GRACE_SECS,
            "憋氣緩衝({}) 應比溺水緩衝({}) 更寬",
            BREATH_GRACE_SECS,
            crate::voxel_player_stats::DROWN_GRACE_SECS
        );
    }

    #[test]
    fn breath_fraction_bad_values_safe() {
        // 壞值保守回滿氣，永不誤扣/爆。
        assert_eq!(breath_fraction(f32::NAN), 1.0);
        assert_eq!(breath_fraction(f32::INFINITY), 0.0); // +∞ >= FULL → 0（也不爆）
        assert_eq!(breath_fraction(-5.0), 1.0);
    }
}
