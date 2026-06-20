//! ROADMAP 403 林間揮斧——伐木從「一鍵採一下」升級成「踩著節拍連揮的伐木小遊戲」。
//!
//! 樹（`gather::NodeKind::Tree`）是世上最古老的採集節點，自開服就只是「站旁邊一鍵 +1 木材」的
//! 純被動採集，從沒有任何技巧介入。本切片給這條被遺忘的維度第一個真玩法：玩家走近大樹按「🪓 伐木」
//! 開一趟「連揮」，斧落有節拍——一條穩定的拍子來回脈動，**踩在拍點上揮斧＝乾淨的一擊**，
//! 連續踩準節拍，最後一斧就能把整棵樹放倒、抱走滿懷木材；亂揮則只能勉強削下幾片。
//!
//! 全純記憶體、純函式、零持久化、零 migration、零 LLM。
//! 刻意選一套**全新骨架**：踩節拍的「節奏連擊」（rhythm combo）——
//! 不同於 346 釣魚的反應計時、347 觀星的空間連線、348 採礦的 press-your-luck、
//! 349 料理的順序記憶、350 汲泉的擺盪準星。本切片的精髓在「連續踩準拍點」累積乾淨擊數，
//! 乾淨擊數越多、放倒時抱走的木材越多。
//!
//! **平衡（誠實交代）**：與釣魚／採礦小遊戲一致，本玩法是「平行的採集活動」——
//! 發木材＋工匠熟練度，**不發等級經驗、不滾稀有度**（那些仍走既有一鍵 `Gather`）。
//! 放倒時最多吃掉整棵樹的耐久（5），不會無中生有；亂揮（0 乾淨擊）只拿 1 木材，
//! 與一鍵採一下同量級。供給仍受既有節點耐久與重生限制，對木材經濟近乎零擾動。

/// 拍子週期（秒）：一條穩定的節拍每這麼久脈動一次，玩家踩著拍點揮斧。
/// 偏慢，讓玩家來得及瞄準拍點、也壓低網路延遲造成的判定漂移。
pub const BEAT_SECS: f32 = 1.1;

/// 乾淨擊窗口（秒）：揮斧時刻與最近拍點的距離在此內＝「乾淨的一擊」。
/// 療癒向、刻意寬鬆好上手。
pub const HIT_WINDOW: f32 = 0.2;

/// 放倒一棵樹要揮幾斧（徒手／沒斧頭時）。
pub const STRIKES_TO_FELL: u8 = 4;

/// 帶斧頭時放倒一棵樹要揮幾斧（ROADMAP 433）：比徒手少一斧（更俐落、更快放倒）。
/// 刻意只少一斧而非砍半：保留「踩準節拍」的玩法份量，工具是錦上添花、不是跳過小遊戲。
pub const AXE_STRIKES_TO_FELL: u8 = 3;

/// 帶斧頭放倒時，額外多抱走的木材份數（ROADMAP 433）：鋒利的刃多削下一束。
/// 補回「斧數變少→可累積的乾淨擊上限變少→木材反而變少」這個反直覺缺口，
/// 讓斧頭在「速度」與「每棵樹產出」兩面都不會比徒手差——升級該是嚴格變好。
pub const AXE_WOOD_BONUS: u32 = 1;

/// 依「身上有沒有斧頭」回放倒一棵樹要揮的斧數。純查表、可測。
pub fn strikes_to_fell(has_axe: bool) -> u8 {
    if has_axe { AXE_STRIKES_TO_FELL } else { STRIKES_TO_FELL }
}

/// 連揮逾時（秒）：開揮後這麼久沒揮滿就放棄這趟（樹留著、可重來）。
pub const CHOP_TIMEOUT_SECS: f32 = 12.0;

/// 放倒後的冷卻（秒）：只擋「開新一趟連揮」，給節奏一點呼吸。
pub const CHOP_COOLDOWN_SECS: f32 = 1.5;

/// 給定揮斧時刻（距開揮經過的秒數），回傳該時刻在拍子上的相位 [0,1)。
/// 0＝正落在拍點上。純函式——前端用同一條公式渲染脈動的節拍環。
pub fn beat_fraction(elapsed_secs: f32) -> f32 {
    let mut frac = (elapsed_secs.max(0.0) / BEAT_SECS).fract();
    if frac < 0.0 {
        frac += 1.0; // fract 對負數回負；上方已 max(0)，此為雙保險
    }
    frac
}

/// 判定某時刻揮斧是否「踩準拍點」（乾淨的一擊）：距最近拍點在 `HIT_WINDOW` 秒內即乾淨。
/// 純函式、確定可重現——伺服器判定與前端渲染窗口用同一套標準。
pub fn is_clean_strike(elapsed_secs: f32) -> bool {
    let frac = beat_fraction(elapsed_secs);
    // 到最近拍點（0 或 1）的相位距離，換算回秒。
    let dist_secs = frac.min(1.0 - frac) * BEAT_SECS;
    dist_secs <= HIT_WINDOW
}

/// 放倒一棵樹時，依乾淨擊數＋有無斧頭決定一次吃掉幾段樹身耐久（＝抱走幾份木材）。
/// 乾淨擊越多、放倒越俐落、木材越多；全亂揮（0 乾淨）仍有 1，與一鍵採一下同量級。
/// 帶斧頭再 +`AXE_WOOD_BONUS`（鋒利的刃多削一束），補回斧數變少導致的乾淨擊上限縮減。
/// 上限是樹的耐久本身（在呼叫端以實際採到的份數封頂，不會無中生有）。
pub fn fell_takes(clean: u8, has_axe: bool) -> u32 {
    clean as u32 + 1 + if has_axe { AXE_WOOD_BONUS } else { 0 }
}

/// 放倒一棵樹得的工匠熟練度：與乾淨擊數同向（俐落的伐木更精進手藝）。
pub fn mastery_xp(clean: u8) -> u32 {
    clean as u32 + 1
}

/// 一趟進行中的連揮（記憶體前置、`Copy`、重啟清空）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChopSwing {
    /// 距開揮經過的時間（秒）。
    elapsed: f32,
    /// 已揮幾斧。
    strikes: u8,
    /// 其中踩準拍點的乾淨擊數。
    clean: u8,
    /// 開揮當下身上有沒有斧頭（決定放倒門檻與木材加成；一趟內鎖定，中途換裝不影響本趟）。
    has_axe: bool,
}

/// 一次揮斧的判定結果，供呼叫端廣播給前端演出（每斧一則、放倒那斧帶 `felled=true`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrikeResult {
    /// 這一斧是否踩準拍點。
    pub clean: bool,
    /// 累計已揮幾斧。
    pub strikes: u8,
    /// 累計乾淨擊數。
    pub total_clean: u8,
    /// 這一斧是否揮滿、放倒了樹。
    pub felled: bool,
}

impl ChopSwing {
    /// 開一趟新的連揮。`has_axe`＝開揮當下身上有沒有斧頭（鎖定本趟的放倒門檻與木材加成）。
    pub fn start(has_axe: bool) -> Self {
        ChopSwing { elapsed: 0.0, strikes: 0, clean: 0, has_axe }
    }

    /// 本趟開揮時身上有沒有斧頭（呼叫端結算木材時用，與放倒門檻一致）。
    pub fn has_axe(self) -> bool {
        self.has_axe
    }

    /// 距開揮經過的時間（秒）——隨快照廣播給前端，渲染節拍環的脈動相位。
    pub fn elapsed(self) -> f32 {
        self.elapsed
    }

    /// 已揮斧數。
    pub fn strikes(self) -> u8 {
        self.strikes
    }

    /// 乾淨擊數。
    pub fn clean(self) -> u8 {
        self.clean
    }

    /// 是否已揮滿（達放倒門檻）。門檻依本趟有無斧頭而定（帶斧頭更少斧即放倒）。
    pub fn is_complete(self) -> bool {
        self.strikes >= strikes_to_fell(self.has_axe)
    }

    /// 推進一個 tick：累加時間，回傳是否已逾時（呼叫端據此清掉這趟、樹留著可重來）。
    /// 負或零 dt 不前進（守時間單調）。
    pub fn advance(&mut self, dt: f32) -> bool {
        if dt > 0.0 {
            self.elapsed += dt;
        }
        self.elapsed >= CHOP_TIMEOUT_SECS
    }

    /// 揮一斧：以當下時刻判定是否踩準拍點，累加擊數／乾淨擊數，回傳這一斧的結果。
    /// 已揮滿後不再受理（呼叫端應在 `felled` 時即結算清狀態）。
    pub fn strike(&mut self) -> StrikeResult {
        let clean = is_clean_strike(self.elapsed);
        if clean {
            self.clean = self.clean.saturating_add(1);
        }
        self.strikes = self.strikes.saturating_add(1);
        StrikeResult {
            clean,
            strikes: self.strikes,
            total_clean: self.clean,
            felled: self.is_complete(),
        }
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_fraction_within_bounds() {
        for i in 0..1000 {
            let t = i as f32 * 0.017;
            let f = beat_fraction(t);
            assert!((0.0..1.0).contains(&f), "beat_fraction({t}) = {f} 超出 [0,1)");
        }
    }

    #[test]
    fn beat_fraction_zero_at_beats() {
        assert!(beat_fraction(0.0).abs() < 1e-4);
        assert!(beat_fraction(BEAT_SECS).abs() < 1e-4);
        assert!(beat_fraction(BEAT_SECS * 3.0).abs() < 1e-4);
    }

    #[test]
    fn beat_fraction_handles_negative() {
        // 異常負時間不 panic、不出界（夾在拍點起點）。
        let f = beat_fraction(-3.0);
        assert!((0.0..1.0).contains(&f));
    }

    #[test]
    fn clean_on_beat_dirty_off_beat() {
        // 正落在拍點上＝乾淨。
        assert!(is_clean_strike(0.0));
        assert!(is_clean_strike(BEAT_SECS));
        assert!(is_clean_strike(BEAT_SECS * 2.0));
        // 拍點 ± 窗口內仍乾淨。
        assert!(is_clean_strike(HIT_WINDOW - 0.01));
        assert!(is_clean_strike(BEAT_SECS - HIT_WINDOW + 0.01));
        // 兩拍正中間（離拍點最遠）＝不乾淨。
        assert!(!is_clean_strike(BEAT_SECS * 0.5));
        // 剛超出窗口＝不乾淨。
        assert!(!is_clean_strike(HIT_WINDOW + 0.02));
    }

    #[test]
    fn fell_takes_increases_with_clean() {
        assert_eq!(fell_takes(0, false), 1, "全亂揮仍有 1，與一鍵採一下同量級");
        assert!(fell_takes(STRIKES_TO_FELL, false) > fell_takes(0, false));
        for c in 0..STRIKES_TO_FELL {
            assert!(fell_takes(c + 1, false) > fell_takes(c, false), "乾淨擊越多木材越多");
        }
        // 全乾淨（4 擊）剛好吃掉整棵樹的耐久（5）。
        assert_eq!(fell_takes(STRIKES_TO_FELL, false), 5);
    }

    #[test]
    fn axe_fells_in_fewer_strikes() {
        // 斧頭把放倒門檻從 4 斧降到 3 斧（更快放倒），徒手仍 4 斧。
        assert_eq!(strikes_to_fell(false), STRIKES_TO_FELL);
        assert_eq!(strikes_to_fell(true), AXE_STRIKES_TO_FELL);
        assert!(AXE_STRIKES_TO_FELL < STRIKES_TO_FELL, "斧頭該嚴格更快");
    }

    #[test]
    fn axe_is_never_worse_on_wood() {
        // 升級鐵律：同樣乾淨擊數，帶斧頭抱走的木材 ≥ 徒手（多 AXE_WOOD_BONUS，不會反而更少）。
        for c in 0..=STRIKES_TO_FELL {
            assert!(fell_takes(c, true) >= fell_takes(c, false), "斧頭木材不該比徒手少");
            assert_eq!(fell_takes(c, true), fell_takes(c, false) + AXE_WOOD_BONUS);
        }
        // 各自滿乾淨的一棵樹：帶斧頭(3 乾淨)與徒手(4 乾淨)抱走的木材打平（速度才是淨賺）。
        assert_eq!(fell_takes(AXE_STRIKES_TO_FELL, true), fell_takes(STRIKES_TO_FELL, false));
    }

    #[test]
    fn axe_run_fells_at_three_strikes() {
        // 帶斧頭：揮滿 3 斧即放倒（第 3 斧 felled）、第 2 斧還沒。
        let mut c = ChopSwing::start(true);
        assert!(c.has_axe());
        let _ = c.strike();
        let r2 = c.strike();
        assert!(!r2.felled, "帶斧頭第 2 斧還沒放倒");
        let r3 = c.strike();
        assert!(r3.felled, "帶斧頭第 3 斧放倒");
        assert!(c.is_complete());
    }

    #[test]
    fn mastery_increases_with_clean() {
        assert!(mastery_xp(STRIKES_TO_FELL) > mastery_xp(0));
        assert!(mastery_xp(0) >= 1);
    }

    #[test]
    fn start_is_zeroed() {
        let c = ChopSwing::start(false);
        assert!((c.elapsed() - 0.0).abs() < 1e-6);
        assert_eq!(c.strikes(), 0);
        assert_eq!(c.clean(), 0);
        assert!(!c.is_complete());
    }

    #[test]
    fn advance_accumulates_and_times_out() {
        let mut c = ChopSwing::start(false);
        assert!(!c.advance(CHOP_TIMEOUT_SECS - 0.5));
        assert!(c.advance(1.0), "累過逾時門檻回 true");
    }

    #[test]
    fn advance_ignores_non_positive_dt() {
        let mut c = ChopSwing::start(false);
        let before = c;
        assert!(!c.advance(0.0));
        assert!(!c.advance(-2.0));
        assert_eq!(c, before, "非正 dt 不應改變狀態");
    }

    #[test]
    fn strike_counts_and_marks_clean() {
        let mut c = ChopSwing::start(false);
        // 開揮時 elapsed=0 落在拍點上＝乾淨。
        let r1 = c.strike();
        assert!(r1.clean);
        assert_eq!(r1.strikes, 1);
        assert_eq!(r1.total_clean, 1);
        assert!(!r1.felled);
        // 推進到兩拍正中間揮一斧＝不乾淨，乾淨擊數不增。
        c.advance(BEAT_SECS * 0.5);
        let r2 = c.strike();
        assert!(!r2.clean);
        assert_eq!(r2.strikes, 2);
        assert_eq!(r2.total_clean, 1);
    }

    #[test]
    fn fells_at_threshold() {
        let mut c = ChopSwing::start(false);
        let mut last = c.strike();
        for _ in 1..STRIKES_TO_FELL {
            last = c.strike();
        }
        assert!(last.felled, "揮滿 STRIKES_TO_FELL 斧即放倒");
        assert!(c.is_complete());
    }

    #[test]
    fn all_clean_run_fells_with_full_haul() {
        // 每一斧都踩在拍點上：累積滿乾淨擊、放倒時吃掉整棵樹。
        let mut c = ChopSwing::start(false);
        let mut r = c.strike();
        for k in 1..STRIKES_TO_FELL {
            c.advance(BEAT_SECS); // 整拍推進，仍落在拍點上
            let _ = k;
            r = c.strike();
        }
        assert!(r.felled);
        assert_eq!(r.total_clean, STRIKES_TO_FELL);
        assert_eq!(fell_takes(r.total_clean, false), 5);
    }
}
