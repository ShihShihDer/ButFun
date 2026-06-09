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

/// 新手村安全區（圓心 / 半徑，世界像素）。圈內地形一律挖空（Empty），不讓確定性生成把
/// 新手村 / 出生點埋進實心土。**數值與主 crate 的 `positions::default_spawn()` +
/// `SAFE_SPAWN_RADIUS` 對齊；主 crate 有測試 `world_core_safe_zone_matches_game` 守著兩邊一致。**
pub const SAFE_ZONE_CX: f64 = 2344.0;
pub const SAFE_ZONE_CY: f64 = 2296.0;
pub const SAFE_ZONE_RADIUS: f64 = 640.0;

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
///   4 = Crystal（晶石，Deep Rocky 特有稀有礦）
///   5 = Mushroom（蕈菇，Forest 生態域蕈菇聚落特有）
///   6 = AncientRuin（古代遺跡，Sand 生態域沙漠遺跡聚落特有）
///   7 = CoralReef（珊瑚礁，Water 生態域海底特產）
///   8 = WildFlower（野花叢，Meadow 生態域草原特產）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileKind {
    Empty,
    Dirt,
    Stone,
    Ore,
    /// 晶石礦脈——只生在岩地（Rocky）生態域的晶洞聚落中，挖後掉晶石碎片，可高價賣給 NPC。
    Crystal,
    /// 蕈菇叢——只生在森林（Forest）生態域的蕈菇聚落中，挖後掉蕈菇孢子，
    /// 孢子散發異星氣息，NPC 以溢價收購，是探索型玩家的第二條乙太路線。
    Mushroom,
    /// 古代遺跡石——只生在沙漠（Sand）生態域的遺跡聚落中，挖後掉古代碎片，
    /// NPC 以高溢價收購，給探索沙漠的玩家開出第三條乙太路線。
    AncientRuin,
    /// 珊瑚礁——只生在水域（Water）生態域的珊瑚聚落中，挖後掉深海珍珠，
    /// NPC 以最高溢價收購，鼓勵玩家在海岸邊挖掘水下珍寶。
    CoralReef,
    /// 野花叢——只生在草原（Meadow）生態域的野花聚落中，挖後掉野花種子，
    /// NPC 以溢價收購，給穿梭草原的玩家補上第五條乙太路線。
    WildFlower,
}

impl TileKind {
    pub fn code(self) -> u8 {
        match self {
            TileKind::Empty       => 0,
            TileKind::Dirt        => 1,
            TileKind::Stone       => 2,
            TileKind::Ore         => 3,
            TileKind::Crystal     => 4,
            TileKind::Mushroom    => 5,
            TileKind::AncientRuin => 6,
            TileKind::CoralReef   => 7,
            TileKind::WildFlower  => 8,
        }
    }
}

/// 座標（世界像素）→ 確定性地形格種類。
///
/// 同 `biome_at`：同座標必定同結果、不靠亂數/時鐘。使用與生態域相同的 `grass_hash`
/// 雜湊函式，故前端可用 JS `grassHash` 精確對齊（見 `web/game.js` 的 `tileKindAt`）。
/// 水域一律回 `Empty`（水面沒有可挖的實心格）。
pub fn tile_kind_at(wx: f64, wy: f64) -> TileKind {
    // 新手村安全區內一律乾淨地（Empty）——不讓地形生成把城鎮 / 出生點埋住、害玩家卡在土裡。
    let sdx = wx - SAFE_ZONE_CX;
    let sdy = wy - SAFE_ZONE_CY;
    if sdx * sdx + sdy * sdy <= SAFE_ZONE_RADIUS * SAFE_ZONE_RADIUS {
        return TileKind::Empty;
    }
    let biome = biome_at(wx, wy);

    // 水域特例：珊瑚礁聚落（海底特產），其餘水格一律 Empty（水面可通行）。
    // 玩家無法進入水域，但可從岸邊 80px 距離挖掘珊瑚礁取得深海珍珠。
    if biome == Biome::Water {
        let gx = (wx / TILE_PX as f64).floor() as i32;
        let gy = (wy / TILE_PX as f64).floor() as i32;
        let h = grass_hash(
            gx.wrapping_mul(1031) ^ gy.wrapping_mul(2053),
            gx ^ gy.wrapping_mul(1009),
        );
        // 珊瑚礁聚落：次級噪聲（scale 70, seed 555）高於 0.80 的水域（約 20%）形成珊瑚礁。
        // 珊瑚礁內：50% CoralReef，50% Empty——保留水面空間，不讓水域完全堵死。
        let coral_n = value_noise(wx, wy, 70.0, 555);
        if coral_n > 0.80 && h < 0.50 {
            return TileKind::CoralReef;
        }
        return TileKind::Empty;
    }

    // 地形空曠度：低頻 value_noise 決定哪裡是開闊空地、哪裡是實心。
    // 礦區（Rocky）門檻 0.50 → 約 50% 實心（礦石多、像個礦坑可深挖）；其餘生態域 0.82 → 約 18%
    // 實心（開闊好走，實心集中成礦脈/岩體）。（原本全域 0.38＝62% 實心會到處卡，已按玩家要求降。）
    let cave = value_noise(wx, wy, 160.0, 123);
    let cave_threshold = if biome == Biome::Rocky { 0.50 } else { 0.82 };
    if cave < cave_threshold {
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
            // 晶洞判定：次級噪聲（scale 80, seed 777）高於 0.85 的聚落（約 15% 的岩地）形成晶洞。
            // 晶洞內：60% Crystal，40% Stone——挖進去才亮紫，提供「深入探索有回報」的感受。
            let crystal_n = value_noise(wx, wy, 80.0, 777);
            if crystal_n > 0.85 {
                if h < 0.60 { TileKind::Crystal }
                else { TileKind::Stone }
            } else {
                // 普通岩域：礦脈較多(12%)，其餘皆為岩石。
                if h < 0.12 { TileKind::Ore }
                else { TileKind::Stone }
            }
        }
        Biome::Forest => {
            // 蕈菇聚落判定：次級噪聲（scale 80, seed 456）高於 0.82 的區域（約 18% 的森林）形成蕈菇洞。
            // 蕈菇洞內：50% Mushroom，50% Dirt——挖進去才看見發光蕈菇，給探索型玩家視覺驚喜。
            let mushroom_n = value_noise(wx, wy, 80.0, 456);
            if mushroom_n > 0.82 {
                if h < 0.50 { TileKind::Mushroom }
                else { TileKind::Dirt }
            } else {
                // 普通森林：偶爾有岩石(10%)，其餘為泥土。
                if h < 0.10 { TileKind::Stone }
                else { TileKind::Dirt }
            }
        }
        Biome::Meadow => {
            // 野花聚落判定：次級噪聲（scale 75, seed 222）高於 0.81 的區域（約 19% 的草原）形成野花田。
            // 野花田內：55% WildFlower，45% Dirt——挖進去才看見繽紛野花叢，給探索草原的玩家視覺驚喜。
            let flower_n = value_noise(wx, wy, 75.0, 222);
            if flower_n > 0.81 {
                if h < 0.55 { TileKind::WildFlower }
                else { TileKind::Dirt }
            } else {
                // 普通草原：偶爾有石塊(5%)，其餘為泥土。
                if h < 0.05 { TileKind::Stone }
                else { TileKind::Dirt }
            }
        }
        Biome::Sand => {
            // 沙漠遺跡判定：次級噪聲（scale 90, seed 333）高於 0.83 的區域（約 17% 的沙漠）形成遺跡聚落。
            // 遺跡內：55% AncientRuin，45% Stone——挖進去才看見古代石刻，給探索型玩家視覺驚喜。
            let ruin_n = value_noise(wx, wy, 90.0, 333);
            if ruin_n > 0.83 {
                if h < 0.55 { TileKind::AncientRuin }
                else { TileKind::Stone }
            } else {
                // 普通沙漠：偶爾有石塊(8%)，其餘為泥土（沙下層）。
                if h < 0.08 { TileKind::Stone }
                else { TileKind::Dirt }
            }
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
    // 分軸、**小步掃掠**推進（swept）：把一步拆成 STEP 大小的小步，一路貼到牆前才停——
    // 不會像「整步會撞就整個不動」那樣卡在離牆一整步遠、或在窄路完全卡死（治「移動被卡住」）。
    // 撞到牆角時做**轉角修正**（corner correction，Celeste 那類大型遊戲的招）：往垂直方向
    // 微推一點點（最多 CORNER）看能不能繞過，治「窄隧道偏中線就被牆角卡住」。
    const STEP: f32 = 2.0;
    const CORNER: f32 = 6.0;
    // 已陷在 blocked 內（生成/傳送落在實心格、被推入水等）→ 放行逃脫，直接到目標、不卡死。
    if blocked(cur_x, cur_y) {
        return (new_x, new_y);
    }
    let mut x = cur_x;
    let mut y = cur_y;

    // X 軸
    let dx = new_x - cur_x;
    let nsx = (dx.abs() / STEP).ceil() as i32;
    if nsx > 0 {
        let inc = dx / nsx as f32;
        for _ in 0..nsx {
            if !blocked(x + inc, y) {
                x += inc;
                continue;
            }
            // 轉角修正：往 ±y 微推，找最近一個能讓這小步通過的偏移；繞不過就停（貼牆）。
            let mut slipped = false;
            let mut off = STEP;
            while off <= CORNER {
                for cand in [y - off, y + off] {
                    if !blocked(x + inc, cand) && !blocked(x, cand) {
                        y = cand;
                        x += inc;
                        slipped = true;
                        break;
                    }
                }
                if slipped {
                    break;
                }
                off += STEP;
            }
            if !slipped {
                break;
            }
        }
    }

    // Y 軸（先 X 後 Y → 撞牆自動沿牆滑）
    let dy = new_y - cur_y;
    let nsy = (dy.abs() / STEP).ceil() as i32;
    if nsy > 0 {
        let inc = dy / nsy as f32;
        for _ in 0..nsy {
            if !blocked(x, y + inc) {
                y += inc;
                continue;
            }
            let mut slipped = false;
            let mut off = STEP;
            while off <= CORNER {
                for cand in [x - off, x + off] {
                    if !blocked(cand, y + inc) && !blocked(cand, y) {
                        x = cand;
                        y += inc;
                        slipped = true;
                        break;
                    }
                }
                if slipped {
                    break;
                }
                off += STEP;
            }
            if !slipped {
                break;
            }
        }
    }

    (x, y)
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

    // ── resolve_move：分軸小步掃掠 + 轉角修正 ──
    #[test]
    fn move_open_target_moves_fully() {
        let r = resolve_move(0.0, 0.0, 20.0, 0.0, |_, _| false);
        assert!((r.0 - 20.0).abs() < 0.01 && r.1.abs() < 0.01, "{:?}", r);
    }

    #[test]
    fn creeps_right_up_to_wall() {
        // 牆在 x>=10：玩家從 0 想衝到 20，應「貼到牆前」(≈8~10)，而不是停在原地或老遠。
        let blocked = |x: f32, _y: f32| x >= 10.0;
        let r = resolve_move(0.0, 0.0, 20.0, 0.0, blocked);
        assert!(r.0 >= 8.0 && r.0 < 10.0, "應貼到牆前, got {:?}", r);
    }

    #[test]
    fn slides_along_wall() {
        // 牆在 x>=10，同時想往 +x +y：x 被擋貼牆，y 仍應照走。
        let blocked = |x: f32, _y: f32| x >= 10.0;
        let r = resolve_move(0.0, 0.0, 20.0, 20.0, blocked);
        assert!(r.0 >= 8.0 && r.0 < 10.0, "x 應貼牆, got {:?}", r);
        assert!((r.1 - 20.0).abs() < 0.01, "y 應照滑, got {:?}", r);
    }

    #[test]
    fn corner_correction_slips_past_small_nub() {
        // 一小塊凸起 (x∈[10,13), y<5) 擋路，玩家在 y=4 往右走：轉角修正應往下微推繞過、越過凸起。
        let blocked = |x: f32, y: f32| x >= 10.0 && x < 13.0 && y < 5.0;
        let r = resolve_move(0.0, 4.0, 20.0, 4.0, blocked);
        assert!(r.0 > 13.0, "應繞過小凸起、x 越過 13, got {:?}", r);
    }

    #[test]
    fn long_wall_does_not_teleport_through() {
        // 一道長牆(x>=10)，轉角修正(最多 CORNER)繞不過 → 停在牆前，不穿牆。
        let blocked = |x: f32, _y: f32| x >= 10.0;
        let r = resolve_move(0.0, 0.0, 40.0, 0.0, blocked);
        assert!(r.0 < 10.0, "不該穿牆, got {:?}", r);
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
    fn water_biome_non_coral_is_empty() {
        // 水域裡大部分格應為 Empty（珊瑚礁只佔少數，水面整體可通行）。
        let mut empty_count = 0usize;
        let mut total = 0usize;
        for gy in 0..100i32 {
            for gx in 0..100i32 {
                let x = gx as f64 * 64.0;
                let y = gy as f64 * 64.0;
                if biome_at(x, y) == Biome::Water {
                    total += 1;
                    if tile_kind_at(x + 16.0, y + 16.0) == TileKind::Empty {
                        empty_count += 1;
                    }
                }
            }
        }
        if total > 0 {
            // 珊瑚礁佔比約 10%（20% 聚落 × 50% 密度），故 Empty 應 > 80%。
            let empty_ratio = empty_count as f64 / total as f64;
            assert!(
                empty_ratio > 0.75,
                "水域應有>75%空格保持可通行，實際={:.1}%",
                empty_ratio * 100.0
            );
        }
    }

    #[test]
    fn coral_reef_exists_in_water_biome() {
        // 掃水域範圍，確認確實能生成珊瑚礁格（不是機率設太低全找不到）。
        let mut found = false;
        'outer: for gy in 0..300i32 {
            for gx in 0..300i32 {
                let x = gx as f64 * 32.0;
                let y = gy as f64 * 32.0;
                if biome_at(x, y) == Biome::Water && tile_kind_at(x + 16.0, y + 16.0) == TileKind::CoralReef {
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "水域生態域中應存在珊瑚礁格");
    }

    #[test]
    fn coral_reef_only_in_water_biome() {
        // CoralReef 格不應出現在非水域生態域（rocky/forest/meadow/sand）。
        for gy in 0..100i32 {
            for gx in 0..100i32 {
                let x = gx as f64 * 64.0 + 16.0;
                let y = gy as f64 * 64.0 + 16.0;
                let b = biome_at(x, y);
                if b != Biome::Water {
                    let k = tile_kind_at(x, y);
                    assert_ne!(k, TileKind::CoralReef, "非水域生態域 {:?} 不應生珊瑚礁，座標=({x},{y})", b);
                }
            }
        }
    }

    #[test]
    fn tile_code_includes_coral_reef() {
        assert_eq!(TileKind::CoralReef.code(), 7);
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
        assert_eq!(TileKind::Empty.code(),       0);
        assert_eq!(TileKind::Dirt.code(),        1);
        assert_eq!(TileKind::Stone.code(),       2);
        assert_eq!(TileKind::Ore.code(),         3);
        assert_eq!(TileKind::Crystal.code(),     4);
        assert_eq!(TileKind::Mushroom.code(),    5);
        assert_eq!(TileKind::AncientRuin.code(), 6);
        assert_eq!(TileKind::CoralReef.code(),   7);
        assert_eq!(TileKind::WildFlower.code(),  8);
    }

    #[test]
    fn crystal_cave_exists_in_rocky_biome() {
        // 掃岩地範圍，確認確實能生成晶石格（不是機率設太低全找不到）。
        let mut found_crystal = false;
        'outer: for gy in 0..200i32 {
            for gx in 0..200i32 {
                let x = gx as f64 * 32.0;
                let y = gy as f64 * 32.0;
                if biome_at(x, y) == Biome::Rocky && tile_kind_at(x + 16.0, y + 16.0) == TileKind::Crystal {
                    found_crystal = true;
                    break 'outer;
                }
            }
        }
        assert!(found_crystal, "岩地生態域中應存在晶石格");
    }

    #[test]
    fn crystal_only_in_rocky_biome() {
        // Crystal 格不應出現在非岩地生態域（forest/meadow/sand）。
        for gy in 0..100i32 {
            for gx in 0..100i32 {
                let x = gx as f64 * 64.0;
                let y = gy as f64 * 64.0;
                let b = biome_at(x, y);
                if b != Biome::Rocky && b != Biome::Water {
                    let k = tile_kind_at(x + 16.0, y + 16.0);
                    assert_ne!(k, TileKind::Crystal, "非岩地生態域 {:?} 不應生晶石，座標=({x},{y})", b);
                }
            }
        }
    }

    #[test]
    fn mushroom_cave_exists_in_forest_biome() {
        // 掃森林範圍，確認確實能生成蕈菇格（不是機率設太低全找不到）。
        let mut found = false;
        'outer: for gy in 0..200i32 {
            for gx in 0..200i32 {
                let x = gx as f64 * 32.0;
                let y = gy as f64 * 32.0;
                if biome_at(x, y) == Biome::Forest && tile_kind_at(x + 16.0, y + 16.0) == TileKind::Mushroom {
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "森林生態域中應存在蕈菇格");
    }

    #[test]
    fn mushroom_only_in_forest_biome() {
        // Mushroom 格不應出現在非森林生態域（rocky/meadow/sand）。
        for gy in 0..100i32 {
            for gx in 0..100i32 {
                let x = gx as f64 * 64.0;
                let y = gy as f64 * 64.0;
                let b = biome_at(x, y);
                if b != Biome::Forest && b != Biome::Water {
                    let k = tile_kind_at(x + 16.0, y + 16.0);
                    assert_ne!(k, TileKind::Mushroom, "非森林生態域 {:?} 不應生蕈菇，座標=({x},{y})", b);
                }
            }
        }
    }

    #[test]
    fn ancient_ruin_exists_in_sand_biome() {
        // 掃沙漠範圍，確認確實能生成古代遺跡格（不是機率設太低全找不到）。
        let mut found = false;
        'outer: for gy in 0..200i32 {
            for gx in 0..200i32 {
                let x = gx as f64 * 32.0;
                let y = gy as f64 * 32.0;
                if biome_at(x, y) == Biome::Sand && tile_kind_at(x + 16.0, y + 16.0) == TileKind::AncientRuin {
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "沙漠生態域中應存在古代遺跡格");
    }

    #[test]
    fn ancient_ruin_only_in_sand_biome() {
        // AncientRuin 格不應出現在非沙漠生態域（forest/rocky/meadow）。
        // 用格中心同一點同時查 biome 和 tile，避免跨生態域邊界的假陽性。
        for gy in 0..100i32 {
            for gx in 0..100i32 {
                let x = gx as f64 * 64.0 + 16.0; // 格中心
                let y = gy as f64 * 64.0 + 16.0;
                let b = biome_at(x, y);
                if b != Biome::Sand && b != Biome::Water {
                    let k = tile_kind_at(x, y);
                    assert_ne!(k, TileKind::AncientRuin, "非沙漠生態域 {:?} 不應生遺跡，座標=({x},{y})", b);
                }
            }
        }
    }

    #[test]
    fn tile_code_includes_ancient_ruin() {
        assert_eq!(TileKind::AncientRuin.code(), 6);
    }

    #[test]
    fn wild_flower_exists_in_meadow_biome() {
        // 掃草原範圍，確認確實能生成野花叢格（不是機率設太低全找不到）。
        let mut found = false;
        'outer: for gy in 0..300i32 {
            for gx in 0..300i32 {
                let x = gx as f64 * 32.0;
                let y = gy as f64 * 32.0;
                if biome_at(x, y) == Biome::Meadow && tile_kind_at(x + 16.0, y + 16.0) == TileKind::WildFlower {
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "草原生態域中應存在野花叢格");
    }

    #[test]
    fn wild_flower_only_in_meadow_biome() {
        // WildFlower 格不應出現在非草原生態域（rocky/forest/sand）。
        for gy in 0..100i32 {
            for gx in 0..100i32 {
                let x = gx as f64 * 64.0 + 16.0;
                let y = gy as f64 * 64.0 + 16.0;
                let b = biome_at(x, y);
                if b != Biome::Meadow && b != Biome::Water {
                    let k = tile_kind_at(x, y);
                    assert_ne!(k, TileKind::WildFlower, "非草原生態域 {:?} 不應生野花叢，座標=({x},{y})", b);
                }
            }
        }
    }

    #[test]
    fn tile_code_includes_wild_flower() {
        assert_eq!(TileKind::WildFlower.code(), 8);
    }
}

#[cfg(test)]
mod d2_tests {
    use super::*;

    #[test]
    fn safe_zone_stays_empty() {
        // 新手村中心應為 Empty
        assert_eq!(tile_kind_at(SAFE_ZONE_CX, SAFE_ZONE_CY), TileKind::Empty);
        // 邊緣也應為 Empty
        assert_eq!(tile_kind_at(SAFE_ZONE_CX + SAFE_ZONE_RADIUS - 1.0, SAFE_ZONE_CY), TileKind::Empty);
    }

    #[test]
    fn outside_safe_zone_has_solid_world() {
        // 找一塊遠離安全區的岩地或森林，確認實心格比例大幅提升（~60%）。
        let mut solid_count = 0usize;
        let mut total = 0usize;
        // 避開安全區 (2344, 2296)
        for gy in 0..100 {
            for gx in 0..100 {
                let x = gx as f64 * 32.0;
                let y = gy as f64 * 32.0;
                let b = biome_at(x, y);
                if b != Biome::Water {
                    total += 1;
                    if tile_kind_at(x + 16.0, y + 16.0) != TileKind::Empty {
                        solid_count += 1;
                    }
                }
            }
        }
        let ratio = solid_count as f64 / total as f64;
        // D-2 反轉後，實心比例應接近 1.0 - 0.38 = 0.62
        assert!(ratio > 0.5 && ratio < 0.75, "實心比例應在 50%~75% 之間，實際={:.1}%", ratio * 100.0);
    }
}
