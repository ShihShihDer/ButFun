//! 玩家最後位置的伺服器端記憶（Phase 0-E 的記憶體前置）。
//!
//! 目前存在記憶體：同一帳號（已登入）重連時回到離線前的位置，而不是被重設到
//! 地圖中央。這層刻意做成可抽換點——之後接 Postgres 時，把這個 store 換成
//! `PgStore`（同樣的 recall / remember 介面）即可，不用動 WebSocket / 遊戲迴圈。
//!
//! 注意：只記「已登入」玩家（穩定 id）；訪客每次連線 id 隨機、記了也對不上，
//! 故不記，避免 map 無界成長。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use uuid::Uuid;

use crate::state::{WORLD_HEIGHT, WORLD_WIDTH};

/// 玩家進場時的預設位置（地圖中央）。沒有歷史位置時用它。
pub fn default_spawn() -> (f32, f32) {
    (WORLD_WIDTH / 2.0, WORLD_HEIGHT / 2.0)
}

/// 依「是否有記住的歷史位置」決定進場座標。純函式，便於測試。
pub fn spawn_at(recalled: Option<(f32, f32)>) -> (f32, f32) {
    recalled.unwrap_or_else(default_spawn)
}

/// 記住玩家最後位置的儲存層。MVP：記憶體；之後可 swap 成 Postgres。
#[derive(Clone, Default)]
pub struct PositionStore {
    inner: Arc<RwLock<HashMap<Uuid, (f32, f32)>>>,
}

impl PositionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取出某玩家上次離線時的位置（沒有就 None）。
    pub fn recall(&self, id: Uuid) -> Option<(f32, f32)> {
        self.inner.read().unwrap().get(&id).copied()
    }

    /// 記住某玩家目前位置（離線時呼叫）。
    pub fn remember(&self, id: Uuid, x: f32, y: f32) {
        self.inner.write().unwrap().insert(id, (x, y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_falls_back_to_center_when_no_history() {
        assert_eq!(spawn_at(None), default_spawn());
    }

    #[test]
    fn spawn_uses_recalled_position() {
        assert_eq!(spawn_at(Some((123.0, 456.0))), (123.0, 456.0));
    }

    #[test]
    fn recall_is_none_before_remember() {
        let store = PositionStore::new();
        assert_eq!(store.recall(Uuid::new_v4()), None);
    }

    #[test]
    fn remember_then_recall_round_trips() {
        let store = PositionStore::new();
        let id = Uuid::new_v4();
        store.remember(id, 10.0, 20.0);
        assert_eq!(store.recall(id), Some((10.0, 20.0)));
    }

    #[test]
    fn remember_overwrites_previous_position() {
        let store = PositionStore::new();
        let id = Uuid::new_v4();
        store.remember(id, 10.0, 20.0);
        store.remember(id, 30.0, 40.0);
        assert_eq!(store.recall(id), Some((30.0, 40.0)));
    }

    #[test]
    fn stores_are_independent_per_player() {
        let store = PositionStore::new();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store.remember(a, 1.0, 1.0);
        assert_eq!(store.recall(b), None);
        assert_eq!(store.recall(a), Some((1.0, 1.0)));
    }
}
