//! 暖食飽足 buff（ROADMAP 395）——進食料理後的限時療癒增益。
//!
//! 在此之前，七道料理（烤魚／星燦刺身／深海濃湯／煎蛋／麵包／蔬菜湯／焗烤馬鈴薯）
//! 吃下去只是「立刻回一次血」——按一下就結束、跟喝藥水沒兩樣，料理本身沒有任何
//! 「值得花心思去煮」的後續。本模組讓進食第一次長出**一段持續的飽足狀態**：吃完
//! 料理會獲得限時「暖食」buff，期間 HP 緩慢回復；不同料理營養不同——愈豐盛的料理
//! 飽足愈久、回得愈多。於是「煮什麼、什麼時候吃」第一次有了取捨，料理從一次性的
//! 回血鈕長成有玩法感的療癒選擇。
//!
//! ## 設計鐵律
//! - **記憶體模式、會過期**：buff 純記憶體前置（重連／重啟清空），零持久化、零 migration。
//! - **療癒向、零平衡風險**：只緩慢回 HP，不送物品／乙太／戰力，不碰戰鬥平衡；
//!   倒地時自然無效（`Vitals::heal` 對 hp==0 回 0）。
//! - **純函式可測**：buff 規格查表（`meal_buff_for`）與每幀推進（`tick`）皆與 IO／鎖無關、
//!   結果確定可重現。
//! - **不疊加**：再吃一道料理直接覆蓋（刷新）為新 buff，不累積、不失控。

use crate::inventory::ItemKind;

/// 暖食飽足 buff：進食料理後的限時 HP 緩慢回復狀態。
///
/// 用「剩餘秒數」遞減模式（而非絕對時間點），讓推進只依賴 `dt`、好測且與時鐘無關。
/// 回血以小數累積（`accum`）湊整再回，避免每幀取整把零頭丟掉。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MealBuff {
    /// 觸發這份飽足的料理（給前端顯示對應圖示／文字）。
    pub kind: ItemKind,
    /// buff 總時長（秒）——固定不變，供前端算進度條。
    pub total_secs: f32,
    /// 剩餘時長（秒）——每幀遞減，歸零即過期。
    pub remaining_secs: f32,
    /// 每秒回復 HP。
    pub hp_per_sec: f32,
    /// 未滿一點的回血累積（跨幀保留小數，湊滿 1 才回一點）。
    accum: f32,
}

impl MealBuff {
    /// 建一份新飽足 buff（剩餘＝總時長）。
    fn new(kind: ItemKind, total_secs: f32, hp_per_sec: f32) -> Self {
        MealBuff {
            kind,
            total_secs,
            remaining_secs: total_secs,
            hp_per_sec,
            accum: 0.0,
        }
    }

    /// 推進 `dt` 秒：遞減剩餘時長、累積回血，回傳「本幀該回多少 HP」（湊整後的整數）。
    ///
    /// 非正／非有限 `dt` 為 no-op、回 0（比照 `Vitals::tick` 擋壞 dt）。
    pub fn tick(&mut self, dt: f32) -> u32 {
        if dt <= 0.0 || !dt.is_finite() {
            return 0;
        }
        self.remaining_secs = (self.remaining_secs - dt).max(0.0);
        self.accum += self.hp_per_sec * dt;
        let whole = self.accum.floor();
        self.accum -= whole;
        whole as u32
    }

    /// 飽足是否仍在持續（剩餘時長 > 0）。歸零即可清除。
    pub fn is_active(&self) -> bool {
        self.remaining_secs > 0.0
    }

    /// 依拿手熟練倍率放大這份飽足（總時長＋每秒回血）——ROADMAP 407 拿手菜。
    /// **剛吃下時**呼叫（剩餘＝放大後的總時長）。倍率非有限或 < 1 一律保守當作 1.0
    /// （絕不縮短／削弱玩家本來就有的飽足）。詳見 `dish_mastery::scale_meal`。
    pub fn nourished(mut self, dur_mult: f32, regen_mult: f32) -> Self {
        let dm = if dur_mult.is_finite() && dur_mult >= 1.0 { dur_mult } else { 1.0 };
        let rm = if regen_mult.is_finite() && regen_mult >= 1.0 { regen_mult } else { 1.0 };
        self.total_secs *= dm;
        self.remaining_secs = self.total_secs;
        self.hp_per_sec *= rm;
        self
    }

    /// 飽足進度 0.0~1.0（剩餘比例，1.0＝剛吃飽、0.0＝即將散去），給前端畫光暈／倒數。
    pub fn progress(&self) -> f32 {
        if self.total_secs <= 0.0 {
            return 0.0;
        }
        (self.remaining_secs / self.total_secs).clamp(0.0, 1.0)
    }

    /// 圍爐分食（ROADMAP 462）：把這份暖食的一份暖意「分」給身旁旅人——
    /// 時長按 `frac` 縮短（半份＝順手暖一下、不是把整鍋端給人），每秒回血照舊（溫柔不打折），
    /// `kind` 沿用同一道料理（前端共用同一圈暖食光暈）。**剛分到時**呼叫（剩餘＝縮短後的總時長、回血累積歸零）。
    /// `frac` 非有限或越界一律保守夾進 `(0, 1]`；分出的時長保底至少 `MIN_SHARE_SECS` 秒（不致一閃即逝）、
    /// 且絕不超過來源（分享永遠 ≤ 自己吃下的那份）。純函式、不改自身。
    pub fn shared(&self, frac: f32) -> MealBuff {
        let f = if frac.is_finite() && frac > 0.0 { frac.min(1.0) } else { 0.5 };
        // 縮短後仍保底一小段、但封頂在來源時長（frac≤1 時本就成立，保險再夾一次）。
        let secs = (self.total_secs * f).clamp(MIN_SHARE_SECS.min(self.total_secs), self.total_secs);
        MealBuff {
            kind: self.kind,
            total_secs: secs,
            remaining_secs: secs,
            hp_per_sec: self.hp_per_sec,
            accum: 0.0,
        }
    }
}

/// 分食時長保底（秒）：分到的暖意再短也留這麼一小段，免得「聞個味就沒了」。
pub const MIN_SHARE_SECS: f32 = 6.0;

#[cfg(test)]
impl MealBuff {
    /// 測試用公開建構子：讓其他模組（如 `meal_share`）的單元測試也能造一份已知參數的 buff。
    pub fn new_for_test(kind: ItemKind, total_secs: f32, hp_per_sec: f32) -> Self {
        MealBuff::new(kind, total_secs, hp_per_sec)
    }
}

/// 查表：某件道具吃下去會不會帶來暖食飽足 buff，會的話帶什麼參數。
///
/// 只有「料理」（採集→烹飪而成的食物）才有飽足；藥水／精粹／材料一律 `None`。
/// 數值刻意溫和、療癒向——愈豐盛的料理飽足愈久、回得愈多，但總量與既有立即回血
/// 同量級，不破壞戰鬥／經濟平衡。
pub fn meal_buff_for(item: ItemKind) -> Option<MealBuff> {
    // (總時長秒, 每秒回血)。
    let (secs, per_sec) = match item {
        ItemKind::GrilledFish => (20.0, 0.4),  // 烤魚——基礎療癒食物
        ItemKind::FriedEgg => (24.0, 0.4),     // 煎蛋
        ItemKind::Honey => (18.0, 0.3),        // 蜂蜜——甜食，輕盈的暖食飽足
        ItemKind::CarrotSoup => (28.0, 0.4),   // 蔬菜湯
        ItemKind::Bread => (30.0, 0.5),        // 麵包
        ItemKind::PotatoGratin => (34.0, 0.6), // 焗烤馬鈴薯——農地最豐盛
        ItemKind::StarSashimi => (40.0, 0.7),  // 星燦刺身——稀有漁獲
        ItemKind::DeepBroth => (50.0, 0.9),    // 深海濃湯——最豐盛、飽足最久
        _ => return None,
    };
    Some(MealBuff::new(item, secs, per_sec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_dishes_all_grant_buff() {
        // 七道料理都帶飽足 buff，且參數正向。
        for item in [
            ItemKind::GrilledFish,
            ItemKind::FriedEgg,
            ItemKind::CarrotSoup,
            ItemKind::Bread,
            ItemKind::PotatoGratin,
            ItemKind::StarSashimi,
            ItemKind::DeepBroth,
        ] {
            let b = meal_buff_for(item).expect("料理應有飽足 buff");
            assert_eq!(b.kind, item);
            assert!(b.total_secs > 0.0);
            assert!(b.hp_per_sec > 0.0);
            assert!(b.is_active());
            assert_eq!(b.remaining_secs, b.total_secs);
        }
    }

    #[test]
    fn non_dishes_grant_nothing() {
        // 藥水／材料／乙太一律無飽足。
        for item in [
            ItemKind::Wood,
            ItemKind::HealingPotion,
            ItemKind::CrystalPotion,
            ItemKind::Ether,
            ItemKind::FishSmall,
        ] {
            assert!(meal_buff_for(item).is_none(), "{item:?} 不該有飽足 buff");
        }
    }

    #[test]
    fn richer_dish_lasts_longer() {
        // 深海濃湯比烤魚飽足更久。
        let basic = meal_buff_for(ItemKind::GrilledFish).unwrap();
        let rich = meal_buff_for(ItemKind::DeepBroth).unwrap();
        assert!(rich.total_secs > basic.total_secs);
    }

    #[test]
    fn tick_counts_down_and_heals() {
        // 1 hp/s、推進 1 秒應回 1 點、剩餘時長遞減。
        let mut b = MealBuff::new(ItemKind::Bread, 10.0, 1.0);
        let healed = b.tick(1.0);
        assert_eq!(healed, 1);
        assert!((b.remaining_secs - 9.0).abs() < 1e-6);
    }

    #[test]
    fn fractional_heal_accumulates_no_loss() {
        // 0.4 hp/s：頭兩幀（各 1 秒）累積不到 1 點先回 0，第三幀湊過 1 才回 1，零頭不丟。
        let mut b = MealBuff::new(ItemKind::GrilledFish, 20.0, 0.4);
        assert_eq!(b.tick(1.0), 0); // 0.4
        assert_eq!(b.tick(1.0), 0); // 0.8
        assert_eq!(b.tick(1.0), 1); // 1.2 → 回 1、餘 0.2
        // 五秒共 2.0 點，前面已回 1，再兩幀（0.6,1.0）滿第二點。
        assert_eq!(b.tick(1.0), 0); // 0.6
        assert_eq!(b.tick(1.0), 1); // 1.0 → 回 1
    }

    #[test]
    fn expires_after_total_time() {
        let mut b = MealBuff::new(ItemKind::FriedEgg, 3.0, 1.0);
        b.tick(2.0);
        assert!(b.is_active());
        b.tick(2.0); // 超過總時長
        assert!(!b.is_active());
        assert_eq!(b.remaining_secs, 0.0); // 夾在 0、不變負
    }

    #[test]
    fn progress_decreases_from_one_to_zero() {
        let mut b = MealBuff::new(ItemKind::Bread, 10.0, 0.0);
        assert!((b.progress() - 1.0).abs() < 1e-6);
        b.tick(5.0);
        assert!((b.progress() - 0.5).abs() < 1e-6);
        b.tick(10.0);
        assert!((b.progress() - 0.0).abs() < 1e-6);
    }

    #[test]
    fn bad_dt_is_noop() {
        let mut b = MealBuff::new(ItemKind::Bread, 10.0, 5.0);
        assert_eq!(b.tick(0.0), 0);
        assert_eq!(b.tick(-1.0), 0);
        assert_eq!(b.tick(f32::NAN), 0);
        assert_eq!(b.tick(f32::INFINITY), 0);
        assert!((b.remaining_secs - 10.0).abs() < 1e-6); // 完全沒推進
    }

    #[test]
    fn shared_is_half_duration_same_regen() {
        // ROADMAP 462 圍爐分食：分給旅人的是「半份」——時長減半、每秒回血照舊、品種沿用。
        let src = MealBuff::new(ItemKind::DeepBroth, 50.0, 0.9);
        let s = src.shared(0.5);
        assert_eq!(s.kind, src.kind, "分食沿用同一道料理");
        assert!((s.total_secs - 25.0).abs() < 1e-6, "時長減半");
        assert!((s.remaining_secs - 25.0).abs() < 1e-6, "剛分到＝滿剩餘");
        assert!((s.hp_per_sec - 0.9).abs() < 1e-6, "每秒回血不打折");
        assert!(s.is_active());
    }

    #[test]
    fn shared_never_exceeds_source_and_has_floor() {
        // 分享永遠 ≤ 自己吃下的那份（frac 越界也夾住）；再短也保底 MIN_SHARE_SECS、不致一閃即逝。
        let src = MealBuff::new(ItemKind::GrilledFish, 20.0, 0.4);
        // frac 上界越界（2.0）夾成 1.0 → 至多等於來源，不會「分得比自己還多」。
        let full = src.shared(2.0);
        assert!(full.total_secs <= src.total_secs + 1e-6, "分食不超過來源");
        // frac 壞值退預設 0.5。
        let nan = src.shared(f32::NAN);
        assert!((nan.total_secs - 10.0).abs() < 1e-6, "壞 frac 退半份");
        // 極小 frac 仍保底至少 MIN_SHARE_SECS（來源夠長時）。
        let tiny = src.shared(0.001);
        assert!((tiny.total_secs - MIN_SHARE_SECS).abs() < 1e-6, "極小份額仍保底");
        // 來源本身比保底還短時，分食不會「無中生有」拉長到超過來源。
        let short = MealBuff::new(ItemKind::Honey, 3.0, 0.3);
        assert!(short.shared(0.5).total_secs <= 3.0 + 1e-6, "短來源不被分食拉長");
    }
}
