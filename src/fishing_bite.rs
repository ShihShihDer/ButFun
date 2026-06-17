//! ROADMAP 346 釣魚上鉤小遊戲——拋竿、等咬鉤、把握時機收竿。
//!
//! 把 ROADMAP 47 的「一鍵就得魚」升級成一段你來我往的小遊戲：
//!   1. 拋竿（`ClientMsg::Fish`）→ 進入「等待咬鉤」，魚會在 1.5~4.5 秒後某個時刻咬鉤。
//!   2. 魚咬鉤瞬間，浮標抖動、冒出「❗」；你有 `BITE_WINDOW_SECS` 的反應窗口。
//!   3. 在窗口內收竿（`ClientMsg::Reel`）＝釣到魚；**反應越快、魚越好**。
//!      太早收竿（魚還沒咬）會把魚嚇跑、窗口過了沒收竿魚也會跑掉——都空手而回。
//!
//! 全純記憶體、純函式、零持久化、零 migration。魚只是料理素材＋農夫熟練度，
//! 不進任何戰鬥／經濟核心結算 → 即便反應好提高了好魚機率，仍是**零平衡風險**。

use crate::inventory::ItemKind;
use crate::season::Season;

/// 拋竿後到魚咬鉤的最短等待（秒）。
pub const BITE_MIN_SECS: f32 = 1.5;
/// 拋竿後到魚咬鉤的最長等待（秒）。
pub const BITE_MAX_SECS: f32 = 4.5;
/// 魚咬鉤後的反應窗口（秒）：超過這段沒收竿，魚就脫鉤跑掉。
pub const BITE_WINDOW_SECS: f32 = 1.4;
/// 「完美」反應的上限（秒）：咬鉤後這麼快收竿＝完美。
pub const PERFECT_REACT_SECS: f32 = 0.4;
/// 「不錯」反應的上限（秒）：完美與此之間＝不錯，再慢就是普通。
pub const GOOD_REACT_SECS: f32 = 0.9;

/// 進行中的一趟釣魚所處階段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FishingPhase {
    /// 已拋竿，等魚上鉤。
    Waiting,
    /// 魚已咬鉤，反應窗口倒數中。
    Biting,
}

impl FishingPhase {
    /// 前端用的 snake_case 線格式（隨 `PlayerView` 廣播，畫浮標狀態）。
    pub fn as_str(self) -> &'static str {
        match self {
            FishingPhase::Waiting => "waiting",
            FishingPhase::Biting => "biting",
        }
    }
}

/// 一趟進行中的釣魚（記憶體前置、`Copy`、重啟清空）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FishingCast {
    phase: FishingPhase,
    /// 距拋竿經過的時間（秒）。
    elapsed: f32,
    /// 咬鉤時刻：拋竿後幾秒魚會上鉤（`Waiting → Biting` 的門檻）。
    bite_at: f32,
}

/// `advance` 一個 tick 後可能發生的轉場（供 game loop 決定要不要清狀態／播效果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiteEvent {
    /// 沒有狀態轉場（仍在等／仍在反應窗口內）。
    None,
    /// 魚剛咬鉤（`Waiting → Biting`）：前端該抖浮標、冒「❗」。
    JustBit,
    /// 魚等太久沒收竿、脫鉤跑掉了：呼叫端應清掉這趟釣魚。
    Escaped,
}

/// 收竿（`reel`）的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReelOutcome {
    /// 魚還沒咬就收竿，把魚嚇跑了——空手而回。
    TooEarly,
    /// 成功釣到魚，附帶這次反應的品質（決定魚種加權）。
    Caught(FishQuality),
}

/// 收竿反應品質：反應越快越好，提高好魚機率。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FishQuality {
    /// 普通（反應較慢，但仍在窗口內）。
    Ok,
    /// 不錯。
    Good,
    /// 完美（咬鉤後極快收竿）。
    Perfect,
}

impl FishQuality {
    /// 前端飄字／播報用的中文標籤（i18n 佔位）。
    pub fn label(self) -> &'static str {
        match self {
            FishQuality::Ok => "上鉤",
            FishQuality::Good => "漂亮",
            FishQuality::Perfect => "完美",
        }
    }

    /// 前端用的 snake_case 線格式。
    pub fn as_str(self) -> &'static str {
        match self {
            FishQuality::Ok => "ok",
            FishQuality::Good => "good",
            FishQuality::Perfect => "perfect",
        }
    }
}

/// 依確定性種子算這一趟的咬鉤延遲（秒），落在 `[BITE_MIN_SECS, BITE_MAX_SECS]`。
///
/// 種子建議帶 `player_id_low64 ^ fish_attempt_count`，讓每趟等待長短都不同。
pub fn bite_delay(seed: u64) -> f32 {
    let span = BITE_MAX_SECS - BITE_MIN_SECS;
    // seed % 1000 → [0,1) 的偽隨機分數，映到延遲區間。
    let frac = (seed % 1000) as f32 / 1000.0;
    BITE_MIN_SECS + frac * span
}

/// 把咬鉤後的反應時間（秒）分級。
pub fn reaction_quality(react_secs: f32) -> FishQuality {
    if react_secs <= PERFECT_REACT_SECS {
        FishQuality::Perfect
    } else if react_secs <= GOOD_REACT_SECS {
        FishQuality::Good
    } else {
        FishQuality::Ok
    }
}

/// 依品質加權擲出魚種。品質越高，星星魚／深海魚機率越高。
///
/// 三檔分布（小魚／星星魚／深海魚）：
///   - `Ok`      ＝ 70 / 25 / 5（與 ROADMAP 47 原版一致——慢手仍照舊機率）
///   - `Good`    ＝ 55 / 35 / 10
///   - `Perfect` ＝ 40 / 40 / 20
///
/// 魚不影響戰鬥／經濟結算 → 提高好魚機率仍零平衡風險，只是把「把握時機」變得有回報。
pub fn roll_fish_quality(seed: u64, quality: FishQuality) -> ItemKind {
    let r = seed % 100;
    // 各檔的「小魚上限／星星魚上限」，超過星星魚上限即深海魚。
    let (small_max, star_max) = match quality {
        FishQuality::Ok => (69, 94),      // 0-69 小 / 70-94 星 / 95-99 深
        FishQuality::Good => (54, 89),    // 0-54 小 / 55-89 星 / 90-99 深
        FishQuality::Perfect => (39, 79), // 0-39 小 / 40-79 星 / 80-99 深
    };
    if r <= small_max {
        ItemKind::FishSmall
    } else if r <= star_max {
        ItemKind::FishStar
    } else {
        ItemKind::FishDeep
    }
}

// ─── 季節漁汛（ROADMAP 363：季節第一次漫進釣魚）────────────────────────────────
//
// 季節早已驅動農業成長、野生動物季節行為、季節採集節點與 NPC 攀談，唯獨「釣魚」
// 至今與四季無關。本切片把釣魚接進這條季節生態環——四季水域各有一種「當季當紅魚」，
// 當季那種魚上鉤率明顯提高（玩家會明顯感到「這個季節水裡的魚不一樣」）。
//
// 零平衡風險的關鍵：這是**漁獲組成的重分配，不是新增獎勵**——你每段時間能釣的魚數量
// 仍受冷卻／反應小遊戲節制（總量不變），漁汛只改變「同樣這幾尾裡，哪種魚更常上鉤」。
// 不開新獎勵路徑、不送乙太、不碰任何結算。

/// 當季當紅魚的權重加成（百分點，疊加在 `roll_fish_quality` 的品質基礎權重上）。
///
/// 取中等值：當季魚明顯更常上鉤、玩家有感，但仍不會把稀有魚變成隨手可得
/// （最有利情形＝完美收竿的秋汛，深海魚約 32/132 ≈ 24%；其餘季節遠低於此）。
pub const SEASON_SIGNATURE_BONUS: u32 = 12;

/// 今季「當季當紅魚」——四季各對應一種釣魚掉落（與既有三種魚對應，不新增物品）。
///
/// 對應的季節氣味：
///   - 春：融雪洄游季，星星魚成群回到淺灘繁殖。
///   - 夏：盛夏淺灘水暖，小魚成群躍水。
///   - 秋：秋寒水位下降，深海魚靠岸覓食。
///   - 冬：寒冬星星魚群聚深潭避寒。
/// （三種魚對四季，星星魚於春／冬各領一季——氣味文案不同、玩家仍每季有感。）
pub fn signature_fish(season: Season) -> ItemKind {
    match season {
        Season::Spring => ItemKind::FishStar,
        Season::Summer => ItemKind::FishSmall,
        Season::Autumn => ItemKind::FishDeep,
        Season::Winter => ItemKind::FishStar,
    }
}

/// 季節加權版的魚種擲骰：先取品質基礎權重，再把「當季當紅魚」的權重 +`SEASON_SIGNATURE_BONUS`，
/// 於放大後的總權重上重新擲骰。品質越高仍越容易出好魚（與 `roll_fish_quality` 同向），
/// 季節只是把當季那種魚的那一份加厚。
///
/// 種子建議同 `roll_fish_quality`（`player_id_low64 ^ fish_attempt_count`）。
pub fn roll_fish_seasonal(seed: u64, quality: FishQuality, season: Season) -> ItemKind {
    // 品質基礎權重（小魚／星星魚／深海魚），總和恆為 100，與 roll_fish_quality 的分檔一致。
    let mut weights: [u32; 3] = match quality {
        FishQuality::Ok => [70, 25, 5],
        FishQuality::Good => [55, 35, 10],
        FishQuality::Perfect => [40, 40, 20],
    };
    // 當季當紅魚那一份加厚。
    let sig_idx = match signature_fish(season) {
        ItemKind::FishSmall => 0,
        ItemKind::FishStar => 1,
        _ => 2, // FishDeep
    };
    weights[sig_idx] += SEASON_SIGNATURE_BONUS;

    // 於放大後的總權重上擲骰，走累積區間。
    let total = weights[0] + weights[1] + weights[2];
    let r = seed % total as u64;
    if r < weights[0] as u64 {
        ItemKind::FishSmall
    } else if r < (weights[0] + weights[1]) as u64 {
        ItemKind::FishStar
    } else {
        ItemKind::FishDeep
    }
}

impl FishingCast {
    /// 拋竿，開一趟新的釣魚（`Waiting`），咬鉤時刻由種子決定。
    pub fn cast(seed: u64) -> Self {
        FishingCast {
            phase: FishingPhase::Waiting,
            elapsed: 0.0,
            bite_at: bite_delay(seed),
        }
    }

    /// 目前階段。
    pub fn phase(self) -> FishingPhase {
        self.phase
    }

    /// 推進一個 tick：累加時間，回傳這一 tick 發生的轉場事件。
    ///
    /// - `Waiting` 累到 `bite_at` → 轉 `Biting`、回 `JustBit`。
    /// - `Biting` 反應時間超過 `BITE_WINDOW_SECS` → 回 `Escaped`（呼叫端清掉這趟）。
    /// - 其餘回 `None`。
    pub fn advance(&mut self, dt: f32) -> BiteEvent {
        // 負或零 dt 不前進（守時間單調，避免倒退）。
        if dt > 0.0 {
            self.elapsed += dt;
        }
        match self.phase {
            FishingPhase::Waiting => {
                if self.elapsed >= self.bite_at {
                    self.phase = FishingPhase::Biting;
                    BiteEvent::JustBit
                } else {
                    BiteEvent::None
                }
            }
            FishingPhase::Biting => {
                if self.react_secs() > BITE_WINDOW_SECS {
                    BiteEvent::Escaped
                } else {
                    BiteEvent::None
                }
            }
        }
    }

    /// 咬鉤後經過的反應時間（秒）；尚未咬鉤時為 0。
    fn react_secs(self) -> f32 {
        match self.phase {
            FishingPhase::Waiting => 0.0,
            FishingPhase::Biting => (self.elapsed - self.bite_at).max(0.0),
        }
    }

    /// 收竿判定（純函式、不改狀態；呼叫端依結果決定給魚與清狀態）。
    ///
    /// - `Waiting`（魚還沒咬）→ `TooEarly`（嚇跑）。
    /// - `Biting` → `Caught(quality)`，品質由反應時間決定。
    ///   （窗口已過的情況由 `advance` 先回 `Escaped` 清掉，不會走到這裡。）
    pub fn reel(self) -> ReelOutcome {
        match self.phase {
            FishingPhase::Waiting => ReelOutcome::TooEarly,
            FishingPhase::Biting => ReelOutcome::Caught(reaction_quality(self.react_secs())),
        }
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bite_delay_within_bounds() {
        for seed in 0u64..2000 {
            let d = bite_delay(seed);
            assert!(
                d >= BITE_MIN_SECS && d <= BITE_MAX_SECS,
                "bite_delay({seed}) = {d} 超出 [{BITE_MIN_SECS}, {BITE_MAX_SECS}]"
            );
        }
    }

    #[test]
    fn bite_delay_deterministic() {
        assert_eq!(bite_delay(42), bite_delay(42));
        // 不同種子至少能拉開差距（端點覆蓋）。
        assert!((bite_delay(0) - BITE_MIN_SECS).abs() < 1e-6);
        assert!(bite_delay(999) > bite_delay(0));
    }

    #[test]
    fn cast_starts_waiting() {
        let c = FishingCast::cast(123);
        assert_eq!(c.phase(), FishingPhase::Waiting);
        assert_eq!(c.reel(), ReelOutcome::TooEarly, "剛拋竿就收＝太早");
    }

    #[test]
    fn waiting_then_bites_at_threshold() {
        // bite_at 由 seed=0 → 正好 BITE_MIN_SECS（1.5）
        let mut c = FishingCast::cast(0);
        // 推進到接近但未到門檻：仍 Waiting、無事件
        assert_eq!(c.advance(BITE_MIN_SECS - 0.1), BiteEvent::None);
        assert_eq!(c.phase(), FishingPhase::Waiting);
        // 再推過門檻：剛咬鉤
        assert_eq!(c.advance(0.2), BiteEvent::JustBit);
        assert_eq!(c.phase(), FishingPhase::Biting);
        // 再 advance 不會重複回 JustBit（已在 Biting、窗口內）
        assert_eq!(c.advance(0.1), BiteEvent::None);
    }

    #[test]
    fn biting_escapes_after_window() {
        let mut c = FishingCast::cast(0); // bite_at = 1.5
        assert_eq!(c.advance(1.5), BiteEvent::JustBit);
        // 反應窗口內仍可收
        assert_eq!(c.advance(BITE_WINDOW_SECS - 0.1), BiteEvent::None);
        assert!(matches!(c.reel(), ReelOutcome::Caught(_)));
        // 超過窗口 → 脫鉤
        assert_eq!(c.advance(0.2), BiteEvent::Escaped);
    }

    #[test]
    fn reel_while_waiting_is_too_early() {
        let mut c = FishingCast::cast(500);
        c.advance(0.5); // 還沒到 bite_at
        assert_eq!(c.reel(), ReelOutcome::TooEarly);
    }

    #[test]
    fn reaction_quality_tiers() {
        assert_eq!(reaction_quality(0.0), FishQuality::Perfect);
        assert_eq!(reaction_quality(PERFECT_REACT_SECS), FishQuality::Perfect);
        assert_eq!(reaction_quality(PERFECT_REACT_SECS + 0.01), FishQuality::Good);
        assert_eq!(reaction_quality(GOOD_REACT_SECS), FishQuality::Good);
        assert_eq!(reaction_quality(GOOD_REACT_SECS + 0.01), FishQuality::Ok);
        assert_eq!(reaction_quality(BITE_WINDOW_SECS), FishQuality::Ok);
    }

    #[test]
    fn reel_quality_reflects_reaction_speed() {
        let mut c = FishingCast::cast(0); // bite_at = 1.5
        c.advance(1.5); // 剛咬鉤
        c.advance(0.2); // react = 0.2 → 完美
        assert_eq!(c.reel(), ReelOutcome::Caught(FishQuality::Perfect));

        let mut c2 = FishingCast::cast(0);
        c2.advance(1.5);
        c2.advance(0.7); // react = 0.7 → 不錯
        assert_eq!(c2.reel(), ReelOutcome::Caught(FishQuality::Good));

        let mut c3 = FishingCast::cast(0);
        c3.advance(1.5);
        c3.advance(1.2); // react = 1.2 → 普通
        assert_eq!(c3.reel(), ReelOutcome::Caught(FishQuality::Ok));
    }

    #[test]
    fn advance_ignores_non_positive_dt() {
        let mut c = FishingCast::cast(500);
        let before = c;
        assert_eq!(c.advance(0.0), BiteEvent::None);
        assert_eq!(c.advance(-1.0), BiteEvent::None);
        assert_eq!(c, before, "非正 dt 不應改變狀態");
    }

    #[test]
    fn roll_fish_quality_always_returns_fish() {
        for q in [FishQuality::Ok, FishQuality::Good, FishQuality::Perfect] {
            for seed in 0u64..200 {
                let f = roll_fish_quality(seed, q);
                assert!(
                    matches!(f, ItemKind::FishSmall | ItemKind::FishStar | ItemKind::FishDeep),
                    "roll_fish_quality({seed}, {q:?}) 回傳非魚: {f:?}"
                );
            }
        }
    }

    #[test]
    fn ok_quality_matches_original_distribution() {
        // Ok 檔應與 ROADMAP 47 原版機率邊界一致（70/25/5）。
        assert_eq!(roll_fish_quality(0, FishQuality::Ok), ItemKind::FishSmall);
        assert_eq!(roll_fish_quality(69, FishQuality::Ok), ItemKind::FishSmall);
        assert_eq!(roll_fish_quality(70, FishQuality::Ok), ItemKind::FishStar);
        assert_eq!(roll_fish_quality(94, FishQuality::Ok), ItemKind::FishStar);
        assert_eq!(roll_fish_quality(95, FishQuality::Ok), ItemKind::FishDeep);
    }

    #[test]
    fn better_quality_raises_rare_fish_odds() {
        // 統計 0..100 種子：品質越高，深海魚計數越多、小魚越少（單調）。
        let count = |q: FishQuality| {
            let mut small = 0;
            let mut deep = 0;
            for s in 0u64..100 {
                match roll_fish_quality(s, q) {
                    ItemKind::FishSmall => small += 1,
                    ItemKind::FishDeep => deep += 1,
                    _ => {}
                }
            }
            (small, deep)
        };
        let (ok_small, ok_deep) = count(FishQuality::Ok);
        let (good_small, good_deep) = count(FishQuality::Good);
        let (perf_small, perf_deep) = count(FishQuality::Perfect);
        assert!(perf_deep > good_deep && good_deep > ok_deep, "深海魚應隨品質升高");
        assert!(perf_small < good_small && good_small < ok_small, "小魚應隨品質降低");
    }

    // ── 季節漁汛（ROADMAP 363）─────────────────────────────────────────────
    #[test]
    fn signature_fish_covers_four_seasons() {
        // 四季各有對應當紅魚；春／冬同為星星魚，夏小魚，秋深海魚。
        assert_eq!(signature_fish(Season::Spring), ItemKind::FishStar);
        assert_eq!(signature_fish(Season::Summer), ItemKind::FishSmall);
        assert_eq!(signature_fish(Season::Autumn), ItemKind::FishDeep);
        assert_eq!(signature_fish(Season::Winter), ItemKind::FishStar);
    }

    #[test]
    fn seasonal_roll_always_returns_fish() {
        for season in [Season::Spring, Season::Summer, Season::Autumn, Season::Winter] {
            for q in [FishQuality::Ok, FishQuality::Good, FishQuality::Perfect] {
                for seed in 0u64..200 {
                    let f = roll_fish_seasonal(seed, q, season);
                    assert!(
                        matches!(
                            f,
                            ItemKind::FishSmall | ItemKind::FishStar | ItemKind::FishDeep
                        ),
                        "roll_fish_seasonal({seed},{q:?},{season:?}) 回傳非魚: {f:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn in_season_fish_is_more_likely_than_baseline() {
        // 對每季：當季當紅魚在季節加權下的出現次數，應「嚴格多於」同品質非季節版。
        let count_sig = |roll: &dyn Fn(u64) -> ItemKind, sig: ItemKind| {
            (0u64..1000).filter(|&s| roll(s) == sig).count()
        };
        for season in [Season::Spring, Season::Summer, Season::Autumn, Season::Winter] {
            let sig = signature_fish(season);
            for q in [FishQuality::Ok, FishQuality::Good, FishQuality::Perfect] {
                let base = count_sig(&|s| roll_fish_quality(s, q), sig);
                let seasoned = count_sig(&|s| roll_fish_seasonal(s, q, season), sig);
                assert!(
                    seasoned > base,
                    "{season:?}/{q:?}: 當季魚 {sig:?} 季節版({seasoned}) 應多於基礎版({base})"
                );
            }
        }
    }

    #[test]
    fn seasonal_roll_total_catch_count_unchanged() {
        // 零平衡風險佐證：季節加權只改「組成」，不改「總漁獲尾數」——
        // 同一批種子，每顆都恰好回傳一尾魚（不多不少），與基礎版一樣。
        let n = 500u64;
        let caught = (0..n)
            .filter(|&s| {
                matches!(
                    roll_fish_seasonal(s, FishQuality::Good, Season::Autumn),
                    ItemKind::FishSmall | ItemKind::FishStar | ItemKind::FishDeep
                )
            })
            .count();
        assert_eq!(caught as u64, n, "每次擲骰都應恰好得一尾魚");
    }

    #[test]
    fn seasonal_roll_deterministic() {
        assert_eq!(
            roll_fish_seasonal(42, FishQuality::Perfect, Season::Spring),
            roll_fish_seasonal(42, FishQuality::Perfect, Season::Spring)
        );
    }

    #[test]
    fn phase_and_quality_wire_strings_stable() {
        assert_eq!(FishingPhase::Waiting.as_str(), "waiting");
        assert_eq!(FishingPhase::Biting.as_str(), "biting");
        assert_eq!(FishQuality::Ok.as_str(), "ok");
        assert_eq!(FishQuality::Good.as_str(), "good");
        assert_eq!(FishQuality::Perfect.as_str(), "perfect");
    }
}
