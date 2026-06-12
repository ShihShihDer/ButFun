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

/// 翠幽星（Verdant Star）在主世界的 X 最小座標。
/// X ≥ 此值視為翠幽星領域，tile_kind_at 會改變地形生成邏輯（生成翠玉藤聚落）。
/// 與 `crate::state::VERDANT_SPAWN_X`（22400）一致——翠幽星出生點深在此邊界內側。
pub const VERDANT_ZONE_MIN_X: f64 = 20_000.0;

/// 赤焰星（Crimson Star）在主世界的 X 最大座標。
/// X ≤ 此值視為赤焰星領域，tile_kind_at 會生成熔岩石聚落。
/// 與 `crate::state::CRIMSON_SPAWN_X`（-18000）一致——赤焰星出生點深在此邊界內側。
pub const CRIMSON_ZONE_MAX_X: f64 = -15_000.0;

/// 霧醚星（Aether Star）在主世界的 X 最大座標。
/// X ≤ 此值視為霧醚星領域（比赤焰星更深的遠西方），tile_kind_at 會生成霧醚晶霧聚落（覆蓋赤焰星邏輯）。
/// 與 `crate::state::AETHER_SPAWN_X`（-32000）一致——霧醚星出生點深在此邊界內側。
pub const AETHER_ZONE_MAX_X: f64 = -30_000.0;

/// 虛空星（Void Star）在主世界的 X 最小座標。
/// X ≥ 此值視為虛空星領域，tile_kind_at 會生成虛空晶體聚落（覆蓋翠幽星邏輯）。
/// 與 `crate::state::VOID_SPAWN_X`（42000）一致——虛空星出生點深在此邊界內側。
pub const VOID_ZONE_MIN_X: f64 = 38_000.0;

/// 星源星（Origin Star）在主世界的 X 最大座標。
/// X ≤ 此值視為星源星領域（比霧醚星更深的遠西方），tile_kind_at 會生成源晶聚落（覆蓋霧醚星邏輯）。
/// 與 `crate::state::ORIGIN_SPAWN_X`（-52000）一致——星源星出生點深在此邊界內側。
pub const ORIGIN_ZONE_MAX_X: f64 = -50_000.0;

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

// ── 城鎮（圍牆主城＋各星球據點）────────────────────────────────────────────
//
// 城鎮是「**格座標上的確定性幾何**」：方形城牆（TownWall，不可挖）圍住淨空的城內，
// 四邊正中留城門。全部判定走整數格座標（floor(wx/32)）——碰撞、渲染、wasm 預測、
// JS 後備取樣同一個函式就逐位元一致，不會有「畫起來是門、撞起來是牆」的半格鬼牆。
// 城規（伺服器強制）：城內不生地形、不生怪、怪不得踏入、禁放置方塊；城牆挖不掉。

/// 一座城鎮的定義（格座標）。
pub struct TownDef {
    /// 中心格座標（floor(世界px / 32)）。
    pub cgx: i32,
    pub cgy: i32,
    /// 城牆所在的 Chebyshev 半徑（格）。牆內（< half）全淨空。
    pub half_tiles: i32,
    /// 城門開口半寬（格）：每邊正中 `2*gate_half+1` 格寬的缺口。
    pub gate_half_tiles: i32,
    /// 顯示名（前端地標/官網用；遊戲規則不讀它）。
    pub name: &'static str,
}

/// 全部城鎮。主城在新手村（出生點 2344,2296 → 格 73,71）；各星球據點設在該星球
/// 出生點（與 `state::*_SPAWN_*` 對齊：x/32 向下取整，y=3000 → 格 93）。
pub const TOWNS: &[TownDef] = &[
    // 新手村主城：68×68 格（約 2176px 見方），涵蓋公共農地與商人。
    TownDef { cgx: 73, cgy: 71, half_tiles: 34, gate_half_tiles: 1, name: "新手村主城" },
    // 各星球據點：28×28 格，圍住該星球出生點與駐站商人。
    TownDef { cgx: 700, cgy: 93, half_tiles: 14, gate_half_tiles: 1, name: "翠幽據點" },   // 22400,3000
    TownDef { cgx: -563, cgy: 93, half_tiles: 14, gate_half_tiles: 1, name: "赤焰據點" },  // -18000,3000
    TownDef { cgx: 1312, cgy: 93, half_tiles: 14, gate_half_tiles: 1, name: "虛空據點" },  // 42000,3000
    TownDef { cgx: -1000, cgy: 93, half_tiles: 14, gate_half_tiles: 1, name: "霧醚據點" }, // -32000,3000
    TownDef { cgx: -1625, cgy: 93, half_tiles: 14, gate_half_tiles: 1, name: "星源據點" }, // -52000,3000
];

/// 該格是否為城牆（含城門判斷：門口缺口不是牆）。
pub fn town_wall_at_tile(gx: i32, gy: i32) -> bool {
    TOWNS.iter().any(|t| {
        let dx = gx - t.cgx;
        let dy = gy - t.cgy;
        let cheb = dx.abs().max(dy.abs());
        if cheb != t.half_tiles {
            return false;
        }
        // 在牆圈上：取「沿牆方向」的偏移當 perp；邊正中 |perp| ≤ gate_half 是城門缺口。
        // 角落兩軸都 == cheb → perp == cheb > gate_half，必為牆（門只開在邊正中）。
        let perp = if dx.abs() == cheb { dy.abs() } else { dx.abs() };
        perp > t.gate_half_tiles
    })
}

/// 該格是否在某座城鎮**範圍內**（含牆圈本身：淨空區不生地形、禁放置）。
/// 取 `<=`（含牆圈）而非 `<`：城門格在牆圈上但不是牆，必須一樣淨空＋禁放置，
/// 否則自然地形會在門口長出土塊、或玩家能用方塊把城門堵死。
pub fn town_interior_at_tile(gx: i32, gy: i32) -> bool {
    TOWNS.iter().any(|t| {
        (gx - t.cgx).abs().max((gy - t.cgy).abs()) <= t.half_tiles
    })
}

/// 世界像素 → 是否在城鎮牆內（淨空區）。
pub fn town_interior_at(wx: f64, wy: f64) -> bool {
    town_interior_at_tile(
        (wx / TILE_PX as f64).floor() as i32,
        (wy / TILE_PX as f64).floor() as i32,
    )
}

/// 世界像素 → 是否在城鎮保護圈內（牆內＋牆外 8 格緩衝）。
/// 敵人**不生成也不踏入**這個範圍——不只擋在牆外，連城門口都不准堵。
pub fn town_protected_at(wx: f64, wy: f64) -> bool {
    let gx = (wx / TILE_PX as f64).floor() as i32;
    let gy = (wy / TILE_PX as f64).floor() as i32;
    TOWNS.iter().any(|t| {
        (gx - t.cgx).abs().max((gy - t.cgy).abs()) <= t.half_tiles + 8
    })
}

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

/// wasm/前端入口:座標 → 地形格種類整數編碼(見 `TileKind::code`)。與 `biome_code` 同款
/// 純數值介面——前端載入本 crate 的 .wasm 後改呼叫這支,地貌就跟伺服器**同一份實作**,
/// 從根本消滅「JS 鏡像漂移 → 隱形空氣牆」一整類 bug(JS 版僅留作載入失敗的後備)。
///
/// # Safety
/// 純值計算、無指標、無共享狀態;`extern "C"` 僅為 wasm 匯出穩定符號。
#[no_mangle]
pub extern "C" fn tile_kind_code(x: f64, y: f64) -> u32 {
    tile_kind_at(x, y).code() as u32
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
///   9 = JadeVine（翠玉藤，翠幽星 X≥20000 特有聚落，挖後掉翠幽碎片）
///  10 = LavaRock（熔岩石，赤焰星 X≤-15000 特有聚落，挖後掉熔晶碎片）
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
    /// 翠玉藤——只生在翠幽星（VERDANT_ZONE_MIN_X ≤ X < VOID_ZONE_MIN_X）的聚落中，挖後掉翠幽碎片，
    /// 翠幽星 NPC 以高溢價收購，是首個跨星球特產，鼓勵玩家深入探索異星。
    JadeVine,
    /// 熔岩石——只生在赤焰星（X ≤ CRIMSON_ZONE_MAX_X）的聚落中，挖後掉熔晶碎片，
    /// 赤焰星 NPC 以最高溢價收購，蒸汽龐克異星的核心礦產，鼓勵玩家深入高溫熔岩地帶。
    LavaRock,
    /// 虛空晶體——只生在虛空星（X ≥ VOID_ZONE_MIN_X）的聚落中，挖後掉虛空碎片，
    /// 虛空星 NPC 以最高溢價收購，宇宙深淵凝聚的黑暗晶石，鼓勵玩家探索宇宙邊界。
    VoidCrystal,
    /// 霧醚晶霧——只生在霧醚星（X ≤ AETHER_ZONE_MAX_X）的聚落中，挖後掉霧醚碎片，
    /// 霧醚星 NPC 以最高溢價收購，乙太迷霧凝結的神秘晶石，鼓勵玩家深入宇宙遠西。
    AetherMist,
    /// 源晶——只生在星源星（X ≤ ORIGIN_ZONE_MAX_X）的聚落中，挖後掉源晶碎片，
    /// 星源星 NPC 以最高溢價收購，乙太文明源頭的原初結晶，鼓勵玩家深入宇宙最遠西境。
    OriginCrystal,
    /// 城牆——城鎮的圍牆結構（見 `TOWNS`）。**不可挖、不掉落、不可放置**，
    /// 由確定性幾何生成（不吃 delta），是玩家安全區的硬邊界。
    TownWall,
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
            TileKind::JadeVine    => 9,
            TileKind::LavaRock    => 10,
            TileKind::VoidCrystal    => 11,
            TileKind::AetherMist     => 12,
            TileKind::OriginCrystal  => 13,
            TileKind::TownWall       => 14,
        }
    }
}

/// 座標（世界像素）→ 確定性地形格種類。
///
/// 同 `biome_at`：同座標必定同結果、不靠亂數/時鐘。使用與生態域相同的 `grass_hash`
/// 雜湊函式，故前端可用 JS `grassHash` 精確對齊（見 `web/game.js` 的 `tileKindAt`）。
/// 水域一律回 `Empty`（水面沒有可挖的實心格）。
pub fn tile_kind_at(wx: f64, wy: f64) -> TileKind {
    // 格索引（整數）。城鎮幾何最優先：城牆是不可挖結構、牆內一律淨空。
    let gx = (wx / TILE_PX as f64).floor() as i32;
    let gy = (wy / TILE_PX as f64).floor() as i32;
    if town_wall_at_tile(gx, gy) {
        return TileKind::TownWall;
    }
    if town_interior_at_tile(gx, gy) {
        return TileKind::Empty;
    }

    // 隱形牆根治：把浮點座標 snap 到格中心，確保同格所有位置（含四個角落）返回
    // 完全相同的地形種類。渲染層本就在格中心取樣；碰撞層以精確角落座標呼叫
    // 此函式——未 snap 時格邊緣的 biome/noise 值可能跨越閾值，造成
    // 「渲染=空格 + 碰撞=實心」的隱形牆。Snap 後兩者一致，隱形牆消失。
    let wx = (gx as f64 + 0.5) * TILE_PX as f64;
    let wy = (gy as f64 + 0.5) * TILE_PX as f64;

    let biome = biome_at(wx, wy);

    // 水域特例：珊瑚礁聚落（海底特產），其餘水格一律 Empty（水面可通行）。
    // 玩家無法進入水域，但可從岸邊 80px 距離挖掘珊瑚礁取得深海珍珠。
    if biome == Biome::Water {
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

    // 穩定格雜湊值 [0,1)（gx/gy 已在函式頂端算好）
    let h = grass_hash(
        gx.wrapping_mul(1031) ^ gy.wrapping_mul(2053),
        gx ^ gy.wrapping_mul(1009),
    );

    // 虛空星特有：虛空晶體聚落。所有 wx ≥ VOID_ZONE_MIN_X 的實心格都可能生虛空晶體，
    // 覆蓋翠幽星及所有非水域生態的普通材質，讓整個虛空星充滿宇宙深淵晶石。
    // 次級噪聲（scale 80, seed 2023）決定聚落邊界（約 22% 的虛空星實心格形成聚落）；
    // 聚落內 68% 為 VoidCrystal，32% 保留原有材質，維持地形多樣性。
    // 注意：用 `else if` 避免翠玉藤出現在虛空星範圍（VOID_ZONE_MIN_X ≥ VERDANT_ZONE_MIN_X）。
    if wx >= VOID_ZONE_MIN_X && biome != Biome::Water {
        let void_n = value_noise(wx, wy, 80.0, 2023);
        if void_n > 0.78 && h < 0.68 {
            return TileKind::VoidCrystal;
        }
    } else if wx >= VERDANT_ZONE_MIN_X && biome != Biome::Water {
        // 翠幽星特有：翠玉藤聚落。所有 VERDANT_ZONE_MIN_X ≤ wx < VOID_ZONE_MIN_X 的實心格都可能長翠玉藤，
        // 覆蓋所有非水域生態的普通材質，讓整個翠幽星到處都有特產可挖。
        // 次級噪聲（scale 85, seed 999）決定聚落邊界（約 20% 的翠幽星實心格形成聚落）；
        // 聚落內 65% 為 JadeVine，35% 保留原有材質，維持地形多樣性。
        let jade_n = value_noise(wx, wy, 85.0, 999);
        if jade_n > 0.80 && h < 0.65 {
            return TileKind::JadeVine;
        }
    }

    // 星源星特有：源晶聚落。所有 wx ≤ ORIGIN_ZONE_MAX_X 的實心格都可能生源晶，
    // 覆蓋霧醚星及所有非水域生態的普通材質（星源星比霧醚星更遠的極西境）。
    // 次級噪聲（scale 80, seed 3141）決定聚落邊界（約 25% 的星源星實心格形成源晶聚落）；
    // 聚落內 75% 為 OriginCrystal，25% 保留原有材質，維持地形多樣性。
    if wx <= ORIGIN_ZONE_MAX_X && biome != Biome::Water {
        let origin_n = value_noise(wx, wy, 80.0, 3141);
        if origin_n > 0.75 && h < 0.75 {
            return TileKind::OriginCrystal;
        }
    // 霧醚星特有：霧醚晶霧聚落。所有 ORIGIN_ZONE_MAX_X < wx ≤ AETHER_ZONE_MAX_X 的實心格都可能生霧醚晶霧，
    // 覆蓋赤焰星及所有非水域生態的普通材質（霧醚星比赤焰星更遠的遠西方）。
    // 次級噪聲（scale 85, seed 2077）決定聚落邊界（約 23% 的霧醚星實心格形成霧醚聚落）；
    // 聚落內 72% 為 AetherMist，28% 保留原有材質，維持地形多樣性。
    } else if wx <= AETHER_ZONE_MAX_X && biome != Biome::Water {
        let aether_n = value_noise(wx, wy, 85.0, 2077);
        if aether_n > 0.77 && h < 0.72 {
            return TileKind::AetherMist;
        }
    // 赤焰星特有：熔岩石聚落。所有 AETHER_ZONE_MAX_X < wx ≤ CRIMSON_ZONE_MAX_X 的實心格都可能生熔岩石，
    // 覆蓋所有非水域生態的普通材質，讓整個赤焰星到處都有熔晶可挖。
    // 次級噪聲（scale 90, seed 1337）決定聚落邊界（約 25% 的赤焰星實心格形成熔岩聚落）；
    // 聚落內 70% 為 LavaRock，30% 保留原有材質，維持地形多樣性。
    } else if wx <= CRIMSON_ZONE_MAX_X && biome != Biome::Water {
        let lava_n = value_noise(wx, wy, 90.0, 1337);
        if lava_n > 0.75 && h < 0.70 {
            return TileKind::LavaRock;
        }
    }

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

// ── 共用玩家移動物理（伺服器權威 + 前端 wasm 預測都呼叫這一份）─────────────────
//
// 客戶端移動預測要能成立，前提是「預測用的物理 == 伺服器的物理」，否則預測會常常
// 猜錯、角色被拉回去（橡皮筋感）。所以把 Player::step 的整段移動數學搬到這裡：
// 主 crate 的 `Player::step` 與下方 wasm 出口 `step_player` 都是薄包裝。

/// 玩家移動速度（px/s）。主 crate `state::PLAYER_SPEED` re-export 這份。
pub const PLAYER_SPEED: f32 = 320.0;
/// 玩家碰撞盒半徑（px）。四角判定用；略小於半格（16px）讓走 32px 隧道不卡。
pub const PLAYER_TILE_RADIUS: f32 = 8.0;

/// 世界像素座標 → (chunk x, chunk y, 格 x, 格 y)。與主 crate `tiles::world_to_cell`
/// 同一份（主 crate re-export 這份）——f32 除法語意，伺服器與 wasm 逐位元一致。
pub fn world_to_cell(wx: f32, wy: f32) -> (i32, i32, u8, u8) {
    let gx = (wx / TILE_PX).floor() as i32;
    let gy = (wy / TILE_PX).floor() as i32;
    let n = TILES_PER_CHUNK as i32;
    (
        gx.div_euclid(n),
        gy.div_euclid(n),
        gx.rem_euclid(n) as u8,
        gy.rem_euclid(n) as u8,
    )
}

/// 依方向鍵把位置往前推進 `dt` 秒（含對角線正規化、水域阻擋、實心格碰撞解算）。
/// `tile_solid(x, y)`：該世界像素座標是否為實心地形格（呼叫端決定 delta 來源）。
///
/// 規則（原 `Player::step`，逐行搬移、行為不變）：
/// - 「中心落在實心格」→ 受困（傳送落地等罕見情況），以中心點判斷逃脫、允許自由移動；
///   一般走路用碰撞盒四角（半徑 `PLAYER_TILE_RADIUS`）精準阻擋、可沿牆滑行。
/// - 水域同樣四角判定；已陷在水裡（中心在水）改用中心判定，保留逃脫通道。
pub fn step_with_keys<F: Fn(f32, f32) -> bool>(
    x: f32,
    y: f32,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    dt: f32,
    tile_solid: F,
) -> (f32, f32) {
    let mut dx = 0.0_f32;
    let mut dy = 0.0_f32;
    if up {
        dy -= 1.0;
    }
    if down {
        dy += 1.0;
    }
    if left {
        dx -= 1.0;
    }
    if right {
        dx += 1.0;
    }
    // 對角線正規化，避免斜走變快。
    if dx != 0.0 && dy != 0.0 {
        let inv = 1.0 / (2.0_f32).sqrt();
        dx *= inv;
        dy *= inv;
    }
    let new_x = x + dx * PLAYER_SPEED * dt;
    let new_y = y + dy * PLAYER_SPEED * dt;
    let r = PLAYER_TILE_RADIUS;
    let is_center_stuck = tile_solid(x, y);
    let is_on_water = biome_at(x as f64, y as f64) == Biome::Water;
    let corners = [(r, r), (-r, r), (r, -r), (-r, -r)];
    let any_corner = |cx: f32, cy: f32| corners.iter().any(|&(ox, oy)| tile_solid(cx + ox, cy + oy));
    let water_corner = |cx: f32, cy: f32| {
        corners
            .iter()
            .any(|&(ox, oy)| biome_at((cx + ox) as f64, (cy + oy) as f64) == Biome::Water)
    };
    resolve_move(x, y, new_x, new_y, |px, py| {
        // 一般時水域用四角；已陷在水裡時改用中心，留逃脫通道、不卡死。
        let water_blocked = if is_on_water {
            biome_at(px as f64, py as f64) == Biome::Water
        } else {
            water_corner(px, py)
        };
        if water_blocked {
            return true;
        }
        // 受困時以中心點判定（保留逃脫通道）；一般時以四角判定（精準碰牆）。
        if is_center_stuck {
            tile_solid(px, py)
        } else {
            any_corner(px, py)
        }
    })
}

// ── wasm 出口：地形差異儲存 + 移動預測 ────────────────────────────────────────
//
// 前端把伺服器廣播的地形差異（挖/放過的格）餵進 `tile_delta_set`，`step_player` 的
// 碰撞就跟伺服器看到同一個世界（含玩家蓋的牆/挖開的洞），預測才不會在自家門口撞鬼。
// wasm 單執行緒，thread_local 即可；BTreeMap（不用 HashMap）避免 wasm32 取亂數種子問題。

use std::cell::{Cell, RefCell};

thread_local! {
    /// (chunk x, chunk y, 格 x, 格 y) → TileKind::code()。
    static TILE_DELTAS: RefCell<std::collections::BTreeMap<(i32, i32, u8, u8), u8>> =
        RefCell::new(std::collections::BTreeMap::new());
    /// step_player 的輸出（extern "C" 不便回傳 tuple，用 getter 取）。
    static STEP_OUT: Cell<(f64, f64)> = Cell::new((0.0, 0.0));
}

/// wasm/前端入口：記錄一筆地形差異（挖=code 0 / 放=非 0）。同格重複呼叫＝覆蓋。
#[no_mangle]
pub extern "C" fn tile_delta_set(cx: i32, cy: i32, tx: u32, ty: u32, kind_code: u32) {
    TILE_DELTAS.with(|m| {
        m.borrow_mut()
            .insert((cx, cy, tx as u8, ty as u8), kind_code as u8);
    });
}

/// wasm/前端入口：清空全部地形差異（重連重播前呼叫，避免殘留舊世界）。
#[no_mangle]
pub extern "C" fn tile_delta_clear() {
    TILE_DELTAS.with(|m| m.borrow_mut().clear());
}

/// wasm/前端入口：客戶端移動預測一步。`keys` 位元旗標：1=上 2=下 4=左 8=右。
/// 碰撞判定與伺服器 `Player::step` 完全相同（同一份 `step_with_keys` + 同款 delta 查表）。
/// 結果經 `step_out_x` / `step_out_y` 取回。
#[no_mangle]
pub extern "C" fn step_player(x: f64, y: f64, keys: u32, dt: f64) {
    let solid = |px: f32, py: f32| {
        let key = world_to_cell(px, py);
        let delta = TILE_DELTAS.with(|m| m.borrow().get(&key).copied());
        match delta {
            Some(code) => code != 0, // 0 = Empty
            None => tile_kind_at(px as f64, py as f64) != TileKind::Empty,
        }
    };
    let (nx, ny) = step_with_keys(
        x as f32,
        y as f32,
        keys & 1 != 0,
        keys & 2 != 0,
        keys & 4 != 0,
        keys & 8 != 0,
        dt as f32,
        solid,
    );
    STEP_OUT.with(|c| c.set((nx as f64, ny as f64)));
}

/// 取回最近一次 `step_player` 的結果 X。
#[no_mangle]
pub extern "C" fn step_out_x() -> f64 {
    STEP_OUT.with(|c| c.get().0)
}

/// 取回最近一次 `step_player` 的結果 Y。
#[no_mangle]
pub extern "C" fn step_out_y() -> f64 {
    STEP_OUT.with(|c| c.get().1)
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
    fn tile_kind_same_across_tile() {
        // 隱形牆根治：同格所有取樣點（含四角、中心）必須返回完全相同的地形種類。
        // 若此測試失敗，表示 snap 到格中心的邏輯被破壞——渲染/碰撞再度發散。
        let offsets: &[(f64, f64)] = &[
            (0.5, 0.5),   // 中心
            (0.0, 0.0),   // 左上角
            (31.9, 0.0),  // 右上角
            (0.0, 31.9),  // 左下角
            (31.9, 31.9), // 右下角
            (16.0, 0.0),  // 上邊中
            (0.0, 16.0),  // 左邊中
        ];
        // 抽樣大量格（含城鎮外、各生態域、負座標）
        for gy in (-5..=30i32).step_by(3) {
            for gx in (-5..=30i32).step_by(3) {
                let base_x = gx as f64 * 32.0;
                let base_y = gy as f64 * 32.0;
                let expected = tile_kind_at(base_x + 16.0, base_y + 16.0); // 中心
                for &(ox, oy) in offsets {
                    let got = tile_kind_at(base_x + ox, base_y + oy);
                    assert_eq!(
                        got, expected,
                        "格({gx},{gy}) 在偏移({ox},{oy}) 返回 {got:?}，中心返回 {expected:?}——隱形牆！"
                    );
                }
            }
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
        assert_eq!(TileKind::JadeVine.code(),    9);
    }

    #[test]
    fn jade_vine_exists_in_verdant_zone() {
        // 翠幽星（X ≥ 20000）應存在翠玉藤格。
        let mut found = false;
        'outer: for gy in 0..200i32 {
            for gx in 0..200i32 {
                let x = VERDANT_ZONE_MIN_X + gx as f64 * 32.0;
                let y = gy as f64 * 32.0;
                if biome_at(x, y) != Biome::Water && tile_kind_at(x + 16.0, y + 16.0) == TileKind::JadeVine {
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "翠幽星應存在翠玉藤格（JadeVine）");
    }

    #[test]
    fn jade_vine_only_in_verdant_zone() {
        // JadeVine 不應出現在 X < 20000 的普通世界。
        for gy in 0..100i32 {
            for gx in 0..100i32 {
                let x = gx as f64 * 64.0 + 16.0;
                let y = gy as f64 * 64.0 + 16.0;
                // 只掃 X < VERDANT_ZONE_MIN_X 的區域
                assert!(x < VERDANT_ZONE_MIN_X, "測試設計錯誤：掃到翠幽星了");
                let k = tile_kind_at(x, y);
                assert_ne!(k, TileKind::JadeVine, "普通世界 ({x},{y}) 不應生翠玉藤");
            }
        }
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
        // D-2 反轉後原本預期 62%，但為了移動流暢度已調降（Rocky 50%、其餘 18%）。
        assert!(ratio > 0.05 && ratio < 0.60, "實心比例偏離預期，實際={:.1}%", ratio * 100.0);
    }

    #[test]
    fn tile_code_includes_lava_rock() {
        assert_eq!(TileKind::LavaRock.code(), 10);
    }

    #[test]
    fn lava_rock_exists_in_crimson_zone() {
        // 赤焰星（X ≤ -15000）應存在熔岩石格。
        let mut found = false;
        'outer: for gy in 0..200i32 {
            for gx in 0..200i32 {
                let x = CRIMSON_ZONE_MAX_X - gx as f64 * 32.0;
                let y = gy as f64 * 32.0;
                if biome_at(x, y) != Biome::Water && tile_kind_at(x + 16.0, y + 16.0) == TileKind::LavaRock {
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "赤焰星應存在熔岩石格（LavaRock）");
    }

    #[test]
    fn lava_rock_only_in_crimson_zone() {
        // LavaRock 不應出現在 X > -15000 的普通世界（故鄉 / 翠幽星）。
        for gy in 0..100i32 {
            for gx in 0..100i32 {
                let x = gx as f64 * 64.0 + 16.0;
                let y = gy as f64 * 64.0 + 16.0;
                let k = tile_kind_at(x, y);
                assert_ne!(k, TileKind::LavaRock, "普通世界 ({x},{y}) 不應生熔岩石");
            }
        }
    }

    #[test]
    fn tile_code_includes_aether_mist() {
        assert_eq!(TileKind::AetherMist.code(), 12);
    }

    #[test]
    fn aether_mist_exists_in_aether_zone() {
        // 霧醚星（X ≤ -30000）應存在霧醚晶霧格。
        let mut found = false;
        'outer: for gy in 0..200i32 {
            for gx in 0..200i32 {
                let x = AETHER_ZONE_MAX_X - gx as f64 * 32.0;
                let y = gy as f64 * 32.0;
                if biome_at(x, y) != Biome::Water && tile_kind_at(x + 16.0, y + 16.0) == TileKind::AetherMist {
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "霧醚星應存在霧醚晶霧格（AetherMist）");
    }

    #[test]
    fn aether_mist_only_in_aether_zone() {
        // AetherMist 不應出現在 X > -30000 的普通世界（故鄉 / 翠幽星 / 赤焰星）。
        for gy in 0..100i32 {
            for gx in 0..100i32 {
                let x = gx as f64 * 64.0 + 16.0;
                let y = gy as f64 * 64.0 + 16.0;
                let k = tile_kind_at(x, y);
                assert_ne!(k, TileKind::AetherMist, "普通世界 ({x},{y}) 不應生霧醚晶霧");
            }
        }
    }

    #[test]
    fn lava_rock_not_in_aether_zone() {
        // 霧醚星區域（X ≤ -30000）內不應生成熔岩石（AetherMist 已覆蓋赤焰星邏輯）。
        let mut lava_count = 0usize;
        for gy in 0..50i32 {
            for gx in 0..50i32 {
                let x = AETHER_ZONE_MAX_X - 2000.0 - gx as f64 * 32.0;
                let y = gy as f64 * 32.0;
                if tile_kind_at(x + 16.0, y + 16.0) == TileKind::LavaRock {
                    lava_count += 1;
                }
            }
        }
        assert_eq!(lava_count, 0, "霧醚星區域不應生成熔岩石格");
    }

    #[test]
    fn town_walls_gates_and_interior() {
        // 主城（格 73,71、half 34）：牆格=TownWall、城門格=Empty、牆內=Empty、
        // 城門上下兩格仍是牆（門只開正中 2*gate_half+1=3 格）。
        let t = &TOWNS[0];
        let px = |gx: i32, gy: i32| (gx as f64 * 32.0 + 16.0, gy as f64 * 32.0 + 16.0);
        // 北牆（門外 5 格處）是牆
        let (wx, wy) = px(t.cgx + 5, t.cgy - t.half_tiles);
        assert_eq!(tile_kind_at(wx, wy), TileKind::TownWall, "北牆應為城牆");
        // 北門正中是開口
        let (gx2, gy2) = px(t.cgx, t.cgy - t.half_tiles);
        assert_eq!(tile_kind_at(gx2, gy2), TileKind::Empty, "城門開口應淨空");
        // 門邊第 2 格（perp=2 > gate_half=1）回到牆
        let (ex, ey) = px(t.cgx + 2, t.cgy - t.half_tiles);
        assert_eq!(tile_kind_at(ex, ey), TileKind::TownWall, "門邊第 2 格應為牆");
        // 牆內任意格淨空
        let (ix, iy) = px(t.cgx + 10, t.cgy - 10);
        assert_eq!(tile_kind_at(ix, iy), TileKind::Empty, "牆內應淨空");
        // 角落必為牆（門不開在角落）
        let (cx2, cy2) = px(t.cgx + t.half_tiles, t.cgy + t.half_tiles);
        assert_eq!(tile_kind_at(cx2, cy2), TileKind::TownWall, "角落應為牆");
        // 每座城鎮都有牆＋淨空中心
        for t in TOWNS {
            let (wx, wy) = px(t.cgx + 5, t.cgy - t.half_tiles);
            assert_eq!(tile_kind_at(wx, wy), TileKind::TownWall, "{} 北牆", t.name);
            let (mx, my) = px(t.cgx, t.cgy);
            assert_eq!(tile_kind_at(mx, my), TileKind::Empty, "{} 中心", t.name);
        }
    }

    #[test]
    fn town_protected_covers_interior_plus_margin() {
        let t = &TOWNS[0];
        let c = |gx: i32| gx as f64 * 32.0 + 16.0;
        assert!(town_protected_at(c(t.cgx), c(t.cgy)), "城中心在保護圈");
        assert!(
            town_protected_at(c(t.cgx + t.half_tiles + 8), c(t.cgy)),
            "牆外 8 格緩衝仍在保護圈"
        );
        assert!(
            !town_protected_at(c(t.cgx + t.half_tiles + 9), c(t.cgy)),
            "緩衝外不在保護圈"
        );
    }

    #[test]
    fn old_safe_zone_circle_is_inside_main_town() {
        // 舊圓形安全區（640px）必須被主城方形完整涵蓋——出生點/公共農地的「乾淨地」
        // 保證不因改制而縮水（改一邊要想到另一邊的相容）。
        for a in 0..72 {
            let th = a as f64 * std::f64::consts::PI / 36.0;
            let x = SAFE_ZONE_CX + SAFE_ZONE_RADIUS * th.cos();
            let y = SAFE_ZONE_CY + SAFE_ZONE_RADIUS * th.sin();
            assert!(town_interior_at(x, y), "舊安全圓邊界點 ({x:.0},{y:.0}) 應仍在主城內");
        }
    }

    #[test]
    fn step_with_keys_moves_at_player_speed() {
        // 開闊地直走 1 秒 = PLAYER_SPEED px；斜走有正規化、總位移相同。
        // (1360, 200) 是已驗證的無水開闊地（state.rs 速度測試同款座標）。
        let (nx, ny) = step_with_keys(1360.0, 200.0, false, false, false, true, 1.0, |_, _| false);
        assert!((nx - (1360.0 + PLAYER_SPEED)).abs() < 0.001, "nx={nx}");
        assert_eq!(ny, 200.0);
        let (dx, dy) = step_with_keys(1360.0, 200.0, false, true, false, true, 1.0, |_, _| false);
        let dist = ((dx - 1360.0).powi(2) + (dy - 200.0).powi(2)).sqrt();
        assert!((dist - PLAYER_SPEED).abs() < 0.01, "斜走位移 {dist} 應等於 PLAYER_SPEED");
    }

    #[test]
    fn wasm_step_player_blocked_by_delta_wall_and_freed_by_dig() {
        // wasm 預測必須看得到前端餵入的地形差異：玩家放一格牆 → step_player 被擋；
        // 「挖掉」該格（delta 設 0）→ 立刻能通過。出發點取新手村安全區（保證無天然實心）。
        tile_delta_clear();
        let (sx, sy) = (SAFE_ZONE_CX as f32, SAFE_ZONE_CY as f32);
        // 右方兩格放一面牆（kind=2 Stone）
        let wall_x = sx + TILE_PX * 2.0;
        let (cx, cy, tx, ty) = world_to_cell(wall_x, sy);
        tile_delta_set(cx, cy, tx as u32, ty as u32, 2);
        // 往右走 1 秒（足以撞上）：應被牆擋下（碰撞盒右緣不越過牆左緣）。
        step_player(sx as f64, sy as f64, 8, 1.0);
        let bx = step_out_x() as f32;
        let wall_left = (wall_x / TILE_PX).floor() * TILE_PX;
        assert!(
            bx + PLAYER_TILE_RADIUS <= wall_left + 1.0,
            "應被 delta 牆擋下：bx={bx} 牆左緣={wall_left}"
        );
        assert!(bx > sx, "撞牆前應該有前進（沿路滑到牆邊）");
        // 挖掉那格 → 同樣輸入應走得遠超過牆位置。
        tile_delta_set(cx, cy, tx as u32, ty as u32, 0);
        step_player(sx as f64, sy as f64, 8, 1.0);
        let fx = step_out_x() as f32;
        assert!(fx > wall_left + TILE_PX, "挖開後應通過：fx={fx}");
        tile_delta_clear();
    }

    #[test]
    fn wasm_step_player_matches_step_with_keys() {
        // 無 delta 時，wasm 出口必須與原生 step_with_keys 完全等價（同一份碰撞閉包語意）。
        tile_delta_clear();
        let solid = |px: f32, py: f32| tile_kind_at(px as f64, py as f64) != TileKind::Empty;
        for (x, y, keys) in [
            (2344.0_f32, 2296.0_f32, 8_u32), // 村中心往右
            (1360.0, 200.0, 2 | 8),          // 開闊地右下斜走
            (3000.0, 3000.0, 1),             // 世界中心往上
        ] {
            let (ex, ey) = step_with_keys(
                x, y,
                keys & 1 != 0, keys & 2 != 0, keys & 4 != 0, keys & 8 != 0,
                0.5, solid,
            );
            step_player(x as f64, y as f64, keys, 0.5);
            assert_eq!(step_out_x() as f32, ex, "x 不等價 @({x},{y},keys={keys})");
            assert_eq!(step_out_y() as f32, ey, "y 不等價 @({x},{y},keys={keys})");
        }
    }

    #[test]
    fn wasm_exports_match_native_logic() {
        // wasm 出口（biome_code / tile_kind_code）必須與原生 API 完全等價——
        // 前端載入 .wasm 後呼叫的就是這兩支，等價性是「前後端同一份地形」的根基。
        // 取樣涵蓋安全區、各生態域、各星區（含負座標）與非整數座標。
        let pts: [(f64, f64); 9] = [
            (2344.0, 2296.0),       // 新手村安全區中心
            (1360.5, 200.25),       // 一般陸地（非整數座標）
            (3000.0, 3000.0),       // 世界中心
            (25000.0, 1234.0),      // 翠幽星
            (40000.0, -500.0),      // 虛空星
            (-16000.0, 800.0),      // 赤焰星
            (-31000.0, -2500.0),    // 霧醚星
            (-51000.0, 999.0),      // 星源星
            (-123.75, 456.5),       // 負座標
        ];
        for (x, y) in pts {
            assert_eq!(
                biome_code(x, y),
                biome_at(x, y).code(),
                "biome_code 與 biome_at 不一致 @({x},{y})"
            );
            assert_eq!(
                tile_kind_code(x, y),
                tile_kind_at(x, y).code() as u32,
                "tile_kind_code 與 tile_kind_at 不一致 @({x},{y})"
            );
        }
    }

    #[test]
    fn tile_kind_codes_are_stable() {
        // 整數編碼是 wasm 邊界 / 前端對照表 / DB 的穩定契約——別重排。
        // 前端 web/game.js 的 WASM_TILE_NAMES 順序靠這份編碼；改一邊必改另一邊。
        assert_eq!(TileKind::Empty.code(), 0);
        assert_eq!(TileKind::Dirt.code(), 1);
        assert_eq!(TileKind::Stone.code(), 2);
        assert_eq!(TileKind::Ore.code(), 3);
        assert_eq!(TileKind::Crystal.code(), 4);
        assert_eq!(TileKind::Mushroom.code(), 5);
        assert_eq!(TileKind::AncientRuin.code(), 6);
        assert_eq!(TileKind::CoralReef.code(), 7);
        assert_eq!(TileKind::WildFlower.code(), 8);
        assert_eq!(TileKind::JadeVine.code(), 9);
        assert_eq!(TileKind::LavaRock.code(), 10);
        assert_eq!(TileKind::VoidCrystal.code(), 11);
        assert_eq!(TileKind::AetherMist.code(), 12);
        assert_eq!(TileKind::OriginCrystal.code(), 13);
        assert_eq!(TileKind::TownWall.code(), 14);
        assert_eq!(Biome::Water.code(), 0);
        assert_eq!(Biome::Sand.code(), 1);
        assert_eq!(Biome::Meadow.code(), 2);
        assert_eq!(Biome::Forest.code(), 3);
        assert_eq!(Biome::Rocky.code(), 4);
    }
}
