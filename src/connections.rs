//! 每名玩家當前的「在線連線數」計數。
//!
//! 為什麼需要：已登入玩家的 `player.id == user.id`，所以同一帳號開兩個分頁／兩台裝置
//! 時，兩條 WebSocket 連線會共用同一個玩家 id。GDD 明確說「同 Google 帳號跨裝置／重連
//! 即同玩家」——兩條連線就該共用同一個權威狀態。但若不計數，先離線的那條 `cleanup` 會
//! 把這個共用 id 整個從玩家清單移除，讓另一條還連著的 session 從世界憑空消失（不再進
//! 快照、輸入被靜默丟棄）——正是傷到那個「跨裝置同玩家」招牌功能的體驗 bug。
//!
//! 解法：替每個玩家 id 記在線連線數。第一條連線進場才建立玩家（沿用記憶位置）；之後
//! 同帳號的連線只增加計數、共用既有權威狀態（不用舊存檔覆蓋當前位置，避免畫面瞬移）；
//! 最後一條離線（計數歸零）才真正移除玩家、記下位置、廣播 `PlayerLeft`。
//!
//! 刻意做成可測的小 store（比照 `positions.rs` 的模式）；計數增減的判斷抽成 `acquire`／
//! `release` 兩個純粹方法，便於單元測試。呼叫端在持有 `players` 寫鎖時呼叫，藉「先 players
//! 再 conns」的固定鎖序避免死鎖。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use uuid::Uuid;

/// 記錄每個玩家 id 當前的在線連線數。MVP：記憶體（連線狀態本就無需持久化）。
#[derive(Clone, Default)]
pub struct ConnectionCounts {
    inner: Arc<RwLock<HashMap<Uuid, u32>>>,
}

impl ConnectionCounts {
    pub fn new() -> Self {
        Self::default()
    }

    /// 某玩家 id 多了一條連線。回傳 `true` 代表這是該玩家的**第一條**連線
    /// （呼叫端據此決定要不要建立新玩家、從記憶位置進場）。
    pub fn acquire(&self, id: Uuid) -> bool {
        let mut counts = self.inner.write().unwrap();
        let c = counts.entry(id).or_insert(0);
        let first = *c == 0;
        *c += 1;
        first
    }

    /// 某玩家 id 少了一條連線。回傳 `true` 代表這是該玩家的**最後一條**連線
    /// （呼叫端據此決定要不要移除玩家、記下位置、廣播離線）。計數歸零後刪掉該鍵，
    /// 避免 map 無界成長。沒有計數紀錄（理論上不會發生）一律當作最後一條，保守地移除。
    pub fn release(&self, id: Uuid) -> bool {
        let mut counts = self.inner.write().unwrap();
        match counts.get_mut(&id) {
            Some(c) => {
                *c -= 1;
                if *c == 0 {
                    counts.remove(&id);
                    true
                } else {
                    false
                }
            }
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_acquire_reports_first_then_not() {
        let conns = ConnectionCounts::new();
        let id = Uuid::new_v4();
        assert!(conns.acquire(id), "第一條連線應回報 first=true");
        assert!(!conns.acquire(id), "同 id 第二條連線不是 first");
        assert!(!conns.acquire(id), "第三條也不是");
    }

    #[test]
    fn release_reports_last_only_when_count_hits_zero() {
        let conns = ConnectionCounts::new();
        let id = Uuid::new_v4();
        conns.acquire(id); // 1
        conns.acquire(id); // 2
        assert!(!conns.release(id), "還剩一條，不是最後一條");
        assert!(conns.release(id), "計數歸零，這是最後一條");
    }

    #[test]
    fn single_connection_acquire_then_release_is_last() {
        // 訪客／單一連線的常見路徑：進場是第一條，離線即最後一條。
        let conns = ConnectionCounts::new();
        let id = Uuid::new_v4();
        assert!(conns.acquire(id));
        assert!(conns.release(id));
    }

    #[test]
    fn release_without_record_is_treated_as_last() {
        // 防呆：沒有對應計數時保守當作最後一條（呼叫端會照樣移除玩家）。
        let conns = ConnectionCounts::new();
        assert!(conns.release(Uuid::new_v4()));
    }

    #[test]
    fn counts_are_independent_per_player() {
        let conns = ConnectionCounts::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        conns.acquire(a);
        assert!(conns.acquire(b), "b 的第一條連線不受 a 影響");
        assert!(conns.release(a), "a 只有一條，release 即最後一條");
        assert!(conns.release(b), "b 同理");
    }

    #[test]
    fn id_can_be_reused_after_full_release() {
        // 同帳號完全離線後再重連：計數鍵已刪，下次 acquire 又是 first（從記憶位置重新進場）。
        let conns = ConnectionCounts::new();
        let id = Uuid::new_v4();
        assert!(conns.acquire(id));
        assert!(conns.release(id));
        assert!(conns.acquire(id), "完全離線後重連又是第一條");
    }
}
