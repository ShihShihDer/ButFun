//! ROADMAP 350 汲泉聚精——夜泉乙太從「一鍵領取」升級成「擺盪準星汲取小遊戲」。
//!
//! ROADMAP 162 的夜間乙太泉本是走近一鍵 +8 乙太的純被動領取。本切片給它第一個真玩法：
//! 玩家走近泉眼開始「汲取」，泉湧強弱在一條量表上來回擺盪——準星掃過「峰湧」甜蜜區時
//! 按鍵鎖定，停得越準汲到越多乙太（峰湧 ＞ 豐盈 ＞ 涓滴）。沒在窗口內鎖定就「汲取中斷」、
//! 泉眼留著可重試。
//!
//! 全純記憶體、純函式、零持久化、零 migration、零 LLM。
//! 刻意選一套**全新骨架**：準星在量表上「來回擺盪、停在甜蜜區」（timing-meter sweep），
//! 不同於 346 釣魚的反應計時、347 觀星的空間連線、348 採礦的 press-your-luck、349 料理的順序記憶。
//!
//! **平衡（誠實交代）**：把原本固定 8 乙太拆成憑技巧的 5／8／12——「豐盈」剛好等於原本 8，
//! 停得準（峰湧）才超過、隨便按（涓滴）反而更少。供給仍受既有限制（每夜 5 個泉眼、夜間限定、
//! 須出城探索），故對乙太經濟近乎零擾動、甚至對亂按的人略減。

/// 準星擺盪頻率（每秒掃完一個完整來回的次數）。週期 ＝ 1/SWEEP_HZ 秒。
/// 偏慢，讓玩家來得及瞄準、也壓低網路延遲造成的判定漂移。
pub const SWEEP_HZ: f32 = 0.6;

/// 汲取窗口逾時（秒）：開始汲取後這麼久沒鎖定就中斷（泉眼留著、可重試）。
pub const DRAW_TIMEOUT_SECS: f32 = 6.0;

/// 甜蜜區半寬：準星值（0..1）與中心 0.5 的距離在此內＝「峰湧」。
pub const SURGE_HALF: f32 = 0.06;
/// 「豐盈」半寬：距中心在此內（但超過 SURGE_HALF）＝「豐盈」；再外＝「涓滴」。
pub const BOUNTIFUL_HALF: f32 = 0.18;

/// 汲取到的乙太檔位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawBand {
    /// 涓滴：準星離甜蜜區太遠，汲到最少。
    Trickle,
    /// 豐盈：靠近峰湧（剛好等於原本一鍵 8 乙太）。
    Bountiful,
    /// 峰湧：正中甜蜜區，汲到最多。
    Surge,
}

impl DrawBand {
    /// 這一檔汲到的乙太。
    pub fn reward(self) -> u32 {
        match self {
            DrawBand::Trickle => 5,
            DrawBand::Bountiful => 8,
            DrawBand::Surge => 12,
        }
    }

    /// 前端飄字／報讀器用的中文標籤（i18n 佔位）。
    pub fn label(self) -> &'static str {
        match self {
            DrawBand::Trickle => "涓滴",
            DrawBand::Bountiful => "豐盈",
            DrawBand::Surge => "峰湧",
        }
    }

    /// 前端用的 snake_case 線格式。
    pub fn as_str(self) -> &'static str {
        match self {
            DrawBand::Trickle => "trickle",
            DrawBand::Bountiful => "bountiful",
            DrawBand::Surge => "surge",
        }
    }
}

/// 三角波準星：給定汲取經過時間（秒），回傳量表上的準星位置，值落在 [0,1]，
/// 從 0 線性掃到 1 再掃回 0（來回擺盪），週期 1/SWEEP_HZ 秒。
/// 純函式、確定可重現——伺服器鎖定時與前端渲染用同一條公式，準星位置一致。
pub fn cursor_at(elapsed_secs: f32) -> f32 {
    // 取相位 [0,1)：先 max(0) 守 elapsed 異常為負時不出界。
    let mut phase = (elapsed_secs.max(0.0) * SWEEP_HZ).fract();
    if phase < 0.0 {
        phase += 1.0; // fract 對負數回負；上方已 max(0)，此為雙保險
    }
    if phase < 0.5 {
        phase * 2.0
    } else {
        (1.0 - phase) * 2.0
    }
}

/// 依準星位置（0..1）判定汲取檔位。中心 0.5 為甜蜜區。
pub fn band_at(cursor: f32) -> DrawBand {
    let d = (cursor - 0.5).abs();
    if d <= SURGE_HALF {
        DrawBand::Surge
    } else if d <= BOUNTIFUL_HALF {
        DrawBand::Bountiful
    } else {
        DrawBand::Trickle
    }
}

/// 一趟進行中的汲取（記憶體前置、`Copy`、重啟清空）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AetherDraw {
    /// 正在汲取的泉眼節點 id（鎖定時用來 `try_collect`）。
    node_id: u32,
    /// 距開始汲取經過的時間（秒）。
    elapsed: f32,
}

impl AetherDraw {
    /// 開一趟新的汲取。
    pub fn start(node_id: u32) -> Self {
        AetherDraw { node_id, elapsed: 0.0 }
    }

    /// 正在汲取的泉眼 id。
    pub fn node_id(self) -> u32 {
        self.node_id
    }

    /// 距開始汲取經過的時間（秒）——隨快照廣播給前端，渲染準星位置。
    pub fn elapsed(self) -> f32 {
        self.elapsed
    }

    /// 目前準星位置（0..1）。
    pub fn cursor(self) -> f32 {
        cursor_at(self.elapsed)
    }

    /// 推進一個 tick：累加時間，回傳是否已逾時（呼叫端據此清掉這趟、泉眼留著可重試）。
    /// 負或零 dt 不前進（守時間單調）。
    pub fn advance(&mut self, dt: f32) -> bool {
        if dt > 0.0 {
            self.elapsed += dt;
        }
        self.elapsed >= DRAW_TIMEOUT_SECS
    }

    /// 鎖定（純函式、不改狀態）：回傳目前準星所在檔位，呼叫端據此給乙太、清狀態。
    pub fn lock(self) -> DrawBand {
        band_at(self.cursor())
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 一個完整擺盪週期的秒數。
    fn period() -> f32 {
        1.0 / SWEEP_HZ
    }

    #[test]
    fn cursor_within_bounds() {
        // 掃過多個時間點，準星永遠落在 [0,1]。
        for i in 0..1000 {
            let t = i as f32 * 0.013;
            let c = cursor_at(t);
            assert!((0.0..=1.0).contains(&c), "cursor_at({t}) = {c} 超出 [0,1]");
        }
    }

    #[test]
    fn cursor_sweeps_zero_center_one_center() {
        let p = period();
        // 起點在量表左端。
        assert!((cursor_at(0.0) - 0.0).abs() < 1e-4);
        // 1/4 週期掃到中心（甜蜜區）。
        assert!((cursor_at(p * 0.25) - 0.5).abs() < 1e-4);
        // 1/2 週期掃到右端。
        assert!((cursor_at(p * 0.5) - 1.0).abs() < 1e-4);
        // 3/4 週期掃回中心。
        assert!((cursor_at(p * 0.75) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn cursor_deterministic_and_periodic() {
        let p = period();
        assert_eq!(cursor_at(0.37), cursor_at(0.37));
        // 整數個週期後回到同一位置。
        assert!((cursor_at(0.37) - cursor_at(0.37 + p)).abs() < 1e-4);
    }

    #[test]
    fn cursor_handles_negative_elapsed() {
        // 異常負時間不 panic、不出界（夾在起點）。
        let c = cursor_at(-5.0);
        assert!((0.0..=1.0).contains(&c));
    }

    #[test]
    fn band_center_is_surge() {
        // 用安全落在甜蜜區內的值（避開浮點邊界毛邊）。
        assert_eq!(band_at(0.5), DrawBand::Surge);
        assert_eq!(band_at(0.5 + SURGE_HALF - 0.005), DrawBand::Surge, "甜蜜區內仍算峰湧");
        assert_eq!(band_at(0.5 - SURGE_HALF + 0.005), DrawBand::Surge);
    }

    #[test]
    fn band_mid_is_bountiful() {
        assert_eq!(band_at(0.5 + SURGE_HALF + 0.01), DrawBand::Bountiful);
        assert_eq!(band_at(0.5 + BOUNTIFUL_HALF - 0.005), DrawBand::Bountiful, "豐盈區內仍算豐盈");
    }

    #[test]
    fn band_edges_are_trickle() {
        assert_eq!(band_at(0.5 + BOUNTIFUL_HALF + 0.01), DrawBand::Trickle);
        assert_eq!(band_at(0.0), DrawBand::Trickle);
        assert_eq!(band_at(1.0), DrawBand::Trickle);
    }

    #[test]
    fn reward_increases_with_band() {
        assert!(DrawBand::Surge.reward() > DrawBand::Bountiful.reward());
        assert!(DrawBand::Bountiful.reward() > DrawBand::Trickle.reward());
        assert!(DrawBand::Trickle.reward() > 0);
    }

    #[test]
    fn bountiful_matches_legacy_flat_reward() {
        // 「豐盈」刻意對齊 ROADMAP 162 原本固定 8 乙太——停得準才超過、亂按反而更少。
        assert_eq!(
            DrawBand::Bountiful.reward(),
            crate::night_aether_springs::ETHER_REWARD
        );
    }

    #[test]
    fn wire_strings_stable() {
        assert_eq!(DrawBand::Trickle.as_str(), "trickle");
        assert_eq!(DrawBand::Bountiful.as_str(), "bountiful");
        assert_eq!(DrawBand::Surge.as_str(), "surge");
    }

    #[test]
    fn start_begins_at_left_edge() {
        let d = AetherDraw::start(7);
        assert_eq!(d.node_id(), 7);
        assert!((d.elapsed() - 0.0).abs() < 1e-6);
        assert!((d.cursor() - 0.0).abs() < 1e-4);
        // 起點在左端＝涓滴：剛開始就鎖一定吃虧，逼玩家等準星掃到甜蜜區。
        assert_eq!(d.lock(), DrawBand::Trickle);
    }

    #[test]
    fn advance_accumulates_and_times_out() {
        let mut d = AetherDraw::start(1);
        // 逾時前：advance 回 false。
        assert!(!d.advance(DRAW_TIMEOUT_SECS - 0.5));
        // 累過逾時門檻：回 true（呼叫端清狀態）。
        assert!(d.advance(1.0));
    }

    #[test]
    fn advance_ignores_non_positive_dt() {
        let mut d = AetherDraw::start(3);
        let before = d;
        assert!(!d.advance(0.0));
        assert!(!d.advance(-2.0));
        assert_eq!(d, before, "非正 dt 不應改變狀態");
    }

    #[test]
    fn lock_at_sweet_spot_is_surge() {
        // 推進到 1/4 週期（準星掃到中心）鎖定＝峰湧；證明「停得準」真有回報。
        let mut d = AetherDraw::start(9);
        d.advance(period() * 0.25);
        assert_eq!(d.lock(), DrawBand::Surge);
        assert_eq!(d.node_id(), 9, "推進不改 node_id");
    }
}
