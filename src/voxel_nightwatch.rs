//! 乙太方界·夜裡點燈守望 v1——把居民對夜之威脅的被動「躲回家」升級成**集體主動守望**。
//!
//! 北極星是「AI 居民湧現出一個小社會」。暗影生物 v1（`voxel_shadow.rs`）已上線：夜間村外
//! 暗處生成柔和光影、光圈是庇護、居民見到只會怕著回家（絕不掉血）。**這一刀補的缺口**是：
//! 居民對夜的威脅除了怕、還會**一起做點什麼**——入夜見暗影靠近時，就近朝暗處親手點起一盞
//! 火把、口裡念一句「天黑了，點上燈吧」。一盞盞燈在夜裡亮起，村莊邊緣長出一圈守望的暖光。
//!
//! 調性是**療癒的集體守望**，不是打怪：
//!   - 點的是既有光源方塊（火把 [`Block::Torch`]），走既有 `set_block`＋持久化＋廣播管線。
//!   - **自然收斂、不洗燈**：只在「附近還沒有光」的暗處點（[`WATCH_MIN_LIGHT_SPACING`]）——
//!     村邊一旦被燈圈圍住就再也找不到夠暗的點，居民自動停手（無需硬性計數也會收斂）；
//!     另有每位冷卻 + 全村一夜上限雙重安全閥。
//!   - 點下的火把讓亮區擴大，暗影誤入即化輕煙（沿用 `voxel_shadow` 既有「光=庇護」機制）——
//!     所以這是真正接進既有系統的**集體防禦**，不是純裝飾。
//!
//! 這裡只放「與連線/鎖無關」的確定性純邏輯（觸發判定/選點/收斂條件/台詞），全部抽成可測純
//! 函式；真正的 tick 驅動、快照、set_block、廣播、持久化都在 `voxel_ws.rs::tick_nightwatch`
//! （嚴守 prod 死鎖鐵律：各 store 短鎖循序取放、不巢狀、IO/廣播全在鎖外）。零 LLM。

use crate::voxel::Block;
use crate::voxel_shadow as vshadow;
use crate::voxel_time::TimePhase;

// ── 調性參數（集中一處，日後平衡好調）───────────────────────────────────────────

/// 守望檢查間隔（秒）：低頻掃一次「哪位居民身邊有暗影、該點燈了」——非 60fps 熱迴圈，
/// 成本比照暗影 tick 的零頭。刻意比暗影生成間隔稍慢，讓燈一盞一盞從容亮起。
pub const WATCH_CHECK_SECS: f32 = 4.0;
/// 居民察覺暗影、動念點燈的半徑（方塊）：暗影漂到這麼近，居民就會就近點盞燈守望——
/// 比暗影害怕半徑（[`vshadow::FEAR_RADIUS`] = 9）大一圈，讓「先點燈、再退家」的順序自然成立。
pub const WATCH_NOTICE_RADIUS: f32 = 16.0;
/// 點燈位置：朝暗影方向、離居民這麼遠處放（方塊）。把光往暗處推一小步（守望的姿態），
/// 又不至於離人太遠變成憑空冒燈。
pub const WATCH_PLACE_DIST: f32 = 3.0;
/// 點燈最小間距（方塊）：選點附近已有光源（火把/燈/營火）就不重複點——
/// 這是**自然收斂**的核心：村邊被燈圈圍住後就找不到夠暗的點，居民自動停手、不洗燈。
pub const WATCH_MIN_LIGHT_SPACING: f32 = 7.0;
/// 同一位居民兩次點燈的冷卻（秒）：一位居民不會一口氣連點，讓守望是全村輪流的從容節奏。
pub const WATCH_COOLDOWN_SECS: f32 = 40.0;
/// 全村一夜點燈上限：最後一道安全閥，即使收斂/冷卻因故失效也絕不把世界鋪滿火把。
/// 黎明重置（隨一夜守望狀態）。
pub const WATCH_MAX_LAMPS_PER_NIGHT: u32 = 24;

/// 守望點的光源方塊：火把（Torch = 31）——最樸素溫暖的一盞，與暗影「光=庇護」同一組。
pub const WATCH_LAMP_BLOCK: Block = Block::Torch;

// ── 面向玩家字串（集中一處，i18n 友善）──────────────────────────────────────────

/// 今夜第一盞守望燈亮起時的 Feed（一夜一次）：告訴玩家村民正在一起守著夜。
pub const WATCH_FEED_KIND: &str = "點燈守望";
pub const WATCH_FEED_ACTOR: &str = "村子";
pub const WATCH_FEED_DETAIL: &str = "夜裡暗影出沒，村民們點起一盞盞火把，一起守著溫暖的夜。";

/// 居民點燈時冒的泡泡台詞池（彼此提醒「天黑了，點上燈吧」的守望氛圍）。
pub const WATCH_LINES: &[&str] = &[
    "天黑了，點上燈吧！",
    "來，這裡再添一盞燈。",
    "有燈就不怕，大家守著點。",
    "把燈點亮，暗影就不敢近。",
];

/// 依 seed 從點燈台詞池挑一句（確定性、可測）。
pub fn watch_line(seed: usize) -> &'static str {
    WATCH_LINES[seed % WATCH_LINES.len()]
}

// ── 觸發判定 / 選點 / 收斂（全部純函式、可測）──────────────────────────────────

/// 守望時段：與暗影活動時段一致（入夜過渡 Evening＋深夜 Night）。黎明/白天不守望。
pub fn is_watch_time(phase: TimePhase) -> bool {
    vshadow::is_shadow_time(phase)
}

/// 這夜是否還在點燈上限內（達 [`WATCH_MAX_LAMPS_PER_NIGHT`] 即停手）。純函式、可測。
pub fn under_night_cap(lamps_tonight: u32) -> bool {
    lamps_tonight < WATCH_MAX_LAMPS_PER_NIGHT
}

/// 找居民（腳底 x,z）[`WATCH_NOTICE_RADIUS`] 內最近的一隻暗影水平座標（供決定點燈方向）。
/// 都太遠回 None（這位居民此刻不需點燈）。`shadows` 給 (x, y, z)，只用水平面判距。純函式、可測。
pub fn nearest_shadow_within(
    rx: f32,
    rz: f32,
    shadows: &[(f32, f32, f32)],
    radius: f32,
) -> Option<(f32, f32)> {
    let r2 = radius * radius;
    let mut best: Option<((f32, f32), f32)> = None;
    for &(sx, _sy, sz) in shadows {
        let dx = sx - rx;
        let dz = sz - rz;
        let d2 = dx * dx + dz * dz;
        if d2 > r2 {
            continue;
        }
        if best.map_or(true, |(_, bd2)| d2 < bd2) {
            best = Some(((sx, sz), d2));
        }
    }
    best.map(|(p, _)| p)
}

/// 由居民位置朝暗影方向、[`WATCH_PLACE_DIST`] 遠處算出要放火把的柱座標（世界整數 x,z）。
/// 「把光往暗處推一小步」的守望姿態。居民與暗影重合（極少數退化情形）時朝 +x 放。純函式、可測。
pub fn lamp_column(rx: f32, rz: f32, sx: f32, sz: f32) -> (i32, i32) {
    let dx = sx - rx;
    let dz = sz - rz;
    let len = (dx * dx + dz * dz).sqrt();
    let (ux, uz) = if len < 1e-4 { (1.0, 0.0) } else { (dx / len, dz / len) };
    let px = rx + ux * WATCH_PLACE_DIST;
    let pz = rz + uz * WATCH_PLACE_DIST;
    (px.floor() as i32, pz.floor() as i32)
}

/// 這個候選點是否「夠暗、值得點一盞」：附近 [`spacing`] 內沒有任何既有光源才算暗。
/// 這是自然收斂的關鍵——燈越點越多、暗點越少，最後全暗處被填滿就沒得點了。純函式、可測。
pub fn spot_is_dark(cx: i32, cy: i32, cz: i32, lights: &[(i32, i32, i32)], spacing: f32) -> bool {
    let s2 = spacing * spacing;
    !lights.iter().any(|&(lx, ly, lz)| {
        let dx = (lx - cx) as f32;
        let dy = (ly - cy) as f32;
        let dz = (lz - cz) as f32;
        dx * dx + dy * dy + dz * dz <= s2
    })
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_time_matches_shadow_time() {
        // 守望時段必須與暗影活動時段完全一致（有暗影才需要守望）。
        assert!(is_watch_time(TimePhase::Night), "深夜要守望");
        assert!(is_watch_time(TimePhase::Evening), "入夜過渡就開始守望");
        assert!(!is_watch_time(TimePhase::Dawn), "黎明暗影散、不再守望");
        assert!(!is_watch_time(TimePhase::Day), "白天沒有暗影、不守望");
        assert!(!is_watch_time(TimePhase::Dusk), "黃昏還沒入夜");
    }

    #[test]
    fn night_cap_stops_flooding() {
        assert!(under_night_cap(0), "一夜之初可以點");
        assert!(under_night_cap(WATCH_MAX_LAMPS_PER_NIGHT - 1), "上限前一盞仍可點");
        assert!(!under_night_cap(WATCH_MAX_LAMPS_PER_NIGHT), "到上限就停手（安全閥）");
        assert!(!under_night_cap(WATCH_MAX_LAMPS_PER_NIGHT + 5), "超過也停");
    }

    #[test]
    fn nearest_shadow_within_picks_closest() {
        let shadows = vec![(20.0, 5.0, 0.0), (4.0, 5.0, 3.0), (100.0, 5.0, 0.0)];
        // 居民在原點，通知半徑內最近的是 (4,3)。
        let got = nearest_shadow_within(0.0, 0.0, &shadows, WATCH_NOTICE_RADIUS);
        assert_eq!(got, Some((4.0, 3.0)), "挑通知半徑內最近的暗影");
        // 全部超出通知半徑 → None（這位不需點燈）。
        let far = vec![(WATCH_NOTICE_RADIUS + 5.0, 5.0, 0.0)];
        assert_eq!(nearest_shadow_within(0.0, 0.0, &far, WATCH_NOTICE_RADIUS), None, "太遠不點");
        assert_eq!(nearest_shadow_within(0.0, 0.0, &[], WATCH_NOTICE_RADIUS), None, "沒暗影不點");
    }

    #[test]
    fn lamp_column_pushes_toward_shadow() {
        // 暗影在 +x 正方向 → 火把放在居民 +x 側（把光往暗處推）。
        let (cx, cz) = lamp_column(0.0, 0.0, 10.0, 0.0);
        assert_eq!(cx, WATCH_PLACE_DIST.floor() as i32, "朝 +x 推 WATCH_PLACE_DIST 格");
        assert_eq!(cz, 0, "純 +x 方向 z 不變");
        // 暗影在 -z 方向 → 火把放在 -z 側。
        let (_cx2, cz2) = lamp_column(0.0, 0.0, 0.0, -10.0);
        assert!(cz2 < 0, "朝 -z 推");
        // 退化：居民與暗影重合 → 朝 +x 放（不 panic、不除以 0）。
        let (cx3, cz3) = lamp_column(5.0, 5.0, 5.0, 5.0);
        assert_eq!((cx3, cz3), ((5.0 + WATCH_PLACE_DIST).floor() as i32, 5), "重合時朝 +x 退化放置");
    }

    #[test]
    fn spot_is_dark_enforces_spacing() {
        // 已有一盞燈在 (0,10,0)。
        let lights = vec![(0, 10, 0)];
        // 間距內的候選點 → 不夠暗（不重複點，自然收斂）。
        assert!(!spot_is_dark(3, 10, 0, &lights, WATCH_MIN_LIGHT_SPACING), "既有燈旁不重複點");
        // 間距外的候選點 → 夠暗、值得點。
        let far = WATCH_MIN_LIGHT_SPACING as i32 + 2;
        assert!(spot_is_dark(far, 10, 0, &lights, WATCH_MIN_LIGHT_SPACING), "夠遠的暗處值得點");
        // 全無光源 → 一定夠暗。
        assert!(spot_is_dark(0, 10, 0, &[], WATCH_MIN_LIGHT_SPACING), "無光處必暗");
    }

    #[test]
    fn watch_line_deterministic_and_nonempty() {
        for s in 0..8 {
            assert!(!watch_line(s).is_empty(), "台詞不得空");
        }
        assert_eq!(watch_line(0), watch_line(WATCH_LINES.len()), "seed 取模循環");
    }

    #[test]
    fn lamp_block_is_a_shelter_light() {
        // 點下去的火把必須是暗影認得的庇護光源，否則「集體防禦」接不上既有機制。
        assert!(vshadow::is_light_block(WATCH_LAMP_BLOCK), "守望燈必須是庇護光源");
    }
}
