//! 敵人模型（Phase 1 戰鬥 MVP「自動打怪」的純邏輯地基）。
//!
//! 這層只管「一隻敵人怎麼被打、被打倒後掉什麼、之後怎麼重生」，是純資料 + 純函式，
//! 無 IO、不碰 WebSocket / 遊戲迴圈，便於自動測試。延續 `gather.rs` / `crops.rs` /
//! `vehicle.rs` 的前置慣例：純邏輯先落地、標 `allow(dead_code)`，接線輪（世界撒佈敵人、
//! 角色自動攻擊附近敵人、掉落物進背包、遊戲迴圈每 tick 推進重生）才有呼叫端。
//!
//! 之後接上：
//!   - 世界：在曠野撒佈若干 `Enemy`（比照 `gather_field.rs` 之於 `gather.rs`，
//!     另立一層 `enemy_field` 管「敵人擺哪、角色自動鎖定最近的哪一隻」）。
//!   - ws / 遊戲迴圈：角色靠近時自動攻擊 → `attack(power)`；打倒回傳掉落 → `add` 進背包。
//!   - 遊戲迴圈：每 tick 對被打倒的敵人呼叫 `tick(dt)` 倒數重生。
//!   - 持久化（接 0-E）：把敵人狀態序列化（載入時走 `is_loadable` 驗證）。
//!
//! 戰鬥迴圈刻意鏡像採集（`ResourceNode`）：敵人有「生命」（像耐久），每次攻擊扣血、
//! 打到 0 即被打倒並**一次性**掉落戰利品，接著進入重生倒數，倒數到了滿血復活再次可打——
//! 敵人不是無限白刷，打完一處得換地方或等它重生，給世界一點節奏。
//!
//! 主題是療癒的蒸汽龐克太空歌劇，敵人不是嚇人的怪物，而是失控的機械 / 野化的乙太生靈；
//! 「打倒」更接近安撫 / 拆解，落下可用的素材與乙太。`threat` 是這隻敵人的危險度（之後
//! 玩家有生命值那條切片才會用到，現在只是刻畫敵人強弱的調校常數）。

use serde::{Deserialize, Serialize};

use crate::inventory::ItemKind;

/// 敵人的種類。種類決定生命多寡、掉落什麼、危險度、重生多久。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnemyKind {
    /// 銹蝕巡邏機：故障的舊機械，皮厚、打倒落下礦石（拆解的廢鐵）。
    ScrapDrone,
    /// 迷途乙太靈：野化的乙太生靈，較脆、安撫後散出乙太（種田之外的另一條乙太來源）。
    EtherWisp,
}

// 整個模組是前置地基：接線輪（世界撒佈敵人、ws 自動攻擊、遊戲迴圈推進重生）才有呼叫端，
// 在此之前公開項目皆無外部呼叫，比照 `gather.rs` / `plots.rs` 逐項標 `allow(dead_code)`。
#[allow(dead_code)]
impl EnemyKind {
    /// 此種類滿血時的生命值（要扣到 0 才算打倒）。
    pub fn max_hp(self) -> u32 {
        match self {
            EnemyKind::ScrapDrone => 6,
            EnemyKind::EtherWisp => 4,
        }
    }

    /// 打倒後掉落的戰利品 `(物品, 數量)`。刻意沿用既有 `ItemKind`，不另開新物品
    /// 變體——戰鬥因此自包含、不動 backend 正在接線的 `inventory.rs`，掉落也直接咬進
    /// 採集 / 合成已有的資源經濟。
    pub fn drop_loot(self) -> (ItemKind, u32) {
        match self {
            // 銹蝕機械拆出廢鐵（礦石）。
            EnemyKind::ScrapDrone => (ItemKind::Stone, 2),
            // 乙太靈散出乙太，但量少、貼合「稀有資源」手感。
            EnemyKind::EtherWisp => (ItemKind::Ether, 1),
        }
    }

    /// 危險度：這隻敵人每次反擊對玩家造成的傷害。**目前無呼叫端**——玩家生命值
    /// 那條切片才會用到，先以調校常數刻畫敵人強弱、為接線預留。
    pub fn threat(self) -> u32 {
        match self {
            EnemyKind::ScrapDrone => 2,
            EnemyKind::EtherWisp => 1,
        }
    }

    /// 被打倒後到滿血復活所需的重生秒數。
    pub fn respawn_secs(self) -> f32 {
        match self {
            EnemyKind::ScrapDrone => 50.0,
            EnemyKind::EtherWisp => 35.0,
        }
    }
}

/// 世界裡一隻可被打倒、之後會重生的敵人。
///
/// 狀態只有「剩餘生命」與「重生倒數」兩個欄位，階段（存活 / 被打倒）皆由生命推導，
/// 維持單一真實來源——比照 `ResourceNode` 以剩餘耐久推導可採 / 採空。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Enemy {
    /// 敵人種類（決定生命 / 掉落 / 危險度 / 重生時間）。
    kind: EnemyKind,
    /// 剩餘生命（可再承受的傷害量）。歸零＝被打倒。
    remaining_hp: u32,
    /// 重生倒數（秒）。只有被打倒（`remaining_hp == 0`）時才有意義；倒數到 0 滿血復活。
    respawn_timer: f32,
}

#[allow(dead_code)] // 同上：前置地基，接線輪才有呼叫端。
impl Enemy {
    /// 生出一隻滿血、可立即被攻擊的新敵人。
    pub fn new(kind: EnemyKind) -> Self {
        Self {
            kind,
            remaining_hp: kind.max_hp(),
            respawn_timer: 0.0,
        }
    }

    /// 敵人種類。
    pub fn kind(&self) -> EnemyKind {
        self.kind
    }

    /// 剩餘生命。
    pub fn remaining_hp(&self) -> u32 {
        self.remaining_hp
    }

    /// 是否已被打倒（需等重生）。
    pub fn is_defeated(&self) -> bool {
        self.remaining_hp == 0
    }

    /// 是否還活著、可被攻擊。
    pub fn is_alive(&self) -> bool {
        self.remaining_hp > 0
    }

    /// 攻擊一下，造成 `power` 點傷害。
    ///
    /// 回傳語意刻意只在**打倒的那一下**給出掉落 `Some((物品, 數量))`，其餘情況回 `None`：
    ///   - 未致命的攻擊：扣血、回 `None`（還沒倒、不掉東西）。
    ///   - 致命的那一下：扣到 0、啟動重生倒數、回 `Some(掉落)`（戰利品只給一次）。
    ///   - 攻擊已被打倒（重生中）的敵人：no-op、回 `None`（不重複掉落）。
    ///   - `power == 0`：no-op、回 `None`（比照 `ResourceNode::gather` 對採空的 no-op）。
    ///
    /// `power` 由接線層決定（將來武器 / 戰鬥技能讓每下傷害更高，比照採集的工具倍率），
    /// 這層只吃整數傷害；血量過量被打（`power` 大於剩餘血）一律夾到 0，不會變負、不會多掉。
    pub fn attack(&mut self, power: u32) -> Option<(ItemKind, u32)> {
        if power == 0 || self.remaining_hp == 0 {
            return None;
        }
        // 飽和扣血：傷害超過剩餘血時夾到 0，不會 underflow。
        self.remaining_hp = self.remaining_hp.saturating_sub(power);
        if self.remaining_hp == 0 {
            // 被打倒：開始倒數重生，並一次性掉落戰利品。
            self.respawn_timer = self.kind.respawn_secs();
            Some(self.kind.drop_loot())
        } else {
            None
        }
    }

    /// 推進 `dt` 秒。只有被打倒的敵人在倒數；倒數到 0 滿血復活、再次可被攻擊。
    /// 還活著或非正 `dt` 皆為 no-op（比照 `ResourceNode::tick` 擋非正 dt）。
    pub fn tick(&mut self, dt: f32) {
        if self.remaining_hp > 0 || dt <= 0.0 {
            return;
        }
        self.respawn_timer -= dt;
        if self.respawn_timer <= 0.0 {
            self.remaining_hp = self.kind.max_hp();
            self.respawn_timer = 0.0;
        }
    }

    /// 從存檔載入的值是否「健全」：生命不超過該種類上限、重生倒數有限且非負。
    /// 這是與調校常數無關的最小不變式——正常流程（`new` 滿血、`attack` 只遞減、
    /// `tick` 倒數一律夾在 `>= 0`）絕不會產生界外生命或 `NaN`/`Inf`/負倒數，所以這些
    /// 只會來自壞檔或被竄改的存檔。`remaining_hp` 是 `u32`、型別本身就擋掉 `NaN`/負值，
    /// 故只需驗上界。延續 `gather::is_loadable` / `field::from_tiles` 的載入時驗證脈絡；
    /// 接 0-E 載入路徑時，連同本 impl 區塊的 `allow(dead_code)` 一併移除。
    pub fn is_loadable(&self) -> bool {
        self.respawn_timer.is_finite()
            && self.respawn_timer >= 0.0
            && self.remaining_hp <= self.kind.max_hp()
    }

    /// 測試用：直接組出指定狀態（含壞值）的敵人，驗證載入防線。
    #[cfg(test)]
    pub fn from_raw(kind: EnemyKind, remaining_hp: u32, respawn_timer: f32) -> Self {
        Self {
            kind,
            remaining_hp,
            respawn_timer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [EnemyKind; 2] = [EnemyKind::ScrapDrone, EnemyKind::EtherWisp];

    #[test]
    fn new_enemy_is_full_hp_and_alive() {
        for kind in KINDS {
            let e = Enemy::new(kind);
            assert_eq!(e.remaining_hp(), kind.max_hp());
            assert!(e.is_alive());
            assert!(!e.is_defeated());
        }
    }

    #[test]
    fn non_lethal_attack_damages_but_drops_nothing() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        // 一下打不死（max_hp 6 > 1），扣血但不掉落。
        assert_eq!(e.attack(1), None);
        assert_eq!(e.remaining_hp(), EnemyKind::ScrapDrone.max_hp() - 1);
        assert!(e.is_alive());
    }

    #[test]
    fn killing_blow_drops_loot_and_starts_respawn() {
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        // 一口氣打掉所有血：致命那下回傳掉落。
        let loot = e.attack(EnemyKind::EtherWisp.max_hp());
        assert_eq!(loot, Some(EnemyKind::EtherWisp.drop_loot()));
        assert!(e.is_defeated());
        assert!(!e.is_alive());
    }

    #[test]
    fn loot_drops_exactly_once_on_the_lethal_blow() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        let max = EnemyKind::ScrapDrone.max_hp();
        // 逐下打：只有最後一下（扣到 0）掉落，其餘都 None。
        let mut drops = Vec::new();
        for _ in 0..max {
            if let Some(loot) = e.attack(1) {
                drops.push(loot);
            }
        }
        assert_eq!(drops, vec![EnemyKind::ScrapDrone.drop_loot()]);
        assert!(e.is_defeated());
    }

    #[test]
    fn attacking_a_defeated_enemy_is_noop_and_drops_nothing() {
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        e.attack(EnemyKind::EtherWisp.max_hp());
        let defeated = e.clone();
        // 重生中再打：不掉落、不改變狀態（不重複掉戰利品）。
        assert_eq!(e.attack(10), None);
        assert_eq!(e, defeated);
    }

    #[test]
    fn zero_power_attack_is_noop() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        let before = e.clone();
        assert_eq!(e.attack(0), None);
        assert_eq!(e, before);
    }

    #[test]
    fn overkill_clamps_to_zero_and_drops_once() {
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        // 傷害遠超血量：夾到 0、只掉一次、不 underflow 變負。
        let loot = e.attack(EnemyKind::EtherWisp.max_hp() + 999);
        assert_eq!(loot, Some(EnemyKind::EtherWisp.drop_loot()));
        assert_eq!(e.remaining_hp(), 0);
        assert!(e.is_defeated());
    }

    #[test]
    fn defeated_enemy_respawns_after_timer() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        e.attack(EnemyKind::ScrapDrone.max_hp());
        assert!(e.is_defeated());
        // 還沒到重生時間，仍被打倒。
        e.tick(EnemyKind::ScrapDrone.respawn_secs() - 1.0);
        assert!(e.is_defeated());
        // 補足剩餘時間，滿血復活、再次可打。
        e.tick(1.0);
        assert!(e.is_alive());
        assert_eq!(e.remaining_hp(), EnemyKind::ScrapDrone.max_hp());
    }

    #[test]
    fn tick_on_living_enemy_is_noop() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        e.attack(2); // 受了點傷但還活著
        let before = e.clone();
        e.tick(100.0);
        assert_eq!(e, before);
    }

    #[test]
    fn zero_or_negative_dt_is_noop() {
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        e.attack(EnemyKind::EtherWisp.max_hp());
        let defeated = e.clone();
        e.tick(0.0);
        assert_eq!(e, defeated);
        e.tick(-5.0);
        assert_eq!(e, defeated);
    }

    #[test]
    fn full_cycle_kill_respawn_kill_again() {
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        // 打倒。
        assert!(e.attack(EnemyKind::EtherWisp.max_hp()).is_some());
        assert!(e.is_defeated());
        // 一次大步推過重生時間，滿血復活。
        e.tick(EnemyKind::EtherWisp.respawn_secs());
        assert!(e.is_alive());
        // 復活後又能再打倒一次、再掉一次。
        assert_eq!(
            e.attack(EnemyKind::EtherWisp.max_hp()),
            Some(EnemyKind::EtherWisp.drop_loot())
        );
    }

    #[test]
    fn each_kind_drops_an_existing_resource() {
        // 掉落沿用既有採集 / 經濟資源，戰鬥自包含、不另開物品變體。
        assert_eq!(EnemyKind::ScrapDrone.drop_loot(), (ItemKind::Stone, 2));
        assert_eq!(EnemyKind::EtherWisp.drop_loot(), (ItemKind::Ether, 1));
    }

    #[test]
    fn is_loadable_accepts_normal_and_rejects_corrupt() {
        // 正常流程產出的狀態都該可載入。
        assert!(Enemy::new(EnemyKind::ScrapDrone).is_loadable());
        let mut e = Enemy::new(EnemyKind::EtherWisp);
        e.attack(EnemyKind::EtherWisp.max_hp());
        assert!(e.is_loadable()); // 被打倒且帶重生倒數，仍健全
        // 壞值：生命超過上限、NaN / Inf / 負重生倒數。
        assert!(!Enemy::from_raw(EnemyKind::ScrapDrone, 99, 0.0).is_loadable());
        assert!(!Enemy::from_raw(EnemyKind::EtherWisp, 0, f32::NAN).is_loadable());
        assert!(!Enemy::from_raw(EnemyKind::EtherWisp, 0, f32::INFINITY).is_loadable());
        assert!(!Enemy::from_raw(EnemyKind::EtherWisp, 0, -1.0).is_loadable());
    }

    #[test]
    fn serde_round_trip_preserves_state() {
        let mut e = Enemy::new(EnemyKind::ScrapDrone);
        e.attack(2); // 打了一下，留個半血狀態
        let json = serde_json::to_string(&e).unwrap();
        let back: Enemy = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}
