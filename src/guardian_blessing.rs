/// 守護者元素祝福（ROADMAP 533）
///
/// 擊敗世界守護者的參戰玩家，將獲得對應元素的祝福光環，持續 2 小時。
/// 其他玩家能看到光環顏色，知道你剛才討伐了哪位守護者。
/// 祝福期間殺敵額外獲得少量乙太，作為持續探索野外的激勵。
///
/// 純記憶體模式：重啟後清零（守護者 4 小時重生，祝福 2 小時，間距合理）。

use std::collections::HashMap;
use uuid::Uuid;

/// 祝福持續秒數（2 小時）。
pub const BLESSING_DURATION_SECS: f32 = 7200.0;

/// 守護者元素種類（與 `world_boss::ALL_VARIANTS` 序號對齊）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlessingKind {
    Chaos, // 🗿 東方·混沌守護者
    Frost, // 🐉 北方·晶霜巨龍
    Flame, // 🦎 西方·熔焰蜥龍
    Void,  // 👻 南方·深淵幽靈
}

impl BlessingKind {
    /// 依守護者擊敗序號（defeat_count 累計，注意這是「將擊敗的那次」的 count，即 defeat_count-1 的輪次）。
    pub fn from_variant_index(idx: u32) -> BlessingKind {
        match idx % 4 {
            0 => BlessingKind::Chaos,
            1 => BlessingKind::Frost,
            2 => BlessingKind::Flame,
            _ => BlessingKind::Void,
        }
    }

    /// 前端 wire 字串（穩定契約，別重排）。
    pub fn wire_str(self) -> &'static str {
        match self {
            Self::Chaos => "chaos",
            Self::Frost => "frost",
            Self::Flame => "flame",
            Self::Void  => "void",
        }
    }

    /// 繁中名稱（面向玩家字串，留 i18n 空間）。
    pub fn zh_name(self) -> &'static str {
        match self {
            Self::Chaos => "混沌祝福",
            Self::Frost => "冰霜祝福",
            Self::Flame => "烈焰祝福",
            Self::Void  => "虛空祝福",
        }
    }

    /// 元素 emoji（守護者 emoji，讓 HUD 一目了然）。
    pub fn emoji(self) -> &'static str {
        match self {
            Self::Chaos => "🗿",
            Self::Frost => "🐉",
            Self::Flame => "🦎",
            Self::Void  => "👻",
        }
    }

    /// 祝福期間殺敵額外乙太獎勵。
    pub fn kill_ether_bonus(self) -> u32 {
        3
    }
}

/// 單玩家的守護者祝福狀態。
#[derive(Debug, Clone)]
pub struct PlayerBlessing {
    pub kind: BlessingKind,
    pub remaining_secs: f32,
}

impl PlayerBlessing {
    pub fn new(kind: BlessingKind) -> Self {
        Self { kind, remaining_secs: BLESSING_DURATION_SECS }
    }

    /// 推進時間，返回是否仍有效。非正或非有限的 dt 不推進（防呆）。
    pub fn tick(&mut self, dt: f32) -> bool {
        if dt > 0.0 && dt.is_finite() {
            self.remaining_secs = (self.remaining_secs - dt).max(0.0);
        }
        self.remaining_secs > 0.0
    }

    pub fn is_active(&self) -> bool {
        self.remaining_secs > 0.0
    }
}

/// 全服守護者祝福狀態儲存（記憶體模式，重啟清零）。
#[derive(Default)]
pub struct GuardianBlessingStore {
    inner: HashMap<Uuid, PlayerBlessing>,
}

impl GuardianBlessingStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 授予（或更新）祝福：相同玩家會換種類並重置計時。
    pub fn grant(&mut self, player_id: Uuid, kind: BlessingKind) {
        self.inner.insert(player_id, PlayerBlessing::new(kind));
    }

    /// 推進全部祝福計時，自動清除已過期者。
    pub fn tick(&mut self, dt: f32) {
        self.inner.retain(|_, b| b.tick(dt));
    }

    /// 查玩家的有效祝福（快照/HUD 用）。
    pub fn get(&self, player_id: Uuid) -> Option<&PlayerBlessing> {
        self.inner.get(&player_id).filter(|b| b.is_active())
    }

    /// 取殺敵乙太紅利（無祝福回 0）。
    pub fn kill_bonus_ether(&self, player_id: Uuid) -> u32 {
        self.inner
            .get(&player_id)
            .filter(|b| b.is_active())
            .map(|b| b.kind.kill_ether_bonus())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_index_cycles_correctly() {
        assert!(matches!(BlessingKind::from_variant_index(0), BlessingKind::Chaos));
        assert!(matches!(BlessingKind::from_variant_index(1), BlessingKind::Frost));
        assert!(matches!(BlessingKind::from_variant_index(2), BlessingKind::Flame));
        assert!(matches!(BlessingKind::from_variant_index(3), BlessingKind::Void));
        // 循環
        assert!(matches!(BlessingKind::from_variant_index(4), BlessingKind::Chaos));
        assert!(matches!(BlessingKind::from_variant_index(7), BlessingKind::Void));
    }

    #[test]
    fn blessing_tick_expires_correctly() {
        let mut b = PlayerBlessing::new(BlessingKind::Frost);
        assert!(b.tick(7199.0)); // 還剩 1 秒
        assert!(b.is_active());
        assert!(!b.tick(2.0)); // 超時
        assert!(!b.is_active());
    }

    #[test]
    fn tick_bad_dt_no_panic() {
        let mut b = PlayerBlessing::new(BlessingKind::Flame);
        // 非正/非有限 dt 不推進（防呆）
        b.tick(-1.0);
        b.tick(f32::NAN);
        b.tick(f32::INFINITY);
        b.tick(0.0);
        // 仍滿血計時（上面都被早退）
        assert_eq!(b.remaining_secs, BLESSING_DURATION_SECS);
        assert!(b.is_active());
    }

    #[test]
    fn grant_overrides_previous_kind() {
        let id = Uuid::new_v4();
        let mut store = GuardianBlessingStore::new();
        store.grant(id, BlessingKind::Chaos);
        store.grant(id, BlessingKind::Frost);
        let b = store.get(id).unwrap();
        assert!(matches!(b.kind, BlessingKind::Frost));
        // 計時已重置到滿值
        assert_eq!(b.remaining_secs, BLESSING_DURATION_SECS);
    }

    #[test]
    fn store_tick_removes_expired() {
        let id = Uuid::new_v4();
        let mut store = GuardianBlessingStore::new();
        store.grant(id, BlessingKind::Void);
        store.tick(7201.0);
        assert!(store.get(id).is_none());
    }

    #[test]
    fn kill_bonus_only_when_blessed() {
        let id = Uuid::new_v4();
        let mut store = GuardianBlessingStore::new();
        assert_eq!(store.kill_bonus_ether(id), 0); // 無祝福
        store.grant(id, BlessingKind::Flame);
        assert_eq!(store.kill_bonus_ether(id), 3);
    }

    #[test]
    fn kill_bonus_gone_after_expiry() {
        let id = Uuid::new_v4();
        let mut store = GuardianBlessingStore::new();
        store.grant(id, BlessingKind::Chaos);
        store.tick(7201.0); // 過期
        assert_eq!(store.kill_bonus_ether(id), 0);
    }

    #[test]
    fn wire_str_stable() {
        assert_eq!(BlessingKind::Chaos.wire_str(), "chaos");
        assert_eq!(BlessingKind::Frost.wire_str(), "frost");
        assert_eq!(BlessingKind::Flame.wire_str(), "flame");
        assert_eq!(BlessingKind::Void.wire_str(), "void");
    }

    #[test]
    fn emoji_and_zh_name_present() {
        for kind in [
            BlessingKind::Chaos,
            BlessingKind::Frost,
            BlessingKind::Flame,
            BlessingKind::Void,
        ] {
            assert!(!kind.emoji().is_empty());
            assert!(!kind.zh_name().is_empty());
            assert!(kind.kill_ether_bonus() > 0);
        }
    }
}
