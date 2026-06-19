//! 牧場系統（ROADMAP 48）——農田地塊養雞產蛋。
//!
//! 玩家在「農田（Farm）」類型的地塊上花乙太購入雞，雞每 60 秒自動下蛋（最多堆積
//! 10 顆）。玩家靠近自己的地塊可「收雞蛋」，每次收雞蛋給 8 點農夫熟練度 XP。
//! 雞蛋可賣給 NPC（2 乙太/顆）或合成「煎蛋」（2 顆 → 1 煎蛋，回血 10）。
//!
//! 這是 Phase 2「深度與自動化」的第一步：在自己買的農田地塊上看到雞在跑，
//! 並多了一條農夫熟練度提升的活動路線。
//!
//! ## ROADMAP 368：牧群孳息（flock brood & hatch）
//! 自 48 起，雞群就只能「花錢買」、買到 3 隻就到頂、之後永遠原地不動——你照不照顧
//! 牠都一個樣。本切片第一次讓**你的照料有後果**：細心收蛋（讓母雞安心）、又在窩裡
//! 留下一兩顆蛋讓牠孵，母雞就會孵出小雞、小雞長成新的母雞——牧群**不花一文錢、靠你
//! 用心養，自己越養越熱鬧**。湧現由「玩家怎麼照料」驅動（真能動性），且雞越多窩越暖、
//! 孵得越快（個體數牽動群體孵化），是非社交、非環境調制維度的另一種群體成長。
//!   - **孵化只能養出 1 隻 earned 母雞**（`FLOCK_CAP`=4 > 購買上限 `MAX_CHICKENS`=3）：
//!     第 4 隻只能靠孵、買不到——「圓滿的牧群只能用心養出來」。經濟近乎零擾動：至多
//!     多 1 隻雞、且孵化會**吃掉一顆窩裡的蛋**（一顆蛋換一隻雞，緩慢、難得），蛋上限
//!     仍是 10、每顆收成乙太不變。
//!
//! 設計取捨：
//!   - **記憶體模式**：雞與蛋不寫 DB（同 pet.rs 做法），重啟後玩家重新購入。
//!     好處：零 migration 風險，快速上線；代價：每次伺服器重啟要重買雞。
//!   - **每塊農田最多買 3 隻雞**：避免堆雞刷蛋；第 4 隻得靠孵化「掙」來。
//!   - **蛋最多堆 10 顆**：防無限堆積，逼玩家定期來收。

use std::collections::HashMap;

/// 購入一隻雞的乙太費用。
pub const BUY_CHICKEN_COST: u32 = 15;

/// 每隻雞每批下蛋的間隔（秒）。多隻雞共用同一計時器、同時下蛋。
pub const EGG_INTERVAL_SECS: f32 = 60.0;

/// 農夫熟練度 XP（每次收雞蛋操作）。
pub const COLLECT_FARMER_XP: u32 = 8;

/// 單塊農田地塊**可購買**的雞隻上限（花乙太買到此數就到頂）。
pub const MAX_CHICKENS: u32 = 3;

/// 牧群的**實際**上限（含孵化掙來的）。比購買上限多 1：第 `FLOCK_CAP` 隻買不到，
/// 只能靠細心照料孵出來——「圓滿的牧群只能用心養出來」。
pub const FLOCK_CAP: u32 = 4;

/// 農田地塊上雞蛋的最大堆積數。
pub const MAX_EGGS: u32 = 10;

// ─── ROADMAP 368：牧群孳息（brood & hatch）─────────────────────────────────────
//
// 機制：母雞「安心」（玩家近期收過蛋＝有在照料）且窩裡留有蛋可孵時，會累積孵育進度；
// 雞越多窩越暖、孵得越快（個體數牽動群體）。進度滿則孵化一隻小雞（吃掉窩裡一顆蛋），
// 小雞經一段時間長成新母雞。全程純啟發式、確定性、零 LLM、零持久化、記憶體模式。

/// 收一次蛋後「母雞安心」的維持秒數——這段時間內牧群才會孵育。逼玩家**規律照料**
/// （長時間不來收蛋，母雞便不再孵）。
pub const TENDED_WINDOW_SECS: f32 = 240.0;

/// 孵育滿格所需的「安心孵育」累計量（單位＝雞·秒）。孵育速率＝當前母雞數（雞越多越快），
/// 故 1 隻雞約需 `BROOD_THRESHOLD` 秒、2 隻約半、3 隻約 1/3——刻意偏長，讓添丁緩慢難得。
pub const BROOD_THRESHOLD: f32 = 300.0;

/// 小雞長成母雞所需的時間（秒）。
pub const CHICK_MATURE_SECS: f32 = 180.0;

// ─── ROADMAP 409：牧群羈絆（flock bond & 暖心金蛋）──────────────────────────────
//
// 自 48／368 起，母雞對玩家始終只是「會下蛋的數字」——你照不照顧、來得勤不勤，牠對你的
// 態度一模一樣。368 讓照料長出「牧群的量」（孵小雞），本切片第一次讓照料長出「牧群對你
// 的感情」：每次親手來收蛋（用心照顧）就攢一分羈絆，母雞漸漸從陌生→熟悉→親近→黏人；
// 羈絆夠深的牧群會認得你、在你來時圍到身邊撒嬌，偶爾還下一顆暖心金蛋——撿起來心頭一暖
// （沿用煎蛋的暖食飽足、純緩慢回血，零經濟、零新物品）。久不照料，感情會慢慢淡（不懲罰、
// 只是溫和退回）。全程記憶體模式、確定性、零持久化、零 migration、零 LLM。
//
// 與 368 分工乾淨：368 是「量」（孵化長出第幾隻雞），本切片是「關係」（牧群對你多親）；
// 與 pet_personality（358，玩家寵物的個體脾氣）也不同：那是個體先天性格雜湊、不隨互動變，
// 這裡是整群牧群對「你照顧得勤不勤」的後天感情，會隨你的行為累積與消退。

/// 每次親手收蛋（用心照顧一回）攢得的羈絆值。
pub const BOND_PER_COLLECT: f32 = 1.0;

/// 羈絆值上限（攢到此即封頂，避免無限累積）。
pub const BOND_MAX: f32 = 20.0;

/// 久不照料（`tended_secs` 已歸零）時，羈絆每秒退潮的速度。刻意極緩——
/// 約 100 秒退 1 分，規律來收蛋的玩家幾乎不掉，長期不理才慢慢淡回陌生。
pub const BOND_DECAY_PER_SEC: f32 = 0.01;

/// 羈絆四階的門檻（羈絆值 ≥ 門檻即進入該階）。
pub const FAMILIAR_AT: f32 = 3.0; // 熟悉：來收過幾回，母雞不再怕生
pub const CLOSE_AT: f32 = 8.0; // 親近：認得你了，偶爾下暖心金蛋
pub const CUDDLY_AT: f32 = 15.0; // 黏人：你一來就圍到身邊撒嬌、金蛋更勤

/// 牧群對地主的羈絆階位（後天、隨照料累積與消退）。
///
/// 變體由淺至深排序（`PartialOrd`／`Ord` 即此序），是 `from_bond` 與前端愛心數的穩定契約——
/// **只可尾端追加、不可重排**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BondTier {
    /// 陌生：剛養、或久未照料退回——母雞對你還生分。
    Stranger,
    /// 熟悉：來收過幾回蛋，母雞不再怕生。
    Familiar,
    /// 親近：認得你了，偶爾下一顆暖心金蛋。
    Close,
    /// 黏人：最深的羈絆——你一來牠們就圍到身邊撒嬌，金蛋下得更勤。
    Cuddly,
}

impl BondTier {
    /// 由羈絆值判定階位（確定性、純查表）。
    pub fn from_bond(bond: f32) -> BondTier {
        if !bond.is_finite() || bond < FAMILIAR_AT {
            BondTier::Stranger
        } else if bond < CLOSE_AT {
            BondTier::Familiar
        } else if bond < CUDDLY_AT {
            BondTier::Close
        } else {
            BondTier::Cuddly
        }
    }

    /// 前端在牧場上方畫的愛心數（陌生 0、熟悉 1、親近 2、黏人 3）。
    pub fn hearts(&self) -> u8 {
        match self {
            BondTier::Stranger => 0,
            BondTier::Familiar => 1,
            BondTier::Close => 2,
            BondTier::Cuddly => 3,
        }
    }

    /// 是否親到「會下暖心金蛋」的程度（親近以上）。
    pub fn lays_golden(&self) -> bool {
        *self >= BondTier::Close
    }

    /// 穩定 wire key（snake_case 契約，給前端對應文字與演出）。
    pub fn wire(&self) -> &'static str {
        match self {
            BondTier::Stranger => "stranger",
            BondTier::Familiar => "familiar",
            BondTier::Close => "close",
            BondTier::Cuddly => "cuddly",
        }
    }
}

/// 親近以上的牧群每下一批蛋時，是否額外凝出一顆暖心金蛋（確定性、好測）。
/// 親近約 1/6 批、黏人約 1/4 批——刻意稀少難得；非親近階一律 false。
/// `seed` 用該批的下蛋序號（`egg_batches`），同一批永遠同結果、可重現。
pub fn roll_golden(seed: u64, tier: BondTier) -> bool {
    let chance: u64 = match tier {
        BondTier::Cuddly => 4,
        BondTier::Close => 6,
        _ => return false,
    };
    // 簡單確定性雜湊，避免相鄰 seed 規律可預測。
    let h = seed.wrapping_mul(2_654_435_761).rotate_left(13);
    h % chance == 0
}

/// 收一次蛋的結果（ROADMAP 409 起改用結構，攜帶羈絆與金蛋資訊）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CollectOutcome {
    /// 本次收得的蛋數（含金蛋；0＝無蛋或地塊不存在）。
    pub eggs: u32,
    /// 本次給的農夫熟練度 XP。
    pub xp: u32,
    /// 本次收得的暖心金蛋顆數（已含在 `eggs` 內，供前端演出與套暖食）。
    pub golden: u32,
    /// 若這次照顧讓羈絆升上新階位，帶該新階位（供升階慶賀）；否則 None。
    pub bond_up: Option<BondTier>,
}

/// 單一農田地塊的牧場狀態（記憶體模式）。
#[derive(Debug, Clone)]
pub struct RanchState {
    /// 現有成年母雞數（0~FLOCK_CAP；購買至多 MAX_CHICKENS，其餘靠孵化）。
    pub chicken_count: u32,
    /// 目前堆積的蛋數（0~MAX_EGGS）。
    pub egg_count: u32,
    /// 距下一批下蛋還剩幾秒（每 tick 由 game.rs 遞減）。
    pub egg_timer: f32,
    /// 本地塊已完成的下蛋批次（用於偽隨機種子）。
    pub egg_batches: u64,
    /// ROADMAP 368：「母雞安心」剩餘秒數——收蛋時補滿 `TENDED_WINDOW_SECS`，每 tick 遞減；
    /// >0 表示近期有照料、牧群會孵育。
    pub tended_secs: f32,
    /// ROADMAP 368：累計孵育進度（雞·秒），滿 `BROOD_THRESHOLD` 即孵化一隻小雞。
    pub brood_accum: f32,
    /// ROADMAP 368：小雞成長剩餘秒數；>0 表示窩裡有一隻小雞正在長大（同時至多孵一隻）。
    pub chick_secs: f32,
    /// ROADMAP 409：牧群對地主的羈絆值（0~BOND_MAX）。每次用心收蛋 +1，久不照料慢慢退潮。
    pub bond: f32,
    /// ROADMAP 409：窩裡待收的暖心金蛋顆數（已計入 egg_count，是其中特別的幾顆）。
    pub golden_eggs: u32,
}

impl RanchState {
    fn new() -> Self {
        Self {
            chicken_count: 0,
            egg_count: 0,
            egg_timer: EGG_INTERVAL_SECS,
            egg_batches: 0,
            tended_secs: 0.0,
            brood_accum: 0.0,
            chick_secs: 0.0,
            bond: 0.0,
            golden_eggs: 0,
        }
    }

    /// 牧群當前的羈絆階位（純讀、即時算）。
    pub fn bond_tier(&self) -> BondTier {
        BondTier::from_bond(self.bond)
    }

    /// 窩裡是否有一隻小雞正在長大。
    pub fn has_chick(&self) -> bool {
        self.chick_secs > 0.0
    }
}

/// 全伺服器所有農田地塊的牧場狀態（記憶體模式）。
#[derive(Default)]
pub struct RanchRegistry {
    plots: HashMap<u32, RanchState>,
}

impl RanchRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 嘗試在 `plot_id` 地塊購入一隻雞。
    /// - 失敗條件：地塊不存在（呼叫端保證農田類型）、已達 MAX_CHICKENS。
    /// - 成功：雞數 +1，回 `true`。
    pub fn buy_chicken(&mut self, plot_id: u32) -> bool {
        let state = self.plots.entry(plot_id).or_insert_with(RanchState::new);
        if state.chicken_count >= MAX_CHICKENS {
            return false;
        }
        state.chicken_count += 1;
        true
    }

    /// 收取 `plot_id` 地塊的雞蛋。
    /// 回傳 `CollectOutcome`（蛋數、XP、暖心金蛋顆數、是否升羈絆）；無蛋或地塊不存在時回預設（全 0）。
    pub fn collect_eggs(&mut self, plot_id: u32) -> CollectOutcome {
        let state = match self.plots.get_mut(&plot_id) {
            Some(s) => s,
            None => return CollectOutcome::default(),
        };
        let eggs = state.egg_count;
        if eggs == 0 {
            return CollectOutcome::default();
        }
        let golden = state.golden_eggs.min(eggs);
        state.egg_count = 0;
        state.golden_eggs = 0;
        // ROADMAP 368：來收過蛋＝有在照料，母雞安心，重置安心計時器。牧群只在被規律照料時孵育。
        state.tended_secs = TENDED_WINDOW_SECS;
        // ROADMAP 409：親手收蛋＝用心照顧一回，攢一分羈絆（封頂）；跨過階位門檻則回報升階。
        let before = state.bond_tier();
        state.bond = (state.bond + BOND_PER_COLLECT).min(BOND_MAX);
        let after = state.bond_tier();
        let bond_up = if after > before { Some(after) } else { None };
        CollectOutcome { eggs, xp: COLLECT_FARMER_XP, golden, bond_up }
    }

    /// 取得某地塊的雞隻數與蛋數（供快照廣播用）。地塊不存在回 `(0, 0)`。
    pub fn state_of(&self, plot_id: u32) -> (u32, u32) {
        self.plots.get(&plot_id).map(|s| (s.chicken_count, s.egg_count)).unwrap_or((0, 0))
    }

    /// 取得某地塊是否有正在長大的小雞（供測試／快照）。地塊不存在回 `false`。
    pub fn has_chick(&self, plot_id: u32) -> bool {
        self.plots.get(&plot_id).map(|s| s.has_chick()).unwrap_or(false)
    }

    /// 每遊戲 tick 更新所有有雞地塊的蛋計時器，並推進 ROADMAP 368 孵育/成長。
    pub fn tick(&mut self, dt: f32) {
        // 防呆：非有限或非正的 dt 直接跳過（守 prod 既有早退慣例）。
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        for state in self.plots.values_mut() {
            if state.chicken_count == 0 {
                // 無雞：清掉殘留的孵育/安心進度（避免賣光雞後又回填），不產蛋；羈絆與金蛋一併歸零。
                state.tended_secs = 0.0;
                state.brood_accum = 0.0;
                state.chick_secs = 0.0;
                state.bond = 0.0;
                state.golden_eggs = 0;
                continue;
            }
            state.egg_timer -= dt;
            if state.egg_timer <= 0.0 {
                state.egg_timer = EGG_INTERVAL_SECS;
                // 每批下蛋數：1~2 顆，由確定性種子決定。
                let seed = state.egg_batches;
                let batch = roll_egg_batch(seed);
                state.egg_batches = state.egg_batches.wrapping_add(1);
                // 雞多下得多（雞隻數倍乘），但不超過 MAX_EGGS 上限。
                let produced = (batch * state.chicken_count).min(MAX_EGGS - state.egg_count.min(MAX_EGGS));
                state.egg_count = (state.egg_count + produced).min(MAX_EGGS);
                // ROADMAP 409：親近以上的牧群，這批偶爾凝出一顆暖心金蛋（不超過窩裡實際蛋數）。
                if produced > 0 && roll_golden(seed, state.bond_tier()) {
                    state.golden_eggs = (state.golden_eggs + 1).min(state.egg_count);
                }
            }
            // ROADMAP 409：久不照料（安心已歸零）時，羈絆極緩退潮回陌生（不懲罰、只溫和淡回）。
            if state.tended_secs <= 0.0 {
                state.bond = (state.bond - BOND_DECAY_PER_SEC * dt).max(0.0);
            }

            // ── ROADMAP 368：牧群孳息 ──
            // 安心計時器遞減（收蛋時補滿）。
            state.tended_secs = (state.tended_secs - dt).max(0.0);
            if state.has_chick() {
                // 已有小雞在長大：推進成長，到期長成一隻新母雞（封頂 FLOCK_CAP）。
                state.chick_secs -= dt;
                if state.chick_secs <= 0.0 {
                    state.chick_secs = 0.0;
                    state.chicken_count = (state.chicken_count + 1).min(FLOCK_CAP);
                }
            } else {
                // 尚無小雞：在「安心＋有蛋可孵＋未滿群」時累積孵育，雞越多越快。
                let rate = brood_rate(state.chicken_count, state.tended_secs > 0.0, state.egg_count);
                if rate > 0.0 {
                    state.brood_accum += rate * dt;
                    if state.brood_accum >= BROOD_THRESHOLD {
                        // 孵化：吃掉窩裡一顆蛋換一隻小雞，重置孵育進度、起跑成長計時。
                        state.brood_accum = 0.0;
                        state.egg_count = state.egg_count.saturating_sub(1);
                        state.chick_secs = CHICK_MATURE_SECS;
                    }
                }
            }
        }
    }

    /// 匯出全部有活躍狀態（有雞或有蛋）的地塊快照（供 Snapshot 廣播）。
    pub fn all_active_views(&self) -> Vec<RanchPlotView> {
        self.plots.iter()
            .filter(|(_, s)| s.chicken_count > 0 || s.egg_count > 0)
            .map(|(&plot_id, s)| RanchPlotView {
                plot_id,
                chicken_count: s.chicken_count,
                egg_count: s.egg_count,
                chick: s.has_chick(),
                brooding: !s.has_chick()
                    && brood_rate(s.chicken_count, s.tended_secs > 0.0, s.egg_count) > 0.0,
                bond_hearts: s.bond_tier().hearts(),
                golden: s.golden_eggs > 0,
            })
            .collect()
    }
}

/// 確定性種子決定本批下 1 或 2 顆蛋（各 50%）。
pub fn roll_egg_batch(seed: u64) -> u32 {
    if seed % 2 == 0 { 2 } else { 1 }
}

/// ROADMAP 368：孵育速率（雞·秒／秒）——純函式、確定性、好測。
/// 只在「有母雞 ＆ 近期有照料（安心）＆ 窩裡有蛋可孵 ＆ 牧群未滿」時 > 0，
/// 且速率＝當前母雞數（雞越多窩越暖、孵得越快——個體數牽動群體）。否則回 0。
pub fn brood_rate(chicken_count: u32, tended: bool, egg_count: u32) -> f32 {
    if chicken_count >= 1 && chicken_count < FLOCK_CAP && tended && egg_count >= 1 {
        chicken_count as f32
    } else {
        0.0
    }
}

/// 快照裡一塊農田地塊的牧場可見狀態（送給前端）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RanchPlotView {
    pub plot_id: u32,
    pub chicken_count: u32,
    pub egg_count: u32,
    /// ROADMAP 368：窩裡是否有一隻小雞正在長大（前端畫 🐤）。
    #[serde(default)]
    pub chick: bool,
    /// ROADMAP 368：是否正在孵育中（安心＋有蛋＋未滿群，前端可提示「孵育中」）。
    #[serde(default)]
    pub brooding: bool,
    /// ROADMAP 409：牧群羈絆的愛心數（0 陌生／1 熟悉／2 親近／3 黏人），前端在牧場上方畫愛心、
    /// 達 3（黏人）時母雞會圍到地主身邊撒嬌。
    #[serde(default)]
    pub bond_hearts: u8,
    /// ROADMAP 409：窩裡是否有暖心金蛋待收（前端畫金光）。
    #[serde(default)]
    pub golden: bool,
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 購雞基本流程：首次購雞成功，雞數增加。
    #[test]
    fn buy_chicken_success() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(0));
        assert_eq!(reg.state_of(0), (1, 0));
    }

    /// 同一地塊可購至 MAX_CHICKENS，超出則拒絕。
    #[test]
    fn buy_chicken_respects_max() {
        let mut reg = RanchRegistry::new();
        for _ in 0..MAX_CHICKENS {
            assert!(reg.buy_chicken(5));
        }
        assert!(!reg.buy_chicken(5), "超過上限應被拒");
        assert_eq!(reg.state_of(5).0, MAX_CHICKENS);
    }

    /// 收蛋：有蛋才能收，收完歸零，回傳蛋數與農夫 XP。
    #[test]
    fn collect_eggs_works() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(1));
        // 直接注入幾顆蛋模擬 tick 完畢。
        reg.plots.get_mut(&1).unwrap().egg_count = 3;
        let out = reg.collect_eggs(1);
        assert_eq!(out.eggs, 3);
        assert_eq!(out.xp, COLLECT_FARMER_XP);
        assert_eq!(reg.state_of(1).1, 0, "收完後應歸零");
    }

    /// 無蛋時收蛋回傳預設（全 0）。
    #[test]
    fn collect_eggs_none_when_empty() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(2));
        let out = reg.collect_eggs(2);
        assert_eq!((out.eggs, out.xp), (0, 0));
    }

    /// 地塊不存在時收蛋回傳預設（全 0）。
    #[test]
    fn collect_eggs_nonexistent_plot() {
        let mut reg = RanchRegistry::new();
        assert_eq!(reg.collect_eggs(999), CollectOutcome::default());
    }

    /// tick 使計時器倒數，到期後生蛋。
    #[test]
    fn tick_produces_eggs_after_interval() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(3));
        // 大量 tick 讓計時器跑完。
        reg.tick(EGG_INTERVAL_SECS + 0.1);
        let (_, eggs) = reg.state_of(3);
        assert!(eggs > 0, "計時器到期後應有蛋；實際蛋數 = {eggs}");
    }

    /// 沒有雞的地塊不應累積蛋。
    #[test]
    fn tick_no_eggs_without_chickens() {
        let mut reg = RanchRegistry::new();
        // 手動建 state 但雞數 0（不透過 buy_chicken）。
        reg.plots.insert(4, RanchState::new());
        reg.tick(EGG_INTERVAL_SECS * 5.0);
        assert_eq!(reg.state_of(4).1, 0, "無雞不應產蛋");
    }

    /// 蛋不超過 MAX_EGGS 上限。
    #[test]
    fn egg_count_capped_at_max() {
        let mut reg = RanchRegistry::new();
        for _ in 0..MAX_CHICKENS {
            reg.buy_chicken(6);
        }
        // 跑夠多批，蛋不斷累積。
        for _ in 0..20 {
            reg.tick(EGG_INTERVAL_SECS + 0.1);
        }
        let (_, eggs) = reg.state_of(6);
        assert!(eggs <= MAX_EGGS, "蛋不得超過 MAX_EGGS={MAX_EGGS}；實際 = {eggs}");
    }

    /// roll_egg_batch：覆蓋 1 和 2 兩種結果。
    #[test]
    fn roll_egg_batch_covers_both_values() {
        let has_one = (0..10).any(|i| roll_egg_batch(i) == 1);
        let has_two = (0..10).any(|i| roll_egg_batch(i) == 2);
        assert!(has_one, "應有返回 1 的種子");
        assert!(has_two, "應有返回 2 的種子");
    }

    /// BUY_CHICKEN_COST 在合理範圍（1~100 乙太）。
    #[test]
    fn buy_chicken_cost_is_reasonable() {
        assert!(BUY_CHICKEN_COST >= 1 && BUY_CHICKEN_COST <= 100);
    }

    // ─── ROADMAP 368：牧群孳息 ───────────────────────────────────────────────

    /// 購買到頂只到 MAX_CHICKENS（3），第 4 隻買不到（得靠孵化）。
    #[test]
    fn buy_capped_below_flock_cap() {
        let mut reg = RanchRegistry::new();
        for _ in 0..MAX_CHICKENS {
            assert!(reg.buy_chicken(0));
        }
        assert!(!reg.buy_chicken(0), "購買不得超過 MAX_CHICKENS");
        assert_eq!(reg.state_of(0).0, MAX_CHICKENS);
        assert!(MAX_CHICKENS < FLOCK_CAP, "孵化上限應高於購買上限");
    }

    /// brood_rate：齊備條件才 > 0，且雞越多越快。
    #[test]
    fn brood_rate_conditions() {
        // 無雞 → 0
        assert_eq!(brood_rate(0, true, 5), 0.0);
        // 未照料 → 0
        assert_eq!(brood_rate(2, false, 5), 0.0);
        // 窩裡無蛋 → 0
        assert_eq!(brood_rate(2, true, 0), 0.0);
        // 牧群已滿（FLOCK_CAP）→ 0
        assert_eq!(brood_rate(FLOCK_CAP, true, 5), 0.0);
        // 齊備：速率＝雞數，雞越多越快
        assert_eq!(brood_rate(1, true, 1), 1.0);
        assert!(brood_rate(3, true, 1) > brood_rate(1, true, 1), "雞越多孵越快");
    }

    /// 收蛋會讓母雞「安心」（補滿 tended_secs）。
    #[test]
    fn collect_eggs_marks_tended() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(1));
        reg.plots.get_mut(&1).unwrap().egg_count = 2;
        assert_eq!(reg.plots.get(&1).unwrap().tended_secs, 0.0);
        reg.collect_eggs(1);
        assert_eq!(reg.plots.get(&1).unwrap().tended_secs, TENDED_WINDOW_SECS);
    }

    /// 孵化瞬間：達門檻時孵出一隻小雞、**吃掉窩裡一顆蛋**、重置孵育進度。
    #[test]
    fn hatch_consumes_one_egg() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(2)); // 1 隻母雞
        {
            let s = reg.plots.get_mut(&2).unwrap();
            s.tended_secs = TENDED_WINDOW_SECS;
            s.egg_count = 5;
            s.brood_accum = BROOD_THRESHOLD - 0.5; // 差一步達標
            s.egg_timer = 100.0; // 大到單次 tick 不會下蛋，孤立蛋數斷言
        }
        reg.tick(1.0); // rate=1.0 → 越過門檻 → 孵化
        assert!(reg.has_chick(2), "達門檻後應孵出一隻小雞");
        assert_eq!(reg.state_of(2).1, 4, "孵化吃掉窩裡一顆蛋（5→4）");
        assert_eq!(reg.plots.get(&2).unwrap().brood_accum, 0.0, "孵化後孵育進度歸零");
        assert_eq!(reg.state_of(2).0, 1, "小雞長大前母雞仍 1 隻");
    }

    /// 完整成長流程：孵出小雞後，經 CHICK_MATURE_SECS 長成一隻新母雞。
    #[test]
    fn chick_matures_into_new_hen() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(9));
        reg.plots.get_mut(&9).unwrap().chick_secs = CHICK_MATURE_SECS;
        let mut t = 0.0;
        while t < CHICK_MATURE_SECS + 5.0 && reg.state_of(9).0 < 2 {
            reg.tick(1.0);
            t += 1.0;
        }
        assert_eq!(reg.state_of(9).0, 2, "小雞長成後母雞應 +1");
        assert!(!reg.has_chick(9), "長成後窩裡不再有小雞");
    }

    /// 牧群孵化封頂於 FLOCK_CAP（不會無限孵）。
    #[test]
    fn flock_growth_capped_at_flock_cap() {
        let mut reg = RanchRegistry::new();
        for _ in 0..MAX_CHICKENS {
            reg.buy_chicken(3);
        }
        // 持續供給安心＋蛋，狂跑很久。
        for _ in 0..5000 {
            {
                let s = reg.plots.get_mut(&3).unwrap();
                s.tended_secs = TENDED_WINDOW_SECS;
                if s.egg_count == 0 {
                    s.egg_count = MAX_EGGS;
                }
            }
            reg.tick(1.0);
        }
        assert_eq!(reg.state_of(3).0, FLOCK_CAP, "牧群不得超過 FLOCK_CAP");
    }

    /// 不照料（tended_secs 自然歸零）就不孵育——孵育進度不增長。
    #[test]
    fn no_brood_without_tending() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(4));
        {
            let s = reg.plots.get_mut(&4).unwrap();
            s.tended_secs = 0.0; // 從未照料
            s.egg_count = MAX_EGGS;
        }
        for _ in 0..1000 {
            // 每輪都把蛋補滿、但絕不照料。
            reg.plots.get_mut(&4).unwrap().egg_count = MAX_EGGS;
            reg.tick(1.0);
        }
        assert!(!reg.has_chick(4), "未照料不應孵化");
        assert_eq!(reg.state_of(4).0, 1, "未照料牧群不長");
    }

    /// 賣光雞後（chicken_count=0）孵育/安心/小雞進度全清零。
    #[test]
    fn clears_progress_when_no_chickens() {
        let mut reg = RanchRegistry::new();
        reg.plots.insert(7, RanchState {
            chicken_count: 0,
            egg_count: 0,
            egg_timer: EGG_INTERVAL_SECS,
            egg_batches: 0,
            tended_secs: 100.0,
            brood_accum: 100.0,
            chick_secs: 50.0,
            bond: 10.0,
            golden_eggs: 2,
        });
        reg.tick(1.0);
        let s = reg.plots.get(&7).unwrap();
        assert_eq!((s.tended_secs, s.brood_accum, s.chick_secs), (0.0, 0.0, 0.0));
        // ROADMAP 409：賣光雞，羈絆與金蛋一併歸零。
        assert_eq!(s.bond, 0.0, "無雞應清空羈絆");
        assert_eq!(s.golden_eggs, 0, "無雞應清空金蛋");
    }

    // ─── ROADMAP 409：牧群羈絆 ───────────────────────────────────────────────

    /// 羈絆階位門檻判定正確（含界值與壞值保守回陌生）。
    #[test]
    fn bond_tier_thresholds() {
        assert_eq!(BondTier::from_bond(0.0), BondTier::Stranger);
        assert_eq!(BondTier::from_bond(FAMILIAR_AT - 0.01), BondTier::Stranger);
        assert_eq!(BondTier::from_bond(FAMILIAR_AT), BondTier::Familiar);
        assert_eq!(BondTier::from_bond(CLOSE_AT), BondTier::Close);
        assert_eq!(BondTier::from_bond(CUDDLY_AT), BondTier::Cuddly);
        assert_eq!(BondTier::from_bond(BOND_MAX), BondTier::Cuddly);
        // 壞值（NaN／負）保守回陌生、不 panic。
        assert_eq!(BondTier::from_bond(f32::NAN), BondTier::Stranger);
        assert_eq!(BondTier::from_bond(-5.0), BondTier::Stranger);
    }

    /// 階位由淺至深嚴格遞增、愛心數 0~3 對應。
    #[test]
    fn bond_tier_order_and_hearts() {
        assert!(BondTier::Stranger < BondTier::Familiar);
        assert!(BondTier::Familiar < BondTier::Close);
        assert!(BondTier::Close < BondTier::Cuddly);
        assert_eq!(BondTier::Stranger.hearts(), 0);
        assert_eq!(BondTier::Familiar.hearts(), 1);
        assert_eq!(BondTier::Close.hearts(), 2);
        assert_eq!(BondTier::Cuddly.hearts(), 3);
        // 只有親近以上會下金蛋。
        assert!(!BondTier::Familiar.lays_golden());
        assert!(BondTier::Close.lays_golden());
        assert!(BondTier::Cuddly.lays_golden());
    }

    /// 每次收蛋攢一分羈絆、封頂於 BOND_MAX，且剛跨門檻才回報升階。
    #[test]
    fn collecting_grows_bond_and_reports_tier_up() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(1));
        // 收滿 FAMILIAR_AT 次的前一回不升階、剛好那回升到熟悉。
        let mut last_up = None;
        for _ in 0..FAMILIAR_AT as u32 {
            reg.plots.get_mut(&1).unwrap().egg_count = 1; // 每回補一顆蛋好收
            last_up = reg.collect_eggs(1).bond_up;
        }
        assert_eq!(last_up, Some(BondTier::Familiar), "剛跨熟悉門檻該回報升階");
        // 同階再收不重複回報升階。
        reg.plots.get_mut(&1).unwrap().egg_count = 1;
        assert_eq!(reg.collect_eggs(1).bond_up, None, "未跨新門檻不回報升階");
        // 狂收封頂不超過 BOND_MAX。
        for _ in 0..100 {
            reg.plots.get_mut(&1).unwrap().egg_count = 1;
            reg.collect_eggs(1);
        }
        assert!(reg.plots.get(&1).unwrap().bond <= BOND_MAX);
        assert_eq!(reg.plots.get(&1).unwrap().bond_tier(), BondTier::Cuddly);
    }

    /// 久不照料（安心歸零）羈絆會極緩退潮，最終回到 0／陌生。
    #[test]
    fn bond_decays_when_neglected() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(2));
        {
            let s = reg.plots.get_mut(&2).unwrap();
            s.bond = 5.0;
            s.tended_secs = 0.0; // 久未照料
            s.egg_timer = 1e9; // 不下蛋干擾
        }
        // 5.0 / 0.01 = 500 秒可退乾淨；多跑些確保歸零。
        for _ in 0..600 {
            reg.plots.get_mut(&2).unwrap().tended_secs = 0.0;
            reg.tick(1.0);
        }
        assert_eq!(reg.plots.get(&2).unwrap().bond, 0.0, "久不照料應退回 0");
        assert_eq!(reg.plots.get(&2).unwrap().bond_tier(), BondTier::Stranger);
    }

    /// 金蛋判定：非親近階一律不下；親近／黏人在對的種子下會下、且確定可重現。
    #[test]
    fn golden_roll_only_for_close_and_above() {
        // 陌生／熟悉一律 false（掃一段種子皆不下）。
        for seed in 0..50u64 {
            assert!(!roll_golden(seed, BondTier::Stranger));
            assert!(!roll_golden(seed, BondTier::Familiar));
        }
        // 親近／黏人至少有命中（在某些種子會下金蛋）。
        let close_hits = (0..50u64).filter(|&s| roll_golden(s, BondTier::Close)).count();
        let cuddly_hits = (0..50u64).filter(|&s| roll_golden(s, BondTier::Cuddly)).count();
        assert!(close_hits > 0, "親近應偶爾下金蛋");
        assert!(cuddly_hits > 0, "黏人應偶爾下金蛋");
        // 黏人比親近更勤（機率更高）。
        assert!(cuddly_hits >= close_hits, "黏人下金蛋應不少於親近");
        // 同種子同階位永遠同結果（確定可重現）。
        assert_eq!(roll_golden(7, BondTier::Cuddly), roll_golden(7, BondTier::Cuddly));
    }

    /// 收蛋會把窩裡的金蛋一併收走並回報顆數、收完歸零。
    #[test]
    fn collect_returns_and_clears_golden() {
        let mut reg = RanchRegistry::new();
        assert!(reg.buy_chicken(3));
        {
            let s = reg.plots.get_mut(&3).unwrap();
            s.egg_count = 4;
            s.golden_eggs = 2;
        }
        let out = reg.collect_eggs(3);
        assert_eq!(out.eggs, 4);
        assert_eq!(out.golden, 2, "金蛋顆數應一併回報");
        assert_eq!(reg.plots.get(&3).unwrap().golden_eggs, 0, "收完金蛋歸零");
    }

    /// tick 對非有限/非正 dt 早退、不 panic。
    #[test]
    fn tick_guards_bad_dt() {
        let mut reg = RanchRegistry::new();
        reg.buy_chicken(8);
        reg.tick(f32::NAN);
        reg.tick(-1.0);
        reg.tick(0.0);
        // 不 panic、狀態不變即可。
        assert_eq!(reg.state_of(8), (1, 0));
    }
}
