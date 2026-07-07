//! 乙太方界·放養雞 v1（自主提案切片，ROADMAP 870）——世界環境軸線第二種可馴服動物。
//!
//! **真缺口**：847 起野兔讓 wildlife 系統長出「閒晃／受驚／馴服／跟隨／生寶寶」一整套
//! 生機，但整個系統至今只有這一種陸地生物；雞舍（880/`voxel_coop`）雖已補上「動物產物」
//! 這條資源軸，卻是一座你放下就不再互動的靜態方塊——世界從沒有一種「你親手馴服、之後
//! 牠會跟著你走、還會主動回饋你」的活生生動物。本刀讓陪伴與資源產出第一次疊在同一隻
//! 生物身上：一隻在村莊周圍啄食閒晃的雞，餵牠一把種子馴服後，牠會跟著你到處走，
//! 而且會定期主動生一顆蛋（掉在牠腳邊，走近就能撿起）。
//!
//! **與 847 野兔的差異**：雞不怕人——沒有受驚/逃跑分支（`should_flee`/`flee_target`
//! 用不到，永遠只是閒晃或跟隨），用小麥種子（`vfarm::SEEDS_ID`）而非胡蘿蔔馴服，
//! 馴服後除了跟隨還會定期下蛋。跟隨/馴服判定沿用 `voxel_wildlife` 既有的通用純函式
//! （`should_tame`/`should_follow`/`should_close_follow_gap`），本模組只補雞專屬的
//! 「馴服回饋句」與「下蛋節奏」兩件事，避免重複造輪子。
//!
//! **與 880 雞舍（`voxel_coop`）的差異**：雞舍是靜態方塊、被動產出、放下不用管；
//! 放養雞需要你主動追上馴服，此後牠跟著你到處走、蛋是牠走到哪生到哪（掉落物，
//! 需要你走近撿），是「陪伴型」的第二條產蛋管道，兩者互不取代——蛋物品本身共用
//! `vcoop::EGG_ID`，沒有另開新物品 id。
//!
//! **v1 刻意收斂**：不做繁殖（有需求時可比照 855 兔子繁殖延伸）、不分玩家身份（任何
//! 靠近的玩家都能被跟）、無寵物 UI。零 LLM、零鎖、零 IO——落地/跟隨/下蛋的 tick 驅動、
//! 掉落物落地、快照廣播全在 `voxel_ws.rs`。

/// 已馴服的雞平均隔多久下一次蛋（秒）。
pub const LAY_INTERVAL_SECS: f32 = 90.0;

/// 下蛋間隔的隨機浮動量（± 秒）——避免同時馴服的多隻雞蛋鐘完全同步、下蛋節奏顯得機械。
/// 必須小於 [`LAY_INTERVAL_SECS`]，確保算出來的冷卻永遠是正數。
pub const LAY_JITTER_SECS: f32 = 40.0;

/// 依 `roll`（呼叫端傳 `rand::random::<f32>()`，理論範圍 `[0,1)`）算出下一次下蛋要等的
/// 秒數：純函式、確定性、可測。`roll` 越界（<0 或 >1，理論上不會發生但防禦性 clamp）
/// 一律先夾回 `[0,1]` 再換算，永不產生負值或不合理極端值。
pub fn next_lay_cooldown(roll: f32) -> f32 {
    let r = roll.clamp(0.0, 1.0);
    LAY_INTERVAL_SECS + (r * 2.0 - 1.0) * LAY_JITTER_SECS
}

/// 已馴服的雞這一 tick 該不該下蛋：冷卻倒數歸零（或以下）即觸發。純函式、零狀態。
pub fn should_lay(lay_cd: f32) -> bool {
    lay_cd <= 0.0
}

/// 馴服成功那一刻的回饋句（確定性輪替，`pick` 由呼叫端提供隨機源）。與野兔的胡蘿蔔
/// 馴服句（🥕）刻意用不同的收成意象（🌾）區隔兩種動物。
const TAME_LINES: [&str; 4] = [
    "🌾 牠歪著頭啄了幾口，不再對你保持警戒。",
    "🌾 牠安穩地啄食你手心裡的種子，看來是認得你了。",
    "🌾 牠咕咕叫了兩聲，湊過來多啄了幾口。",
    "🌾 牠啄完最後一粒種子，抬頭盯著你——像是在等下一把。",
];

/// 依 `pick` 取一句馴服回饋（越界安全取模，永不 panic）。
pub fn tame_line(pick: usize) -> &'static str {
    TAME_LINES[pick % TAME_LINES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_smaller_than_interval_keeps_cooldown_positive() {
        // 設計不變量：JITTER 必須小於 INTERVAL，否則低 roll 值會算出負冷卻（一放就下蛋）。
        assert!(LAY_JITTER_SECS < LAY_INTERVAL_SECS);
    }

    #[test]
    fn next_lay_cooldown_midpoint_is_exact_interval() {
        assert!((next_lay_cooldown(0.5) - LAY_INTERVAL_SECS).abs() < 1e-4);
    }

    #[test]
    fn next_lay_cooldown_bounds_at_roll_extremes() {
        let lo = next_lay_cooldown(0.0);
        let hi = next_lay_cooldown(1.0);
        assert!((lo - (LAY_INTERVAL_SECS - LAY_JITTER_SECS)).abs() < 1e-4);
        assert!((hi - (LAY_INTERVAL_SECS + LAY_JITTER_SECS)).abs() < 1e-4);
        assert!(lo < hi);
    }

    #[test]
    fn next_lay_cooldown_clamps_out_of_range_roll() {
        // 理論上呼叫端不會給界外值，但防禦性 clamp 不該 panic 或產生更極端的結果。
        let below = next_lay_cooldown(-1.0);
        let above = next_lay_cooldown(2.0);
        assert!((below - (LAY_INTERVAL_SECS - LAY_JITTER_SECS)).abs() < 1e-4);
        assert!((above - (LAY_INTERVAL_SECS + LAY_JITTER_SECS)).abs() < 1e-4);
    }

    #[test]
    fn next_lay_cooldown_always_positive() {
        for i in 0..=20 {
            let roll = i as f32 / 20.0;
            assert!(next_lay_cooldown(roll) > 0.0);
        }
    }

    #[test]
    fn should_lay_triggers_at_or_below_zero() {
        assert!(should_lay(0.0));
        assert!(should_lay(-0.5));
        assert!(!should_lay(0.01));
    }

    #[test]
    fn tame_line_picks_vary_and_stay_nonempty() {
        let lines: std::collections::HashSet<&str> = (0..TAME_LINES.len()).map(tame_line).collect();
        assert_eq!(lines.len(), TAME_LINES.len(), "四句應各不相同");
        for l in &lines {
            assert!(!l.is_empty());
        }
    }

    #[test]
    fn tame_line_index_wraps_without_panic() {
        let _ = tame_line(usize::MAX);
        let _ = tame_line(TAME_LINES.len());
        let _ = tame_line(0);
    }
}
