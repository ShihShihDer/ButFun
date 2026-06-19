//! ROADMAP 423 蓄力重擊——戰鬥進攻第一個技巧：按住蓄力、放開打出一記重擊。
//!
//! 408 臨陣格擋、410 翻滾閃避給了戰鬥兩條**防禦**路（凝盾硬擋／翻身閃開）；本切片補上
//! **進攻**的第一個技巧——**蓄力重擊**：按住攻擊鈕凝聚乙太，蓄得越久、放開那一擊越重
//! （半蓄 ×1.5、滿蓄 ×2.0），讓「揮一刀」第一次有了「輕揮」與「蓄力重砍」的手感分別。
//!
//! 全純記憶體、純函式、零持久化、零 migration、零 LLM。
//!
//! **換骨架（戰鬥的第一個進攻技巧，明確別於既有兩招防禦）**：
//!   - 408／410 都是**防禦**（卸傷／免傷、被敵人威脅時用）；蓄力重擊是**進攻**
//!     （放大自己打出去那一擊、主動出手時用），是戰鬥套路裡互補的另一極。
//!   - 不同於暴擊（ROADMAP 387，CritStrike 附魔每 5 次自動雙傷、玩家不可控）：蓄力**全憑玩家
//!     手感**——願意花時間蓄就換一記重擊，是技巧表達而非被動觸發。
//!
//! **平衡（誠實交代，純手感向、零數值膨脹、零經濟擾動）**：蓄力重擊**不是 DPS 升級、反而更低**——
//!   滿蓄 ×2.0 要先花 `CHARGE_FULL_SECS`(≈1.1s) 蓄力＋一次攻擊冷卻＋放開後 `CHARGE_COOLDOWN_SECS`(2.5s)
//!   才能再蓄，平均一記重擊的時間裡，連續輕揮（攻擊冷卻 0.6s）的總傷其實更高。它換的是**爆發**
//!   （一刀把高血精英的血條打掉一大截、配合 387 傷害數字的爽快），不是穩定輸出，故**不碰整體強度平衡**：
//!   不改掉落／經驗／擊倒速率（攻擊仍走既有 `Attack` 管線），只是把「這一擊」的傷害乘上去。
//!   與暴擊／連殺／元素／戰吼正交相乘（同既有 `power` 乘法鏈），戰吼群攻不吃蓄力（與暴擊一致的取捨）。

/// 起蓄到「半蓄」的門檻（秒）：蓄不到這麼久放開＝輕揮（無加成、不耗冷卻、可立即再來）。
/// 讓「點一下就放」仍是順手的普通攻擊，蓄力是額外的選擇而非負擔。
pub const CHARGE_MIN_SECS: f32 = 0.45;

/// 蓄到「滿蓄」的門檻（秒）：蓄滿這麼久即達最高檔，再按住也不會更重（`advance` 把時間夾在此）。
pub const CHARGE_FULL_SECS: f32 = 1.1;

/// 放開重擊後的冷卻（秒）：> 蓄滿耗時，確保蓄力是「換爆發」而非「疊 DPS」——
/// 連續蓄力打不過連續輕揮的總傷（見模組註解的平衡交代）。
pub const CHARGE_COOLDOWN_SECS: f32 = 2.5;

/// 蓄好的重擊「待擊」存活窗（秒）：放開後這麼久內的下一次攻擊吃到加成，逾時自動消散
/// （避免蓄了不打、把重擊無限期存著）。略長於一次攻擊冷卻(0.6s)，容兩訊息（放開→攻擊）的順序餘裕。
pub const CHARGE_READY_TTL_SECS: f32 = 1.2;

/// 蓄力檔位：放開時依蓄力時間結算，決定那一擊的傷害倍率。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeTier {
    /// 輕揮（蓄不足門檻）：無加成。
    None,
    /// 半蓄：×1.5。
    Half,
    /// 滿蓄：×2.0。
    Full,
}

impl ChargeTier {
    /// 依蓄力時間（秒）結算檔位。負或 NaN 一律當沒蓄（None），不汙染下游。
    pub fn from_secs(secs: f32) -> Self {
        if !(secs >= CHARGE_MIN_SECS) {
            // 注意用 `>=` 的反向寫法：NaN 走這條（NaN 任何比較皆 false）回 None。
            ChargeTier::None
        } else if secs >= CHARGE_FULL_SECS {
            ChargeTier::Full
        } else {
            ChargeTier::Half
        }
    }

    /// 這一擊的傷害倍率（疊乘進既有 power 乘法鏈）。
    pub fn damage_mult(self) -> f32 {
        match self {
            ChargeTier::None => 1.0,
            ChargeTier::Half => 1.5,
            ChargeTier::Full => 2.0,
        }
    }

    /// 是否真的蓄出了加成（半蓄以上）——None 時不必設「待擊」、不必走重擊廣播。
    pub fn has_bonus(self) -> bool {
        self.damage_mult() > 1.0
    }

    /// 廣播給前端的線格式（前端據此演出蓄力光環顏色／重擊飄字強度）。
    pub fn wire(self) -> u8 {
        match self {
            ChargeTier::None => 0,
            ChargeTier::Half => 1,
            ChargeTier::Full => 2,
        }
    }
}

/// 一趟進行中的蓄力（記憶體前置、`Copy`、重啟清空）。
///
/// 只記一個累計時間：前端據 `elapsed`/`progress` 渲染逐漸收束的蓄力環，伺服器於放開時結算檔位。
/// 與 `dodge::DodgeRoll`／`guard::GuardBrace` 同脈絡——純函式、確定可重現、`Copy`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChargedStrike {
    /// 距起蓄經過的時間（秒），夾在 `[0, CHARGE_FULL_SECS]`。
    elapsed: f32,
}

impl ChargedStrike {
    /// 開一趟新的蓄力。
    pub fn start() -> Self {
        ChargedStrike { elapsed: 0.0 }
    }

    /// 距起蓄經過的時間（秒）——隨快照廣播給前端渲染蓄力環。
    pub fn elapsed(self) -> f32 {
        self.elapsed
    }

    /// 蓄力進度 `[0,1]`（滿蓄＝1）：前端據此填蓄力環。
    pub fn progress(self) -> f32 {
        (self.elapsed / CHARGE_FULL_SECS).clamp(0.0, 1.0)
    }

    /// 此刻放開會結算到的檔位（不改狀態，純查）。
    pub fn tier(self) -> ChargeTier {
        ChargeTier::from_secs(self.elapsed)
    }

    /// 推進一個 tick：累加時間並夾在滿蓄上限（蓄滿後再按住也不會更重）。
    /// 負或零 dt 不前進（守時間單調）。不會自行結束——蓄力靠玩家放開（`take`）結算。
    pub fn advance(&mut self, dt: f32) {
        if dt > 0.0 {
            self.elapsed = (self.elapsed + dt).min(CHARGE_FULL_SECS);
        }
    }
}

/// 蓄好、等下一擊兌現的「待擊」重擊（放開後存活 `CHARGE_READY_TTL_SECS`，被攻擊消費或逾時消散）。
///
/// 鏡像 `guard::GuardShield`——一個限時、單次性的乘法 buff，由遊戲迴圈倒數、攻擊管線消費。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChargeReady {
    tier: ChargeTier,
    /// 剩餘存活秒數，歸零即消散。
    remaining: f32,
}

impl ChargeReady {
    /// 從一個有加成的檔位起一份待擊（呼叫端應先確認 `tier.has_bonus()`）。
    pub fn new(tier: ChargeTier) -> Self {
        ChargeReady { tier, remaining: CHARGE_READY_TTL_SECS }
    }

    /// 這一擊的傷害倍率。
    pub fn damage_mult(self) -> f32 {
        self.tier.damage_mult()
    }

    /// 檔位（供命中廣播帶給前端演出重擊強度）。
    pub fn tier(self) -> ChargeTier {
        self.tier
    }

    /// 倒數一個 tick：遞減剩餘秒數，回傳是否已消散（呼叫端據此清空）。
    /// 負或零 dt 不前進。
    pub fn advance(&mut self, dt: f32) -> bool {
        if dt > 0.0 {
            self.remaining -= dt;
        }
        self.remaining <= 0.0
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // 編譯期不變式：冷卻 > 蓄滿耗時，確保蓄力是換爆發、非疊 DPS（連蓄打不過連揮）。
    const _: () = assert!(CHARGE_COOLDOWN_SECS > CHARGE_FULL_SECS);
    // 編譯期不變式：滿蓄門檻 > 半蓄門檻（檔位單調遞進）。
    const _: () = assert!(CHARGE_FULL_SECS > CHARGE_MIN_SECS);
    // 編譯期不變式：待擊窗 > 一次攻擊冷卻(0.6s)，放開→攻擊兩訊息間不會白蓄。
    const _: () = assert!(CHARGE_READY_TTL_SECS > 0.6);

    #[test]
    fn tier_thresholds() {
        // 蓄不足門檻＝輕揮、無加成。
        assert_eq!(ChargeTier::from_secs(0.0), ChargeTier::None);
        assert_eq!(ChargeTier::from_secs(CHARGE_MIN_SECS - 0.01), ChargeTier::None);
        // 跨過半蓄門檻＝半蓄。
        assert_eq!(ChargeTier::from_secs(CHARGE_MIN_SECS), ChargeTier::Half);
        assert_eq!(ChargeTier::from_secs(CHARGE_FULL_SECS - 0.01), ChargeTier::Half);
        // 跨過滿蓄門檻＝滿蓄。
        assert_eq!(ChargeTier::from_secs(CHARGE_FULL_SECS), ChargeTier::Full);
        assert_eq!(ChargeTier::from_secs(10.0), ChargeTier::Full);
    }

    #[test]
    fn tier_bad_values_are_none() {
        // 負、NaN 一律當沒蓄（不汙染傷害乘法鏈）。
        assert_eq!(ChargeTier::from_secs(-3.0), ChargeTier::None);
        assert_eq!(ChargeTier::from_secs(f32::NAN), ChargeTier::None);
        assert!(!ChargeTier::None.has_bonus());
        assert!(ChargeTier::Half.has_bonus());
        assert!(ChargeTier::Full.has_bonus());
    }

    #[test]
    fn tier_mult_and_wire() {
        assert!((ChargeTier::None.damage_mult() - 1.0).abs() < 1e-6);
        assert!((ChargeTier::Half.damage_mult() - 1.5).abs() < 1e-6);
        assert!((ChargeTier::Full.damage_mult() - 2.0).abs() < 1e-6);
        assert_eq!((ChargeTier::None.wire(), ChargeTier::Half.wire(), ChargeTier::Full.wire()), (0, 1, 2));
    }

    #[test]
    fn charge_starts_zeroed() {
        let c = ChargedStrike::start();
        assert!((c.elapsed() - 0.0).abs() < 1e-6);
        assert!((c.progress() - 0.0).abs() < 1e-6);
        assert_eq!(c.tier(), ChargeTier::None);
    }

    #[test]
    fn charge_builds_then_saturates() {
        let mut c = ChargedStrike::start();
        c.advance(CHARGE_MIN_SECS);
        assert_eq!(c.tier(), ChargeTier::Half);
        c.advance(CHARGE_FULL_SECS); // 累過滿蓄
        assert_eq!(c.tier(), ChargeTier::Full);
        // 夾在滿蓄上限：再蓄也不超過、進度封頂 1。
        assert!(c.elapsed() <= CHARGE_FULL_SECS + 1e-6);
        assert!((c.progress() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn charge_ignores_non_positive_dt() {
        let mut c = ChargedStrike::start();
        let before = c;
        c.advance(0.0);
        c.advance(-1.0);
        assert_eq!(c, before, "非正 dt 不應改變蓄力");
    }

    #[test]
    fn ready_expires_after_ttl() {
        let mut r = ChargeReady::new(ChargeTier::Full);
        assert!((r.damage_mult() - 2.0).abs() < 1e-6);
        assert_eq!(r.tier(), ChargeTier::Full);
        // 窗內不消散。
        assert!(!r.advance(CHARGE_READY_TTL_SECS - 0.1));
        // 推過存活窗即消散。
        assert!(r.advance(0.2));
    }

    #[test]
    fn ready_advance_ignores_non_positive_dt() {
        let mut r = ChargeReady::new(ChargeTier::Half);
        assert!(!r.advance(0.0));
        assert!(!r.advance(-5.0));
    }
}
