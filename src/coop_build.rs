//! 協力共建·邊境瞭望塔（ROADMAP 546）——邊境荒野上立著一座座「瞭望塔工地」，
//! 但一個人扛不動樑、立不起塔：**至少兩位玩家同時站到工地旁一起出力**，塔身才會
//! 一節一節升起；圍上來合力的人越多，蓋得越快。塔一旦落成，便永久鎮在那座邊境——
//! 入夜亮起暖燈、把周圍一圈野獸長久逼退，成為社群親手蓋出來、把荒野前線變安全的前哨。
//!
//! 設計取捨（刻意換骨架、不複製既有套路）：
//! - 篝火（474／眾人拾柴 545）是「**在場才有**」的臨時暖意光環——人一走、火一熄，安全圈就消失。
//!   瞭望塔反過來：合力出的工**會累積成永久**——進度只進不退（沒人時停工、但已蓋的不倒），
//!   蓋滿即落成、永久鎮守。從「聚在一起當下更安全」推進到「**一起勞動把世界永久變安全**」，
//!   是「多人協作改變世界」這條真維度的不同動詞（建造／升塔）與不同物件（工地／塔，非火）。
//! - 真正的 co-op 勞動：明定 `MIN_BUILDERS=2`——**少於兩人推不動**，逼出「揪人合力」而非單人慢磨，
//!   呼應 reviewer 點名要的「多人合力搬運／共建一座設施」而非「聚在一起的呈現層」。
//! - 原創實作，不抄任何外部遊戲碼。
//!
//! 成本／安全紀律：
//! - 純記憶體模式，重啟回到未完工的工地（與篝火／載具同款，零持久化、零 migration、零 LLM、
//!   不碰玩家存檔與經濟）。「永久」＝同一伺服器生命週期內不衰退、不需在場維持，落成後長駐。
//! - 純邏輯可獨立測試（協力工門檻、進度累積只進不退、落成判定、壓制區查詢），不依賴 WebSocket／遊戲迴圈。
//! - 平衡分寸：壓制半徑 modest、工地數量固定有限（只在邊境撒兩座），落成是「把前線變安全」的
//!   局部前哨、非全圖無敵；蓋塔要真的揪到兩人以上同場出力，人海亦有 `EFFECTIVE_BUILDER_CAP` 封頂。

/// 工地完工所需的總進度點（0→100）。
pub const MAX_PROGRESS: f32 = 100.0;
/// 推得動工地的最低協力工人數——**少於兩人立不起塔**（co-op 門檻，逼出「揪人合力」）。
pub const MIN_BUILDERS: u8 = 2;
/// 站工地這個半徑（像素）內、活著的玩家才算「正在協力建造」這座塔。
pub const BUILD_RADIUS: f32 = 150.0;
/// 每位協力工每秒貢獻的進度點。2 人＝1.2/s（約 83 秒落成），人越多越快。
const BUILD_RATE: f32 = 0.6;
/// 進度速率對協力工人數的封頂——超過這麼多人也不再加快，避免人海戰術秒蓋（軟封頂）。
const EFFECTIVE_BUILDER_CAP: u8 = 6;
/// 落成瞭望塔的野獸壓制半徑（像素）——落在此半徑的敵人本幀被逼退、暫放棄追擊。
/// 沿用篝火既有的 `apply_campfire_calm` 安撫路徑（同一份敵人安撫實作，不另立機制）。
pub const SUPPRESS_RADIUS: f32 = 180.0;

/// 純函式：依本拍協力工人數，算這一拍該替工地累加多少進度（ROADMAP 546）。
/// 少於 `MIN_BUILDERS` 或 `dt<=0`＝停工回 0（一個人推不動）；否則進度 = 有效工數 × 速率 × dt，
/// 有效工數封頂在 `EFFECTIVE_BUILDER_CAP`（人海亦不再加快）。確定性、無副作用、好測。
pub fn build_increment(builders: u8, dt: f32) -> f32 {
    if dt <= 0.0 || builders < MIN_BUILDERS {
        return 0.0;
    }
    let effective = builders.min(EFFECTIVE_BUILDER_CAP) as f32;
    effective * BUILD_RATE * dt
}

/// 一座邊境瞭望塔（純記憶體）。未落成時是工地、進度累積；蓋滿即 `done`、永久鎮守。
#[derive(Debug, Clone)]
pub struct Watchtower {
    pub id: u32,
    pub wx: f32,
    pub wy: f32,
    /// 建造進度（0..=`MAX_PROGRESS`）。**只進不退**——沒人時停工，但已蓋的不會倒。
    pub progress: f32,
    /// 本拍協力工人數（由 `sync_builders` 每拍依玩家座標重算；前端據此顯示「N 人合力中」）。
    pub builders: u8,
    /// 是否已落成。落成後不再吃進度、永久回傳壓制區。
    pub done: bool,
}

impl Watchtower {
    /// 進度百分比（0..=100，整數）——供快照精簡廣播給前端畫進度條。
    pub fn percent(&self) -> u8 {
        (self.progress / MAX_PROGRESS * 100.0).round().clamp(0.0, 100.0) as u8
    }
}

/// 全服協力瞭望塔狀態（純記憶體，重啟回到未完工的工地）。
pub struct CoopBuildField {
    towers: Vec<Watchtower>,
}

impl CoopBuildField {
    /// 在邊境撒下開局的瞭望塔工地。座標選在保護城外、可行走非水的兩處邊境前線
    /// （北境草原 + 東境森林，皆已驗證 `tile_kind_at==Empty` 且非 Water，見單元測試），
    /// 是野獸出沒、最需要前哨鎮守的地帶。
    pub fn new() -> Self {
        let spots = [
            (2344.0, 800.0),  // 北境瞭望塔工地（草原前線，獸潮常從北城門外湧來）
            (3900.0, 2296.0), // 東境瞭望塔工地（森林前線）
        ];
        let towers = spots
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| Watchtower {
                id: i as u32,
                wx: x,
                wy: y,
                progress: 0.0,
                builders: 0,
                done: false,
            })
            .collect();
        Self { towers }
    }

    /// 目前所有瞭望塔（含工地與已落成者），供快照廣播給前端顯示。
    pub fn all(&self) -> &[Watchtower] {
        &self.towers
    }

    /// 已落成瞭望塔的野獸壓制區（中心 X／Y／半徑）——供敵人安撫判定使用。
    /// 只有 `done` 的塔回壓制區；工地不壓制。沿用篝火 `apply_campfire_calm` 同一路徑。
    pub fn suppress_zones(&self) -> Vec<(f32, f32, f32)> {
        self.towers
            .iter()
            .filter(|t| t.done)
            .map(|t| (t.wx, t.wy, SUPPRESS_RADIUS))
            .collect()
    }

    /// 依本拍玩家座標，重算每座未落成工地的協力工人數（站 `BUILD_RADIUS` 內、座標有限的玩家）。
    /// 落成的塔不再數工。純邏輯、確定性；呼叫端先讀玩家權威座標（讀鎖即放），出鎖後再以本欄寫鎖
    /// 呼叫此函式（守鎖序不巢狀）。傳進的座標應已過濾為「活著（非倒地）的玩家」。
    pub fn sync_builders(&mut self, player_positions: &[(f32, f32)]) {
        let r2 = BUILD_RADIUS * BUILD_RADIUS;
        for t in self.towers.iter_mut() {
            if t.done {
                t.builders = 0;
                continue;
            }
            let mut count: u8 = 0;
            for &(px, py) in player_positions {
                if !px.is_finite() || !py.is_finite() {
                    continue;
                }
                let dx = px - t.wx;
                let dy = py - t.wy;
                if dx * dx + dy * dy <= r2 {
                    count = count.saturating_add(1);
                }
            }
            t.builders = count;
        }
    }

    /// 推進建造（`dt` 秒）：每座未落成工地依協力工人數累加進度（只進不退），
    /// 蓋滿 `MAX_PROGRESS` 即標記 `done`、進度封在滿值。已落成者略過。
    pub fn tick(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        for t in self.towers.iter_mut() {
            if t.done {
                continue;
            }
            t.progress += build_increment(t.builders, dt);
            if t.progress >= MAX_PROGRESS {
                t.progress = MAX_PROGRESS;
                t.done = true;
            }
        }
    }
}

impl Default for CoopBuildField {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_sites_are_walkable_frontier() {
        // 撒下的工地必須：座標有限、落在保護城外（前線才需鎮守、壓制才有意義）、
        // 地形可行走（tile_kind_at==Empty 且非 Water，玩家走得到、塔立得住）。
        let f = CoopBuildField::new();
        assert!(!f.all().is_empty(), "開局應撒下至少一座工地");
        for t in f.all() {
            assert!(t.wx.is_finite() && t.wy.is_finite(), "工地座標必為有限值");
            assert!(
                !crate::positions::is_in_safe_zone(t.wx, t.wy),
                "工地該在保護城外的邊境前線：({},{})",
                t.wx,
                t.wy
            );
            assert_eq!(
                world_core::tile_kind_at(t.wx as f64, t.wy as f64),
                world_core::TileKind::Empty,
                "工地該落在可行走空地（非實心格）：({},{})",
                t.wx,
                t.wy
            );
            assert_ne!(
                world_core::biome_at(t.wx as f64, t.wy as f64),
                world_core::Biome::Water,
                "工地不該落在水域：({},{})",
                t.wx,
                t.wy
            );
            // 開局都是未落成的工地。
            assert!(!t.done && t.progress == 0.0);
        }
    }

    #[test]
    fn solo_builder_cannot_make_progress() {
        // 少於兩人推不動——一個人站再久進度都是 0（co-op 門檻）。
        assert_eq!(build_increment(0, 1.0), 0.0);
        assert_eq!(build_increment(1, 1.0), 0.0);
        assert!(build_increment(2, 1.0) > 0.0, "兩人就推得動");
    }

    #[test]
    fn more_builders_build_faster_up_to_cap() {
        let two = build_increment(2, 1.0);
        let four = build_increment(4, 1.0);
        assert!(four > two, "人越多蓋越快");
        // 軟封頂：超過 EFFECTIVE_BUILDER_CAP 不再加快。
        let capped = build_increment(EFFECTIVE_BUILDER_CAP, 1.0);
        let over = build_increment(EFFECTIVE_BUILDER_CAP.saturating_add(3), 1.0);
        assert_eq!(capped, over, "人海封頂後速率不再上升");
    }

    #[test]
    fn non_positive_dt_is_noop() {
        assert_eq!(build_increment(5, 0.0), 0.0);
        assert_eq!(build_increment(5, -1.0), 0.0);
    }

    #[test]
    fn progress_accumulates_and_does_not_decay() {
        let mut f = CoopBuildField::new();
        let id0 = f.all()[0].id;
        // 兩人站工地 0 旁出力。
        let (sx, sy) = (f.all()[0].wx, f.all()[0].wy);
        let crew = [(sx, sy), (sx + 10.0, sy)];
        f.sync_builders(&crew);
        f.tick(1.0);
        let p1 = f.all().iter().find(|t| t.id == id0).unwrap().progress;
        assert!(p1 > 0.0, "兩人出力進度該前進");
        // 所有人離開（沒人在場）——進度該停在原地、不衰退（與篝火臨時光環的關鍵差異）。
        f.sync_builders(&[]);
        f.tick(5.0);
        let p2 = f.all().iter().find(|t| t.id == id0).unwrap().progress;
        assert_eq!(p2, p1, "沒人時停工但已蓋的不倒（只進不退）");
    }

    #[test]
    fn completes_and_emits_suppress_zone() {
        let mut f = CoopBuildField::new();
        let (sx, sy) = (f.all()[0].wx, f.all()[0].wy);
        let crew = [(sx, sy), (sx + 10.0, sy), (sx - 10.0, sy)];
        // 未落成前不壓制。
        assert!(f.suppress_zones().is_empty());
        // 一群人猛蓋到落成。
        for _ in 0..200 {
            f.sync_builders(&crew);
            f.tick(1.0);
            if f.all().iter().any(|t| t.done) {
                break;
            }
        }
        let t0 = &f.all()[0];
        assert!(t0.done && t0.progress == MAX_PROGRESS, "該蓋到落成、進度封頂");
        assert_eq!(t0.percent(), 100);
        // 落成後回傳一塊壓制區（半徑 SUPPRESS_RADIUS）。
        let zones = f.suppress_zones();
        assert!(zones.iter().any(|&(zx, zy, r)| zx == sx && zy == sy && r == SUPPRESS_RADIUS));
    }

    #[test]
    fn done_tower_ignores_builders_and_stays_done() {
        let mut f = CoopBuildField::new();
        // 強制讓第一座落成。
        f.towers[0].progress = MAX_PROGRESS;
        f.towers[0].done = true;
        let (sx, sy) = (f.towers[0].wx, f.towers[0].wy);
        f.sync_builders(&[(sx, sy), (sx + 5.0, sy)]);
        assert_eq!(f.all()[0].builders, 0, "落成的塔不再數協力工");
        f.tick(10.0);
        assert!(f.all()[0].done && f.all()[0].progress == MAX_PROGRESS);
    }

    #[test]
    fn bad_positions_are_ignored_in_builder_count() {
        let mut f = CoopBuildField::new();
        let (sx, sy) = (f.towers[0].wx, f.towers[0].wy);
        let crew = [
            (sx, sy),
            (f32::NAN, sy),
            (sx, f32::INFINITY),
            (sx + 5.0, sy),
        ];
        f.sync_builders(&crew);
        assert_eq!(f.all()[0].builders, 2, "壞座標不算進協力工數");
    }
}
