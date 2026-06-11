//! 灑水器系統（ROADMAP 112）。
//!
//! 純邏輯層：灑水器資料結構 + 放置 / 移除 / 定時澆灌邏輯。
//! 持久化層：`SprinklerPersist` 有 `new()` (記憶體) / `from_pool(PgPool)` (Postgres) 兩態，
//! 沿用 tile_store / field_store 的三態抽換結構——加灑水器時 INSERT、啟動時 SELECT 全載回。
//!
//! 玩家合成「灑水器」後可放置在農地範圍內（FARM_REACH 內）；每隔 SPRINKLER_INTERVAL
//! 秒自動對周圍 SPRINKLER_RADIUS 像素內的作物格呼叫 `Field::water(col, row)`，
//! 省去手動逐格澆水的重複勞動。
//!
//! 與「下雨澆田（ROADMAP 109）」共用同一套 `Field::water` 介面——下雨時對全部格
//! 呼叫水，灑水器則依半徑選格——同一套澆水邏輯，兩條路徑。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::field::{Field, TILE_SIZE};

/// 灑水器觸發間隔（秒）：每隔這麼久自動澆一次。
pub const SPRINKLER_INTERVAL: f32 = 30.0;
/// 灑水器半徑（像素）：以放置點為圓心，圓內的作物格都會被澆到。
/// 2 格半徑 = 2 × TILE_SIZE = 96 像素；1 個灑水器放在 6×4 農地正中央可覆蓋大部分格子。
pub const SPRINKLER_RADIUS: f32 = TILE_SIZE * 2.0;

/// 一個放置好的灑水器。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprinklerData {
    /// 在 DB 持久化後拿到的自增 ID（0 表示尚未落地）。
    pub db_id: i64,
    /// 放置點的世界座標（像素）。
    pub wx: f32,
    pub wy: f32,
    /// 距下次自動澆水還剩幾秒。
    #[serde(skip)]
    pub cooldown: f32,
}

impl SprinklerData {
    /// 建新灑水器（db_id = 0，cooldown 預設滿間隔——放好就開始倒數）。
    pub fn new(wx: f32, wy: f32) -> Self {
        Self {
            db_id: 0,
            wx,
            wy,
            cooldown: SPRINKLER_INTERVAL,
        }
    }

    /// 推進 `dt` 秒；若倒數到 0 則澆灌 `field` 內半徑內的所有作物格，並重置倒數。
    /// 回傳「這次 tick 澆了幾格」（給呼叫端決定要不要廣播）。
    pub fn tick(&mut self, dt: f32, field: &mut Field) -> u32 {
        self.cooldown -= dt;
        if self.cooldown > 0.0 {
            return 0;
        }
        self.cooldown = SPRINKLER_INTERVAL;
        self.water_nearby(field)
    }

    /// 對 `field` 內所有在半徑內的作物格澆水，回傳澆到的格數。
    pub fn water_nearby(&self, field: &mut Field) -> u32 {
        let (ox, oy) = field.origin();
        let cols = crate::field::FIELD_COLS;
        let rows = field.rows();
        let mut count = 0u32;
        for row in 0..rows {
            for col in 0..cols {
                // 格中心世界座標。
                let cx = ox + col as f32 * TILE_SIZE + TILE_SIZE * 0.5;
                let cy = oy + row as f32 * TILE_SIZE + TILE_SIZE * 0.5;
                let dx = cx - self.wx;
                let dy = cy - self.wy;
                if dx * dx + dy * dy <= SPRINKLER_RADIUS * SPRINKLER_RADIUS {
                    if field.water(col, row) {
                        count += 1;
                    }
                }
            }
        }
        count
    }
}

/// 全伺服器所有灑水器的記憶體快照。一個玩家可有多個灑水器，用 user_id → Vec 索引。
#[derive(Default)]
pub struct SprinklerStore {
    by_owner: HashMap<Uuid, Vec<SprinklerData>>,
}

impl SprinklerStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 為 `owner` 新增一個灑水器，回傳它在 Vec 裡的 index。
    pub fn add(&mut self, owner: Uuid, data: SprinklerData) -> usize {
        let vec = self.by_owner.entry(owner).or_default();
        vec.push(data);
        vec.len() - 1
    }

    /// 移除 `owner` 的 db_id 符合的灑水器（用於未來「撿起」功能；目前未接線）。
    #[allow(dead_code)]
    pub fn remove(&mut self, owner: Uuid, db_id: i64) -> bool {
        if let Some(vec) = self.by_owner.get_mut(&owner) {
            let before = vec.len();
            vec.retain(|s| s.db_id != db_id);
            return vec.len() < before;
        }
        false
    }

    /// 啟動時從 DB 載入：把 `(owner, SprinklerData)` 批次插入（db_id 已填好）。
    pub fn load(&mut self, rows: Vec<(Uuid, SprinklerData)>) {
        for (owner, data) in rows {
            self.by_owner.entry(owner).or_default().push(data);
        }
    }

    /// 所有灑水器（給遊戲迴圈 tick 用）：回傳 `&mut (owner_id, SprinklerData)` 的可變參照。
    pub fn all_mut(&mut self) -> impl Iterator<Item = (&Uuid, &mut Vec<SprinklerData>)> {
        self.by_owner.iter_mut()
    }

    /// 把 db_id==0 且位置匹配的最後一個灑水器更新為真實 db_id（非同步 INSERT 回填用）。
    pub fn update_db_id(&mut self, owner: Uuid, wx: f32, wy: f32, db_id: i64) {
        if let Some(vec) = self.by_owner.get_mut(&owner) {
            for s in vec.iter_mut().rev() {
                if s.db_id == 0 && (s.wx - wx).abs() < 0.1 && (s.wy - wy).abs() < 0.1 {
                    s.db_id = db_id;
                    break;
                }
            }
        }
    }

    /// 給快照廣播用：列出所有灑水器的位置與歸屬。
    pub fn views(&self) -> Vec<SprinklerView> {
        self.by_owner
            .iter()
            .flat_map(|(owner, vec)| {
                vec.iter().map(|s| SprinklerView {
                    owner: *owner,
                    wx: s.wx,
                    wy: s.wy,
                })
            })
            .collect()
    }
}

/// 給前端的灑水器快照（輕量，不含 cooldown）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SprinklerView {
    pub owner: Uuid,
    pub wx: f32,
    pub wy: f32,
}

// ── 持久化層 ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
enum PersistBackend {
    Memory,
    Postgres(PgPool),
}

/// 灑水器持久化 store（同 tile_store 模式）。
/// `Memory`：無 DB 時，放置記憶體、重啟歸零。
/// `Postgres`：有 DATABASE_URL 時，INSERT 落地、啟動 SELECT 全載回。
#[derive(Clone)]
pub struct SprinklerPersist {
    backend: PersistBackend,
}

impl Default for SprinklerPersist {
    fn default() -> Self {
        Self::new()
    }
}

impl SprinklerPersist {
    /// 記憶體模式（無 DB / 測試）。
    pub fn new() -> Self {
        Self { backend: PersistBackend::Memory }
    }

    /// Postgres 模式：啟動時把 sprinklers 全部載回，回傳 (owner, SprinklerData)。
    pub async fn from_pool(pool: PgPool) -> (Self, Vec<(Uuid, SprinklerData)>) {
        let rows = match sqlx::query("SELECT id, user_id, wx, wy FROM sprinklers")
            .fetch_all(&pool)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("sprinklers 載入失敗，以空灑水器起動：{e}");
                vec![]
            }
        };
        let entries: Vec<(Uuid, SprinklerData)> = rows
            .into_iter()
            .filter_map(|row| {
                use sqlx::Row;
                let db_id: i64 = row.try_get("id").ok()?;
                let user_id: Uuid = row.try_get("user_id").ok()?;
                let wx: f64 = row.try_get("wx").ok()?;
                let wy: f64 = row.try_get("wy").ok()?;
                let mut d = SprinklerData::new(wx as f32, wy as f32);
                d.db_id = db_id;
                Some((user_id, d))
            })
            .collect();
        (Self { backend: PersistBackend::Postgres(pool) }, entries)
    }

    /// 非同步 INSERT；回傳分配到的 DB id（Memory 模式回 0）。
    pub async fn insert(&self, user_id: Uuid, wx: f32, wy: f32) -> i64 {
        match &self.backend {
            PersistBackend::Memory => 0,
            PersistBackend::Postgres(pool) => {
                let result: Result<i64, _> = sqlx::query_scalar(
                    "INSERT INTO sprinklers (user_id, wx, wy) VALUES ($1, $2, $3) RETURNING id",
                )
                .bind(user_id)
                .bind(wx as f64)
                .bind(wy as f64)
                .fetch_one(pool)
                .await;
                result.unwrap_or_else(|e| {
                    tracing::warn!("sprinkler INSERT 失敗：{e}");
                    0
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::{Field, FIELD_COLS, FIELD_ROWS, FIELD_ORIGIN_X, FIELD_ORIGIN_Y};

    fn center_of_field() -> (f32, f32) {
        let cx = FIELD_ORIGIN_X + (FIELD_COLS as f32 / 2.0) * TILE_SIZE;
        let cy = FIELD_ORIGIN_Y + (FIELD_ROWS as f32 / 2.0) * TILE_SIZE;
        (cx, cy)
    }

    #[test]
    fn new_sprinkler_has_full_cooldown() {
        let s = SprinklerData::new(0.0, 0.0);
        assert!((s.cooldown - SPRINKLER_INTERVAL).abs() < 0.001);
    }

    #[test]
    fn tick_does_not_water_before_interval() {
        let mut field = Field::new();
        field.till(0, 0);
        field.plant(0, 0);
        let (cx, cy) = center_of_field();
        let mut s = SprinklerData::new(cx, cy);
        // 不到間隔，不澆。
        let watered = s.tick(SPRINKLER_INTERVAL * 0.5, &mut field);
        assert_eq!(watered, 0);
    }

    #[test]
    fn tick_waters_nearby_crops_after_interval() {
        let mut field = Field::new();
        // 在農地中央種幾格（確保在半徑 2 格內）。
        field.till(2, 1); field.plant(2, 1);
        field.till(3, 1); field.plant(3, 1);
        field.till(2, 2); field.plant(2, 2);
        let (cx, cy) = center_of_field();
        let mut s = SprinklerData::new(cx, cy);
        // 快進到間隔觸發。
        let watered = s.tick(SPRINKLER_INTERVAL + 0.1, &mut field);
        // 圓心在地塊中央（半徑 2 格），中央格必在範圍內。
        assert!(watered >= 3, "應澆到至少 3 格作物，實際 {watered}");
    }

    #[test]
    fn tick_resets_cooldown_after_trigger() {
        let mut field = Field::new();
        let (cx, cy) = center_of_field();
        let mut s = SprinklerData::new(cx, cy);
        s.tick(SPRINKLER_INTERVAL + 0.1, &mut field);
        // 重置後剩 SPRINKLER_INTERVAL 秒（允許浮點誤差）。
        assert!(s.cooldown > 0.0 && s.cooldown <= SPRINKLER_INTERVAL);
    }

    #[test]
    fn water_nearby_only_waters_planted_cells() {
        let mut field = Field::new();
        // 在半徑內：(2,1) 翻土但沒種；(3,1) 有種。
        field.till(2, 1);
        field.till(3, 1); field.plant(3, 1);
        let (cx, cy) = center_of_field();
        let s = SprinklerData::new(cx, cy);
        let watered = s.water_nearby(&mut field);
        // 未種的格 water() 回 false，不計入。
        assert_eq!(watered, 1);
    }

    #[test]
    fn sprinkler_outside_radius_does_not_water() {
        let mut field = Field::new();
        field.till(0, 0); field.plant(0, 0);
        // 灑水器放在距離農地很遠的地方。
        let far_x = FIELD_ORIGIN_X + 9999.0;
        let far_y = FIELD_ORIGIN_Y + 9999.0;
        let s = SprinklerData::new(far_x, far_y);
        let mut f2 = field.clone();
        let watered = s.water_nearby(&mut f2);
        assert_eq!(watered, 0);
    }
}
