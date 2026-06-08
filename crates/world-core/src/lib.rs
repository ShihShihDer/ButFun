//! ButFun 世界核心(與渲染無關的確定性世界邏輯)。
//!
//! 設計:世界的「真相」放這裡,**同時編進伺服器(原生 lib)與前端(wasm32 cdylib)**,
//! 前後端跑同一份碼 → 逐位元一致(對齊問題消失)、且與渲染無關(2D 現在、3D/AR/VR 未來
//! 都讀同一套世界邏輯,只換渲染器)。
//!
//! 第一塊:程序生成生態域 `biome_at`。刻意用 **f64 + i32/u32 整數運算**,逐位元對齊前端
//! `web/game.js` 既有的 `grassHash`/`biomeNoise`/`biomeAt`(JS Number 即 f64、位元運算即 int32)
//! ——這樣前端改呼叫本 wasm 後,畫出的地貌與伺服器判定的地形完全相同。
//!
//! 之後:碰撞查詢、生態域內容、Delta-Save 等都往這裡長,伺服器與前端共用。

pub const CHUNK_SIZE: f32 = 512.0;

/// 一個地形格的邊長（像素）。CHUNK_SIZE / TILE_PX = 16 格 / chunk。
pub const TILE_PX: f32 = 32.0;
/// 每個 chunk 在單一軸上的格數（512 / 32 = 16）。
pub const TILES_PER_CHUNK: usize = 16;

/// 座標 → 區塊鍵。
pub fn chunk_key(x: f32, y: f32) -> (i32, i32) {
    (
        (x / CHUNK_SIZE).floor() as i32,
        (y / CHUNK_SIZE).floor() as i32,
    )
}

/// 生態域種類。`code()` 是給 wasm/前端的穩定整數編碼(別重排,前端對應顏色靠它)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    Water,
    Sand,
    Meadow,
    Forest,
    Rocky,
}

impl Biome {
    /// 穩定整數編碼(wasm 邊界用):0=water 1=sand 2=meadow 3=forest 4=rocky。
    pub fn code(self) -> u32 {
        match self {
            Biome::Water => 0,
            Biome::Sand => 1,
            Biome::Meadow => 2,
            Biome::Forest => 3,
            Biome::Rocky => 4,
        }
    }
}

/// 確定性整數雜湊 → [0,1) 的 f64。逐位元對齊 JS `grassHash`:
/// `Math.imul` = i32 wrapping 乘;`>>>` = u32 邏輯右移;`|0`/`^` = i32;最後 `>>>0 / 2^32`。
fn grass_hash(ix: i32, iy: i32) -> f64 {
    let h0 = ix
        .wrapping_mul(374_761_393)
        .wrapping_add(iy.wrapping_mul(668_265_263));
    let h1 = (h0 ^ ((h0 as u32 >> 13) as i32)).wrapping_mul(1_274_126_177);
    let hu = (h1 ^ ((h1 as u32 >> 16) as i32)) as u32;
    (hu as f64) / 4_294_967_296.0
}

/// 平滑 value noise → [0,1)。逐位元對齊 JS `biomeNoise`:格點雜湊 + smoothstep 雙線性內插。
/// 全程 f64(對齊 JS Number);種子混入比照 JS `(a|0)*1009 + seed` / `(b|0)*9176 + seed*31`。
fn value_noise(wx: f64, wy: f64, scale: f64, seed: i32) -> f64 {
    let gx = wx / scale;
    let gy = wy / scale;
    let x0 = gx.floor() as i64;
    let y0 = gy.floor() as i64;
    let fx = gx - (x0 as f64);
    let fy = gy - (y0 as f64);
    let h = |a: i64, b: i64| -> f64 {
        let ix = (a as i32).wrapping_mul(1009).wrapping_add(seed);
        let iy = (b as i32).wrapping_mul(9176).wrapping_add(seed.wrapping_mul(31));
        grass_hash(ix, iy)
    };
    let v00 = h(x0, y0);
    let v10 = h(x0 + 1, y0);
    let v01 = h(x0, y0 + 1);
    let v11 = h(x0 + 1, y0 + 1);
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let a = v00 + (v10 - v00) * sx;
    let b = v01 + (v11 - v01) * sx;
    a + (b - a) * sy
}

/// 座標 → 生態域。逐位元對齊 JS `biomeAt`:海拔 e(scale 1500、seed 7)、濕度 m(1200、137);
/// 門檻一致。非有限座標(NaN/Inf)時所有比較為 false → 落到 Meadow(與 JS 同行為)。
pub fn biome_at(wx: f64, wy: f64) -> Biome {
    // 非有限座標(NaN/Inf)防護:正常遊戲座標一律有限,這只擋壞輸入(避免 floor→i64 溢位)。
    // 回 Meadow 當安全預設。real 座標都有限,故不影響與 JS 的對齊。
    if !wx.is_finite() || !wy.is_finite() {
        return Biome::Meadow;
    }
    let e = value_noise(wx, wy, 1500.0, 7);
    let m = value_noise(wx, wy, 1200.0, 137);
    if e < 0.30 {
        Biome::Water
    } else if e < 0.355 {
        Biome::Sand
    } else if e > 0.76 {
        Biome::Rocky
    } else if m > 0.56 {
        Biome::Forest
    } else {
        Biome::Meadow
    }
}

/// wasm/前端入口:座標 → 生態域整數編碼(見 `Biome::code`)。純數值進出,瀏覽器可直接
/// `WebAssembly.instantiate` 後呼叫 `instance.exports.biome_code(x, y)`,免 wasm-bindgen。
///
/// # Safety
/// 純值計算、無指標、無共享狀態;`extern "C"` 僅為 wasm 匯出穩定符號。
#[no_mangle]
pub extern "C" fn biome_code(x: f64, y: f64) -> u32 {
    biome_at(x, y).code()
}

/// 地形格種類（可挖 / 可建）。
///
/// 穩定整數編碼（別重排，前端與 DB 靠它）：
///   0 = Empty（空氣/可通行）
///   1 = Dirt（泥土）
///   2 = Stone（石塊）
///   3 = Ore（礦脈）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileKind {
    Empty,
    Dirt,
    Stone,
    Ore,
}

impl TileKind {
    pub fn code(self) -> u8 {
        match self {
            TileKind::Empty => 0,
            TileKind::Dirt  => 1,
            TileKind::Stone => 2,
            TileKind::Ore   => 3,
        }
    }
}

/// 座標（世界像素）→ 確定性地形格種類。
///
/// 同 `biome_at`：同座標必定同結果、不靠亂數/時鐘。使用與生態域相同的 `grass_hash`
/// 雜湊函式，故前端可用 JS `grassHash` 精確對齊（見 `web/game.js` 的 `tileKindAt`）。
/// 水域一律回 `Empty`（水面沒有可挖的實心格）。
pub fn tile_kind_at(wx: f64, wy: f64) -> TileKind {
    let biome = biome_at(wx, wy);
    if biome == Biome::Water {
        return TileKind::Empty;
    }
    // 格索引（整數）→ 穩定雜湊值 [0,1)
    let gx = (wx / TILE_PX as f64).floor() as i32;
    let gy = (wy / TILE_PX as f64).floor() as i32;
    let h = grass_hash(
        gx.wrapping_mul(1031) ^ gy.wrapping_mul(2053),
        gx ^ gy.wrapping_mul(1009),
    );
    match biome {
        Biome::Rocky => {
            if h < 0.05 { TileKind::Ore }
            else if h < 0.40 { TileKind::Stone }
            else { TileKind::Empty }
        }
        Biome::Forest => {
            if h < 0.08 { TileKind::Stone }
            else if h < 0.22 { TileKind::Dirt }
            else { TileKind::Empty }
        }
        Biome::Meadow => {
            if h < 0.06 { TileKind::Stone }
            else if h < 0.12 { TileKind::Dirt }
            else { TileKind::Empty }
        }
        Biome::Sand => {
            if h < 0.04 { TileKind::Stone }
            else if h < 0.08 { TileKind::Dirt }
            else { TileKind::Empty }
        }
        Biome::Water => TileKind::Empty,
    }
}

/// 滑動碰撞:從 `(cur)` 朝 `(new)` 移動,遇到 `blocked` 區域沿牆滑(render-agnostic 純邏輯,
/// 供伺服器移動接線用——例如水域擋路時 `blocked = |x,y| biome_at==Water && 不在農地內`)。
/// 規則:已在 blocked 內 → 放行(讓受困者能逃脫);否則目標可走就走;不行就試只走 X、再試只走 Y;
/// 全擋就不動。(草擬 gemini-cli、Claude 審校:修了未用變數 lint。)
pub fn resolve_move<F: Fn(f32, f32) -> bool>(
    cur_x: f32,
    cur_y: f32,
    new_x: f32,
    new_y: f32,
    blocked: F,
) -> (f32, f32) {
    // 已在 blocked 內(放行逃脫)或目標可走 → 直接到目標。
    if blocked(cur_x, cur_y) || !blocked(new_x, new_y) {
        (new_x, new_y)
    } else if !blocked(new_x, cur_y) {
        (new_x, cur_y) // 沿 X 滑
    } else if !blocked(cur_x, new_y) {
        (cur_x, new_y) // 沿 Y 滑
    } else {
        (cur_x, cur_y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biome_at_is_deterministic() {
        // 同座標永遠同結果(不靠亂數/時鐘)。
        for &(x, y) in &[(0.0, 0.0), (1234.5, 678.9), (5999.0, 12.0), (-50.0, 3000.0)] {
            assert_eq!(biome_at(x, y), biome_at(x, y));
        }
    }

    #[test]
    fn noise_stays_in_unit_range() {
        for i in 0..200 {
            let v = value_noise(i as f64 * 37.0, i as f64 * 53.0, 1500.0, 7);
            assert!((0.0..1.0).contains(&v), "noise 越界: {v}");
        }
    }

    #[test]
    fn world_has_variety() {
        // 掃整張世界粗格,生態域種類應夠多樣(不是一片到底)。
        use std::collections::BTreeSet;
        let mut kinds: BTreeSet<u32> = BTreeSet::new();
        let mut y = 0.0;
        while y < 6000.0 {
            let mut x = 0.0;
            while x < 6000.0 {
                kinds.insert(biome_at(x, y).code());
                x += 80.0;
            }
            y += 80.0;
        }
        assert!(
            kinds.len() >= 3,
            "整張世界生態域種類太少({}):噪聲門檻可能失衡",
            kinds.len()
        );
    }

    #[test]
    fn non_finite_falls_to_meadow() {
        // 與 JS 同:NaN/Inf 的比較全 false → 落到 Meadow,不 panic。
        assert_eq!(biome_at(f64::NAN, 0.0), Biome::Meadow);
        assert_eq!(biome_at(f64::INFINITY, f64::NEG_INFINITY), Biome::Meadow);
    }

    #[test]
    fn code_round_trips_all_variants() {
        assert_eq!(Biome::Water.code(), 0);
        assert_eq!(Biome::Sand.code(), 1);
        assert_eq!(Biome::Meadow.code(), 2);
        assert_eq!(Biome::Forest.code(), 3);
        assert_eq!(Biome::Rocky.code(), 4);
    }

    // ── resolve_move 滑動碰撞(gemini 草擬、Claude 審校)──
    #[test]
    fn move_open_target_moves_fully() {
        assert_eq!(resolve_move(0.0, 0.0, 1.0, 1.0, |_, _| false), (1.0, 1.0));
    }

    #[test]
    fn move_blocked_target_slides_on_x() {
        let blocked = |x: f32, y: f32| x > 0.5 && y > 0.5;
        assert_eq!(resolve_move(0.0, 0.0, 1.0, 1.0, blocked), (1.0, 0.0));
    }

    #[test]
    fn move_blocked_target_slides_on_y() {
        let blocked = |x: f32, _y: f32| x > 0.5;
        assert_eq!(resolve_move(0.0, 0.0, 1.0, 1.0, blocked), (0.0, 1.0));
    }

    #[test]
    fn move_fully_blocked_stays_put() {
        let blocked = |x: f32, y: f32| !(-0.5..=0.5).contains(&x) || !(-0.5..=0.5).contains(&y);
        assert_eq!(resolve_move(0.0, 0.0, 1.0, 1.0, blocked), (0.0, 0.0));
    }

    #[test]
    fn move_starting_inside_blocked_escapes() {
        let blocked = |x: f32, y: f32| x < 0.5 && y < 0.5;
        assert_eq!(resolve_move(0.0, 0.0, 1.0, 1.0, blocked), (1.0, 1.0));
    }

    #[test]
    fn tile_kind_at_is_deterministic() {
        // 同座標永遠同結果（不靠亂數/時鐘）。
        for &(x, y) in &[(0.0, 0.0), (1024.5, 512.0), (5000.0, 3000.0), (-64.0, 128.0)] {
            assert_eq!(tile_kind_at(x, y), tile_kind_at(x, y));
        }
    }

    #[test]
    fn water_biome_is_always_empty() {
        // 找一個水域座標，確認 tile_kind_at 回 Empty（水面無實心格）。
        let mut found = false;
        'outer: for gy in 0..50i32 {
            for gx in 0..50i32 {
                let x = gx as f64 * 200.0;
                let y = gy as f64 * 200.0;
                if biome_at(x, y) == Biome::Water {
                    assert_eq!(tile_kind_at(x, y), TileKind::Empty, "水域應回 Empty");
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "掃描範圍內應存在至少一個水域座標");
    }

    #[test]
    fn rocky_biome_has_stone_tiles() {
        // 岩地生態域應有一定比例的 Stone/Ore 格（確認生成邏輯有效，不是全 Empty）。
        let mut stone_count = 0usize;
        let mut total = 0usize;
        for gy in 0..50i32 {
            for gx in 0..50i32 {
                let x = gx as f64 * 64.0;
                let y = gy as f64 * 64.0;
                if biome_at(x, y) == Biome::Rocky {
                    total += 1;
                    let k = tile_kind_at(x + 16.0, y + 16.0); // tile 中心
                    if k == TileKind::Stone || k == TileKind::Ore {
                        stone_count += 1;
                    }
                }
            }
        }
        if total > 0 {
            let ratio = stone_count as f64 / total as f64;
            assert!(ratio > 0.1, "岩地應有>10%實心格，實際={:.1}%", ratio * 100.0);
        }
    }

    #[test]
    fn tile_code_round_trips() {
        assert_eq!(TileKind::Empty.code(), 0);
        assert_eq!(TileKind::Dirt.code(),  1);
        assert_eq!(TileKind::Stone.code(), 2);
        assert_eq!(TileKind::Ore.code(),   3);
    }
}
