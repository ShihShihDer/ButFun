//! 破綻時機（ROADMAP 488）——給「走近自動開打」的戰鬥加上一層「看準時機」的主動深度。
//!
//! 至今戰鬥是站定就自動結算傷害，玩家對「何時出手」沒有任何能動性。本模組讓**兇名精英**
//! （notorious，level ≥ base_level+3）在纏鬥中**週期性露出可乘的破綻**：破綻開啟的短短一窗內
//! 命中，傷害大幅加成（破綻直擊）。玩家學會「等牠露破綻再砍」，自動戰鬥第一次有了節奏與判斷。
//!
//! 設計取捨：
//! - **只有兇名精英有破綻**——大眾雜魚維持秒殺手感、不增認知負擔；菁英才值得「讀招」。
//!   （成本／視覺紀律：破綻光環只在少數精英身上，不洗版。）
//! - **療癒向、非硬核**：破綻只會「幫」玩家（多打傷害），錯過不懲罰，窗也夠寬（數秒），
//!   不是要求精準幀的動作遊戲——對齊本作「療癒多人世界」的調性。
//! - **零新狀態、零持久化**：破綻相位完全由「敵人 id + 當前時刻」確定性推導（鏡像 424
//!   怪物王蓄力 boss_slam 的零狀態做法），不在 PlacedEnemy 上存任何欄位、不動建構處與存檔。
//!
//! 本檔為**純函式**（`is_open` / `bonus_power` / `phase_offset`）方便單元測試；
//! 唯一非純的 `now_secs` 只是取系統時鐘的接線小工具，game.rs 快照與 ws.rs 傷害結算兩處
//! 各自呼叫它取得**同一道牆上時鐘**（wall clock），故「畫出來的破綻光」與「實際加成窗」一致。

use std::time::{SystemTime, UNIX_EPOCH};

/// 一輪破綻週期長度（秒）：閉合一段時間後短暫開啟，循環往復。
pub const WEAKPOINT_CYCLE_SECS: f64 = 9.0;
/// 破綻開啟的時長（秒）：一輪週期裡可乘之窗。刻意比動作遊戲寬（療癒向、好抓）。
pub const WEAKPOINT_OPEN_SECS: f64 = 2.2;
/// 破綻直擊的傷害加成分子／分母：`power × NUM / DEN`（此處 = ×1.5，+50%）。
pub const WEAKPOINT_BONUS_NUM: u32 = 3;
pub const WEAKPOINT_BONUS_DEN: u32 = 2;

/// 依敵人 id 取一個穩定的相位偏移（0..CYCLE）：讓場上各精英的破綻**錯開閃**，
/// 不會整片同步開合（既不擾眼、也不會出現「全場同時可乘」的失衡瞬間）。
/// 確定性雜湊——同一隻怪永遠同一相位。
pub fn phase_offset(id: (i32, i32, usize)) -> f64 {
    // 簡單 FNV 風格混合，足夠把相鄰 id 打散到不同相位。
    let mut h: u64 = 1469598103934665603;
    for v in [id.0 as i64 as u64, id.1 as i64 as u64, id.2 as u64] {
        h ^= v;
        h = h.wrapping_mul(1099511628211);
    }
    // 取 0..1 再乘週期 → 0..CYCLE。
    let frac = (h % 100_000) as f64 / 100_000.0;
    frac * WEAKPOINT_CYCLE_SECS
}

/// 給定敵人 id 與當前時刻（秒），此刻破綻是否開啟。確定性、可測。
/// `now_secs` 非有限時保守回 `false`（不讓壞時鐘誤開破綻）。
/// 注意：本函式**不判斷是否兇名精英**——那由呼叫端把關（只對 notorious 敵人問破綻）。
pub fn is_open(id: (i32, i32, usize), now_secs: f64) -> bool {
    if !now_secs.is_finite() {
        return false;
    }
    let t = (now_secs + phase_offset(id)).rem_euclid(WEAKPOINT_CYCLE_SECS);
    t < WEAKPOINT_OPEN_SECS
}

/// 破綻直擊：把基礎傷害放大為破綻傷害（×NUM/DEN）。飽和、不溢位。
pub fn bonus_power(power: u32) -> u32 {
    ((power as u64 * WEAKPOINT_BONUS_NUM as u64) / WEAKPOINT_BONUS_DEN as u64).min(u32::MAX as u64)
        as u32
}

/// 取目前牆上時鐘秒數（接線用，非純函式）。取不到時回 0.0。
pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_offset_is_deterministic_and_in_range() {
        let id = (3, -7, 5);
        let a = phase_offset(id);
        let b = phase_offset(id);
        assert_eq!(a, b, "同一 id 相位必須穩定");
        assert!((0.0..WEAKPOINT_CYCLE_SECS).contains(&a), "相位需落在 0..週期");
    }

    #[test]
    fn phase_offset_spreads_across_ids() {
        // 相鄰 id 應落在不同相位（雜湊有打散），避免整片同步。
        let a = phase_offset((0, 0, 0));
        let b = phase_offset((0, 0, 1));
        let c = phase_offset((1, 0, 0));
        assert!(a != b || a != c, "相鄰 id 不該全部同相位");
    }

    #[test]
    fn is_open_cycles_open_then_closed() {
        // 用相位 0 的 id 來推：以「扣掉自身相位」反推一個讓 t≈0 的時刻。
        let id = (0, 0, 0);
        let off = phase_offset(id);
        // t = (now + off) mod CYCLE；要 t=0.0 取 now = CYCLE - off（落在 [0,CYCLE)）。
        let base = WEAKPOINT_CYCLE_SECS - off;
        assert!(is_open(id, base), "窗口起點應為開啟");
        assert!(
            is_open(id, base + WEAKPOINT_OPEN_SECS - 0.01),
            "開啟時長內仍為開"
        );
        assert!(
            !is_open(id, base + WEAKPOINT_OPEN_SECS + 0.01),
            "超過開啟時長即閉合"
        );
        // 過一整個週期後再次開啟（循環）。
        assert!(is_open(id, base + WEAKPOINT_CYCLE_SECS), "下一輪週期再次開啟");
    }

    #[test]
    fn is_open_guards_non_finite() {
        assert!(!is_open((1, 1, 1), f64::NAN));
        assert!(!is_open((1, 1, 1), f64::INFINITY));
    }

    #[test]
    fn bonus_power_is_one_and_half_and_saturates() {
        assert_eq!(bonus_power(0), 0);
        assert_eq!(bonus_power(10), 15);
        assert_eq!(bonus_power(7), 10); // 21/2 向下取整
        assert_eq!(bonus_power(u32::MAX), u32::MAX); // 不溢位、夾在上限
    }

    #[test]
    fn open_window_fraction_is_minority_of_cycle() {
        // 破綻只佔一輪的一小段（不是大半時間都開），確保「等時機」有意義。
        assert!(WEAKPOINT_OPEN_SECS < WEAKPOINT_CYCLE_SECS / 2.0);
    }
}
