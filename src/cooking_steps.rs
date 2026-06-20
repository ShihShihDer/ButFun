//! ROADMAP 349 掌勺照譜烹調——順序記憶（Simon-style）小遊戲。
//!
//! 把 ROADMAP 47 起就躺在伺服器、卻從沒前端入口的 8 道食物配方，升級成有玩家技巧
//! 介入的烹調玩法：
//!   1. 開灶（`ClientMsg::StartCook`）→ 伺服器依菜餚難度產一段「步驟次序」（3~5 步），
//!      隨 `CookStart` 送回前端依序閃示（看譜）。
//!   2. 玩家憑記憶把步驟（🔥起鍋→🥬下料→🥄翻炒→🍳翻面→🧂調味）依序敲回。
//!   3. 收尾（`ClientMsg::SubmitCook`）→ 伺服器以 `score_cook` 逐位比對評級
//!      （手忙腳亂／家常／美味／完美），走既有 `recipe.craft` 扣料產菜，評級回饋工匠熟練度。
//!
//! 刻意選一套**全新骨架**（順序記憶），不同於 346 釣魚的反應計時、347 觀星的空間連線、
//! 348 採礦的 press-your-luck。全純記憶體、純函式、零持久化、零 migration。
//!
//! **零平衡風險**：料理與舊一鍵合成走同一條 `recipe.craft` 產出（同樣扣料產一份菜，
//! 不複製道具、不改菜餚效果），評級只回饋工匠熟練度＋飄字＋完美計數 → 即便步序送到
//! 前端（記憶玩法本就要看譜），作弊也毫無誘因（照樣扣自己的料）。

/// 開灶後到下一趟可開灶的冷卻（秒）：擋連開刷灶，比照釣魚 5s。
pub const COOK_COOLDOWN_SECS: f32 = 5.0;

// ── 步驟調色盤 ────────────────────────────────────────────────────────────────

/// 掌勺的五種步驟（順序記憶的「音符」）。
/// 索引（`as_index`）隨此列舉次序為穩定契約，前端閃示與敲回都靠它對齊。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookStep {
    /// 起鍋燒油。
    Heat,
    /// 下食材。
    Add,
    /// 翻炒。
    Stir,
    /// 翻面。
    Flip,
    /// 調味。
    Season,
}

/// 調色盤大小（步序生成的取模基數）。
pub const STEP_KINDS: u64 = 5;

impl CookStep {
    /// 前端用的 snake_case 線格式（隨 `CookStart` 廣播、`SubmitCook` 解析）。
    pub fn as_str(self) -> &'static str {
        match self {
            CookStep::Heat => "heat",
            CookStep::Add => "add",
            CookStep::Stir => "stir",
            CookStep::Flip => "flip",
            CookStep::Season => "season",
        }
    }

    /// 報讀器／飄字用的繁中標籤（i18n 佔位）。
    pub fn label(self) -> &'static str {
        match self {
            CookStep::Heat => "起鍋",
            CookStep::Add => "下料",
            CookStep::Stir => "翻炒",
            CookStep::Flip => "翻面",
            CookStep::Season => "調味",
        }
    }

    /// 索引（0..STEP_KINDS）；步序生成用。
    pub fn as_index(self) -> u64 {
        match self {
            CookStep::Heat => 0,
            CookStep::Add => 1,
            CookStep::Stir => 2,
            CookStep::Flip => 3,
            CookStep::Season => 4,
        }
    }

    /// 由索引取步驟（步序生成用）；超出調色盤回 `None`。
    pub fn from_index(i: u64) -> Option<CookStep> {
        match i {
            0 => Some(CookStep::Heat),
            1 => Some(CookStep::Add),
            2 => Some(CookStep::Stir),
            3 => Some(CookStep::Flip),
            4 => Some(CookStep::Season),
            _ => None,
        }
    }

    /// 由 snake_case 線格式解析（前端敲回的步驟）；未知字串回 `None`（計為敲錯）。
    pub fn from_str(s: &str) -> Option<CookStep> {
        match s {
            "heat" => Some(CookStep::Heat),
            "add" => Some(CookStep::Add),
            "stir" => Some(CookStep::Stir),
            "flip" => Some(CookStep::Flip),
            "season" => Some(CookStep::Season),
            _ => None,
        }
    }
}

// ── 可烹菜餚 ──────────────────────────────────────────────────────────────────

/// 這 8 道食物配方可走「掌勺」玩法；回傳該菜餚的步序長度（難度），非可烹回 `None`。
///
/// 難度分三檔（家常 3／進階 4／大菜 5），大致對齊回血量：越補的菜越多步。
/// 不在表內的配方（武器／護甲／藥水等）一律不可掌勺，仍走既有一鍵合成。
pub fn cook_steps_len(recipe_id: &str) -> Option<usize> {
    match recipe_id {
        // 家常 3 步
        "grilled_fish" | "fried_egg" | "bread" => Some(3),
        // 進階 4 步
        "carrot_soup" | "star_sashimi" | "potato_gratin" => Some(4),
        // 大菜 5 步
        "deep_broth" | "night_potion" => Some(5),
        _ => None,
    }
}

/// 是否為可掌勺的菜餚。
pub fn is_cookable(recipe_id: &str) -> bool {
    cook_steps_len(recipe_id).is_some()
}

/// 依菜餚與確定性種子產出該趟的步序（長度由 `cook_steps_len` 決定）。
///
/// 非可烹配方回空序列。種子建議帶 `player_id_low64 ^ cook_attempt_count`，讓每趟步序不同。
/// 用簡單 LCG 推進、逐步取模調色盤，確定可重現（伺服器存下此序列作為評分標準答案）。
pub fn recipe_steps(recipe_id: &str, seed: u64) -> Vec<CookStep> {
    let len = match cook_steps_len(recipe_id) {
        Some(n) => n,
        None => return Vec::new(),
    };
    let mut out = Vec::with_capacity(len);
    let mut state = seed;
    for _ in 0..len {
        // LCG（數值取自 Numerical Recipes）推進，取高位較均勻。
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let idx = (state >> 33) % STEP_KINDS;
        out.push(CookStep::from_index(idx).unwrap_or(CookStep::Heat));
    }
    out
}

// ── 評級 ──────────────────────────────────────────────────────────────────────

/// 掌勺評級：逐位比對玩家敲回的步序與標準答案的正確率。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookGrade {
    /// 手忙腳亂（正確率 < 40%）。
    Botched,
    /// 家常（正確率 < 75%）。
    Common,
    /// 美味（正確率 < 100%）。
    Tasty,
    /// 完美（全對且長度一致）。
    Perfect,
}

impl CookGrade {
    /// 前端飄字／報讀器用的繁中標籤（i18n 佔位）。
    pub fn label(self) -> &'static str {
        match self {
            CookGrade::Botched => "手忙腳亂",
            CookGrade::Common => "家常",
            CookGrade::Tasty => "美味",
            CookGrade::Perfect => "完美",
        }
    }

    /// 前端用的 snake_case 線格式。
    pub fn as_str(self) -> &'static str {
        match self {
            CookGrade::Botched => "botched",
            CookGrade::Common => "common",
            CookGrade::Tasty => "tasty",
            CookGrade::Perfect => "perfect",
        }
    }

    /// 該評級回饋的工匠熟練度 XP（比照釣魚回饋農夫熟練度；數值保守、由冷卻夾住）。
    pub fn artisan_xp(self) -> u32 {
        match self {
            CookGrade::Botched => 1,
            CookGrade::Common => 2,
            CookGrade::Tasty => 4,
            CookGrade::Perfect => 7,
        }
    }

    /// 是否完美（供累計「完美料理」計數）。
    pub fn is_perfect(self) -> bool {
        matches!(self, CookGrade::Perfect)
    }

    /// 火候到家的額外盛盤份數（ROADMAP 435）：完美掌勺多盛一份同款料理，
    /// 其餘評級不加贈。把照譜小遊戲的「手藝」第一次連到實際產出——
    /// 不是新經濟水龍頭：用的是同一份食材、且需全對才觸發，由小遊戲難度與冷卻自然夾住。
    pub fn bonus_output(self) -> u32 {
        match self {
            CookGrade::Perfect => 1,
            CookGrade::Botched | CookGrade::Common | CookGrade::Tasty => 0,
        }
    }
}

/// 比對玩家敲回的步序（`input`）與標準答案（`target`），分級。
///
/// 逐位比對：只有「位置與步驟都對」才算一個正確；長度不符的位置一律算錯
/// （input 太短／太長都吃虧）。正確率 = 正確數 / 標準答案長度。
/// 全對且長度一致才 `Perfect`。`target` 為空（理論上不會發生）回 `Botched`。
pub fn score_cook(target: &[CookStep], input: &[CookStep]) -> CookGrade {
    if target.is_empty() {
        return CookGrade::Botched;
    }
    let mut correct = 0usize;
    for (i, want) in target.iter().enumerate() {
        if input.get(i) == Some(want) {
            correct += 1;
        }
    }
    // 全對且玩家沒多敲尾巴 → 完美。
    if correct == target.len() && input.len() == target.len() {
        return CookGrade::Perfect;
    }
    let ratio = correct as f32 / target.len() as f32;
    if ratio >= 0.75 {
        CookGrade::Tasty
    } else if ratio >= 0.4 {
        CookGrade::Common
    } else {
        CookGrade::Botched
    }
}

// ── 進行中的一趟掌勺（記憶體前置、重啟清空）───────────────────────────────────

/// 一趟進行中的掌勺：記下這道菜的權威 id 與標準步序，供 `SubmitCook` 評分。
/// 記憶體前置、不入快照、不持久化、零 migration（鏡像 fishing／mining 等記憶體切片）。
#[derive(Debug, Clone, PartialEq)]
pub struct CookSession {
    /// 這道菜的配方 id（crafting 的穩定 wire key，`&'static`）。
    pub recipe_id: &'static str,
    /// 這趟的標準步序（評分標準答案）。
    pub target: Vec<CookStep>,
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_wire_strings_stable() {
        assert_eq!(CookStep::Heat.as_str(), "heat");
        assert_eq!(CookStep::Add.as_str(), "add");
        assert_eq!(CookStep::Stir.as_str(), "stir");
        assert_eq!(CookStep::Flip.as_str(), "flip");
        assert_eq!(CookStep::Season.as_str(), "season");
    }

    #[test]
    fn step_index_roundtrip() {
        for i in 0..STEP_KINDS {
            let s = CookStep::from_index(i).unwrap();
            assert_eq!(s.as_index(), i);
        }
        assert_eq!(CookStep::from_index(STEP_KINDS), None);
    }

    #[test]
    fn step_from_str_roundtrip() {
        for s in [
            CookStep::Heat,
            CookStep::Add,
            CookStep::Stir,
            CookStep::Flip,
            CookStep::Season,
        ] {
            assert_eq!(CookStep::from_str(s.as_str()), Some(s));
        }
        assert_eq!(CookStep::from_str("nope"), None);
        assert_eq!(CookStep::from_str(""), None);
    }

    #[test]
    fn cookable_set_and_lengths() {
        // 家常 3 步
        assert_eq!(cook_steps_len("grilled_fish"), Some(3));
        assert_eq!(cook_steps_len("fried_egg"), Some(3));
        assert_eq!(cook_steps_len("bread"), Some(3));
        // 進階 4 步
        assert_eq!(cook_steps_len("carrot_soup"), Some(4));
        assert_eq!(cook_steps_len("star_sashimi"), Some(4));
        assert_eq!(cook_steps_len("potato_gratin"), Some(4));
        // 大菜 5 步
        assert_eq!(cook_steps_len("deep_broth"), Some(5));
        assert_eq!(cook_steps_len("night_potion"), Some(5));
        // 非可烹（武器／藥水）不可掌勺
        assert_eq!(cook_steps_len("weapon"), None);
        assert_eq!(cook_steps_len("healing_potion"), None);
        assert!(is_cookable("grilled_fish"));
        assert!(!is_cookable("weapon"));
        assert!(!is_cookable("不存在"));
    }

    #[test]
    fn recipe_steps_length_matches_difficulty() {
        assert_eq!(recipe_steps("grilled_fish", 1).len(), 3);
        assert_eq!(recipe_steps("carrot_soup", 1).len(), 4);
        assert_eq!(recipe_steps("deep_broth", 1).len(), 5);
        // 非可烹回空序列
        assert!(recipe_steps("weapon", 1).is_empty());
    }

    #[test]
    fn recipe_steps_deterministic() {
        assert_eq!(recipe_steps("deep_broth", 42), recipe_steps("deep_broth", 42));
        // 不同種子至少有機會不同（抽樣檢查不是全等）。
        let a = recipe_steps("deep_broth", 1);
        let b = recipe_steps("deep_broth", 2);
        // 兩個 5 步序列不應永遠相等（極低機率巧合，但種子 1/2 實測不同）。
        assert_ne!(a, b);
    }

    #[test]
    fn recipe_steps_within_palette() {
        for seed in 0u64..500 {
            for steps in recipe_steps("deep_broth", seed) {
                assert!(steps.as_index() < STEP_KINDS, "步驟超出調色盤");
            }
        }
    }

    #[test]
    fn perfect_needs_exact_match() {
        let target = recipe_steps("grilled_fish", 7);
        // 完全照抄 → 完美
        assert_eq!(score_cook(&target, &target), CookGrade::Perfect);
    }

    #[test]
    fn multi_too_many_steps_not_perfect() {
        let target = vec![CookStep::Heat, CookStep::Add, CookStep::Stir];
        // 前三步全對但尾巴多敲一下 → 非完美（位置全中但長度超標）。
        let mut input = target.clone();
        input.push(CookStep::Season);
        assert_ne!(score_cook(&target, &input), CookGrade::Perfect);
        // 仍有 3/3 位置正確 → 美味（< 100% 因長度多）。
        assert_eq!(score_cook(&target, &input), CookGrade::Tasty);
    }

    #[test]
    fn grade_tiers_by_ratio() {
        // 4 步：3 對（75%）→ 美味；2 對（50%）→ 家常；1 對（25%）→ 手忙腳亂。
        let target = vec![CookStep::Heat, CookStep::Add, CookStep::Stir, CookStep::Flip];
        // 3/4 對（最後一步錯）
        let three = vec![CookStep::Heat, CookStep::Add, CookStep::Stir, CookStep::Heat];
        assert_eq!(score_cook(&target, &three), CookGrade::Tasty);
        // 2/4 對
        let two = vec![CookStep::Heat, CookStep::Add, CookStep::Heat, CookStep::Heat];
        assert_eq!(score_cook(&target, &two), CookGrade::Common);
        // 1/4 對
        let one = vec![CookStep::Heat, CookStep::Heat, CookStep::Heat, CookStep::Heat];
        assert_eq!(score_cook(&target, &one), CookGrade::Botched);
        // 0/4 對
        let zero = vec![CookStep::Season, CookStep::Season, CookStep::Season, CookStep::Season];
        assert_eq!(score_cook(&target, &zero), CookGrade::Botched);
    }

    #[test]
    fn empty_input_is_botched() {
        let target = vec![CookStep::Heat, CookStep::Add, CookStep::Stir];
        assert_eq!(score_cook(&target, &[]), CookGrade::Botched);
    }

    #[test]
    fn empty_target_is_botched() {
        assert_eq!(score_cook(&[], &[CookStep::Heat]), CookGrade::Botched);
    }

    #[test]
    fn too_short_input_loses_positions() {
        // 標準 5 步，只敲對前 2 步（40%）→ 家常邊界。
        let target = vec![
            CookStep::Heat,
            CookStep::Add,
            CookStep::Stir,
            CookStep::Flip,
            CookStep::Season,
        ];
        let short = vec![CookStep::Heat, CookStep::Add];
        assert_eq!(score_cook(&target, &short), CookGrade::Common); // 2/5 = 40%
    }

    #[test]
    fn grade_wire_strings_and_xp() {
        assert_eq!(CookGrade::Botched.as_str(), "botched");
        assert_eq!(CookGrade::Common.as_str(), "common");
        assert_eq!(CookGrade::Tasty.as_str(), "tasty");
        assert_eq!(CookGrade::Perfect.as_str(), "perfect");
        // XP 單調遞增、完美最高。
        assert!(CookGrade::Perfect.artisan_xp() > CookGrade::Tasty.artisan_xp());
        assert!(CookGrade::Tasty.artisan_xp() > CookGrade::Common.artisan_xp());
        assert!(CookGrade::Common.artisan_xp() >= CookGrade::Botched.artisan_xp());
        assert!(CookGrade::Perfect.is_perfect());
        assert!(!CookGrade::Tasty.is_perfect());
    }

    #[test]
    fn perfect_cook_grants_one_bonus_portion() {
        // ROADMAP 435：只有完美掌勺多盛一份，其餘評級不加贈（手藝才有回報、非數值水龍頭）。
        assert_eq!(CookGrade::Perfect.bonus_output(), 1);
        assert_eq!(CookGrade::Tasty.bonus_output(), 0);
        assert_eq!(CookGrade::Common.bonus_output(), 0);
        assert_eq!(CookGrade::Botched.bonus_output(), 0);
        // 完美才有加贈：bonus 與 is_perfect 對齊。
        for g in [
            CookGrade::Botched,
            CookGrade::Common,
            CookGrade::Tasty,
            CookGrade::Perfect,
        ] {
            assert_eq!(g.bonus_output() > 0, g.is_perfect(), "{g:?}");
        }
    }
}
