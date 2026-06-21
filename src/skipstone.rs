//! ROADMAP 475 打水漂——水域第一次有了「玩」的動詞。
//!
//! 水（`Biome::Water`）至今只是釣魚（47）的場景：站在水邊一鍵垂釣、等浮標。除此之外，
//! 滿世界的池塘、河灣對玩家而言只是一片過不去的藍——能看、能釣，卻碰不到、玩不起來。
//! 本切片給水第一個純粹「跟它玩」的動作：撿顆石頭，蓄一道力道、抓準時機甩手，
//! 石頭便貼著水面一路彈跳出去——力道拿捏得好，漂得又遠又多跳，水面盪開一圈圈漣漪；
//! 用力過猛或太軟，石頭「噗通」一聲沉下去。鄰近玩家都看得見你甩出的這顆石頭與漣漪。
//!
//! ## 設計鐵律
//! - **全新骨架（蓄力定時釋放）**：不同於 346 釣魚的「等咬鉤反應」、403 伐木的「踩節拍連擊」、
//!   348 採礦的 press-your-luck、350 汲泉的擺盪準星。本切片的精髓是**一道來回擺盪的力道計**——
//!   蓄力時力道從 0 漲到滿、再落回 0 來回擺盪，**在甜蜜點放手＝甩得最漂亮**（跳最多次）；
//!   偏離甜蜜點越遠、跳得越少，最差就「噗通」沉底（仍記 1 跳，不會零回饋）。
//! - **純邏輯可測**：`gauge_value`／`skip_count`／`StoneSkip` 皆純函式、`Copy`、確定可重現、
//!   無副作用、無 IO。力道計的「源頭數值」定在這裡，前端 `skipGaugeValue`／`skipStoneCount`
//!   鏡像同一組常數做渲染與演出——一份契約、兩邊一致。
//! - **記憶體前置、零持久化、零 migration**：蓄力中的甩石是 `Player` 上的 `Option<StoneSkip>`
//!   暫態（鏡像 `chopping`／`fishing`／`aether_draw`），斷線／重啟清零、不存檔。
//! - **療癒向、零平衡風險**：打水漂純粹是「跟水玩」的療癒小動作——**不送物品／乙太／戰力／經驗、
//!   不改任何機制**（誠實比照放風箏 470、植樹 370 上線時的純景物定位）。零 LLM、零經濟擾動。

/// 力道計擺盪一個完整來回（0→滿→0）所需秒數。偏慢，讓玩家來得及瞄準甜蜜點、
/// 也壓低網路延遲造成的判定漂移。
pub const GAUGE_SECS: f32 = 1.4;

/// 甜蜜點——力道計上「甩得最漂亮」的位置 [0,1]。刻意取偏高（0.82）而非滿格：
/// 打水漂要「強而貼水」的低平拋投，用盡全力反而砸進水裡，故甜蜜點在高處但不在頂點。
pub const SWEET_SPOT: f32 = 0.82;

/// 每偏離甜蜜點這麼多力道，就少跳一次。療癒向、刻意寬鬆好上手。
pub const SKIP_FALLOFF: f32 = 0.13;

/// 一顆漂亮石頭最多在水面彈跳幾次（甜蜜點放手）。
pub const MAX_SKIPS: u32 = 5;

/// 最少跳幾次——再差的甩投，石頭至少「噗通」濺起一圈漣漪（不給零回饋）。
pub const MIN_SKIPS: u32 = 1;

/// 蓄力逾時（秒）：開蓄後這麼久沒放手就自動鬆手作罷（石頭沒甩出、可重來）。
/// 約三個來回的力道擺盪，足夠瞄準也不會卡著占狀態。
pub const THROW_TIMEOUT_SECS: f32 = 8.0;

/// 甩石後的冷卻（秒）：只擋「撿下一顆石頭開蓄」，給節奏一點呼吸。
pub const SKIP_COOLDOWN_SECS: f32 = 1.2;

/// 由蓄力經過秒數算力道計當下值 [0,1]——一道三角波：前半漲（0→1）、後半落（1→0）來回擺盪。
/// 0＝力道見底、1＝力道滿格。純函式——前端用同一條公式渲染擺盪的力道條。
/// 負時間（異常）保守視為 0（夾在起點），永不出界、永不產生 NaN。
pub fn gauge_value(elapsed_secs: f32) -> f32 {
    if !elapsed_secs.is_finite() || elapsed_secs <= 0.0 {
        return 0.0;
    }
    let phase = (elapsed_secs / GAUGE_SECS).fract(); // [0,1)
    if phase < 0.5 {
        phase * 2.0 // 前半段：0 → 1
    } else {
        2.0 - phase * 2.0 // 後半段：1 → 0
    }
}

/// 由放手當下的力道值算石頭在水面彈跳幾次。
/// 力道越貼近甜蜜點跳越多（甜蜜點＝`MAX_SKIPS`），每偏離 `SKIP_FALLOFF` 少一跳，
/// 最差仍有 `MIN_SKIPS`（噗通沉底也濺一圈）。力道壞值（NaN／±∞）保守回 `MIN_SKIPS`。
pub fn skip_count(gauge: f32) -> u32 {
    if !gauge.is_finite() {
        return MIN_SKIPS;
    }
    let g = gauge.clamp(0.0, 1.0);
    let dist = (g - SWEET_SPOT).abs();
    let lost = (dist / SKIP_FALLOFF).floor() as u32;
    MAX_SKIPS.saturating_sub(lost).max(MIN_SKIPS)
}

/// 一趟進行中的蓄力甩石（記憶體前置、`Copy`、重啟清空）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoneSkip {
    /// 距開蓄經過的時間（秒）——驅動力道計的擺盪相位。
    elapsed: f32,
}

impl StoneSkip {
    /// 撿起一顆石頭、開始蓄力。
    pub fn start() -> Self {
        StoneSkip { elapsed: 0.0 }
    }

    /// 距開蓄經過的時間（秒）——隨快照廣播給前端，渲染力道條的擺盪相位。
    pub fn elapsed(self) -> f32 {
        self.elapsed
    }

    /// 力道計當下值 [0,1]（便利方法，等同 `gauge_value(self.elapsed())`）。
    pub fn gauge(self) -> f32 {
        gauge_value(self.elapsed)
    }

    /// 推進一個 tick：累加時間，回傳是否已逾時（呼叫端據此清掉這趟、作罷重來）。
    /// 負或零 dt 不前進（守時間單調）。
    pub fn advance(&mut self, dt: f32) -> bool {
        if dt > 0.0 {
            self.elapsed += dt;
        }
        self.elapsed >= THROW_TIMEOUT_SECS
    }

    /// 放手甩出：以當下力道值算出石頭彈跳次數。
    pub fn release(self) -> u32 {
        skip_count(self.gauge())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 力道值恆在區間內() {
        for i in 0..2000 {
            let t = i as f32 * 0.013;
            let v = gauge_value(t);
            assert!((0.0..=1.0).contains(&v), "gauge_value({t}) = {v} 超出 [0,1]");
        }
    }

    #[test]
    fn 力道計兩端見底中央滿格() {
        // 起點力道見底。
        assert!(gauge_value(0.0).abs() < 1e-6);
        // 半個週期＝力道滿格。
        assert!((gauge_value(GAUGE_SECS * 0.5) - 1.0).abs() < 1e-4);
        // 一個完整週期回到見底。
        assert!(gauge_value(GAUGE_SECS).abs() < 1e-4);
        // 一個半週期又回到滿格（來回擺盪）。
        assert!((gauge_value(GAUGE_SECS * 1.5) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn 負時間與壞值保守見底() {
        assert!(gauge_value(-3.0).abs() < 1e-6);
        assert!(gauge_value(f32::NAN).abs() < 1e-6);
        assert!(gauge_value(f32::INFINITY).abs() < 1e-6);
    }

    #[test]
    fn 甜蜜點跳最多次() {
        assert_eq!(skip_count(SWEET_SPOT), MAX_SKIPS);
    }

    #[test]
    fn 偏離甜蜜點越遠跳越少() {
        // 往兩側遞減：每隔一個 falloff 少一跳，且嚴格不增。
        let mut prev = skip_count(SWEET_SPOT);
        let mut g = SWEET_SPOT;
        while g < 1.0 {
            g += SKIP_FALLOFF;
            let cur = skip_count(g.min(1.0));
            assert!(cur <= prev, "往上偏離應不增：{prev} → {cur} @ g={g}");
            prev = cur;
        }
        let mut prev = skip_count(SWEET_SPOT);
        let mut g = SWEET_SPOT;
        while g > 0.0 {
            g -= SKIP_FALLOFF;
            let cur = skip_count(g.max(0.0));
            assert!(cur <= prev, "往下偏離應不增：{prev} → {cur} @ g={g}");
            prev = cur;
        }
    }

    #[test]
    fn 兩端極弱極猛都至少一跳() {
        assert_eq!(skip_count(0.0), MIN_SKIPS);
        assert_eq!(skip_count(1.0).max(MIN_SKIPS), skip_count(1.0));
        assert!(skip_count(0.0) >= MIN_SKIPS);
        assert!(skip_count(1.0) >= MIN_SKIPS);
    }

    #[test]
    fn 跳次數壞值保守回最少() {
        assert_eq!(skip_count(f32::NAN), MIN_SKIPS);
        assert_eq!(skip_count(f32::INFINITY), MIN_SKIPS);
        assert_eq!(skip_count(f32::NEG_INFINITY), MIN_SKIPS);
    }

    #[test]
    fn 跳次數恆在合理範圍() {
        for i in 0..=100 {
            let g = i as f32 / 100.0;
            let n = skip_count(g);
            assert!((MIN_SKIPS..=MAX_SKIPS).contains(&n), "skip_count({g}) = {n} 越界");
        }
    }

    #[test]
    fn 開蓄歸零() {
        let s = StoneSkip::start();
        assert!((s.elapsed() - 0.0).abs() < 1e-6);
        assert!(s.gauge().abs() < 1e-6);
    }

    #[test]
    fn 推進累時且逾時回真() {
        let mut s = StoneSkip::start();
        assert!(!s.advance(THROW_TIMEOUT_SECS - 0.5));
        assert!(s.advance(1.0), "累過逾時門檻回 true");
    }

    #[test]
    fn 非正dt不前進() {
        let mut s = StoneSkip::start();
        let before = s;
        assert!(!s.advance(0.0));
        assert!(!s.advance(-2.0));
        assert_eq!(s, before, "非正 dt 不應改變狀態");
    }

    #[test]
    fn 甜蜜點放手甩出最多跳() {
        // 推進到半週期（力道滿格 1.0）……其實甜蜜點在 0.82，非滿格。
        // 直接驗：放手值由 release 走 skip_count(gauge)，甜蜜點力道時跳最多。
        let mut s = StoneSkip::start();
        // 找到力道計剛好掃過甜蜜點的時刻（前半段漲到 0.82：phase=0.41→ t=0.41*GAUGE_SECS）。
        let t = 0.41 * GAUGE_SECS;
        s.advance(t);
        assert!((s.gauge() - SWEET_SPOT).abs() < 0.02, "該時刻力道接近甜蜜點");
        assert_eq!(s.release(), MAX_SKIPS);
    }

    #[test]
    fn 同輸入同輸出可重現() {
        assert_eq!(gauge_value(0.37), gauge_value(0.37));
        assert_eq!(skip_count(0.6), skip_count(0.6));
    }
}
