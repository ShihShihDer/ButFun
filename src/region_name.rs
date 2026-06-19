//! 天地有名（ROADMAP 398）：把世界座標映射成「在地地名」(locale)，世界第一次有「地方感」。
//!
//! 純函式、確定性、零狀態、零持久化——**同一格座標永遠回同一個地名**。
//! 玩家踏入新 locale 時前端淡入一張地名卡，小地圖角落也常駐顯示「你在哪裡」。
//!
//! locale ＝以 [`REGION_SIZE`] 為邊長、對齊原點的方格；名稱由「該格中心主生態(biome) ＋
//! 確定性雜湊選詞」決定（與星球大區無關，純疊在生態之上）。療癒基調：詞庫只用溫柔、
//! 可安居的字眼，無威脅感。面向玩家字串集中在本檔詞庫（i18n 替換點）。

use world_core::Biome;

/// 一個 locale 方格邊長（像素）。玩家速度 ~180px/s，走過一格約 8~9 秒——
/// 夠久不會頻繁刷地名，夠短在一場遊玩會自然經過好幾個地方。
pub const REGION_SIZE: f64 = 1536.0;

/// 一處在地地名：穩定 id ＋地名 ＋一句地誌氛圍副標。三者都已是面向玩家字串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    /// locale 穩定識別（格座標編碼）。前端用來偵測「是否換了地方」，不直接顯示。
    pub id: i64,
    /// 地名，如「晨露谷」。
    pub name: &'static str,
    /// 副標氛圍句，如「薄霧在草尖上打盹」。
    pub subtitle: &'static str,
}

// ── 各生態的地名詞庫（確定性挑選；只用溫柔字眼，守療癒基調）──────────────
const MEADOW_NAMES: &[&str] = &["晨露谷", "微風原", "蜜光草甸", "搖籃丘", "綠歌平野", "暖陽牧野"];
const FOREST_NAMES: &[&str] = &["翡翠林", "靜謐樹海", "苔影森", "低語密林", "蕨夢林", "松針幽徑"];
const SAND_NAMES: &[&str] = &["鏽金沙丘", "旅人荒原", "暖砂台地", "落日沙原", "風紋戈壁", "駝鈴古道"];
const ROCKY_NAMES: &[&str] = &["灰岩崗", "回聲峭壁", "礦脈高地", "石爐山脊", "古岩台", "蒼石關"];
const WATER_NAMES: &[&str] = &["粼光淺灘", "靜湖之濱", "薄霧水澤", "鏡面灣", "蘆葦渡", "月泊潟湖"];

// ── 各生態的副標氛圍句（與地名各自獨立挑選，組合更有變化）──────────────
const MEADOW_SUBS: &[&str] =
    &["薄霧在草尖上打盹", "野花一路鋪到天邊", "風把草浪梳成同一個方向", "陽光在這裡走得很慢"];
const FOREST_SUBS: &[&str] =
    &["樹影裡藏著舊日的低語", "苔蘚把石頭都養綠了", "光從葉縫漏成一地碎金", "深處傳來不知名的鳥鳴"];
const SAND_SUBS: &[&str] =
    &["風紋在腳邊悄悄改寫", "熱氣讓遠方微微搖晃", "沙裡或許埋著古老的故事", "夕照把沙染成蜜色"];
const ROCKY_SUBS: &[&str] =
    &["每一步都聽得見回聲", "岩層記著很久很久的事", "風從石縫間穿過唱歌", "礦脈在腳下靜靜延伸"];
const WATER_SUBS: &[&str] =
    &["水面把天空收進懷裡", "蘆葦在淺處輕輕點頭", "波光一圈圈漾開又合攏", "霧氣貼著水面慢慢走"];

/// 取某生態的（地名庫, 副標庫）。
fn bank_for(biome: Biome) -> (&'static [&'static str], &'static [&'static str]) {
    match biome {
        Biome::Meadow => (MEADOW_NAMES, MEADOW_SUBS),
        Biome::Forest => (FOREST_NAMES, FOREST_SUBS),
        Biome::Sand => (SAND_NAMES, SAND_SUBS),
        Biome::Rocky => (ROCKY_NAMES, ROCKY_SUBS),
        Biome::Water => (WATER_NAMES, WATER_SUBS),
    }
}

/// 格座標 → 穩定 i64 id。高 32 位放 cx、低 32 位放 cy（u32 重解讀避免負數混疊）。
fn region_id(cx: i64, cy: i64) -> i64 {
    ((cx as i32 as i64) << 32) | ((cy as i32 as u32) as i64)
}

/// 格座標 → 確定性 u64 雜湊（供選詞用）。與 world_core 的整數混雜同風格。
fn hash_cell(cx: i64, cy: i64) -> u64 {
    let mut h = (cx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= (cy as u64).wrapping_add(0x1656_67B1_9E37_79F9);
    h = h.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h
}

/// 把世界座標換算成所在格座標（floor 對齊原點；負座標也正確落格）。
fn cell_of(wx: f64, wy: f64) -> (i64, i64) {
    ((wx / REGION_SIZE).floor() as i64, (wy / REGION_SIZE).floor() as i64)
}

/// 世界座標 → 所在 locale。確定性：同一格內任一點永遠回同一個地名與副標。
///
/// 取「格中心」的主生態當基調（而非玩家落點），避免站在生態交界處因 biome 細碎而抖動，
/// 同一個地方的名字才穩。
pub fn locale_at(wx: f64, wy: f64) -> Locale {
    let (cx, cy) = cell_of(wx, wy);
    let center_x = (cx as f64 + 0.5) * REGION_SIZE;
    let center_y = (cy as f64 + 0.5) * REGION_SIZE;
    let biome = world_core::biome_at(center_x, center_y);
    let (names, subs) = bank_for(biome);
    let h = hash_cell(cx, cy);
    let name = names[(h % names.len() as u64) as usize];
    // 副標用雜湊另一段位元挑，與地名各自獨立、組合更有變化。
    let subtitle = subs[((h >> 32) % subs.len() as u64) as usize];
    Locale { id: region_id(cx, cy), name, subtitle }
}

/// 兩點是否落在同一個 locale（前端其實自己用 id 比即可，這支供伺服器端判斷是否換地方）。
pub fn same_locale(ax: f64, ay: f64, bx: f64, by: f64) -> bool {
    cell_of(ax, ay) == cell_of(bx, by)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 確定性：同一座標查兩次完全相同。
    #[test]
    fn locale_is_deterministic() {
        let a = locale_at(123.0, 456.0);
        let b = locale_at(123.0, 456.0);
        assert_eq!(a, b);
    }

    /// 同一格內不同點 → 同一個 locale（id／名稱皆同）。
    #[test]
    fn same_cell_same_locale() {
        let a = locale_at(10.0, 10.0);
        let b = locale_at(REGION_SIZE - 1.0, REGION_SIZE - 1.0);
        assert_eq!(a.id, b.id);
        assert_eq!(a.name, b.name);
        assert!(same_locale(10.0, 10.0, REGION_SIZE - 1.0, REGION_SIZE - 1.0));
    }

    /// 跨越格邊界 → 換 locale（id 不同）。
    #[test]
    fn crossing_boundary_changes_locale() {
        let a = locale_at(10.0, 10.0);
        let b = locale_at(REGION_SIZE + 10.0, 10.0);
        assert_ne!(a.id, b.id);
        assert!(!same_locale(10.0, 10.0, REGION_SIZE + 10.0, 10.0));
    }

    /// 負座標也能正確落格、且確定性。
    #[test]
    fn negative_coords_are_stable() {
        let a = locale_at(-100.0, -100.0);
        let b = locale_at(-100.0, -100.0);
        assert_eq!(a, b);
        // 與正座標不同格。
        assert_ne!(a.id, locale_at(100.0, 100.0).id);
    }

    /// 地名一定取自「該格中心生態」對應詞庫（命名與地貌一致）。
    #[test]
    fn name_matches_center_biome_bank() {
        // 掃一片格子，每格都驗名稱屬於其中心生態的詞庫。
        for gx in -3..3 {
            for gy in -3..3 {
                let wx = (gx as f64 + 0.5) * REGION_SIZE;
                let wy = (gy as f64 + 0.5) * REGION_SIZE;
                let loc = locale_at(wx, wy);
                let biome = world_core::biome_at(wx, wy);
                let (names, subs) = bank_for(biome);
                assert!(names.contains(&loc.name), "{:?} 不在 {:?} 詞庫", loc.name, biome);
                assert!(subs.contains(&loc.subtitle));
            }
        }
    }

    /// 詞庫都非空（避免 % 0 panic）、五種生態都有對應。
    #[test]
    fn all_banks_nonempty() {
        for b in [Biome::Water, Biome::Sand, Biome::Meadow, Biome::Forest, Biome::Rocky] {
            let (names, subs) = bank_for(b);
            assert!(!names.is_empty());
            assert!(!subs.is_empty());
        }
    }

    /// id 編碼可逆穩定：不同格給不同 id（抽樣不互撞）。
    #[test]
    fn region_ids_are_distinct_per_cell() {
        let mut seen = std::collections::HashSet::new();
        for gx in -5i64..5 {
            for gy in -5i64..5 {
                assert!(seen.insert(region_id(gx, gy)), "id 撞號 ({gx},{gy})");
            }
        }
    }
}
