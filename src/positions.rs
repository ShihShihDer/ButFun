//! 玩家最後狀態（位置 + 乙太）的伺服器端記憶（Phase 0-E 的記憶體前置）。
//!
//! 目前存在記憶體：同一帳號（已登入）重連時回到離線前的位置、並保有收成的乙太，
//! 而不是被重設到地圖中央、乙太歸零。這層刻意做成可抽換點——之後接 Postgres 時，
//! 把這個 store 換成 `PgStore`（同樣的 recall / remember 介面）即可，不用動
//! WebSocket / 遊戲迴圈。跨伺服器重啟的持久化仍待 0-E（記憶體版重啟會清空）。
//!
//! 注意：只記「已登入」玩家（穩定 id）；訪客每次連線 id 隨機、記了也對不上，
//! 故不記，避免 map 無界成長。

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::{WORLD_HEIGHT, WORLD_WIDTH};

/// 跨重啟持久化檔（執行期產生、已 gitignore）。記憶體前置寫穿到磁碟,讓玩家位置 +
/// 乙太撐過 server 重啟（換版）。之後接 0-E Postgres 時整層 swap 掉即可。
const STORE_PATH: &str = "data/positions.jsonl";

/// 磁碟一行紀錄：把 id 與 `Saved` 合起來序列化。
#[derive(Serialize, Deserialize)]
struct DiskRow {
    id: Uuid,
    x: f32,
    y: f32,
    ether: u32,
}

/// 玩家進場時的預設位置（地圖中央）。沒有歷史位置時用它。
pub fn default_spawn() -> (f32, f32) {
    (WORLD_WIDTH / 2.0, WORLD_HEIGHT / 2.0)
}

/// 依「是否有記住的歷史位置」決定進場座標。純函式，便於測試。
///
/// 契約：回傳的座標一定有限且落在世界範圍內。記憶體版的歷史位置來自
/// `Player::step` 已夾過的座標，本就合法；但這層刻意防住「載入被竄改/壞掉的
/// 持久化位置」——接 0-E 的 Postgres float 欄位可能存進 `NaN`/`Inf`/界外值，
/// 不檢查就會把玩家生在地圖外、或讓座標變非有限。非有限一律退回地圖中央，
/// 界外則夾回邊界（延續 `cell_at`/`from_tiles` 的持久化載入防線脈絡）。
pub fn spawn_at(recalled: Option<(f32, f32)>) -> (f32, f32) {
    match recalled {
        Some((x, y)) if x.is_finite() && y.is_finite() => {
            (x.clamp(0.0, WORLD_WIDTH), y.clamp(0.0, WORLD_HEIGHT))
        }
        _ => default_spawn(),
    }
}

/// 某玩家離線時記下的最後狀態：位置 + 收成累積的乙太。
///
/// 衍生 serde 作為玩家狀態持久化的格式地基（接 0-E）：`Saved` 是 0-E 要跨重啟存回的
/// 玩家狀態本體，沿用本 repo 既有的 jsonl 持久化路數（`users.jsonl` / `suggestions.jsonl`）
/// 時得逐筆序列化。延續 `Field` / `Crop` / `DayNight` 都在接 0-E 前先補上序列化格式的
/// 前置慣例——補齊「每個存檔又重載的結構都可序列化」這組地基的最後一塊。
///
/// 載入時的防線沿用既有入口、不在此重複：位置一律經 `spawn_at` 驗證（非有限退回地圖中央、
/// 界外夾回邊界，比照 `Field` 用 `from_tiles` 當載入閘門），`ether` 是 `u32`、型別本身就
/// 擋掉 `NaN` / `Inf` / 負值，故衍生 `Deserialize` 不會把壞值原樣放行到世界裡。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub x: f32,
    pub y: f32,
    pub ether: u32,
}

/// 記住玩家最後狀態的儲存層。記憶體 + 寫穿磁碟撐過重啟；之後可 swap 成 Postgres。
#[derive(Clone)]
pub struct PositionStore {
    inner: Arc<RwLock<HashMap<Uuid, Saved>>>,
    /// 持久化檔路徑。`None` = 純記憶體（測試用,不碰磁碟）。
    path: Option<&'static str>,
}

impl Default for PositionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(load_from_disk(STORE_PATH))),
            path: Some(STORE_PATH),
        }
    }

    /// 純記憶體版（測試用）：不載入、不寫磁碟。
    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            path: None,
        }
    }

    /// 取出某玩家上次離線時的狀態（沒有就 None）。
    pub fn recall(&self, id: Uuid) -> Option<Saved> {
        self.inner.read().unwrap().get(&id).copied()
    }

    /// 記住某玩家目前狀態（離線時呼叫）並寫穿到磁碟,撐過 server 重啟。
    pub fn remember(&self, id: Uuid, x: f32, y: f32, ether: u32) {
        self.inner.write().unwrap().insert(id, Saved { x, y, ether });
        self.persist();
    }

    /// 批次記住多名玩家（給遊戲迴圈定期快照線上玩家用）：更新一次、只寫一次磁碟。
    pub fn remember_all<I: IntoIterator<Item = (Uuid, f32, f32, u32)>>(&self, items: I) {
        {
            let mut m = self.inner.write().unwrap();
            for (id, x, y, ether) in items {
                m.insert(id, Saved { x, y, ether });
            }
        }
        self.persist();
    }

    /// 把整份 map 快照寫到磁碟（整檔覆寫,非 append,避免無界成長）。純記憶體版(path=None)不寫。
    fn persist(&self) {
        let Some(path) = self.path else {
            return;
        };
        let rows: Vec<String> = {
            let m = self.inner.read().unwrap();
            m.iter()
                .filter_map(|(id, s)| {
                    serde_json::to_string(&DiskRow {
                        id: *id,
                        x: s.x,
                        y: s.y,
                        ether: s.ether,
                    })
                    .ok()
                })
                .collect()
        };
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // 先寫暫存再 rename,避免寫到一半被重啟而毀檔。
        let tmp = format!("{path}.tmp");
        if let Ok(mut f) = std::fs::File::create(&tmp) {
            let _ = f.write_all(rows.join("\n").as_bytes());
            let _ = f.write_all(b"\n");
            let _ = f.sync_all();
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

fn load_from_disk(path: &str) -> HashMap<Uuid, Saved> {
    let mut map = HashMap::new();
    if let Ok(contents) = std::fs::read_to_string(path) {
        for line in contents.lines() {
            if let Ok(r) = serde_json::from_str::<DiskRow>(line) {
                // 位置經 spawn_at 驗證（壞值退回中央/夾邊界）;ether 型別本身擋壞值。
                let (x, y) = spawn_at(Some((r.x, r.y)));
                map.insert(r.id, Saved { x, y, ether: r.ether });
            }
        }
    }
    map
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
    fn spawn_falls_back_to_center_on_non_finite() {
        // 壞掉的持久化座標（NaN/Inf）不該把玩家生到非有限位置。
        assert_eq!(spawn_at(Some((f32::NAN, 100.0))), default_spawn());
        assert_eq!(spawn_at(Some((100.0, f32::INFINITY))), default_spawn());
        assert_eq!(spawn_at(Some((f32::NEG_INFINITY, f32::NAN))), default_spawn());
    }

    #[test]
    fn spawn_clamps_out_of_bounds_into_world() {
        // 界外的歷史位置夾回世界邊界，而不是把玩家生在地圖外。
        assert_eq!(spawn_at(Some((-50.0, -50.0))), (0.0, 0.0));
        assert_eq!(
            spawn_at(Some((WORLD_WIDTH + 999.0, WORLD_HEIGHT + 999.0))),
            (WORLD_WIDTH, WORLD_HEIGHT)
        );
    }

    #[test]
    fn recall_is_none_before_remember() {
        let store = PositionStore::in_memory();
        assert_eq!(store.recall(Uuid::new_v4()), None);
    }

    #[test]
    fn remember_then_recall_round_trips() {
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, 10.0, 20.0, 5);
        assert_eq!(
            store.recall(id),
            Some(Saved {
                x: 10.0,
                y: 20.0,
                ether: 5
            })
        );
    }

    #[test]
    fn remember_overwrites_previous_state() {
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, 10.0, 20.0, 1);
        store.remember(id, 30.0, 40.0, 9);
        assert_eq!(
            store.recall(id),
            Some(Saved {
                x: 30.0,
                y: 40.0,
                ether: 9
            })
        );
    }

    #[test]
    fn recalled_ether_survives_round_trip() {
        // 收成的乙太要能跟著重連回來，不被歸零。
        let store = PositionStore::in_memory();
        let id = Uuid::new_v4();
        store.remember(id, 0.0, 0.0, 42);
        assert_eq!(store.recall(id).map(|s| s.ether), Some(42));
    }

    #[test]
    fn saved_round_trips_through_serde() {
        // 持久化格式地基：玩家最後狀態序列化再讀回要一模一樣（接 0-E 跨重啟接續）。
        let s = Saved {
            x: 123.5,
            y: 678.25,
            ether: 7,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Saved = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn loaded_bad_position_still_gated_by_spawn_at() {
        // 即使磁碟上被竄改成非有限 / 界外座標的 Saved 載入進來，進場仍一律經 spawn_at
        // 驗證、不會把玩家生到非有限或界外位置（位置的載入閘門是 spawn_at，
        // 比照 Field 的 from_tiles）。ether 是 u32，型別本身就擋掉壞值。
        let bad = Saved {
            x: f32::INFINITY,
            y: WORLD_HEIGHT + 9999.0,
            ether: 1,
        };
        let (x, y) = spawn_at(Some((bad.x, bad.y)));
        assert!(x.is_finite() && y.is_finite());
        assert!((0.0..=WORLD_WIDTH).contains(&x) && (0.0..=WORLD_HEIGHT).contains(&y));
    }

    #[test]
    fn stores_are_independent_per_player() {
        let store = PositionStore::in_memory();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        store.remember(a, 1.0, 1.0, 3);
        assert_eq!(store.recall(b), None);
        assert_eq!(
            store.recall(a),
            Some(Saved {
                x: 1.0,
                y: 1.0,
                ether: 3
            })
        );
    }
}
