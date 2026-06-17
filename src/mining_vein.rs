//! ROADMAP 348 礦脈深掘——「越挖越深、見好就收」的抉擇小遊戲。
//!
//! 採礦這條維度長久以來只有「一鍵採石」（敲一格地形即得礦），沒有任何玩家技巧介入。
//! 本切片給它第一個真玩法，且刻意**換一套全新骨架**——既不是釣魚的「反應計時」、
//! 也不是觀星的「空間連線」，而是**press-your-luck（步步深掘、見好就收）**：
//!
//!   1. 站在岩地（`Biome::Rocky`）旁敲第一下（`ClientMsg::Mine`）→ 開一條礦脈、挖出第一層礦。
//!   2. 每多敲一下就往更深一層，**該層礦量更多**，但礦脈在某個**隱藏深度**會崩塌。
//!      越深震動越明顯（前端有「細微落石／劇烈搖晃」警示），但你**永遠不知道崩在哪一層**。
//!   3. 隨時可「收礦撤出」（`ClientMsg::MineHaul`）把目前累積的礦袋落袋為安、外加探索熟練度；
//!      但若貪心再敲、剛好踩到崩塌深度——**整袋礦全埋在坑裡**，空手而回。
//!
//! 設計取捨：
//!   - **崩塌深度伺服器權威**：`collapse_at` 由種子在 `[MIN_DEPTH, MAX_DEPTH]` 內決定、
//!     只存在伺服器，前端永遠看不到 → 無從預判、無從作弊。
//!   - **純記憶體、純函式、零持久化、零 migration**：礦脈只是 `Player` 上的記憶體前置欄，
//!     重啟清空（鏡像 `fishing` / `pet` 等切片）。
//!   - **無計時、無 game-loop 推進**：press-your-luck 是「抉擇」不是「時機」，故不需每幀推進，
//!     全程在 ws.rs 同一把寫鎖內請求／回應即可（只有冷卻在 game.rs 遞減一行）。
//!   - **平衡（誠實交代，非零風險）**：礦石（`Stone`）確實進合成經濟（不像魚只進料理），
//!     故本玩法**刻意走保守數值＋自帶冷卻＋崩塌懲罰**：每層礦量小、深層才略增，貪心常常整袋落空，
//!     使「期望產出與既有一鍵採石相當、只是多了技巧與變異」，而非開一條刷礦水龍頭。

use crate::inventory::ItemKind;

/// 礦脈崩塌的最淺深度（敲到這層仍安全的最小可能崩塌點）。
pub const MIN_COLLAPSE_DEPTH: u32 = 3;
/// 礦脈崩塌的最深深度（最幸運的礦脈也最多到這層的前一層）。
pub const MAX_COLLAPSE_DEPTH: u32 = 7;

/// 收礦撤出時，每深一層給的探索熟練度 XP（總給 = 撤出當下深度 × 此值）。
pub const HAUL_EXPLORER_XP_PER_DEPTH: u32 = 4;

/// 一輪礦脈結束（收礦或崩塌）後的冷卻秒數，避免連敲刷礦。
pub const MINE_COOLDOWN_SECS: f32 = 8.0;

/// 判定「站在岩地旁」的取樣半徑（px），鏡像釣魚 `is_near_water` 的 80px。
const ROCK_NEAR_RADIUS: f32 = 80.0;

/// 玩家是否站在岩地（`Biome::Rocky`）邊緣——四向各取樣一點，任一落在岩地即可採礦。
///
/// 鏡像 `fishing::is_near_water`：讓「站在地形邊緣就能互動」一致，不必正好踩在格心。
pub fn is_near_rock(px: f32, py: f32) -> bool {
    use world_core::{biome_at, Biome};
    let r = ROCK_NEAR_RADIUS;
    // 中心 + 上下左右五點取樣（任一是岩地即算靠近）。
    let samples = [
        (0.0, 0.0),
        (r, 0.0),
        (-r, 0.0),
        (0.0, r),
        (0.0, -r),
    ];
    samples
        .iter()
        .any(|(dx, dy)| biome_at((px + dx) as f64, (py + dy) as f64) == Biome::Rocky)
}

/// 依確定性種子算這條礦脈的隱藏崩塌深度，落在 `[MIN_COLLAPSE_DEPTH, MAX_COLLAPSE_DEPTH]`。
///
/// 種子建議帶 `player_id_low64 ^ mine_attempt_count`，讓每條礦脈深淺都不同。
pub fn collapse_depth(seed: u64) -> u32 {
    let span = MAX_COLLAPSE_DEPTH - MIN_COLLAPSE_DEPTH + 1; // 含端點
    MIN_COLLAPSE_DEPTH + (seed % span as u64) as u32
}

/// 敲到第 `depth` 層（1 起算）這一下挖出的礦量——越深一層礦量略增（誘你冒險）。
///
/// 1~2 層各 1、3~4 層各 2、5 層以上各 3。刻意壓小：最深安全礦脈
/// （`collapse_at` = 7、撤出於第 6 層）總袋量 = 1+1+2+2+3+3 = 12，與數次一鍵採石相當。
pub fn ore_at_depth(depth: u32) -> u32 {
    match depth {
        0 => 0,
        1..=2 => 1,
        3..=4 => 2,
        _ => 3,
    }
}

/// 礦脈在某一層給玩家的「震動警示」等級——只透露「越來越深、危險上升」的氛圍，
/// **不洩漏確切崩塌層**（崩塌層隱藏），保住 press-your-luck 的張力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tremor {
    /// 尚淺，無明顯震動。
    Calm,
    /// 細微落石——開始有點深了。
    Faint,
    /// 劇烈搖晃——再深恐怕要崩。
    Severe,
}

impl Tremor {
    /// 依「絕對深度」給警示：越深越危險（與崩塌機率正相關，但不等於崩塌層）。
    pub fn at_depth(depth: u32) -> Tremor {
        if depth >= 5 {
            Tremor::Severe
        } else if depth >= 3 {
            Tremor::Faint
        } else {
            Tremor::Calm
        }
    }

    /// 前端用的 snake_case 線格式。
    pub fn as_str(self) -> &'static str {
        match self {
            Tremor::Calm => "calm",
            Tremor::Faint => "faint",
            Tremor::Severe => "severe",
        }
    }
}

/// 敲一下（`strike`）的結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrikeOutcome {
    /// 挖到礦：這一下挖出 `ore` 礦、目前累積袋量 `haul`、此刻深度 `depth`、震動 `tremor`。
    Struck {
        ore: u32,
        haul: u32,
        depth: u32,
        tremor: Tremor,
    },
    /// 踩到隱藏崩塌深度——礦脈塌了、整袋礦全埋（呼叫端應清掉這條礦脈、不給任何礦）。
    Collapsed,
}

/// 一條進行中的礦脈（記憶體前置、`Copy`、重啟清空）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MiningVein {
    /// 目前已挖到的深度（層；0 = 剛開、尚未敲過）。
    depth: u32,
    /// 目前累積在礦袋裡、尚未落袋的礦量。
    haul: u32,
    /// 隱藏崩塌深度：敲到這一層即崩塌（伺服器權威，前端看不到）。
    collapse_at: u32,
}

impl MiningVein {
    /// 開一條新礦脈（尚未敲），崩塌深度由種子決定。
    pub fn open(seed: u64) -> Self {
        MiningVein {
            depth: 0,
            haul: 0,
            collapse_at: collapse_depth(seed),
        }
    }

    /// 目前深度（層）。
    pub fn depth(self) -> u32 {
        self.depth
    }

    /// 目前累積袋量。
    pub fn haul(self) -> u32 {
        self.haul
    }

    /// 目前深度的震動等級（供 `PlayerView` 廣播，畫面顯示警示）。
    pub fn tremor(self) -> Tremor {
        Tremor::at_depth(self.depth)
    }

    /// 再往下敲一層：
    /// - 若新深度 `== collapse_at` → 崩塌（不改累積袋量、回 `Collapsed`，呼叫端清礦脈、不給礦）。
    /// - 否則 → 挖出該層礦、累進袋量、回 `Struck { .. }`。
    pub fn strike(&mut self) -> StrikeOutcome {
        let next = self.depth + 1;
        if next >= self.collapse_at {
            // 崩塌：深度推進但不結算礦（呼叫端會把整條礦脈清掉）。
            self.depth = next;
            StrikeOutcome::Collapsed
        } else {
            self.depth = next;
            let ore = ore_at_depth(self.depth);
            self.haul += ore;
            StrikeOutcome::Struck {
                ore,
                haul: self.haul,
                depth: self.depth,
                tremor: self.tremor(),
            }
        }
    }

    /// 收礦撤出：落袋目前累積袋量（純函式、不改狀態；呼叫端依此給礦＋清礦脈）。
    ///
    /// 回傳 `(礦量, 探索XP)`；探索 XP = 撤出當下深度 × `HAUL_EXPLORER_XP_PER_DEPTH`
    /// （挖得越深、撤得越漂亮，熟練度回報越高）。
    pub fn haul_out(self) -> (u32, u32) {
        (self.haul, self.depth * HAUL_EXPLORER_XP_PER_DEPTH)
    }

    /// 收礦撤出落袋的礦物種類——固定礦石（`Stone`），與一鍵採石同一資源槽。
    pub fn ore_kind() -> ItemKind {
        ItemKind::Stone
    }
}

// ─── 單元測試 ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_depth_within_bounds() {
        for seed in 0u64..3000 {
            let d = collapse_depth(seed);
            assert!(
                (MIN_COLLAPSE_DEPTH..=MAX_COLLAPSE_DEPTH).contains(&d),
                "collapse_depth({seed}) = {d} 超出 [{MIN_COLLAPSE_DEPTH}, {MAX_COLLAPSE_DEPTH}]"
            );
        }
    }

    #[test]
    fn collapse_depth_deterministic_and_varies() {
        assert_eq!(collapse_depth(42), collapse_depth(42));
        // 種子掃過一輪應蓋到全部可能深度（端點都出得來）。
        let mut seen = std::collections::BTreeSet::new();
        for seed in 0u64..50 {
            seen.insert(collapse_depth(seed));
        }
        assert!(seen.contains(&MIN_COLLAPSE_DEPTH));
        assert!(seen.contains(&MAX_COLLAPSE_DEPTH));
    }

    #[test]
    fn ore_at_depth_increases_with_depth() {
        assert_eq!(ore_at_depth(0), 0);
        assert_eq!(ore_at_depth(1), 1);
        assert_eq!(ore_at_depth(2), 1);
        assert_eq!(ore_at_depth(3), 2);
        assert_eq!(ore_at_depth(4), 2);
        assert_eq!(ore_at_depth(5), 3);
        assert_eq!(ore_at_depth(99), 3);
        // 單調不減（越深不會更少）。
        for d in 1..20u32 {
            assert!(ore_at_depth(d + 1) >= ore_at_depth(d), "ore_at_depth 應單調不減");
        }
    }

    #[test]
    fn tremor_escalates_with_depth() {
        assert_eq!(Tremor::at_depth(0), Tremor::Calm);
        assert_eq!(Tremor::at_depth(2), Tremor::Calm);
        assert_eq!(Tremor::at_depth(3), Tremor::Faint);
        assert_eq!(Tremor::at_depth(4), Tremor::Faint);
        assert_eq!(Tremor::at_depth(5), Tremor::Severe);
        assert_eq!(Tremor::at_depth(9), Tremor::Severe);
    }

    #[test]
    fn open_vein_starts_empty() {
        let v = MiningVein::open(123);
        assert_eq!(v.depth(), 0);
        assert_eq!(v.haul(), 0);
        assert_eq!(v.tremor(), Tremor::Calm);
    }

    #[test]
    fn strike_accumulates_haul_until_collapse() {
        // 找一個 collapse_at = MAX(7) 的種子，驗證一路敲到第 6 層的累積。
        let seed = (0u64..).find(|&s| collapse_depth(s) == MAX_COLLAPSE_DEPTH).unwrap();
        let mut v = MiningVein::open(seed);
        let mut expected_haul = 0;
        for depth in 1..=(MAX_COLLAPSE_DEPTH - 1) {
            match v.strike() {
                StrikeOutcome::Struck { ore, haul, depth: d, .. } => {
                    expected_haul += ore_at_depth(depth);
                    assert_eq!(d, depth);
                    assert_eq!(ore, ore_at_depth(depth));
                    assert_eq!(haul, expected_haul);
                }
                StrikeOutcome::Collapsed => panic!("第 {depth} 層不該崩（collapse_at=7）"),
            }
        }
        assert_eq!(v.haul(), expected_haul);
        // 再敲一下（第 7 層）= 崩塌。
        assert_eq!(v.strike(), StrikeOutcome::Collapsed);
    }

    #[test]
    fn strike_collapses_exactly_at_hidden_depth() {
        // collapse_at = MIN(3)：敲第 1、2 層安全，第 3 層崩。
        let seed = (0u64..).find(|&s| collapse_depth(s) == MIN_COLLAPSE_DEPTH).unwrap();
        let mut v = MiningVein::open(seed);
        assert!(matches!(v.strike(), StrikeOutcome::Struck { depth: 1, .. }));
        assert!(matches!(v.strike(), StrikeOutcome::Struck { depth: 2, .. }));
        assert_eq!(v.strike(), StrikeOutcome::Collapsed);
    }

    #[test]
    fn collapse_does_not_award_haul() {
        let seed = (0u64..).find(|&s| collapse_depth(s) == MIN_COLLAPSE_DEPTH).unwrap();
        let mut v = MiningVein::open(seed);
        v.strike(); // 第 1 層
        v.strike(); // 第 2 層
        let haul_before_collapse = v.haul();
        assert!(haul_before_collapse > 0);
        assert_eq!(v.strike(), StrikeOutcome::Collapsed);
        // 崩塌後袋量不變（呼叫端不發礦、直接清礦脈，袋量無意義但確認 strike 沒加礦）。
        assert_eq!(v.haul(), haul_before_collapse, "崩塌那一下不應再加礦");
    }

    #[test]
    fn haul_out_returns_ore_and_scaled_xp() {
        let seed = (0u64..).find(|&s| collapse_depth(s) == MAX_COLLAPSE_DEPTH).unwrap();
        let mut v = MiningVein::open(seed);
        v.strike(); // d1
        v.strike(); // d2
        v.strike(); // d3
        let (ore, xp) = v.haul_out();
        assert_eq!(ore, v.haul());
        assert_eq!(xp, 3 * HAUL_EXPLORER_XP_PER_DEPTH, "撤出 XP = 深度 × 每層 XP");
    }

    #[test]
    fn haul_out_empty_vein_is_zero() {
        let v = MiningVein::open(7);
        assert_eq!(v.haul_out(), (0, 0), "沒敲過就撤＝空手、零 XP");
    }

    #[test]
    fn ore_kind_is_stone() {
        assert_eq!(MiningVein::ore_kind(), ItemKind::Stone);
    }

    #[test]
    fn tremor_wire_strings_stable() {
        assert_eq!(Tremor::Calm.as_str(), "calm");
        assert_eq!(Tremor::Faint.as_str(), "faint");
        assert_eq!(Tremor::Severe.as_str(), "severe");
    }

    #[test]
    fn deeper_haul_strictly_beats_shallow_when_safe() {
        // 同一條深礦脈，撤得越深、袋量越大（誘因正確）。
        let seed = (0u64..).find(|&s| collapse_depth(s) == MAX_COLLAPSE_DEPTH).unwrap();
        let mut shallow = MiningVein::open(seed);
        shallow.strike();
        shallow.strike();
        let (shallow_ore, _) = shallow.haul_out();

        let mut deep = MiningVein::open(seed);
        for _ in 0..(MAX_COLLAPSE_DEPTH - 1) {
            deep.strike();
        }
        let (deep_ore, _) = deep.haul_out();
        assert!(deep_ore > shallow_ore, "挖得越深、安全撤出的礦越多");
    }
}
