//! 親手植樹成蔭（ROADMAP 370）。
//!
//! 自 0-G 種田起源以來，玩家能「形塑」的世界一直被關在自己那塊圍籬田裡——翻土、播種、收成，
//! 全發生在私有農地的格子內；至於田外那片共享的故鄉大地，玩家從不能在上頭留下一點自己的痕跡。
//! 本切片第一次把「形塑共享世界」這個能動性交到玩家手上：你在戶外任一處按下「🌳 植樹」，
//! 就在腳邊種下一株嫩芽——它會**隨真實時間**自己長大：嫩芽 → 樹苗 → 幼樹 → 大樹，
//! 全服每個人都看得見。許多玩家一棵一棵種下去，光禿禿的廣場與荒地會一點一點蒙上綠蔭，
//! 故鄉的大地第一次「因為玩家而長出形狀」。
//!
//! 設計鐵律（刻意與既有系統乾淨分工、避開最近連發的骨架）：
//! - **全新骨架**：不是 366/367 的連通分量、不是 369 的 NPC 需求驅動、更不是飽和的
//!   「環境權威狀態調制既有玩法＋一行飄字」——而是「**玩家親手放置 ＋ 個體隨時間成長**」，
//!   湧現來自眾多玩家各自種樹的集體結果（共享世界一起變綠），玩家有貨真價實的能動性。
//! - **純表現、零規則嵌入**：樹只是會長大的景物，**不可碰撞**（玩家／寵物穿樹而過，
//!   不動 wasm 地形/物理那份單一實作）、不送任何乙太／物品／戰力，**零經濟擾動、零平衡風險**。
//! - **記憶體模式、零持久化、零 migration**：樹群純記憶體，重啟清零（沿用 48 牧場／69 需求
//!   的既有做法）；不碰玩家資料、不碰存檔、無密鑰。
//! - **純邏輯可獨立測試**：`stage_for_age` 是純函式；`WorldGrove` 的種植/成長/快照不碰任何 IO，
//!   全部可在測試裡直接驗。面向玩家字串集中本檔，作為 i18n 替換點，繁中註解。

use uuid::Uuid;

/// 嫩芽 🌱 → 樹苗 🌿 的分界（秒）。種下後最初一段時間是剛冒頭的嫩芽。
pub const SPROUT_SECS: f32 = 25.0;
/// 樹苗 🌿 → 幼樹 🌲 的分界（秒）。
pub const SAPLING_SECS: f32 = 70.0;
/// 幼樹 🌲 → 大樹 🌳 的分界（秒）。約莫兩分半後長成成樹，一場遊玩內看得到全程。
pub const YOUNG_SECS: f32 = 150.0;

/// 樹與樹之間的最小間距（px）——防止玩家把樹疊成一坨，種出來疏落有致。
pub const MIN_SPACING: f32 = 46.0;
/// 每位玩家在世界上至多同時擁有的樹數（避免單人洗版整片地圖）。
pub const PER_PLANTER_CAP: usize = 8;
/// 全世界至多同時存在的樹數（封住廣播與畫面負擔的上界）。
pub const GLOBAL_CAP: usize = 80;
/// 成樹廣播的冷卻秒數——多棵同時成熟時，只飄一行暖訊、不洗世界頻道。
pub const ANNOUNCE_COOLDOWN_SECS: f32 = 45.0;

/// 林蔭小憩（ROADMAP 467）：一株「已長成大樹（🌳 Mature）」的庇蔭半徑（px）。站在成樹冠下
/// 這個範圍內、脫離戰鬥時回血更快（見 `vitals::shade_regen`）。取值貼合樹冠、略小於樹距
/// `MIN_SPACING`(46)——要真的「走到樹下」才庇蔭，不會整片樹蔭連成一塊大澡堂。
/// 前端 `inGroveShade` 須與此值對齊（同一契約），改這裡記得同步前端。
pub const SHADE_RADIUS: f32 = 44.0;

/// 一株樹的成長階段（前端據此選圖示與大小，由小到大）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrowStage {
    /// 🌱 剛冒頭的嫩芽。
    Sprout,
    /// 🌿 抽高的樹苗。
    Sapling,
    /// 🌲 成形的幼樹。
    Young,
    /// 🌳 亭亭如蓋的大樹。
    Mature,
}

impl GrowStage {
    /// 給前端的穩定 wire 值（0=嫩芽 … 3=大樹），作為跨前後端契約，別重排。
    pub fn wire(self) -> u8 {
        match self {
            GrowStage::Sprout => 0,
            GrowStage::Sapling => 1,
            GrowStage::Young => 2,
            GrowStage::Mature => 3,
        }
    }
}

/// 由樹齡（秒）純函式推出成長階段——確定性、單調（樹齡越大階段越高），是本切片的成長核心。
pub fn stage_for_age(age_secs: f32) -> GrowStage {
    // 非有限／負數一律當作剛種下（防呆，不 panic、不 NaN）。
    let a = if age_secs.is_finite() && age_secs > 0.0 {
        age_secs
    } else {
        0.0
    };
    if a < SPROUT_SECS {
        GrowStage::Sprout
    } else if a < SAPLING_SECS {
        GrowStage::Sapling
    } else if a < YOUNG_SECS {
        GrowStage::Young
    } else {
        GrowStage::Mature
    }
}

/// 種植的結果（給呼叫端決定要不要回饋玩家；目前皆靜默，種成功了下一幀快照自然看得到新芽）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlantOutcome {
    /// 種下了一株新嫩芽。
    Planted,
    /// 太靠近既有的樹（< `MIN_SPACING`），不種，免得疊成一坨。
    TooClose,
    /// 這位玩家自己的樹已達 `PER_PLANTER_CAP`，不再種。
    PlanterFull,
    /// 全世界的樹已達 `GLOBAL_CAP`，不再種。
    WorldFull,
    /// 座標非有限等異常，拒絕（防呆）。
    Rejected,
}

/// 一株世界上的樹（記憶體前置、不持久化）。
#[derive(Debug, Clone)]
struct Tree {
    /// 種下這株樹的玩家 id（用於 per-planter 上限統計）。
    planter: Uuid,
    x: f32,
    y: f32,
    /// 樹齡（秒），每幀加 dt，`stage_for_age` 據此推階段。
    age: f32,
    /// 是否已經成樹並廣播過（避免同一株重複報「長成大樹」）。
    announced: bool,
}

/// 給前端的一株樹快照：世界座標 + 成長階段 wire 值。
#[derive(Debug, Clone, Copy)]
pub struct TreeSnapshot {
    pub x: f32,
    pub y: f32,
    pub stage: u8,
}

/// 全世界的樹群執行期狀態：純記憶體、重啟清零、零 migration。
#[derive(Default)]
pub struct WorldGrove {
    trees: Vec<Tree>,
    /// 成樹廣播冷卻倒數（>0 時不再飄新的成樹暖訊）。
    announce_cd: f32,
}

impl WorldGrove {
    pub fn new() -> Self {
        Self::default()
    }

    /// 目前世界上的樹數（供測試／上限判讀）。
    pub fn count(&self) -> usize {
        self.trees.len()
    }

    /// 玩家在 `(x, y)` 種下一株嫩芽。先過全域上限 → 個人上限 → 間距檢查，全通過才種。
    pub fn plant(&mut self, planter: Uuid, x: f32, y: f32) -> PlantOutcome {
        if !x.is_finite() || !y.is_finite() {
            return PlantOutcome::Rejected;
        }
        if self.trees.len() >= GLOBAL_CAP {
            return PlantOutcome::WorldFull;
        }
        let own = self.trees.iter().filter(|t| t.planter == planter).count();
        if own >= PER_PLANTER_CAP {
            return PlantOutcome::PlanterFull;
        }
        if self
            .trees
            .iter()
            .any(|t| (t.x - x).hypot(t.y - y) < MIN_SPACING)
        {
            return PlantOutcome::TooClose;
        }
        self.trees.push(Tree {
            planter,
            x,
            y,
            age: 0.0,
            announced: false,
        });
        PlantOutcome::Planted
    }

    /// 每幀推進所有樹的成長；若本幀有樹「剛長成大樹」且過了冷卻，回一句世界頻道暖訊（i18n 替換點）。
    /// 非正／非有限 dt 不前進（防呆、守單調）。
    pub fn tick(&mut self, dt: f32) -> Option<String> {
        if !dt.is_finite() || dt <= 0.0 {
            return None;
        }
        if self.announce_cd > 0.0 {
            self.announce_cd -= dt;
        }
        let mut newly_matured = 0u32;
        for t in &mut self.trees {
            t.age += dt;
            if !t.announced && stage_for_age(t.age) == GrowStage::Mature {
                t.announced = true;
                newly_matured += 1;
            }
        }
        if newly_matured > 0 && self.announce_cd <= 0.0 {
            self.announce_cd = ANNOUNCE_COOLDOWN_SECS;
            return Some(mature_announce_text());
        }
        None
    }

    /// 林蔭小憩（ROADMAP 467）：所有「已長成大樹（🌳 Mature）」的座標——只有成樹才成蔭，
    /// 嫩芽／樹苗／幼樹不算。供遊戲迴圈每幀取一份快照（grove 讀鎖即取即放、不與 players 寫鎖
    /// 巢狀），判定哪些玩家正站在社群種大的樹蔭下。
    pub fn mature_positions(&self) -> Vec<(f32, f32)> {
        self.trees
            .iter()
            .filter(|t| stage_for_age(t.age) == GrowStage::Mature)
            .map(|t| (t.x, t.y))
            .collect()
    }

    /// 全世界的樹快照（供前端在世界座標上繪製）。
    pub fn view(&self) -> Vec<TreeSnapshot> {
        self.trees
            .iter()
            .map(|t| TreeSnapshot {
                x: t.x,
                y: t.y,
                stage: stage_for_age(t.age).wire(),
            })
            .collect()
    }
}

/// 一株玩家種下的樹長成大樹時，飄向世界頻道的暖訊（面向玩家、i18n 替換點）。
pub fn mature_announce_text() -> String {
    "🌳 一株玩家親手種下的樹苗，靜靜長成了亭亭大樹——故鄉的大地，又添了一抹綠蔭。".to_string()
}

/// 林蔭小憩（ROADMAP 467）純函式：座標 `(px, py)` 是否落在任一成樹的樹蔭半徑（`SHADE_RADIUS`）內。
/// 座標非有限（NaN／±∞）一律保守回 `false`（防呆、不 panic）。確定性、零副作用，可獨立單元測試。
pub fn in_shade(px: f32, py: f32, mature: &[(f32, f32)]) -> bool {
    if !px.is_finite() || !py.is_finite() {
        return false;
    }
    mature
        .iter()
        .any(|&(tx, ty)| (tx - px).hypot(ty - py) <= SHADE_RADIUS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn stage_advances_monotonically_with_age() {
        assert_eq!(stage_for_age(0.0), GrowStage::Sprout);
        assert_eq!(stage_for_age(SPROUT_SECS - 0.1), GrowStage::Sprout);
        assert_eq!(stage_for_age(SPROUT_SECS), GrowStage::Sapling);
        assert_eq!(stage_for_age(SAPLING_SECS - 0.1), GrowStage::Sapling);
        assert_eq!(stage_for_age(SAPLING_SECS), GrowStage::Young);
        assert_eq!(stage_for_age(YOUNG_SECS - 0.1), GrowStage::Young);
        assert_eq!(stage_for_age(YOUNG_SECS), GrowStage::Mature);
        assert_eq!(stage_for_age(99999.0), GrowStage::Mature);
    }

    #[test]
    fn stage_handles_bad_age_as_sprout() {
        // 非有限值（NaN／±∞）與負數一律防呆當作剛種下的嫩芽，不 panic、不溢位。
        assert_eq!(stage_for_age(-5.0), GrowStage::Sprout);
        assert_eq!(stage_for_age(f32::NAN), GrowStage::Sprout);
        assert_eq!(stage_for_age(f32::INFINITY), GrowStage::Sprout);
        assert_eq!(stage_for_age(f32::NEG_INFINITY), GrowStage::Sprout);
    }

    #[test]
    fn wire_values_are_stable_and_ascending() {
        assert_eq!(GrowStage::Sprout.wire(), 0);
        assert_eq!(GrowStage::Sapling.wire(), 1);
        assert_eq!(GrowStage::Young.wire(), 2);
        assert_eq!(GrowStage::Mature.wire(), 3);
    }

    #[test]
    fn plant_adds_a_sprout() {
        let mut g = WorldGrove::new();
        assert_eq!(g.plant(pid(1), 100.0, 100.0), PlantOutcome::Planted);
        assert_eq!(g.count(), 1);
        let v = g.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].stage, GrowStage::Sprout.wire(), "剛種下應是嫩芽");
        assert_eq!(v[0].x, 100.0);
        assert_eq!(v[0].y, 100.0);
    }

    #[test]
    fn rejects_non_finite_coords() {
        let mut g = WorldGrove::new();
        assert_eq!(g.plant(pid(1), f32::NAN, 0.0), PlantOutcome::Rejected);
        assert_eq!(g.plant(pid(1), 0.0, f32::INFINITY), PlantOutcome::Rejected);
        assert_eq!(g.count(), 0);
    }

    #[test]
    fn rejects_planting_too_close() {
        let mut g = WorldGrove::new();
        assert_eq!(g.plant(pid(1), 100.0, 100.0), PlantOutcome::Planted);
        // 距離 < MIN_SPACING：太近，不種。
        assert_eq!(
            g.plant(pid(1), 100.0 + MIN_SPACING - 1.0, 100.0),
            PlantOutcome::TooClose
        );
        // 距離 >= MIN_SPACING：可種。
        assert_eq!(
            g.plant(pid(1), 100.0 + MIN_SPACING + 1.0, 100.0),
            PlantOutcome::Planted
        );
        assert_eq!(g.count(), 2);
    }

    #[test]
    fn enforces_per_planter_cap() {
        let mut g = WorldGrove::new();
        // 同一玩家種到上限（間隔拉開以免被間距擋住）。
        for i in 0..PER_PLANTER_CAP {
            let x = (i as f32) * (MIN_SPACING + 2.0);
            assert_eq!(g.plant(pid(7), x, 0.0), PlantOutcome::Planted);
        }
        // 再種就滿了。
        let x = (PER_PLANTER_CAP as f32) * (MIN_SPACING + 2.0);
        assert_eq!(g.plant(pid(7), x, 0.0), PlantOutcome::PlanterFull);
        // 但別的玩家還能種（個人上限是各算各的）。
        assert_eq!(g.plant(pid(8), x, 0.0), PlantOutcome::Planted);
    }

    #[test]
    fn enforces_global_cap() {
        let mut g = WorldGrove::new();
        // 用許多不同玩家各種一棵、座標離得遠，填到全域上限。
        let mut planted = 0usize;
        let mut n: u128 = 0;
        while planted < GLOBAL_CAP {
            let x = (planted as f32) * (MIN_SPACING + 2.0);
            let y = ((planted / 50) as f32) * (MIN_SPACING + 2.0);
            n += 1;
            if g.plant(pid(n), x, y) == PlantOutcome::Planted {
                planted += 1;
            }
        }
        assert_eq!(g.count(), GLOBAL_CAP);
        // 全世界滿了：新人也種不下。
        assert_eq!(
            g.plant(pid(99999), 99999.0, 99999.0),
            PlantOutcome::WorldFull
        );
    }

    #[test]
    fn tick_grows_trees_over_time() {
        let mut g = WorldGrove::new();
        g.plant(pid(1), 0.0, 0.0);
        assert_eq!(g.view()[0].stage, GrowStage::Sprout.wire());
        // 推進到樹苗階段。
        g.tick(SPROUT_SECS + 1.0);
        assert_eq!(g.view()[0].stage, GrowStage::Sapling.wire());
        // 一路推進到成樹。
        g.tick(YOUNG_SECS);
        assert_eq!(g.view()[0].stage, GrowStage::Mature.wire());
    }

    #[test]
    fn tick_announces_maturation_once_then_cools_down() {
        let mut g = WorldGrove::new();
        g.plant(pid(1), 0.0, 0.0);
        // 還沒成樹：不報。
        assert!(g.tick(SPROUT_SECS).is_none());
        // 跨過成樹門檻：報一次。
        let line = g.tick(YOUNG_SECS).expect("剛成樹應報一次");
        assert!(line.contains("綠蔭"), "暖訊應提到綠蔭");
        // 同一株再 tick 不重報（announced 已設）。
        assert!(g.tick(10.0).is_none(), "同株不重複報成樹");
    }

    #[test]
    fn tick_respects_announce_cooldown_for_simultaneous_maturation() {
        let mut g = WorldGrove::new();
        // 兩株同時種下、座標拉開。
        g.plant(pid(1), 0.0, 0.0);
        g.plant(pid(2), 1000.0, 0.0);
        // 兩株同一幀同時成樹：冷卻只讓其中一次飄訊。
        let first = g.tick(YOUNG_SECS + 1.0);
        assert!(first.is_some(), "同幀多株成樹，至少報一次");
        // 緊接著就算還有「待報」也被冷卻擋住——這裡兩株都已 announced，故必然 None。
        assert!(g.tick(1.0).is_none());
    }

    #[test]
    fn tick_ignores_bad_dt() {
        let mut g = WorldGrove::new();
        g.plant(pid(1), 0.0, 0.0);
        g.tick(-1.0);
        g.tick(f32::NAN);
        assert_eq!(g.view()[0].stage, GrowStage::Sprout.wire(), "壞 dt 不應推進成長");
    }

    // ─── 林蔭小憩 mature_positions / in_shade（ROADMAP 467） ───────────────────

    #[test]
    fn mature_positions_only_lists_grown_trees() {
        let mut g = WorldGrove::new();
        g.plant(pid(1), 0.0, 0.0); // 待會長成大樹
        g.plant(pid(1), 500.0, 0.0); // 剛種下、仍是嫩芽
        // 推到第一株成樹門檻（但第二株也一起長了，故兩株都會成樹）——改用分開的樹齡不易測，
        // 這裡直接驗：剛種完一律無成樹。
        assert!(g.mature_positions().is_empty(), "剛種下都還是嫩芽，無成樹成蔭");
        // 全部跨過成樹門檻後，兩株都列入。
        g.tick(YOUNG_SECS + 1.0);
        let pos = g.mature_positions();
        assert_eq!(pos.len(), 2, "兩株都長成大樹，皆應成蔭");
    }

    #[test]
    fn in_shade_true_only_within_radius() {
        let mature = vec![(100.0_f32, 100.0_f32)];
        // 正中樹下：成蔭。
        assert!(in_shade(100.0, 100.0, &mature));
        // 半徑內邊緣：成蔭。
        assert!(in_shade(100.0 + SHADE_RADIUS - 0.5, 100.0, &mature));
        // 略出半徑：不成蔭。
        assert!(!in_shade(100.0 + SHADE_RADIUS + 1.0, 100.0, &mature));
        // 遠處：不成蔭。
        assert!(!in_shade(1000.0, 1000.0, &mature));
        // 沒有任何成樹：永不成蔭。
        assert!(!in_shade(100.0, 100.0, &[]));
    }

    #[test]
    fn in_shade_rejects_bad_coords() {
        let mature = vec![(0.0_f32, 0.0_f32)];
        assert!(!in_shade(f32::NAN, 0.0, &mature));
        assert!(!in_shade(0.0, f32::INFINITY, &mature));
        assert!(!in_shade(f32::NEG_INFINITY, f32::NAN, &mature));
    }
}
