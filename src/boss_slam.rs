//! 怪物王預警重擊（ROADMAP 424）。
//!
//! 既有怪物王（兇名精英，`level >= base_level + 3`）此前只會「指揮小怪」（boss_ai 117/371），
//! 自己沒有一記讀得到、躲得掉的招牌攻擊。本模組給牠補上**預警重擊**：
//! 每隔 `SLAM_CYCLE_SECS`，怪物王蓄力 `WINDUP_SECS`（地面浮現一圈逐漸填滿的紅色預警圈），
//! 蓄滿瞬間砸下一記範圍重擊，圈內玩家吃一發爆發傷害——
//! 但這傷害走與一般反擊**完全相同**的減傷鏈：護甲先扣、臨陣格擋（408）卸掉大部分、
//! 翻滾閃避（410）在恩典窗內完全閃開。於是 408/410 兩條防禦技第一次有了「明確該對付的威脅」，
//! 戰鬥從被動挨打變成「看招、應招」。
//!
//! 設計紀律（與既有戰鬥模組一致）：
//! - **零持久化、零 migration、零 LLM**：節奏完全由「伺服器時鐘 + 怪物 id」決定性推導，
//!   不存任何新狀態——既不在 `PlacedEnemy` 加欄、也不在 `AppState` 開表。
//! - 每隻怪物王由其 id 雜湊出一個**相位偏移**，讓不同怪物王錯開蓄力、不會整齊劃一一起砸。
//! - 純函式、可獨立測試，不碰 WebSocket / 遊戲迴圈。
//! - 伺服器權威：蓄力進度與重擊落點皆由後端算好廣播，前端只照著畫（鏡像 387 暴擊、410 閃避）。

/// 一次重擊循環的總長（秒）：蓄力 + 冷卻。
pub const SLAM_CYCLE_SECS: f64 = 11.0;
/// 蓄力（預警）時長（秒）：循環尾端這段時間地面浮現逐漸填滿的預警圈，給玩家反應窗。
pub const WINDUP_SECS: f64 = 1.4;
/// 重擊波及半徑（像素）：明顯大於一般接觸射程（`enemy_field::ATTACK_REACH`=64），是要「走出去／擋／滾」的 AoE。
pub const SLAM_RADIUS: f32 = 150.0;
/// 重擊基礎傷害（未減傷前）。
pub const SLAM_BASE_DAMAGE: u32 = 7;
/// 每級額外傷害：怪物王越高級、重擊越痛（與一般威脅隨級成長同向）。
pub const SLAM_DMG_PER_LEVEL: u32 = 1;
/// 重擊傷害硬上限：即使極高級也不致對闖入的低級玩家一擊必殺（仍可格擋／閃避完全化解）。
pub const SLAM_DAMAGE_CAP: u32 = 22;

/// 由怪物 id 雜湊出一個落在 `[0, SLAM_CYCLE_SECS)` 的相位偏移（秒），讓各怪物王錯開蓄力。
/// 決定性、無隨機——同一隻怪物王在伺服器重啟後相位一致。
fn phase_offset(id: (i32, i32, usize)) -> f64 {
    let h = (id.0 as u64)
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add((id.1 as u64).wrapping_mul(0xBF58476D1CE4E5B9))
        .wrapping_add((id.2 as u64).wrapping_mul(0x94D049BB133111EB));
    // 取雜湊低位映射到 [0,1) 再乘循環長度，得到 [0, CYCLE) 的相位偏移。
    let frac = (h % 100_000) as f64 / 100_000.0;
    frac * SLAM_CYCLE_SECS
}

/// 此刻在循環中的位置（秒），落在 `[0, SLAM_CYCLE_SECS)`。`now` 非有限時回 0（不汙染後續判斷）。
fn cycle_pos(id: (i32, i32, usize), now: f64) -> f64 {
    if !now.is_finite() {
        return 0.0;
    }
    (now + phase_offset(id)).rem_euclid(SLAM_CYCLE_SECS)
}

/// 此刻屬於第幾個循環（整數，跨循環即 +1）。用於偵測「剛跨過循環邊界 = 重擊落下」。
fn cycle_index(id: (i32, i32, usize), now: f64) -> i64 {
    if !now.is_finite() {
        return 0;
    }
    ((now + phase_offset(id)) / SLAM_CYCLE_SECS).floor() as i64
}

/// 蓄力進度：若此刻正在蓄力窗（循環尾端 `WINDUP_SECS`）內，回 `Some(0.0..=1.0)`（0=剛開始蓄、1=即將砸下）；
/// 否則回 `None`（沒在蓄力）。供快照廣播給前端畫填滿中的預警圈。
pub fn windup_progress(id: (i32, i32, usize), now: f64) -> Option<f32> {
    let p = cycle_pos(id, now);
    let windup_start = SLAM_CYCLE_SECS - WINDUP_SECS;
    if p >= windup_start {
        let prog = ((p - windup_start) / WINDUP_SECS).clamp(0.0, 1.0);
        Some(prog as f32)
    } else {
        None
    }
}

/// 偵測「在 `prev` 到 `now` 之間，這隻怪物王是否剛蓄滿砸下一記重擊」——
/// 亦即循環序號是否在這段時間內遞增了（跨過了循環邊界）。
/// 呼叫端每拍以 `(now - 上一拍間隔, now)` 詢問，確保每次循環恰好觸發一次。
pub fn just_struck(id: (i32, i32, usize), prev: f64, now: f64) -> bool {
    if !prev.is_finite() || !now.is_finite() || now <= prev {
        return false;
    }
    cycle_index(id, now) > cycle_index(id, prev)
}

/// 玩家是否落在以 `(bx, by)` 為心、`SLAM_RADIUS` 為半徑的重擊圈內。任一座標非有限即不命中。
pub fn is_in_blast(bx: f32, by: f32, px: f32, py: f32) -> bool {
    if !bx.is_finite() || !by.is_finite() || !px.is_finite() || !py.is_finite() {
        return false;
    }
    let dx = bx - px;
    let dy = by - py;
    dx * dx + dy * dy <= SLAM_RADIUS * SLAM_RADIUS
}

/// 某等級怪物王的重擊傷害（未減傷前）：基礎 + 每級加成，封頂於 `SLAM_DAMAGE_CAP`。
pub fn slam_damage(level: u32) -> u32 {
    (SLAM_BASE_DAMAGE + level.saturating_mul(SLAM_DMG_PER_LEVEL)).min(SLAM_DAMAGE_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: (i32, i32, usize) = (3, -2, 1);

    #[test]
    fn windup_only_in_tail_of_cycle() {
        // 構造一個 now 使 cycle_pos 落在循環中段（非蓄力窗）→ None。
        // 用相位偏移 0 的等效：直接掃整個循環，確認只有尾端 WINDUP_SECS 回 Some。
        let off = phase_offset(ID);
        let base = 1000.0 * SLAM_CYCLE_SECS; // 一個遠離 0、相位明確的時間錨
        // 循環中點：必非蓄力
        let mid = base - off + SLAM_CYCLE_SECS * 0.5;
        assert!(windup_progress(ID, mid).is_none());
        // 蓄力窗剛開始（尾端往前 WINDUP_SECS 處）：Some 且接近 0
        let start = base - off + (SLAM_CYCLE_SECS - WINDUP_SECS) + 0.01;
        let p0 = windup_progress(ID, start).expect("應在蓄力窗");
        assert!(p0 >= 0.0 && p0 < 0.2, "蓄力起點進度應接近 0，得 {p0}");
        // 蓄力即將結束（循環尾端）：Some 且接近 1
        let near_end = base - off + SLAM_CYCLE_SECS - 0.01;
        let p1 = windup_progress(ID, near_end).expect("應在蓄力窗");
        assert!(p1 > 0.9, "蓄力終點進度應接近 1，得 {p1}");
    }

    #[test]
    fn just_struck_fires_exactly_once_per_cycle() {
        // 以每秒一拍掃過兩個完整循環，統計觸發次數應恰為 2。
        let mut fires = 0;
        let total = (SLAM_CYCLE_SECS * 2.0).ceil() as i64 + 1;
        let mut prev = 0.0_f64;
        for s in 1..=total {
            let now = s as f64;
            if just_struck(ID, prev, now) {
                fires += 1;
            }
            prev = now;
        }
        assert_eq!(fires, 2, "兩個循環應恰好砸兩記，得 {fires}");
    }

    #[test]
    fn just_struck_rejects_degenerate_time() {
        assert!(!just_struck(ID, 5.0, 5.0)); // now == prev
        assert!(!just_struck(ID, 6.0, 5.0)); // 時間倒退
        assert!(!just_struck(ID, f64::NAN, 5.0));
        assert!(!just_struck(ID, 5.0, f64::INFINITY));
    }

    #[test]
    fn strike_aligns_with_windup_completion() {
        // 蓄力窗結束（進度趨近 1）的下一拍，just_struck 應為真——預警圈填滿即砸下。
        let off = phase_offset(ID);
        let base = 500.0 * SLAM_CYCLE_SECS;
        let near_end = base - off + SLAM_CYCLE_SECS - 0.05; // 蓄力即將滿
        assert!(windup_progress(ID, near_end).map(|p| p > 0.9).unwrap_or(false));
        // 跨過邊界後一拍
        let after = near_end + 0.1;
        assert!(just_struck(ID, near_end, after), "蓄滿跨界應觸發重擊");
        // 跨界後已不在蓄力窗（循環重新開始）
        assert!(windup_progress(ID, after).is_none());
    }

    #[test]
    fn blast_radius_membership() {
        assert!(is_in_blast(100.0, 100.0, 100.0, 100.0)); // 同點
        assert!(is_in_blast(100.0, 100.0, 100.0 + SLAM_RADIUS - 1.0, 100.0)); // 圈內
        assert!(!is_in_blast(100.0, 100.0, 100.0 + SLAM_RADIUS + 1.0, 100.0)); // 圈外
        assert!(!is_in_blast(f32::NAN, 100.0, 100.0, 100.0)); // 壞座標
    }

    #[test]
    fn slam_damage_scales_and_caps() {
        assert_eq!(slam_damage(1), SLAM_BASE_DAMAGE + 1);
        assert_eq!(slam_damage(6), SLAM_BASE_DAMAGE + 6);
        // 極高級被封頂
        assert_eq!(slam_damage(9999), SLAM_DAMAGE_CAP);
        assert!(slam_damage(9999) >= slam_damage(1));
    }

    #[test]
    fn different_bosses_desync() {
        // 兩隻不同 id 的怪物王，同一時刻的蓄力相位通常不同（相位偏移不同）。
        let a = (0, 0, 0);
        let b = (7, 13, 4);
        // 找一個讓 a 正在蓄力的時刻，b 多半不在（不要求恆成立，只驗相位確實不同）。
        assert!((phase_offset(a) - phase_offset(b)).abs() > 1e-9, "不同怪物王相位應不同");
    }

    #[test]
    fn cycle_pos_within_bounds() {
        for s in 0..50 {
            let p = cycle_pos(ID, s as f64 * 0.37);
            assert!(p >= 0.0 && p < SLAM_CYCLE_SECS);
        }
        // 非有限時間退 0，不 panic、不汙染
        assert_eq!(cycle_pos(ID, f64::NAN), 0.0);
    }
}
