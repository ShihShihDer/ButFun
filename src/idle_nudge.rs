//! ROADMAP 426 情境下一步提示 —— 對「已畢業」的玩家，依當下情境浮一句溫柔的「現在何不…」。
//!
//! 建議箱裡最反覆出現的真實玩家心聲是「**夜裡／閒下來時不知道能做什麼**」：新手引導
//! （ROADMAP 396）只在全新帳號跑、走完即畢業永不再現；每日委託是固定清單。兩者都沒有
//! 回答「我已經上手了，現在入夜、手邊沒急事，下一步該往哪？」這個**留存**痛點。
//!
//! 本模組補上這塊空白：一個**永久、面向所有已畢業玩家、純情境反應**的下一步提示——
//! 它不是固定課程，而是讀此刻的世界（是否入夜、是否站在水邊、血量、有沒有自己的田、
//! 是否正忙著別的事），挑出**最該被輕輕點一下**的一件事，浮成 HUD 上一句溫柔提示。
//! 夜晚是這份提示的主場，正面回應「夜間空轉」的呼聲。
//!
//! 與 396 新手引導分工乾淨：
//!   - 引導＝**一次性**、只給**全新**玩家、固定五步核心循環、走完即畢業消失；
//!   - 本模組＝**常駐**、給**已畢業**玩家、無固定課程、純粹隨情境此刻變化。
//! 引導啟用中時本提示讓位（回 `None`），不與引導爭奪同一塊注意力。
//!
//! 成本紀律：純查表邏輯、零 LLM、零持久化、零 migration、不碰玩家資料。
//! 面向玩家的字串一律放前端（i18n 替換點），後端只送穩定 wire key。

/// 一則情境下一步提示。值即對應前端文案表的 wire key。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nudge {
    /// 還沒有自己的田（訪客／未落腳）——引導去開墾一塊地。
    ClaimLand,
    /// 血量偏低——提醒找個安全角落歇口氣回血。
    Rest,
    /// 入夜且站在水邊——夜釣正是時候。
    NightFish,
    /// 入夜、城外乙太泉正湧現——去汲泉，把「夜裡只能等」變成具體可做的事。
    NightSpring,
    /// 入夜、手邊沒急事——抬頭看看星空、放鬆一下，等天明。
    NightStargaze,
    /// 白天站在水邊閒著——何不垂釣片刻。
    Fish,
}

impl Nudge {
    /// 穩定 wire key（送前端對應文案，面向玩家字串不寫死在後端，留 i18n 空間）。
    #[inline]
    pub fn wire_key(self) -> &'static str {
        match self {
            Nudge::ClaimLand => "claim_land",
            Nudge::Rest => "rest",
            Nudge::NightFish => "night_fish",
            Nudge::NightSpring => "night_spring",
            Nudge::NightStargaze => "night_stargaze",
            Nudge::Fish => "fish",
        }
    }
}

/// 算情境提示所需的當下訊號。全部在快照接線點可便宜取得（無新增鎖、無新增持久化）。
#[derive(Clone, Copy, Debug, Default)]
pub struct NudgeCtx {
    /// 新手引導仍在進行中——本提示讓位給引導，回 `None`。
    pub onboarding_active: bool,
    /// 正忙著別的進行式互動（釣魚／採礦／蓄力／汲泉／伐木…）——別打斷，回 `None`。
    pub busy: bool,
    /// 已倒地休息中——別在玩家躺著時還催促，回 `None`。
    pub downed: bool,
    /// 還沒有自己的田（訪客或尚未開墾）。
    pub is_visitor: bool,
    /// 血量偏低（低於上限的 `LOW_HP_FRAC`）。
    pub low_hp: bool,
    /// 此刻是否入夜（黃昏或夜晚都算「夜的氛圍」，夜間活動正當時）。
    pub is_nightish: bool,
    /// 是否站在水邊（可垂釣）。
    pub near_water: bool,
    /// 城外夜間乙太泉是否正湧現且尚有未採的泉（ROADMAP 162/362；入夜時把提示
    /// 從被動看星空升級成「去汲泉」這個具體主動目標）。
    pub night_springs_active: bool,
}

/// 視為「血量偏低」的比例門檻（低於上限這個比例才提醒歇息回血）。
pub const LOW_HP_FRAC: f32 = 0.35;

/// 依當下情境挑一則最該被輕點的提示；沒有特別該提示的情境就回 `None`（不顯示，避免常駐嘮叨）。
///
/// 優先序（由「最該先處理」到「錦上添花」，確定性）：
///   1. 引導中／正忙／倒地 → 不打擾（None）
///   2. 還沒有田 → 先引導落腳開墾（這是核心循環的起點）
///   3. 血量偏低 → 提醒歇息回血
///   4. 入夜且在水邊 → 夜釣
///   5. 入夜且乙太泉湧現 → 去城外汲泉（夜間「具體可做的事」優先於被動看星空）
///   6. 入夜 → 看星空放鬆（夜間留存的後備提示）
///   7. 白天在水邊 → 垂釣
///   8. 其餘（白天、有田、血足、不在水邊、沒在忙）→ None（玩家多半正照料農事，不需提示）
pub fn suggest(ctx: &NudgeCtx) -> Option<Nudge> {
    // 引導讓位、進行中互動不打斷、倒地不催促。
    if ctx.onboarding_active || ctx.busy || ctx.downed {
        return None;
    }
    if ctx.is_visitor {
        return Some(Nudge::ClaimLand);
    }
    if ctx.low_hp {
        return Some(Nudge::Rest);
    }
    if ctx.is_nightish && ctx.near_water {
        return Some(Nudge::NightFish);
    }
    // 入夜時若城外乙太泉正湧現，給「去汲泉」這個具體主動目標，凌駕被動的看星空——
    // 正面回應建議箱裡「夜裡只能等待／觀察、沒事可做」的最高頻呼聲。
    if ctx.is_nightish && ctx.night_springs_active {
        return Some(Nudge::NightSpring);
    }
    if ctx.is_nightish {
        return Some(Nudge::NightStargaze);
    }
    if ctx.near_water {
        return Some(Nudge::Fish);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全綠常態（白天、有田、血足、不在水邊、沒在忙）不該打擾。
    #[test]
    fn calm_daytime_with_land_gives_no_nudge() {
        let ctx = NudgeCtx::default();
        assert_eq!(suggest(&ctx), None);
    }

    /// 引導中一律讓位，無論其他情境多吸引。
    #[test]
    fn onboarding_active_suppresses_everything() {
        let ctx = NudgeCtx {
            onboarding_active: true,
            is_visitor: true,
            low_hp: true,
            is_nightish: true,
            near_water: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), None);
    }

    /// 正忙著別的互動不打斷。
    #[test]
    fn busy_suppresses_everything() {
        let ctx = NudgeCtx {
            busy: true,
            is_nightish: true,
            near_water: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), None);
    }

    /// 倒地休息中不催促。
    #[test]
    fn downed_suppresses_everything() {
        let ctx = NudgeCtx {
            downed: true,
            is_visitor: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), None);
    }

    /// 訪客最優先：引導去開墾，凌駕夜晚／水邊等其他情境。
    #[test]
    fn visitor_takes_top_priority() {
        let ctx = NudgeCtx {
            is_visitor: true,
            is_nightish: true,
            near_water: true,
            low_hp: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), Some(Nudge::ClaimLand));
    }

    /// 血量偏低（非訪客）優先提醒歇息，凌駕夜晚情境。
    #[test]
    fn low_hp_beats_night() {
        let ctx = NudgeCtx {
            low_hp: true,
            is_nightish: true,
            near_water: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), Some(Nudge::Rest));
    }

    /// 入夜且在水邊 → 夜釣。
    #[test]
    fn night_near_water_suggests_night_fish() {
        let ctx = NudgeCtx {
            is_nightish: true,
            near_water: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), Some(Nudge::NightFish));
    }

    /// 入夜、不在水邊、乙太泉沒湧現 → 看星空（夜間後備提示）。
    #[test]
    fn night_inland_no_springs_suggests_stargaze() {
        let ctx = NudgeCtx {
            is_nightish: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), Some(Nudge::NightStargaze));
    }

    /// 入夜、不在水邊、乙太泉湧現 → 去汲泉（具體主動目標，凌駕被動看星空）。
    #[test]
    fn night_inland_with_springs_suggests_spring() {
        let ctx = NudgeCtx {
            is_nightish: true,
            night_springs_active: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), Some(Nudge::NightSpring));
    }

    /// 入夜且在水邊時，夜釣優先於汲泉（人已在水邊，就近垂釣最自然）。
    #[test]
    fn night_near_water_beats_spring() {
        let ctx = NudgeCtx {
            is_nightish: true,
            near_water: true,
            night_springs_active: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), Some(Nudge::NightFish));
    }

    /// 白天即使乙太泉旗標為真（不該發生，泉只夜裡湧現）也不提示汲泉——汲泉純屬夜間情境。
    #[test]
    fn daytime_springs_flag_does_not_suggest_spring() {
        let ctx = NudgeCtx {
            night_springs_active: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), None);
    }

    /// 白天在水邊閒著 → 垂釣。
    #[test]
    fn daytime_near_water_suggests_fish() {
        let ctx = NudgeCtx {
            near_water: true,
            ..Default::default()
        };
        assert_eq!(suggest(&ctx), Some(Nudge::Fish));
    }

    /// wire key 穩定且唯一（前端文案表據此對應，契約不可漂移）。
    #[test]
    fn wire_keys_are_stable_and_unique() {
        let all = [
            Nudge::ClaimLand,
            Nudge::Rest,
            Nudge::NightFish,
            Nudge::NightSpring,
            Nudge::NightStargaze,
            Nudge::Fish,
        ];
        let keys: Vec<&str> = all.iter().map(|n| n.wire_key()).collect();
        assert_eq!(
            keys,
            ["claim_land", "rest", "night_fish", "night_spring", "night_stargaze", "fish"]
        );
        let uniq: std::collections::BTreeSet<&str> = keys.iter().copied().collect();
        assert_eq!(uniq.len(), keys.len(), "wire key 不可重複");
    }
}
