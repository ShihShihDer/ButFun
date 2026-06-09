//! 地形格差異的持久化層（C-1：啟動時載入差異；C-2 起加寫入路徑）。
//!
//! 設計沿用其他 store 的抽換結構：
//!   - `Memory`：測試用，不碰 DB / 磁碟。
//!   - `Postgres`：設了 `DATABASE_URL` 時走 `tile_deltas` 表。
//!
//! C-1 只讀：`loaded_deltas()` 供啟動時種回 `TileWorld`。
//! C-2 起才用：`upsert_delta()`、`delete_delta()`。

use std::collections::HashMap;

use sqlx::postgres::PgPool;
use sqlx::Row;

use world_core::TileKind;

fn kind_str(k: TileKind) -> &'static str {
    match k {
        TileKind::Empty    => "empty",
        TileKind::Dirt     => "dirt",
        TileKind::Stone    => "stone",
        TileKind::Ore      => "ore",
        TileKind::Crystal     => "crystal",
        TileKind::Mushroom    => "mushroom",
        TileKind::AncientRuin => "ancient_ruin",
        TileKind::CoralReef   => "coral_reef",
        TileKind::WildFlower  => "wild_flower",
        TileKind::JadeVine    => "jade_vine",
    }
}

fn parse_kind(s: &str) -> TileKind {
    match s {
        "dirt"         => TileKind::Dirt,
        "stone"        => TileKind::Stone,
        "ore"          => TileKind::Ore,
        "crystal"      => TileKind::Crystal,
        "mushroom"     => TileKind::Mushroom,
        "ancient_ruin" => TileKind::AncientRuin,
        "coral_reef"   => TileKind::CoralReef,
        "wild_flower"  => TileKind::WildFlower,
        "jade_vine"    => TileKind::JadeVine,
        _              => TileKind::Empty,
    }
}

#[derive(Clone)]
enum Backend {
    #[cfg_attr(not(test), allow(dead_code))]
    Memory,
    Postgres(PgPool),
}

/// 地形格差異的持久化 store。
#[derive(Clone)]
pub struct TileStore {
    backend: Backend,
    /// 啟動時從 DB 載入的差異，供 TileWorld 種回。
    loaded: HashMap<(i32, i32, u8, u8), TileKind>,
}

impl Default for TileStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TileStore {
    /// 記憶體模式（無 DB / 測試）：差異從空白開始。
    pub fn new() -> Self {
        Self { backend: Backend::Memory, loaded: HashMap::new() }
    }

    /// Postgres 模式：啟動時把 tile_deltas 全部載回。
    pub async fn from_pool(pool: PgPool) -> Self {
        let loaded = match load_from_db(&pool).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("tile_deltas 載入失敗，以空差異起動（C-1 無玩家修改,影響極小）：{e}");
                HashMap::new()
            }
        };
        Self { backend: Backend::Postgres(pool), loaded }
    }

    /// 啟動時載入的差異，供 `TileWorld::with_deltas` 種回。
    pub fn loaded_deltas(&self) -> HashMap<(i32, i32, u8, u8), TileKind> {
        self.loaded.clone()
    }

    /// C-2 起使用：把一格差異 upsert 到 DB（非同步、失敗只記 log）。
    #[allow(dead_code)]
    pub async fn upsert_delta(&self, cx: i32, cy: i32, tx: u8, ty: u8, kind: TileKind) {
        let Backend::Postgres(pool) = &self.backend else { return; };
        if let Err(e) = sqlx::query(
            "INSERT INTO tile_deltas (chunk_cx, chunk_cy, cell_x, cell_y, kind) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (chunk_cx, chunk_cy, cell_x, cell_y) \
             DO UPDATE SET kind = EXCLUDED.kind",
        )
        .bind(cx)
        .bind(cy)
        .bind(tx as i16)
        .bind(ty as i16)
        .bind(kind_str(kind))
        .execute(pool)
        .await
        {
            tracing::warn!("tile delta upsert ({cx},{cy},{tx},{ty}) 失敗：{e}");
        }
    }

    /// C-2 起使用：刪除一格差異（挖空後回到預設生成值時呼叫）。
    #[allow(dead_code)]
    pub async fn delete_delta(&self, cx: i32, cy: i32, tx: u8, ty: u8) {
        let Backend::Postgres(pool) = &self.backend else { return; };
        if let Err(e) = sqlx::query(
            "DELETE FROM tile_deltas WHERE chunk_cx=$1 AND chunk_cy=$2 AND cell_x=$3 AND cell_y=$4",
        )
        .bind(cx)
        .bind(cy)
        .bind(tx as i16)
        .bind(ty as i16)
        .execute(pool)
        .await
        {
            tracing::warn!("tile delta delete ({cx},{cy},{tx},{ty}) 失敗：{e}");
        }
    }
}

async fn load_from_db(
    pool: &PgPool,
) -> Result<HashMap<(i32, i32, u8, u8), TileKind>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT chunk_cx, chunk_cy, cell_x, cell_y, kind FROM tile_deltas",
    )
    .fetch_all(pool)
    .await?;

    let mut map = HashMap::new();
    for row in rows {
        let cx: i32 = row.get("chunk_cx");
        let cy: i32 = row.get("chunk_cy");
        let tx: i16 = row.get("cell_x");
        let ty: i16 = row.get("cell_y");
        let kind_s: String = row.get("kind");
        map.insert((cx, cy, tx as u8, ty as u8), parse_kind(&kind_s));
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_store_has_empty_deltas() {
        let store = TileStore::new();
        assert!(store.loaded_deltas().is_empty());
    }

    #[test]
    fn kind_str_and_parse_kind_round_trip() {
        for &k in &[TileKind::Empty, TileKind::Dirt, TileKind::Stone, TileKind::Ore, TileKind::Crystal, TileKind::Mushroom, TileKind::AncientRuin, TileKind::CoralReef, TileKind::WildFlower, TileKind::JadeVine] {
            assert_eq!(parse_kind(kind_str(k)), k);
        }
    }

    #[test]
    fn parse_kind_unknown_fallback_to_empty() {
        assert_eq!(parse_kind("banana"), TileKind::Empty);
        assert_eq!(parse_kind(""), TileKind::Empty);
    }
}
