//! 乙太方界·居民跨域探訪 v1——建物全蓋完的居民偶爾遠征拜訪鄰里（ROADMAP 671）。
//!
//! **核心信念**：AI 居民之間有社會連結，不只是彼此孤立的島嶼；
//! 拜訪讓世界感覺有人情往來——玩家走在路上，偶爾能看到一位居民正在「旅途中」
//! 或剛剛抵達鄰居家，整個乙太方界因此更像一個真實的小社會。
//!
//! **設計要點**：
//! - 只有「已蓋完所有建物」的居民才會發起探訪（有了家才有餘力社交）。
//! - 探訪冷卻長（預設 5 分鐘），稀少才有感——頻繁就平淡了。
//! - 純程式化台詞（零 LLM、確定性、可測），不燒額度。
//! - **目標挑選看情誼（ROADMAP 671 深化）**：老朋友明顯更常被造訪、陌生人仍有機會——
//!   關係第一次真的**影響**行為，而不只是被行為單向記錄，讓小圈子更容易自己滾出來。
//! - 鎖 / WS / IO 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。

use crate::voxel_bonds::BondTier;

/// 依情誼層級決定探訪目標的相對權重：老朋友明顯優先，但陌生人/相識仍保留機會
/// （關係要能從零開始累積，不能被完全鎖死只拜訪老朋友）。純函式、確定性。
pub fn tier_weight(tier: BondTier) -> usize {
    match tier {
        BondTier::Stranger => 1,
        BondTier::Acquaintance => 3,
        BondTier::Friend => 6,
    }
}

/// 每 agency tick，「全蓋完 + 冷卻到期」時觸發探訪的機率。
/// 0.008 ≈ 在冷卻到期後平均 12.5 秒才開始一次（`agency tick` 每 ~1 秒一次）。
pub const VISIT_CHANCE: f32 = 0.008;
/// 探訪冷卻（秒）：一次探訪結束後要等這麼久才能再出發（稀少有感）。
pub const VISIT_COOLDOWN_SECS: f32 = 300.0;
/// 抵達後逗留時間（秒）：在鄰居家待這麼久後啟程返家。
pub const VISIT_STAY_SECS: f32 = 60.0;
/// 到達判定半徑（方塊）：靠這麼近 = 視為抵達目的地。
pub const VISIT_ARRIVE_DIST: f32 = 5.0;
/// 探訪後的居民家域偏移（幅度；到達後讓居民在鄰居附近自然閒晃的半徑）。
pub const VISIT_WANDER_RADIUS: f32 = 8.0;

/// 判斷是否應發起一次探訪（純函式，確定性）。
///
/// `all_built`：居民已完成所有建物進展（`done_count >= BUILD_PROGRESSION.len()`）。
/// `visit_cooldown`：剩餘冷卻秒數（> 0 = 冷卻中，不可發起）。
/// `roll`：`rand::random::<f32>()`（0..1，呼叫端傳入，方便測試釘值）。
pub fn should_visit(all_built: bool, visit_cooldown: f32, roll: f32) -> bool {
    all_built && visit_cooldown <= 0.0 && roll < VISIT_CHANCE
}

/// 從鄰居陣列中挑一個目標（確定性，使用外部傳入的 `pick` 避免 random）。
/// **依情誼加權**：老朋友被選中的機率明顯較高，但陌生人/相識仍有機會被造訪
/// （不然關係永遠沒機會從零開始，小圈子也就長不出新成員）。
///
/// `my_idx`：自己在居民陣列中的索引（用來排除自己）。
/// `homes`：所有居民的 (home_x, home_z, name)，含自己，索引需與 `tiers` 對齊。
/// `tiers`：`homes` 對應索引「自己 → 該居民」的情誼層級（自己那格不使用，值任意）。
///   長度不足時，缺的視為 `Stranger`（保守退化，不 panic）。
/// `pick`：確定性選擇值（呼叫端可用位置 bits 等穩定值提供）。
/// 沒有其他居民時回 `None`。
pub fn pick_destination<'a>(
    my_idx: usize,
    homes: &'a [(f32, f32, &'a str)],
    tiers: &[BondTier],
    pick: usize,
) -> Option<(f32, f32, &'a str)> {
    let others: Vec<(f32, f32, &str, BondTier)> = homes
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != my_idx)
        .map(|(i, &(x, z, n))| {
            let tier = tiers.get(i).copied().unwrap_or(BondTier::Stranger);
            (x, z, n, tier)
        })
        .collect();
    if others.is_empty() {
        return None;
    }
    let total: usize = others.iter().map(|(_, _, _, t)| tier_weight(*t)).sum();
    let mut r = pick % total.max(1);
    for &(x, z, n, t) in &others {
        let w = tier_weight(t);
        if r < w {
            return Some((x, z, n));
        }
        r -= w;
    }
    // 理論上加總後不會落到這裡（浮點/取模皆為整數確定性運算），保底回最後一位。
    others.last().map(|&(x, z, n, _)| (x, z, n))
}

/// 居民抵達鄰居家時說的台詞（確定性純函式，依居民名字選句）。
/// `visitor_name`：來訪者，`host_name`：被訪者。
pub fn arrival_say(visitor_name: &str, host_name: &str) -> String {
    const TEMPLATES: &[&str] = &[
        "{host}，我來看你了！",
        "{host}！好久不見。",
        "嗨，{host}，我來串門子～",
    ];
    // 依兩人名字 bytes 之和取模，確定性不走 random（同一對居民每次說同一句，有辨識感）。
    let pick = (visitor_name.bytes().map(|b| b as usize).sum::<usize>()
        + host_name.bytes().map(|b| b as usize).sum::<usize>())
        % TEMPLATES.len();
    TEMPLATES[pick].replace("{host}", host_name)
}

/// 居民準備返家時說的告別台詞（確定性純函式）。
pub fn departure_say(visitor_name: &str) -> String {
    const LINES: &[&str] = &["該回去了，掰掰！", "我要回家囉～", "謝謝招待，再見！"];
    let pick = visitor_name.bytes().map(|b| b as usize).sum::<usize>() % LINES.len();
    LINES[pick].to_string()
}

/// 動態 Feed：出發探訪的標題（面向玩家，繁中）。
pub const FEED_KIND_DEPART: &str = "出發探訪";
/// 動態 Feed：到達鄰居家的標題。
pub const FEED_KIND_ARRIVE: &str = "抵達鄰家";
/// 動態 Feed：返家的標題。
pub const FEED_KIND_RETURN: &str = "探訪歸家";

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── should_visit ──────────────────────────────────────────────────────────

    #[test]
    fn should_visit_triggers_when_all_conditions_met() {
        // 全蓋完 + 冷卻到期 + roll 在閾值內
        assert!(should_visit(true, 0.0, VISIT_CHANCE - 0.001));
    }

    #[test]
    fn should_visit_false_when_not_all_built() {
        assert!(!should_visit(false, 0.0, 0.0));
    }

    #[test]
    fn should_visit_false_when_cooldown_active() {
        assert!(!should_visit(true, 10.0, 0.0));
    }

    #[test]
    fn should_visit_false_when_roll_too_high() {
        // roll >= VISIT_CHANCE 不觸發
        assert!(!should_visit(true, 0.0, VISIT_CHANCE));
        assert!(!should_visit(true, 0.0, 1.0));
    }

    #[test]
    fn should_visit_cooldown_exactly_zero_allowed() {
        assert!(should_visit(true, 0.0, 0.0));
    }

    #[test]
    fn should_visit_negative_cooldown_still_ok() {
        // 冷卻若因 dt 超扣變負數，仍視為可觸發
        assert!(should_visit(true, -1.0, 0.0));
    }

    // ── pick_destination（皆用全 Stranger 情誼＝與加權前行為等價，鎖住不回歸） ──────

    #[test]
    fn pick_destination_skips_self() {
        let homes = [(0.0, 0.0, "露娜"), (0.0, 75.0, "諾娃"), (-75.0, 0.0, "賽勒"), (75.0, 0.0, "奧瑞")];
        let tiers = [BondTier::Stranger; 4];
        // 居民 0（露娜）選目標，結果不會是露娜自己
        for pick in 0..12 {
            let r = pick_destination(0, &homes, &tiers, pick);
            assert!(r.is_some());
            assert_ne!(r.unwrap().2, "露娜");
        }
    }

    #[test]
    fn pick_destination_none_when_alone() {
        let homes = [(0.0, 0.0, "露娜")];
        assert!(pick_destination(0, &homes, &[BondTier::Stranger], 0).is_none());
    }

    #[test]
    fn pick_destination_deterministic() {
        let homes = [(0.0, 0.0, "露娜"), (0.0, 75.0, "諾娃"), (-75.0, 0.0, "賽勒"), (75.0, 0.0, "奧瑞")];
        let tiers = [BondTier::Stranger; 4];
        // 同 my_idx + 同 pick → 同結果（確定性）
        let a = pick_destination(1, &homes, &tiers, 5);
        let b = pick_destination(1, &homes, &tiers, 5);
        assert_eq!(a, b);
    }

    #[test]
    fn pick_destination_covers_all_others() {
        let homes = [(0.0, 0.0, "露娜"), (0.0, 75.0, "諾娃"), (-75.0, 0.0, "賽勒"), (75.0, 0.0, "奧瑞")];
        let tiers = [BondTier::Stranger; 4];
        // 居民 0 探訪時，應能選到 3 位鄰居中每一位
        let results: std::collections::HashSet<&str> = (0..3)
            .map(|pick| pick_destination(0, &homes, &tiers, pick).unwrap().2)
            .collect();
        assert_eq!(results.len(), 3, "應能選到 3 位不同鄰居");
    }

    #[test]
    fn pick_destination_out_of_bounds_idx_still_safe() {
        let homes = [(0.0, 0.0, "露娜"), (0.0, 75.0, "諾娃")];
        // my_idx 超出陣列（呼叫端保護用，但不應 panic）
        let r = pick_destination(99, &homes, &[BondTier::Stranger; 2], 0);
        assert!(r.is_some()); // 99 不等於 0 或 1，全部都是鄰居
    }

    #[test]
    fn pick_destination_tiers_shorter_than_homes_defaults_to_stranger() {
        // tiers 長度不足（呼叫端快照與 homes 略有落差時的保護）：缺的視為 Stranger，不 panic。
        let homes = [(0.0, 0.0, "露娜"), (0.0, 75.0, "諾娃"), (-75.0, 0.0, "賽勒")];
        let r = pick_destination(0, &homes, &[], 0);
        assert!(r.is_some());
    }

    // ── pick_destination 情誼加權（本切片新行為：關係影響探訪目標） ────────────────

    #[test]
    fn tier_weight_friend_beats_acquaintance_beats_stranger() {
        assert!(tier_weight(BondTier::Friend) > tier_weight(BondTier::Acquaintance));
        assert!(tier_weight(BondTier::Acquaintance) > tier_weight(BondTier::Stranger));
        assert!(tier_weight(BondTier::Stranger) >= 1, "陌生人仍要有非零機會，關係才可能從零開始");
    }

    #[test]
    fn pick_destination_prefers_friend_over_stranger() {
        // 自己(0)＋一位陌生人(1)＋一位老朋友(2)：老朋友的權重應明顯佔多數。
        let homes = [(0.0, 0.0, "露娜"), (10.0, 0.0, "諾娃"), (20.0, 0.0, "賽勒")];
        let tiers = [BondTier::Stranger, BondTier::Stranger, BondTier::Friend];
        let total = tier_weight(BondTier::Stranger) + tier_weight(BondTier::Friend);
        let friend_picks = (0..total)
            .filter(|&pick| pick_destination(0, &homes, &tiers, pick).unwrap().2 == "賽勒")
            .count();
        // 老朋友權重 6 / (1+6) 應遠多於陌生人的 1 / 7。
        assert_eq!(friend_picks, tier_weight(BondTier::Friend));
        assert!(friend_picks * 2 > total, "老朋友應佔明顯多數的探訪機率");
    }

    #[test]
    fn pick_destination_weighted_still_covers_stranger_sometimes() {
        // 陌生人權重非零：掃過完整週期，陌生人仍會被選到（關係不會被鎖死永遠拜訪同一人）。
        let homes = [(0.0, 0.0, "露娜"), (10.0, 0.0, "諾娃"), (20.0, 0.0, "賽勒")];
        let tiers = [BondTier::Stranger, BondTier::Stranger, BondTier::Friend];
        let total = tier_weight(BondTier::Stranger) + tier_weight(BondTier::Friend);
        let hit_stranger = (0..total)
            .any(|pick| pick_destination(0, &homes, &tiers, pick).unwrap().2 == "諾娃");
        assert!(hit_stranger, "陌生人仍要偶爾被選到");
    }

    // ── arrival_say ──────────────────────────────────────────────────────────

    #[test]
    fn arrival_say_contains_host_name() {
        let s = arrival_say("露娜", "諾娃");
        assert!(s.contains("諾娃"), "台詞應含被訪者名字: {s}");
    }

    #[test]
    fn arrival_say_deterministic() {
        assert_eq!(arrival_say("露娜", "諾娃"), arrival_say("露娜", "諾娃"));
    }

    #[test]
    fn arrival_say_different_pairs_may_differ() {
        // 不同組合不一定相同（測確定性不測特定輸出）
        let a = arrival_say("露娜", "諾娃");
        let b = arrival_say("賽勒", "奧瑞");
        // 只驗兩者都非空即可（不要求一定不同，因查表可能碰巧相同）
        assert!(!a.is_empty());
        assert!(!b.is_empty());
    }

    // ── departure_say ─────────────────────────────────────────────────────────

    #[test]
    fn departure_say_non_empty() {
        assert!(!departure_say("露娜").is_empty());
        assert!(!departure_say("諾娃").is_empty());
    }

    #[test]
    fn departure_say_deterministic() {
        assert_eq!(departure_say("露娜"), departure_say("露娜"));
    }

    // ── 常數健全 ─────────────────────────────────────────────────────────────

    #[test]
    fn constants_sane() {
        assert!(VISIT_CHANCE > 0.0 && VISIT_CHANCE < 1.0);
        assert!(VISIT_COOLDOWN_SECS > 0.0);
        assert!(VISIT_STAY_SECS > 0.0);
        assert!(VISIT_ARRIVE_DIST > 0.0);
        assert!(VISIT_WANDER_RADIUS > 0.0);
    }

    // ── 邊界 ─────────────────────────────────────────────────────────────────

    #[test]
    fn pick_destination_two_residents_always_picks_other() {
        // 只有 2 位居民：my_idx=0 → 唯一結果是索引 1。
        let homes = [(0.0_f32, 0.0_f32, "露娜"), (50.0, 50.0, "諾娃")];
        let tiers = [BondTier::Stranger; 2];
        for pick in 0..5 {
            let r = pick_destination(0, &homes, &tiers, pick).unwrap();
            assert_eq!(r.2, "諾娃");
        }
    }

    #[test]
    fn departure_say_differs_between_some_residents() {
        // 不同居民的告別台詞查表可能不同（驗確定性選字不全相同）。
        // 僅驗兩者皆非空（hash 碰巧相同不算錯）。
        let a = departure_say("露娜");
        let b = departure_say("奧瑞");
        assert!(!a.is_empty() && !b.is_empty());
    }

    #[test]
    fn should_visit_just_below_chance_triggers() {
        // roll = VISIT_CHANCE - 一點點 → 應觸發。
        assert!(should_visit(true, 0.0, VISIT_CHANCE - 0.0001));
    }
}
