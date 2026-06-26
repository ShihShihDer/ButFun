//! 居民鄰里熟識度帳本（ROADMAP 557）。
//!
//! 會漫遊的居民（見 `resident_npc.rs`）原本每次鄰里相遇（ROADMAP 121）都是泛泛寒暄、
//! 互不記得對方；本模組讓「反覆碰上的同一對居民」累積熟識度，從萍水相逢 →
//! 點頭之交 → 老鄰居。熟到一定程度，他們的招呼會升級成「叫得出名字、嘮得更親」的對話，
//! 讓玩家第一次看見居民們「自己處出交情、長出鄰里情誼」——北極星「由 AI 棲居的世界」
//! 裡，小社會湧現的第一個玩家看得見的證據。
//!
//! 純記憶體、確定性、零 IO／零 LLM；居民退休（壽命到期替補）時清掉其所有條目，
//! 避免退休 id 的帳目無限堆積。

use std::collections::HashMap;

/// 跨過「點頭之交」門檻所需的累計相遇次數。
pub const ACQUAINTANCE_MEETS: u16 = 2;
/// 跨過「老鄰居」門檻所需的累計相遇次數。
pub const FRIEND_MEETS: u16 = 4;
/// 相遇次數上限（到頂即飽和，避免長壽世界數值無限長大；不影響階層判定）。
const MEETS_CAP: u16 = FRIEND_MEETS + 1;

/// 鄰里熟識階層。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeighborTier {
    /// 萍水相逢——還叫不出對方名字。
    Stranger,
    /// 點頭之交——常打照面、漸漸熟了。
    Acquaintance,
    /// 老鄰居——處出交情、見面格外親。
    Friend,
}

/// 由累計相遇次數推得熟識階層（純函式，好測）。
pub fn tier_from_meets(meets: u16) -> NeighborTier {
    if meets >= FRIEND_MEETS {
        NeighborTier::Friend
    } else if meets >= ACQUAINTANCE_MEETS {
        NeighborTier::Acquaintance
    } else {
        NeighborTier::Stranger
    }
}

/// 居民兩兩之間的鄰里熟識度（記憶體模式，重啟清零）。
///
/// key 為「排序後的居民 id 對」，故 (A,B) 與 (B,A) 共用同一筆——熟識天然對稱。
#[derive(Debug, Default)]
pub struct ResidentBonds {
    meets: HashMap<(String, String), u16>,
}

impl ResidentBonds {
    pub fn new() -> Self {
        Self { meets: HashMap::new() }
    }

    /// 把一對 id 正規化成排序後的 key，確保對稱。
    fn key(a: &str, b: &str) -> (String, String) {
        if a <= b {
            (a.to_string(), b.to_string())
        } else {
            (b.to_string(), a.to_string())
        }
    }

    /// 記一次相遇（相遇次數 +1，到頂即飽和），回傳記完後的熟識階層。
    pub fn record_meeting(&mut self, a: &str, b: &str) -> NeighborTier {
        // 自己跟自己不算（防呆，理論上不會發生）。
        if a == b {
            return NeighborTier::Stranger;
        }
        let entry = self.meets.entry(Self::key(a, b)).or_insert(0);
        if *entry < MEETS_CAP {
            *entry += 1;
        }
        tier_from_meets(*entry)
    }

    /// 查一對居民當前的熟識階層（不修改）。
    pub fn tier_of(&self, a: &str, b: &str) -> NeighborTier {
        let meets = self.meets.get(&Self::key(a, b)).copied().unwrap_or(0);
        tier_from_meets(meets)
    }

    /// 查一對居民的累計相遇次數（測試／除錯用）。
    pub fn meets_between(&self, a: &str, b: &str) -> u16 {
        self.meets.get(&Self::key(a, b)).copied().unwrap_or(0)
    }

    /// 居民退休（壽命到期）時呼叫：清掉所有牽涉該 id 的條目，避免帳目無限堆積。
    pub fn forget(&mut self, id: &str) {
        self.meets.retain(|(a, b), _| a != id && b != id);
    }

    /// 目前記錄的居民對數（測試／除錯用）。
    pub fn pair_count(&self) -> usize {
        self.meets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meets_accumulate_and_tier_up() {
        let mut b = ResidentBonds::new();
        // 第 1 次：還是陌生
        assert_eq!(b.record_meeting("resident_0", "resident_1"), NeighborTier::Stranger);
        // 第 2 次：跨過點頭之交
        assert_eq!(b.record_meeting("resident_0", "resident_1"), NeighborTier::Acquaintance);
        // 補到第 4 次：成老鄰居
        b.record_meeting("resident_0", "resident_1");
        assert_eq!(b.record_meeting("resident_0", "resident_1"), NeighborTier::Friend);
        assert_eq!(b.meets_between("resident_0", "resident_1"), 4);
    }

    #[test]
    fn tier_thresholds_are_monotone() {
        assert_eq!(tier_from_meets(0), NeighborTier::Stranger);
        assert_eq!(tier_from_meets(ACQUAINTANCE_MEETS - 1), NeighborTier::Stranger);
        assert_eq!(tier_from_meets(ACQUAINTANCE_MEETS), NeighborTier::Acquaintance);
        assert_eq!(tier_from_meets(FRIEND_MEETS - 1), NeighborTier::Acquaintance);
        assert_eq!(tier_from_meets(FRIEND_MEETS), NeighborTier::Friend);
        assert_eq!(tier_from_meets(FRIEND_MEETS + 100), NeighborTier::Friend);
    }

    #[test]
    fn key_is_symmetric() {
        let mut b = ResidentBonds::new();
        b.record_meeting("resident_2", "resident_5");
        // 反序查詢應拿到同一筆
        assert_eq!(b.meets_between("resident_5", "resident_2"), 1);
        assert_eq!(b.tier_of("resident_5", "resident_2"), NeighborTier::Stranger);
        // 反序再記一次仍累加到同一筆，不另開
        b.record_meeting("resident_5", "resident_2");
        assert_eq!(b.meets_between("resident_2", "resident_5"), 2);
        assert_eq!(b.pair_count(), 1);
    }

    #[test]
    fn meets_saturate_at_cap() {
        let mut b = ResidentBonds::new();
        for _ in 0..50 {
            b.record_meeting("resident_0", "resident_1");
        }
        assert_eq!(b.meets_between("resident_0", "resident_1"), MEETS_CAP);
        assert_eq!(b.tier_of("resident_0", "resident_1"), NeighborTier::Friend);
    }

    #[test]
    fn forget_clears_all_entries_for_id() {
        let mut b = ResidentBonds::new();
        b.record_meeting("resident_0", "resident_1");
        b.record_meeting("resident_0", "resident_2");
        b.record_meeting("resident_1", "resident_2");
        assert_eq!(b.pair_count(), 3);
        // resident_0 退休：兩條含它的條目該清掉，只留 (1,2)
        b.forget("resident_0");
        assert_eq!(b.pair_count(), 1);
        assert_eq!(b.tier_of("resident_0", "resident_1"), NeighborTier::Stranger);
        assert_eq!(b.meets_between("resident_1", "resident_2"), 1);
    }

    #[test]
    fn self_pair_is_ignored() {
        let mut b = ResidentBonds::new();
        assert_eq!(b.record_meeting("resident_3", "resident_3"), NeighborTier::Stranger);
        assert_eq!(b.pair_count(), 0);
    }
}
