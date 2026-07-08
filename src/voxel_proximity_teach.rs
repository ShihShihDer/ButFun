//! 乙太方界·就地指導 v1（自主提案切片）。
//!
//! **真缺口**：717（`voxel_teach`）讓老朋友「登門到訪」時偶爾隨機教一手已學會的技能，
//! 但線上日誌顯示技能發明（716～867 真進化系列）更常見的一幕從沒被接住——某位居民
//! 對著「熔爐」「水井藍圖」這類目標反覆想不出辦法、進入退避冷卻（`invent_backoff`）；
//! 而她身邊，可能正好站著一位老朋友，早就自己發明過同一樣東西——「答案就在旁邊」
//! 卻從未被指出來，兩人只是各忙各的閒晃、擦身而過。717 教的是「隨便一樣她會你不會
//! 的」，本刀教的是「正是你此刻卡住的那一樣」，第一次讓「本事就在身邊」這件事真的
//! 改變了正在發生的困境，而不必等到下次登門到訪才有機會補上。
//!
//! **與 717 的區隔（同精神、不同觸發與標的，非同軸重複換皮）**：717 靠「登門到訪」這個
//! 離散事件觸發、隨機挑一樣可教的技能；本刀靠「平常閒晃時剛好站得夠近」這個持續狀態
//! 觸發、精準挑學生正卡關的那個目標材料。教學台詞/記憶/Feed 沿用既有 `voxel_teach`
//! （同一件事、不同起因，沒必要另造一套詞）；獨立冷卻鍵，兩套教學互不干擾。
//!
//! 純邏輯層：本模組只有「是否該就地指導」的判定，零 IO、零鎖、零 LLM、零 async，
//! 確定性可測。鎖／位置與技能庫查詢／實際執行全在 `voxel_ws.rs`（短鎖即釋、循序
//! 不巢狀，比照 `maybe_pet_admire` 慣例）。

use crate::voxel_bonds::BondTier;

/// 判定「近旁」的半徑（方塊距離，居民↔居民）：比 717 到訪的「同一個屋簷下」更寬鬆，
/// 只要平常閒晃時剛好站得夠近就算，與 `voxel_pet_admire::PET_ADMIRE_RADIUS` 同量級。
pub const PROXIMITY_TEACH_RADIUS: f32 = 6.0;

/// 同一位學生的冷卻（秒）：教過一次後這麼久內不會再被就地指導觸發，避免同一組人
/// 天天黏在一起被連續灌好幾樣技能；冷卻期內仍可正常被 717 登門教學命中
/// （兩套冷卻各自獨立、互不相干）。
pub const PROXIMITY_TEACH_COOLDOWN_SECS: u64 = 240;

/// 是否該觸發就地指導（純函式）：雙方是老朋友 ＋ 站得夠近 ＋ 學生冷卻已過。
/// 「老師真的會、學生真的卡在這個目標」由呼叫端另外查技能庫／退避表決定，
/// 不在本函式職責內（本函式只把「該不該湊過去教」這道關）。
pub fn teach_triggers(tier: BondTier, dist_sq: f32, cooldown_ok: bool) -> bool {
    tier == BondTier::Friend
        && dist_sq <= PROXIMITY_TEACH_RADIUS * PROXIMITY_TEACH_RADIUS
        && cooldown_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teach_triggers_requires_friend_tier() {
        assert!(!teach_triggers(BondTier::Stranger, 0.0, true));
        assert!(!teach_triggers(BondTier::Acquaintance, 0.0, true));
        assert!(teach_triggers(BondTier::Friend, 0.0, true));
    }

    #[test]
    fn teach_triggers_respects_radius() {
        let r = PROXIMITY_TEACH_RADIUS;
        assert!(teach_triggers(BondTier::Friend, r * r - 0.01, true));
        assert!(!teach_triggers(BondTier::Friend, r * r + 0.01, true));
    }

    #[test]
    fn teach_triggers_exactly_at_radius_boundary_counts_as_near() {
        let r = PROXIMITY_TEACH_RADIUS;
        assert!(teach_triggers(BondTier::Friend, r * r, true), "恰好在半徑上應算近旁（<=）");
    }

    #[test]
    fn teach_triggers_respects_cooldown() {
        assert!(!teach_triggers(BondTier::Friend, 0.0, false));
    }
}
