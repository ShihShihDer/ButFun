//! Voxel 世界核心（AI 生態世界 voxel 基底·切片①）。
//!
//! 這裡是「方塊世界的真相」——與渲染無關的確定性世界邏輯，延續 ButFun
//! 「後端權威、前端只畫」的骨架（對齊 `world-core` 的設計哲學，但 voxel 的東西
//! 刻意**全部隔離在本模組**，不碰 game/state/ws/protocol）。
//!
//! 切片① 只做「可走的 voxel 世界」：
//! - 方塊型別 `Block`、16³ 的 `Chunk`、`ChunkCoord` 索引。
//! - 自寫 hash value noise → 確定性程序化地形（高度圖 + 分層填方塊）。
//! - `pack_chunk`：把一個 chunk 壓成精簡 base64，供 WS 串給前端；全空氣回 `None`。
//!
//! 設計取捨：本輪伺服器其實**無狀態程序生成**——不在記憶體存 chunk，收到請求就地用噪聲
//! 算出來。省記憶體、天然無限延伸；之後切片②（放/挖方塊）再加「方塊改動 overlay」即可。
//! 一切函式皆確定性純函式，好測也讓多人之間天然一致（同座標永遠同方塊）。

use base64::Engine;

/// 一個 chunk 的邊長（方塊數）。16³ = 4096 方塊／chunk。
pub const CHUNK: i32 = 16;
/// 一個 chunk 的方塊總數（4096）。
pub const CHUNK_VOL: usize = (CHUNK * CHUNK * CHUNK) as usize;

/// 地形基準高度（世界方塊 Y）。噪聲在此之上下起伏。
pub const BASE_HEIGHT: i32 = 8;
/// 海平面（世界方塊 Y）。高度低於此、且在地表之上的空格填水。
pub const SEA_LEVEL: i32 = 5;
/// 地形噪聲種子（固定 → 全世界、前後端、多人之間地貌一致）。
pub const SEED: u32 = 0x_B0_07_Fu32; // "BOOTF"un · voxel

/// 方塊型別。`#[repr(u8)]` → 直接當 1 byte 串流（pack_chunk 用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Block {
    Air = 0,
    Grass = 1,
    Dirt = 2,
    Stone = 3,
    Sand = 4,
    Wood = 5,
    Leaves = 6,
    Water = 7,
}

impl Block {
    /// 是否為「實心、可站立／會擋路」的方塊（碰撞與面剔除用）。水與空氣不算實心。
    pub fn is_solid(self) -> bool {
        !matches!(self, Block::Air | Block::Water)
    }
}

/// chunk 在世界中的座標（以 chunk 為單位，每軸 ×CHUNK 才是世界方塊座標）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkCoord {
    pub cx: i32,
    pub cy: i32,
    pub cz: i32,
}

/// 一個 chunk 的方塊資料（行主序 `x + z*CHUNK + y*CHUNK*CHUNK`）。
#[derive(Clone, Debug)]
pub struct Chunk {
    pub blocks: Vec<u8>,
}

/// chunk 內 (lx,ly,lz) → 一維索引。座標必須在 0..CHUNK。抽出來好測、好對齊前端解碼。
#[inline]
pub fn local_index(lx: i32, ly: i32, lz: i32) -> usize {
    debug_assert!((0..CHUNK).contains(&lx) && (0..CHUNK).contains(&ly) && (0..CHUNK).contains(&lz));
    (lx + lz * CHUNK + ly * CHUNK * CHUNK) as usize
}

/// 世界方塊座標 → 所屬 chunk 座標（對負數做 floor 除法，避免 -1 落到 chunk 0）。
#[inline]
pub fn chunk_of(wx: i32, wy: i32, wz: i32) -> ChunkCoord {
    ChunkCoord {
        cx: wx.div_euclid(CHUNK),
        cy: wy.div_euclid(CHUNK),
        cz: wz.div_euclid(CHUNK),
    }
}

// ── 自寫 hash value noise（零外部相依、確定性、可測；不抄外部碼）─────────────────

/// 整數座標 → [0,1) 的確定性雜湊（用幾個質數攪和 + xorshift finalize）。
#[inline]
fn hash2(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32)
        .wrapping_mul(0x_27d4_eb2d)
        .wrapping_add((z as u32).wrapping_mul(0x_9e37_79b1))
        .wrapping_add(seed);
    h ^= h >> 15;
    h = h.wrapping_mul(0x_85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0x_c2b2_ae35);
    h ^= h >> 16;
    (h as f32) / (u32::MAX as f32)
}

/// 二維 value noise：格點雜湊 + smoothstep 雙線性內插 → 平滑 [0,1)。
fn value_noise(x: f32, z: f32, seed: u32) -> f32 {
    let x0 = x.floor() as i32;
    let z0 = z.floor() as i32;
    let fx = x - x0 as f32;
    let fz = z - z0 as f32;
    // smoothstep，讓格點之間平滑（不然會看到方塊狀梯田）。
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sz = fz * fz * (3.0 - 2.0 * fz);
    let n00 = hash2(x0, z0, seed);
    let n10 = hash2(x0 + 1, z0, seed);
    let n01 = hash2(x0, z0 + 1, seed);
    let n11 = hash2(x0 + 1, z0 + 1, seed);
    let nx0 = n00 + (n10 - n00) * sx;
    let nx1 = n01 + (n11 - n01) * sx;
    nx0 + (nx1 - nx0) * sz
}

/// 地表高度（世界方塊 Y）：多 octave value noise 疊加。確定性 → 同 (wx,wz) 永遠同高度。
pub fn height_at(wx: i32, wz: i32) -> i32 {
    let x = wx as f32;
    let z = wz as f32;
    // 大尺度起伏（丘陵/盆地）+ 中尺度細節。把 value_noise(0..1) 平移成「以 0 為中心」
    // (n-0.5) → 高度能高於也能低於 BASE_HEIGHT，才會生出低於海平面的窪地 → 湖泊/海。
    // 頻率/振幅手調到「平緩可走、偶有小丘與水塘」，且峰值不超出本輪垂直 chunk 範圍(y<32)。
    let mut h = 0.0_f32;
    h += (value_noise(x / 48.0, z / 48.0, SEED) - 0.5) * 16.0;
    h += (value_noise(x / 18.0, z / 18.0, SEED ^ 0x_9e37_79b9) - 0.5) * 5.0;
    h += (value_noise(x / 7.0, z / 7.0, SEED ^ 0x_1234_5678) - 0.5) * 2.0;
    BASE_HEIGHT + h.round() as i32
}

/// 任一世界座標的方塊（確定性程序生成）。這是「無狀態世界」的核心查詢。
pub fn block_at(wx: i32, wy: i32, wz: i32) -> Block {
    // 地心一律基岩石頭（避免從世界底掉出去；本輪只生成 y>=0 的 chunk）。
    if wy < 0 {
        return Block::Stone;
    }
    let h = height_at(wx, wz);
    if wy > h {
        // 地表之上：海平面（含）以下補水，否則空氣。
        if wy <= SEA_LEVEL {
            return Block::Water;
        }
        return Block::Air;
    }
    if wy == h {
        // 地表層：近海平面用沙，否則草。
        if h <= SEA_LEVEL + 1 {
            return Block::Sand;
        }
        return Block::Grass;
    }
    // 地表以下：上面幾層土，再下石頭。
    if wy >= h - 3 {
        return Block::Dirt;
    }
    Block::Stone
}

/// 生成一整個 chunk（就地用 `block_at` 填）。供需要實體 chunk 的場合（如測試）。
pub fn generate_chunk(coord: ChunkCoord) -> Chunk {
    let mut blocks = vec![0u8; CHUNK_VOL];
    let base_x = coord.cx * CHUNK;
    let base_y = coord.cy * CHUNK;
    let base_z = coord.cz * CHUNK;
    for ly in 0..CHUNK {
        for lz in 0..CHUNK {
            for lx in 0..CHUNK {
                let b = block_at(base_x + lx, base_y + ly, base_z + lz);
                blocks[local_index(lx, ly, lz)] = b as u8;
            }
        }
    }
    Chunk { blocks }
}

/// 把一個 chunk 壓成精簡 base64（4096 bytes → ~5.5KB 字串）供 WS 串流。
/// 全空氣的 chunk 回 `None`——呼叫端據此不傳（高空 chunk 幾乎都被略過，省大量頻寬）。
pub fn pack_chunk(coord: ChunkCoord) -> Option<String> {
    let chunk = generate_chunk(coord);
    if chunk.blocks.iter().all(|&b| b == Block::Air as u8) {
        return None;
    }
    Some(base64::engine::general_purpose::STANDARD.encode(&chunk.blocks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_index_roundtrip_is_unique() {
        // 每個 (lx,ly,lz) 應映到唯一索引，且覆蓋滿 0..CHUNK_VOL。
        let mut seen = vec![false; CHUNK_VOL];
        for ly in 0..CHUNK {
            for lz in 0..CHUNK {
                for lx in 0..CHUNK {
                    let i = local_index(lx, ly, lz);
                    assert!(i < CHUNK_VOL);
                    assert!(!seen[i], "索引重複 @ {lx},{ly},{lz}");
                    seen[i] = true;
                }
            }
        }
        assert!(seen.iter().all(|&s| s), "有索引沒被覆蓋到");
    }

    #[test]
    fn chunk_of_handles_negatives() {
        // 正常正座標。
        assert_eq!(chunk_of(0, 0, 0), ChunkCoord { cx: 0, cy: 0, cz: 0 });
        assert_eq!(chunk_of(15, 15, 15), ChunkCoord { cx: 0, cy: 0, cz: 0 });
        assert_eq!(chunk_of(16, 16, 16), ChunkCoord { cx: 1, cy: 1, cz: 1 });
        // 負座標必須 floor 到 -1，不能落回 0（不然會錯位）。
        assert_eq!(chunk_of(-1, -1, -1), ChunkCoord { cx: -1, cy: -1, cz: -1 });
        assert_eq!(chunk_of(-16, -16, -16), ChunkCoord { cx: -1, cy: -1, cz: -1 });
        assert_eq!(chunk_of(-17, 0, 0).cx, -2);
    }

    #[test]
    fn terrain_is_deterministic() {
        // 同座標多次查詢必須完全一致（多人/前後端一致的根本保證）。
        for &(x, z) in &[(0, 0), (100, -50), (-12345, 6789), (5, 5)] {
            let h1 = height_at(x, z);
            let h2 = height_at(x, z);
            assert_eq!(h1, h2);
            let b1 = block_at(x, h1, z);
            let b2 = block_at(x, h1, z);
            assert_eq!(b1, b2);
        }
    }

    #[test]
    fn surface_and_layers_make_sense() {
        // 取一個陸地點（找個高度明顯高於海平面的座標）。
        let (mut x, mut z) = (0, 0);
        for cand in 0..2000 {
            if height_at(cand, 0) > SEA_LEVEL + 2 {
                x = cand;
                z = 0;
                break;
            }
        }
        let h = height_at(x, z);
        assert!(h > SEA_LEVEL + 1, "測試點應在海平面之上的陸地");
        // 地表是草、其下是土、再下是石、其上是空氣。
        assert_eq!(block_at(x, h, z), Block::Grass);
        assert_eq!(block_at(x, h - 1, z), Block::Dirt);
        assert_eq!(block_at(x, h - 8, z), Block::Stone);
        assert_eq!(block_at(x, h + 1, z), Block::Air);
        // 地心是石頭。
        assert_eq!(block_at(x, -5, z), Block::Stone);
    }

    #[test]
    fn sea_fills_with_water() {
        // 找一個地表低於海平面的點（窪地）→ 海平面以下的空格應是水。
        let mut found = false;
        for cand in 0..5000 {
            if height_at(cand, 17) < SEA_LEVEL {
                let h = height_at(cand, 17);
                // 地表之上、海平面之下 → 水。
                assert_eq!(block_at(cand, h + 1, 17), Block::Water);
                found = true;
                break;
            }
        }
        assert!(found, "5000 格內應找得到一個低於海平面的窪地");
    }

    #[test]
    fn is_solid_matrix() {
        assert!(!Block::Air.is_solid());
        assert!(!Block::Water.is_solid());
        assert!(Block::Grass.is_solid());
        assert!(Block::Stone.is_solid());
        assert!(Block::Wood.is_solid());
    }

    #[test]
    fn pack_chunk_skips_all_air_but_packs_ground() {
        // 高空 chunk（cy 很大）應全空氣 → None。
        let high = pack_chunk(ChunkCoord { cx: 0, cy: 20, cz: 0 });
        assert!(high.is_none(), "高空 chunk 該被略過");
        // 地面 chunk（cy=0）一定有方塊 → Some，且解碼後長度正確。
        let ground = pack_chunk(ChunkCoord { cx: 0, cy: 0, cz: 0 });
        let s = ground.expect("地面 chunk 應有方塊");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(s)
            .expect("應為合法 base64");
        assert_eq!(bytes.len(), CHUNK_VOL);
    }

    #[test]
    fn generate_chunk_matches_block_at() {
        // generate_chunk 必須與逐點 block_at 完全一致（串流與查詢同源）。
        let coord = ChunkCoord { cx: 3, cy: 0, cz: -2 };
        let chunk = generate_chunk(coord);
        for ly in 0..CHUNK {
            for lz in 0..CHUNK {
                for lx in 0..CHUNK {
                    let expect = block_at(
                        coord.cx * CHUNK + lx,
                        coord.cy * CHUNK + ly,
                        coord.cz * CHUNK + lz,
                    ) as u8;
                    assert_eq!(chunk.blocks[local_index(lx, ly, lz)], expect);
                }
            }
        }
    }
}
