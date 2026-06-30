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
//! - 鎖 / WS / IO 全在 `voxel_ws.rs`，本模組零 async、零鎖、零 IO。

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
///
/// `my_idx`：自己在居民陣列中的索引（用來排除自己）。
/// `homes`：所有居民的 (home_x, home_z, name)，含自己。
/// `pick`：確定性選擇值（呼叫端可用位置 bits 等穩定值提供）。
/// 沒有其他居民時回 `None`。
pub fn pick_destination<'a>(
    my_idx: usize,
    homes: &'a [(f32, f32, &'a str)],
    pick: usize,
) -> Option<(f32, f32, &'a str)> {
    let others: Vec<(f32, f32, &str)> = homes
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != my_idx)
        .map(|(_, &r)| r)
        .collect();
    if others.is_empty() {
        return None;
    }
    Some(others[pick % others.len()])
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

    // ── pick_destination ─────────────────────────────────────────────────────

    #[test]
    fn pick_destination_skips_self() {
        let homes = [(0.0, 0.0, "露娜"), (0.0, 75.0, "諾娃"), (-75.0, 0.0, "賽勒"), (75.0, 0.0, "奧瑞")];
        // 居民 0（露娜）選目標，結果不會是露娜自己
        for pick in 0..12 {
            let r = pick_destination(0, &homes, pick);
            assert!(r.is_some());
            assert_ne!(r.unwrap().2, "露娜");
        }
    }

    #[test]
    fn pick_destination_none_when_alone() {
        let homes = [(0.0, 0.0, "露娜")];
        assert!(pick_destination(0, &homes, 0).is_none());
    }

    #[test]
    fn pick_destination_deterministic() {
        let homes = [(0.0, 0.0, "露娜"), (0.0, 75.0, "諾娃"), (-75.0, 0.0, "賽勒"), (75.0, 0.0, "奧瑞")];
        // 同 my_idx + 同 pick → 同結果（確定性）
        let a = pick_destination(1, &homes, 5);
        let b = pick_destination(1, &homes, 5);
        assert_eq!(a, b);
    }

    #[test]
    fn pick_destination_covers_all_others() {
        let homes = [(0.0, 0.0, "露娜"), (0.0, 75.0, "諾娃"), (-75.0, 0.0, "賽勒"), (75.0, 0.0, "奧瑞")];
        // 居民 0 探訪時，應能選到 3 位鄰居中每一位
        let results: std::collections::HashSet<&str> = (0..3)
            .map(|pick| pick_destination(0, &homes, pick).unwrap().2)
            .collect();
        assert_eq!(results.len(), 3, "應能選到 3 位不同鄰居");
    }

    #[test]
    fn pick_destination_out_of_bounds_idx_still_safe() {
        let homes = [(0.0, 0.0, "露娜"), (0.0, 75.0, "諾娃")];
        // my_idx 超出陣列（呼叫端保護用，但不應 panic）
        let r = pick_destination(99, &homes, 0);
        assert!(r.is_some()); // 99 不等於 0 或 1，全部都是鄰居
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
        for pick in 0..5 {
            let r = pick_destination(0, &homes, pick).unwrap();
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
