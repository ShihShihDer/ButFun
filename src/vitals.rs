//! 玩家生命值（Phase 1 戰鬥 MVP「自動打怪」的純邏輯地基）。
//!
//! 這層只管「玩家挨打怎麼扣血、被打趴後怎麼復原、脫離戰鬥怎麼自然回血」，是純資料 +
//! 純函式，無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試。延續 `combat.rs` /
//! `gather.rs` / `vehicle.rs` 的前置慣例：純邏輯先落地、標 `allow(dead_code)`，接線輪
//! （玩家帶 `Vitals`、附近敵人每 tick 用 `EnemyKind::threat` 反擊扣血、快照廣播血量、
//! 前端畫血條）才有呼叫端。
//!
//! 戰鬥要有風險才成立——`combat.rs` 早就備好 `EnemyKind::threat`（敵人每次反擊的傷害），
//! 但一直「無呼叫端、待玩家生命值那條切片才接」。這層正是那塊承接點：敵人的 `threat`
//! 將餵進這裡的 `take_damage`，讓「自動打怪」不再是無傷收割，而有被打趴、得喘口氣的張力。
//!
//! 主題是療癒的蒸汽龐克太空歌劇，**刻意沒有永久死亡**：血歸零不是 game over，而是
//! 「虛脫」——原地休息 `RECOVERY_SECS` 秒後滿血復原（比照 `Enemy` 被打倒後重生的節奏，
//! 只是換成玩家自己）。脫離戰鬥一小段時間後還會自然回血，讓人放鬆探索、不必怕一路掉血掉到底。

use serde::{Deserialize, Serialize};

/// 玩家滿血時的生命值。整數血量與 `Enemy` 的整數傷害咬合，全程不引入浮點誤差。
pub const MAX_HP: u32 = 20;

/// 被打趴（血歸零）後到滿血復原所需的休息秒數。療癒主題：不是死亡，是小憩。
pub const RECOVERY_SECS: f32 = 8.0;

/// 最後一次受擊後，要過這麼久沒再挨打才開始自然回血（剛挨打不會立刻回，保留戰鬥張力）。
pub const REGEN_DELAY_SECS: f32 = 5.0;

/// 脫離戰鬥後每秒自然回復的生命值。
pub const REGEN_PER_SEC: f32 = 1.0;

/// 林蔭小憩（ROADMAP 467）：在社群親手種大的成樹樹蔭下，脫離戰鬥時「額外」加速自然回血的每秒量。
/// 疊在 `REGEN_PER_SEC` 之上（樹蔭下總回血 ≈ 自然 + 此值），刻意溫和——讓社群種成的樹林成為
/// 受傷旅人療傷小憩的去處，但仍受 `regen_cooldown` 約束（剛挨打不生效），不破壞戰鬥張力。
pub const SHADE_REGEN_PER_SEC: f32 = 1.0;

/// 依等級計算玩家的最大血量（基礎 20，每升一級 +2）。純函式，可測試。
pub fn level_max_hp(level: u32) -> u32 {
    MAX_HP + level * 2
}

/// 玩家的生命狀態。
///
/// 狀態以「剩餘生命」為單一真實來源：存活 / 被打趴皆由 `hp` 推導（比照 `Enemy` 以
/// `remaining_hp` 推導存活 / 被打倒）。另有兩個計時輔助欄位驅動「被打趴後復原」與
/// 「脫離戰鬥後自然回血」，但它們都只是過程量，不改變「血歸零＝被打趴」這條判定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vitals {
    /// 剩餘生命。歸零＝被打趴（需休息復原）。
    hp: u32,
    /// 目前有效的最大血量（依等級縮放；`#[serde(default)]` 確保舊格式向後相容）。
    #[serde(default = "default_max_hp")]
    max_hp: u32,
    /// 被打趴後的復原倒數（秒）。只有 `hp == 0` 時才有意義；倒數到 0 滿血復原。
    recovery_timer: f32,
    /// 最後一次受擊後的回血冷卻（秒）。`> 0` 時暫停自然回血（剛挨打不會立刻回血）。
    regen_cooldown: f32,
    /// 自然回血的小數累積。`hp` 是整數，靠它把每秒不足 1 點的回血量湊滿 1 才加上去，
    /// 恆落在 `[0, 1)`（湊滿就減掉整數部分）。
    regen_accum: f32,
}

fn default_max_hp() -> u32 {
    MAX_HP
}

impl Default for Vitals {
    fn default() -> Self {
        Self::new()
    }
}

// 整個模組是前置地基：接線輪（玩家帶 `Vitals`、敵人反擊扣血、快照廣播血量）才有呼叫端，
// 在此之前公開項目皆無外部呼叫，比照 `combat.rs` / `gather.rs` 逐項標 `allow(dead_code)`。
#[allow(dead_code)]
impl Vitals {
    /// 生出一個滿血、未受傷的玩家生命狀態（最大血量預設為等級 0 的基礎值）。
    pub fn new() -> Self {
        Self {
            hp: MAX_HP,
            max_hp: MAX_HP,
            recovery_timer: 0.0,
            regen_cooldown: 0.0,
            regen_accum: 0.0,
        }
    }

    /// 重連 / 出生時設定等級對應的最大血量，並補滿至新上限。
    /// Vitals 不做持久化，每次連線都從 `new()` 開始再呼叫此函式校正等級加成。
    pub fn set_max_hp_full(&mut self, new_max: u32) {
        self.max_hp = new_max.max(MAX_HP); // 最低不低於基礎值
        self.hp = self.max_hp;             // 重連給滿血
    }

    /// 升級時呼叫：以 `full_new_max`（含等級 + 戰士加成 + 屬性加點）更新上限，
    /// 並將新增的 HP 直接補給玩家（升級獎勵感）。
    /// 呼叫端負責傳入完整的新上限，避免 on_level_up 需要知道加點細節。
    pub fn on_level_up(&mut self, full_new_max: u32) {
        let new_max = full_new_max.max(MAX_HP);
        if new_max > self.max_hp {
            let bonus = new_max - self.max_hp;
            self.max_hp = new_max;
            // 升級補 HP，不超過新上限。
            self.hp = (self.hp + bonus).min(self.max_hp);
        }
    }

    /// 屬性加點分配 HP 時呼叫：更新上限但不補滿（加點不送血，只是上限提升）。
    /// 若新上限低於當前血量則保持血量不變（不強制扣血）。
    pub fn update_max_hp(&mut self, new_max: u32) {
        self.max_hp = new_max.max(MAX_HP);
        // 不補滿，只確保當前血量不超過新上限。
        self.hp = self.hp.min(self.max_hp);
    }

    /// 剩餘生命。
    pub fn hp(&self) -> u32 {
        self.hp
    }

    /// 目前有效的最大血量（隨等級縮放）。
    pub fn max_hp(&self) -> u32 {
        self.max_hp
    }

    /// 血量比例 `[0, 1]`，供前端畫血條。
    pub fn fraction(&self) -> f32 {
        if self.max_hp == 0 { return 0.0; }
        self.hp as f32 / self.max_hp as f32
    }

    /// 是否還站得住（還能行動、會被敵人攻擊）。
    pub fn is_alive(&self) -> bool {
        self.hp > 0
    }

    /// 是否已被打趴（血歸零、正在休息復原）。
    pub fn is_downed(&self) -> bool {
        self.hp == 0
    }

    /// 挨一下打，承受 `power` 點傷害。回傳「這一下是否把玩家打趴」。
    ///
    /// 語意刻意對齊 `Enemy::attack`：
    ///   - 未致命：扣血、回 `false`，並重置自然回血冷卻（剛挨打不會馬上回血）。
    ///   - 致命的那一下：扣到 0、啟動復原倒數、回 `true`（被打趴了）。
    ///   - 已被打趴（`hp == 0`）時再挨打：no-op、回 `false`（趴著不會再扣、不會變負）。
    ///   - `power == 0`：no-op、回 `false`。
    ///
    /// `power` 由接線層決定（敵人的 `EnemyKind::threat`，將來防具 / 體質可再削減），
    /// 這層只吃整數傷害；過量傷害飽和夾到 0，不 underflow。
    pub fn take_damage(&mut self, power: u32) -> bool {
        if power == 0 || self.hp == 0 {
            return false;
        }
        // 飽和扣血：傷害超過剩餘血時夾到 0，不會 underflow。
        self.hp = self.hp.saturating_sub(power);
        // 剛挨打：暫停自然回血一段時間，清掉先前湊到一半的回血累積。
        self.regen_cooldown = REGEN_DELAY_SECS;
        self.regen_accum = 0.0;
        if self.hp == 0 {
            // 被打趴：開始休息復原倒數。
            self.recovery_timer = RECOVERY_SECS;
            true
        } else {
            false
        }
    }

    /// 重置自然回血冷卻（蕈菇活化液使用效果）：讓玩家挨打後立刻開始自然回血，無需等待。
    /// 倒地時無效（倒地期間是復原計時器、不是自然回血）。
    pub fn reset_regen_cooldown(&mut self) {
        if self.hp == 0 {
            return;
        }
        self.regen_cooldown = 0.0;
        self.regen_accum = 0.0;
    }

    /// 被同伴扶起時就地恢復的血量：最大血量的一半，至少 1（半血起身——免去回新手村的折返，
    /// 但仍留一點戰鬥張力，不是滿血白嫖）。純函式。
    pub fn revive_hp(&self) -> u32 {
        (self.max_hp / 2).max(1)
    }

    /// 被附近同伴扶起（ROADMAP 464）：**只有倒地（hp == 0）時有效**。半血就地起身、把復原倒數
    /// 清零（故遊戲迴圈那條「自然復原→傳回新手村」的判定不會對被救者觸發，他留在原地），
    /// 並補一段回血冷卻（剛被扶起不立刻自然回血、保留戰鬥張力）。
    /// 回傳「是否真的扶起了」：非倒地時為 no-op、回 `false`（站著的人扶不起來、也不會被亂動血量）。
    pub fn revive(&mut self) -> bool {
        if self.hp != 0 {
            return false;
        }
        self.hp = self.revive_hp();
        self.recovery_timer = 0.0;
        self.regen_cooldown = REGEN_DELAY_SECS;
        self.regen_accum = 0.0;
        true
    }

    /// 道具回血（活力藥水等）：立即恢復 `amount` HP，不超過 `self.max_hp`。
    /// 倒地（hp == 0）時無效，回傳 0。正常回傳實際回復量（可能因接近上限而小於 amount）。
    pub fn heal(&mut self, amount: u32) -> u32 {
        if self.hp == 0 {
            return 0;
        }
        let before = self.hp;
        self.hp = (self.hp + amount).min(self.max_hp);
        self.hp - before
    }

    /// 林蔭小憩額外回血（ROADMAP 467）：在社群種大的成樹樹蔭下、且脫離戰鬥（`regen_cooldown`
    /// 已歸零）、存活且未滿血時，於自然回血之外每秒額外回 `SHADE_REGEN_PER_SEC`，回傳本次實際
    /// 加上去的血量。倒地／剛挨打／已滿血／壞 `dt` 一律 no-op、回 0（不破壞戰鬥張力、不無謂動血量）。
    /// 與 `tick` 共用 `regen_accum` 累積器（等同把樹蔭下的自然回血速率調快、湊滿整數點才加血），
    /// 並維持 `regen_accum ∈ [0, 1)` 的載入不變式。純函式可測。
    /// 呼叫慣例：遊戲迴圈在 `tick(dt)` 之後、僅當玩家正站在 `world_grove::in_shade` 內才呼叫。
    pub fn shade_regen(&mut self, dt: f32) -> u32 {
        if dt <= 0.0 || !dt.is_finite() {
            return 0;
        }
        if self.hp == 0 || self.hp >= self.max_hp || self.regen_cooldown > 0.0 {
            return 0;
        }
        self.regen_accum += SHADE_REGEN_PER_SEC * dt;
        let whole = self.regen_accum.floor();
        let mut healed = 0;
        if whole >= 1.0 {
            let before = self.hp;
            self.hp = (self.hp + whole as u32).min(self.max_hp);
            self.regen_accum -= whole;
            healed = self.hp - before;
        }
        // 滿血後清掉殘餘累積，維持 `regen_accum ∈ [0, 1)`（與 `tick` 同一不變式）。
        if self.hp >= self.max_hp {
            self.regen_accum = 0.0;
        }
        healed
    }

    /// 推進 `dt` 秒：被打趴時倒數復原，存活且脫離戰鬥時自然回血。
    /// 非正 / 非有限 `dt` 皆為 no-op（比照 `Enemy::tick` / `Vehicle::step` 擋壞 dt）。
    pub fn tick(&mut self, dt: f32) {
        if dt <= 0.0 || !dt.is_finite() {
            return;
        }
        if self.hp == 0 {
            // 被打趴：休息倒數，到點滿血復原、清掉所有過程量。
            self.recovery_timer -= dt;
            if self.recovery_timer <= 0.0 {
                self.hp = self.max_hp;
                self.recovery_timer = 0.0;
                self.regen_cooldown = 0.0;
                self.regen_accum = 0.0;
            }
            return;
        }
        // 還活著：先走回血冷卻；剛挨打的這段期間不回血。
        if self.regen_cooldown > 0.0 {
            self.regen_cooldown = (self.regen_cooldown - dt).max(0.0);
            return;
        }
        // 脫離戰鬥、未滿血：累積自然回血，湊滿整數點數才加上去。
        if self.hp < self.max_hp {
            self.regen_accum += REGEN_PER_SEC * dt;
            let whole = self.regen_accum.floor();
            if whole >= 1.0 {
                self.hp = (self.hp + whole as u32).min(self.max_hp);
                self.regen_accum -= whole;
            }
            // 滿血後清掉殘餘累積，維持 `regen_accum` 落在 `[0, 1)` 的不變式。
            if self.hp >= self.max_hp {
                self.regen_accum = 0.0;
            }
        }
    }

    /// 從存檔載入的值是否「健全」：生命不超上限、各計時器有限且非負、回血累積落在 `[0, 1)`。
    /// 這是與調校常數無關的最小不變式——正常流程（`new` 滿血、`take_damage` 只遞減、
    /// `tick` 一律把計時器夾在 `>= 0`、累積維持 `[0, 1)`）絕不會產生界外值，所以這些只會
    /// 來自壞檔或被竄改的存檔。`hp` 是 `u32`、型別本身就擋掉 `NaN` / 負值，故只需驗上界。
    /// 延續 `combat::is_loadable` / `field::from_tiles` 的載入時驗證脈絡；接 0-E 載入路徑時，
    /// 連同本 impl 區塊的 `allow(dead_code)` 一併移除。
    pub fn is_loadable(&self) -> bool {
        self.max_hp >= MAX_HP
            && self.hp <= self.max_hp
            && self.recovery_timer.is_finite()
            && self.recovery_timer >= 0.0
            && self.regen_cooldown.is_finite()
            && self.regen_cooldown >= 0.0
            && self.regen_accum.is_finite()
            && (0.0..1.0).contains(&self.regen_accum)
    }

    /// 測試用：直接組出指定狀態（含壞值）的生命狀態，驗證載入防線。
    #[cfg(test)]
    pub fn from_raw(hp: u32, recovery_timer: f32, regen_cooldown: f32, regen_accum: f32) -> Self {
        Self {
            hp,
            max_hp: MAX_HP,
            recovery_timer,
            regen_cooldown,
            regen_accum,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_vitals_is_full_hp_and_alive() {
        let v = Vitals::new();
        assert_eq!(v.hp(), MAX_HP);
        assert_eq!(v.max_hp(), MAX_HP);
        assert!(v.is_alive());
        assert!(!v.is_downed());
        assert_eq!(v.fraction(), 1.0);
    }

    #[test]
    fn non_lethal_damage_reduces_hp_but_not_downed() {
        let mut v = Vitals::new();
        assert!(!v.take_damage(3));
        assert_eq!(v.hp(), MAX_HP - 3);
        assert!(v.is_alive());
        assert!(!v.is_downed());
    }

    #[test]
    fn lethal_blow_downs_and_starts_recovery() {
        let mut v = Vitals::new();
        // 一口氣打掉所有血：致命那下回傳 true、進入被打趴。
        assert!(v.take_damage(MAX_HP));
        assert_eq!(v.hp(), 0);
        assert!(v.is_downed());
        assert!(!v.is_alive());
        assert_eq!(v.fraction(), 0.0);
    }

    #[test]
    fn overkill_clamps_to_zero_and_downs_once() {
        let mut v = Vitals::new();
        // 傷害遠超血量：夾到 0、回報打趴、不 underflow。
        assert!(v.take_damage(MAX_HP + 999));
        assert_eq!(v.hp(), 0);
        assert!(v.is_downed());
    }

    #[test]
    fn damaging_a_downed_player_is_noop() {
        let mut v = Vitals::new();
        v.take_damage(MAX_HP);
        let downed = v.clone();
        // 趴著再挨打：不再扣、狀態不變、不回報再次打趴。
        assert!(!v.take_damage(10));
        assert_eq!(v, downed);
    }

    #[test]
    fn zero_power_damage_is_noop() {
        let mut v = Vitals::new();
        let before = v.clone();
        assert!(!v.take_damage(0));
        assert_eq!(v, before);
    }

    #[test]
    fn revive_only_works_when_downed() {
        // 站著的人扶不起來：no-op、回 false、狀態完全不動。
        let mut v = Vitals::new();
        let before = v.clone();
        assert!(!v.revive());
        assert_eq!(v, before);
        // 把他打趴後才扶得起來。
        v.take_damage(MAX_HP);
        assert!(v.is_downed());
        assert!(v.revive());
    }

    #[test]
    fn revive_stands_up_at_half_hp_and_clears_recovery() {
        let mut v = Vitals::new();
        v.take_damage(MAX_HP);
        assert!(v.is_downed());
        assert!(v.revive());
        // 半血起身（基礎等級 max=MAX_HP → 半血）、不再倒地。
        assert_eq!(v.hp(), MAX_HP / 2);
        assert!(v.is_alive());
        assert!(!v.is_downed());
        // 復原倒數已清零：遊戲迴圈的「自然復原→傳回新手村」判定不會對被救者觸發。
        // 再 tick 一大段時間也只會自然回血、絕不再被當成「剛從倒地滿血復原」而傳走。
        v.tick(RECOVERY_SECS + 10.0);
        assert!(v.is_alive());
    }

    #[test]
    fn revive_hp_scales_with_max_and_is_at_least_one() {
        // 高等級玩家半血起身用的是縮放後的最大血量。
        let mut v = Vitals::new();
        v.set_max_hp_full(level_max_hp(5)); // max = 30
        v.take_damage(v.max_hp());
        assert!(v.is_downed());
        assert!(v.revive());
        assert_eq!(v.hp(), level_max_hp(5) / 2);
        // revive_hp 永不為 0（即使極小 max 也至少 1）。
        assert!(v.revive_hp() >= 1);
    }

    #[test]
    fn downed_player_recovers_to_full_after_timer() {
        let mut v = Vitals::new();
        v.take_damage(MAX_HP);
        assert!(v.is_downed());
        // 還沒休息夠，仍趴著。
        v.tick(RECOVERY_SECS - 1.0);
        assert!(v.is_downed());
        // 補足剩餘時間，滿血復原、再次站得起來。
        v.tick(1.0);
        assert!(v.is_alive());
        assert_eq!(v.hp(), MAX_HP);
    }

    #[test]
    fn no_regen_during_cooldown_after_being_hit() {
        let mut v = Vitals::new();
        v.take_damage(5);
        let hurt = v.hp();
        // 剛挨打、還在回血冷卻內：不自然回血。
        v.tick(REGEN_DELAY_SECS - 1.0);
        assert_eq!(v.hp(), hurt);
    }

    #[test]
    fn regenerates_after_leaving_combat() {
        let mut v = Vitals::new();
        v.take_damage(5);
        let hurt = v.hp();
        // 撐過回血冷卻，再過幾秒自然回血。
        v.tick(REGEN_DELAY_SECS);
        v.tick(3.0);
        assert!(v.hp() > hurt);
        assert!(v.hp() <= MAX_HP);
    }

    #[test]
    fn regen_never_exceeds_max_hp() {
        let mut v = Vitals::new();
        v.take_damage(1);
        // 撐過冷卻後一大步推進：回血最多到滿血、不溢出。
        v.tick(REGEN_DELAY_SECS);
        v.tick(1000.0);
        assert_eq!(v.hp(), MAX_HP);
        // 滿血後維持滿血、累積已清空（仍可載入）。
        assert!(v.is_loadable());
    }

    #[test]
    fn full_health_tick_is_noop() {
        let mut v = Vitals::new();
        let before = v.clone();
        v.tick(100.0);
        assert_eq!(v, before);
    }

    #[test]
    fn zero_or_negative_or_nonfinite_dt_is_noop() {
        let mut v = Vitals::new();
        v.take_damage(MAX_HP);
        let downed = v.clone();
        v.tick(0.0);
        assert_eq!(v, downed);
        v.tick(-5.0);
        assert_eq!(v, downed);
        v.tick(f32::NAN);
        assert_eq!(v, downed);
        v.tick(f32::INFINITY);
        assert_eq!(v, downed);
    }

    #[test]
    fn full_cycle_down_recover_take_damage_again() {
        let mut v = Vitals::new();
        // 打趴。
        assert!(v.take_damage(MAX_HP));
        assert!(v.is_downed());
        // 一大步推過復原時間，滿血復原。
        v.tick(RECOVERY_SECS);
        assert!(v.is_alive());
        assert_eq!(v.hp(), MAX_HP);
        // 復原後又能再挨打、再被打趴一次。
        assert!(v.take_damage(MAX_HP));
        assert!(v.is_downed());
    }

    #[test]
    fn is_loadable_accepts_normal_and_rejects_corrupt() {
        // 正常流程產出的狀態都該可載入。
        assert!(Vitals::new().is_loadable());
        let mut v = Vitals::new();
        v.take_damage(MAX_HP);
        assert!(v.is_loadable()); // 被打趴且帶復原倒數，仍健全
        // 壞值：生命超過上限、各計時器 NaN / Inf / 負、回血累積界外。
        assert!(!Vitals::from_raw(MAX_HP + 1, 0.0, 0.0, 0.0).is_loadable());
        assert!(!Vitals::from_raw(0, f32::NAN, 0.0, 0.0).is_loadable());
        assert!(!Vitals::from_raw(0, f32::INFINITY, 0.0, 0.0).is_loadable());
        assert!(!Vitals::from_raw(0, -1.0, 0.0, 0.0).is_loadable());
        assert!(!Vitals::from_raw(MAX_HP, 0.0, -1.0, 0.0).is_loadable());
        assert!(!Vitals::from_raw(MAX_HP, 0.0, 0.0, 1.0).is_loadable()); // 累積須 < 1
        assert!(!Vitals::from_raw(MAX_HP, 0.0, 0.0, -0.5).is_loadable());
    }

    #[test]
    fn serde_round_trip_preserves_state() {
        let mut v = Vitals::new();
        v.take_damage(7); // 留個受傷中、帶回血冷卻的狀態
        let json = serde_json::to_string(&v).unwrap();
        let back: Vitals = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    // ── Phase 1 敵人反擊咬進玩家生命值的組合測試 ──────────────────────────
    // `combat.rs` 早就備好 `EnemyKind::threat`（敵人每次反擊的傷害），但一直「無呼叫端、
    // 待玩家生命值那條切片才接」。這個組合測試走一遍那條接縫：用敵人的 `threat` 餵進玩家
    // 的 `take_damage`，鎖住「敵人反擊真的扣到玩家血、扣夠了把玩家打趴」這個設計契約——
    // 接線層只要把每 tick 附近敵人的 `threat` 串進來即可，任一邊的傷害語意漂移都會在此斷掉。

    use crate::combat::EnemyKind;

    #[test]
    fn enemy_threat_damages_player_vitals() {
        let mut v = Vitals::new();
        // 銹蝕巡邏機反擊一下，照它的 threat 扣血。
        let drone = EnemyKind::ScrapDrone.threat();
        assert!(!v.take_damage(drone));
        assert_eq!(v.hp(), MAX_HP - drone);
        // 乙太靈威脅較低，再扣一點點。
        let wisp = EnemyKind::EtherWisp.threat();
        v.take_damage(wisp);
        assert_eq!(v.hp(), MAX_HP - drone - wisp);
    }

    #[test]
    fn enough_enemy_hits_eventually_down_the_player() {
        let mut v = Vitals::new();
        let threat = EnemyKind::ScrapDrone.threat();
        assert!(threat > 0, "敵人反擊應有正傷害，否則永遠打不趴玩家");
        // 持續挨同一隻敵人的反擊，累積到一定下數會被打趴（戰鬥因此有風險）。
        let mut blows = 0;
        while v.is_alive() {
            v.take_damage(threat);
            blows += 1;
            assert!(blows < 1000, "正傷害應在有限下數內把玩家打趴");
        }
        assert!(v.is_downed());
    }

    #[test]
    fn heal_restores_hp_up_to_max() {
        let mut v = Vitals::new();
        v.take_damage(8);
        assert_eq!(v.hp(), MAX_HP - 8);
        // 回復 6 HP。
        let gained = v.heal(6);
        assert_eq!(gained, 6);
        assert_eq!(v.hp(), MAX_HP - 2);
    }

    #[test]
    fn heal_clamps_at_max_hp() {
        let mut v = Vitals::new();
        v.take_damage(2);
        // 試圖回復 10，但只剩 2 HP 的缺口。
        let gained = v.heal(10);
        assert_eq!(gained, 2);
        assert_eq!(v.hp(), MAX_HP);
    }

    #[test]
    fn heal_does_nothing_when_downed() {
        let mut v = Vitals::new();
        v.take_damage(MAX_HP);
        assert!(v.is_downed());
        let gained = v.heal(10);
        assert_eq!(gained, 0);
        assert_eq!(v.hp(), 0);
    }

    #[test]
    fn reset_regen_cooldown_allows_immediate_regen() {
        let mut v = Vitals::new();
        v.take_damage(5);
        // 挨打後 regen_cooldown 被設定，正常要等 5 秒才自然回血。
        // 蕈菇活化液呼叫 reset_regen_cooldown 後，立刻進入自然回血狀態。
        v.reset_regen_cooldown();
        let before = v.hp();
        v.tick(2.0); // 等兩秒，自然回血應已啟動（regen 冷卻已清零）。
        assert!(v.hp() > before, "重置回血冷卻後應立即開始自然回血");
    }

    #[test]
    fn reset_regen_cooldown_is_noop_when_downed() {
        let mut v = Vitals::new();
        v.take_damage(MAX_HP);
        assert!(v.is_downed());
        let before = v.clone();
        v.reset_regen_cooldown(); // 倒地時無效，不應改變狀態。
        // 只有 regen_cooldown / regen_accum 可能被改，但倒地時應 no-op。
        // 倒地恢復仍由 recovery_timer 驅動，不受影響。
        v.tick(RECOVERY_SECS);
        assert!(v.is_alive(), "倒地後仍能正常復原");
        let _ = before;
    }

    // ── 升級加成（ROADMAP 18）測試 ───────────────────────────────────────

    #[test]
    fn level_max_hp_scales_with_level() {
        assert_eq!(level_max_hp(0), 20);
        assert_eq!(level_max_hp(1), 22);
        assert_eq!(level_max_hp(5), 30);
        assert_eq!(level_max_hp(10), 40);
    }

    #[test]
    fn set_max_hp_full_gives_full_health_at_new_max() {
        let mut v = Vitals::new();
        v.take_damage(5); // 15/20 hp
        v.set_max_hp_full(level_max_hp(5)); // Lv.5 → max = 30
        assert_eq!(v.max_hp(), 30);
        assert_eq!(v.hp(), 30, "重連給滿血");
        assert!(v.is_loadable());
    }

    #[test]
    fn on_level_up_increases_max_and_gives_bonus_hp() {
        let mut v = Vitals::new(); // 20/20
        v.on_level_up(level_max_hp(1)); // max → 22, hp → 22（+2 bonus）
        assert_eq!(v.max_hp(), 22);
        assert_eq!(v.hp(), 22);
        v.take_damage(5);               // 17/22
        v.on_level_up(level_max_hp(2)); // max → 24, hp → 19（+2 bonus）
        assert_eq!(v.max_hp(), 24);
        assert_eq!(v.hp(), 19);
    }

    #[test]
    fn heal_respects_level_max_hp() {
        let mut v = Vitals::new();
        v.set_max_hp_full(level_max_hp(5)); // max = 30, hp = 30
        v.take_damage(15);                   // 15/30
        let gained = v.heal(100);            // 試圖超量回血，應夾在 30
        assert_eq!(v.hp(), 30);
        assert_eq!(gained, 15);
    }

    #[test]
    fn tick_recovery_restores_to_level_max_hp() {
        let mut v = Vitals::new();
        v.set_max_hp_full(level_max_hp(5)); // max = 30, hp = 30
        v.take_damage(30);                   // 倒地
        assert!(v.is_downed());
        v.tick(RECOVERY_SECS);
        assert!(v.is_alive());
        assert_eq!(v.hp(), 30, "復原後應回滿等級對應的最大血量");
    }

    // ─── 林蔭小憩 shade_regen（ROADMAP 467） ──────────────────────────────────

    #[test]
    fn shade_regen_heals_when_out_of_combat() {
        let mut v = Vitals::new();
        v.take_damage(10); // 10/20，並起算 regen_cooldown
        // 脫離戰鬥後（冷卻歸零）站在樹蔭下，整秒應額外回血一點。
        v.tick(REGEN_DELAY_SECS); // 把回血冷卻走完（此步不回血）
        let healed = v.shade_regen(1.0);
        assert_eq!(healed, 1, "脫戰後在樹蔭下整秒應額外回 1 點");
        assert_eq!(v.hp(), 11);
    }

    #[test]
    fn shade_regen_blocked_right_after_damage() {
        let mut v = Vitals::new();
        v.take_damage(10); // 剛挨打：regen_cooldown > 0
        // 樹蔭也救不了剛挨打的人：保留戰鬥張力。
        assert_eq!(v.shade_regen(1.0), 0, "剛挨打期間樹蔭不生效");
        assert_eq!(v.hp(), 10);
    }

    #[test]
    fn shade_regen_noop_when_downed_or_full_or_bad_dt() {
        // 倒地：no-op。
        let mut downed = Vitals::new();
        downed.take_damage(MAX_HP);
        assert_eq!(downed.shade_regen(1.0), 0);
        assert!(downed.is_downed());
        // 滿血：no-op（不無謂動血量）。新建滿血者 regen_cooldown 本就為 0。
        let mut full = Vitals::new();
        assert_eq!(full.shade_regen(1.0), 0);
        assert_eq!(full.hp(), MAX_HP);
        // 壞 dt（非正／非有限）：no-op。
        let mut hurt = Vitals::new();
        hurt.take_damage(5);
        hurt.reset_regen_cooldown();
        assert_eq!(hurt.shade_regen(0.0), 0);
        assert_eq!(hurt.shade_regen(-1.0), 0);
        assert_eq!(hurt.shade_regen(f32::NAN), 0);
        assert_eq!(hurt.shade_regen(f32::INFINITY), 0);
        assert_eq!(hurt.hp(), MAX_HP - 5);
    }

    #[test]
    fn shade_regen_keeps_loadable_invariant_and_clamps_to_max() {
        let mut v = Vitals::new();
        v.take_damage(2); // 18/20
        v.reset_regen_cooldown();
        // 連續推進足以回滿，accum 不破壞 [0,1) 不變式、血量夾在上限。
        for _ in 0..200 {
            v.shade_regen(0.05);
        }
        assert_eq!(v.hp(), MAX_HP, "樹蔭回血應夾在最大血量");
        assert!(v.is_loadable(), "shade_regen 後仍須滿足載入不變式（regen_accum ∈ [0,1)）");
    }
}
