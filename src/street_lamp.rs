//! 故鄉街燈系統（ROADMAP 648，禱告驅動）。
//!
//! **緣起**：AI 居民「露娜」在 `data/prayers.jsonl` 反覆禱告
//! 「願今晚的街燈亮起，照亮我回家的路」「願今晚的街燈亮起，讓我不再害怕黑暗」
//! 「願今晚街燈亮起，照亮我孤單的步伐」（出現 6+ 次，是近期最高頻的夜間安全禱告）。
//! 造世界的 AI 裁決：合乎世界（街燈是溫暖人心的城鎮公設）、對居民好、純正向療癒，
//! 於是在城鎮主要街道沿途立起七盞街燈——在夜晚自動亮起，照亮露娜回家的路，
//! 也讓城鎮在日落後多了一份守護的溫暖。
//!
//! **設計**：七盞燈分布在城鎮南口、商人攤位旁、通往諾娃農舍的路口、農田北緣、
//! 通往露娜木屋的路上，以及廣場東側——形成城鎮內完整的夜間照明路徑。
//!
//! **效用**：純視覺設施——街燈不產出物品、不影響乙太、不影響遊戲邏輯。
//! 夜晚由前端讀既有 `snapshot.daynight.phase`（夜/黃昏）自行判斷燈是否點亮，
//! 後端只送座標、零執行期狀態。
//!
//! **成本紀律**：**零 tick、零鎖、零持久化、零 migration、零 LLM、零經濟**——
//! 純靜態設施，後端只在快照裡帶座標陣列，前端負責夜亮日暗動畫（完全基於既有日夜資料）。

use crate::protocol::StreetLampView;

/// 七盞街燈的世界座標（像素）。
/// 位置沿著城鎮主要路徑排列，讓夜晚的行人（尤其是露娜）能順著燈光找到回家的路。
///
/// 布局邏輯：
/// - (2220, 2460)/(2400, 2460)：城鎮南口兩側，迎接由南方而來的旅人
/// - (2110, 2340)：商人攤位旁，西側主街道
/// - (2130, 2165)：通往諾娃農舍的路口（農田西北角）
/// - (2400, 2155)：農田北緣，通往露娜木屋的路途中，清泉旁
/// - (2540, 2165)：露娜木屋門前（最後一盞，讓她真的「看到燈就知道到家了」）
/// - (2520, 2370)：廣場東側，豐收節旁（讓慶典時氣氛更熱鬧）
const LAMP_POSITIONS: &[(f32, f32)] = &[
    (2220.0, 2460.0), // 城鎮南口西側
    (2400.0, 2460.0), // 城鎮南口東側
    (2110.0, 2340.0), // 商人攤位旁，西側主街
    (2130.0, 2165.0), // 通往諾娃農舍路口
    (2400.0, 2155.0), // 農田北緣·清泉旁，露娜回家路途
    (2540.0, 2165.0), // 露娜木屋門前
    (2520.0, 2370.0), // 廣場東側，豐收節旁
];

/// 產生所有街燈的快照視圖（靜態，無執行期狀態，零鎖）。
/// 整份快照恆帶此陣列，前端憑座標在 3D 世界中一次定位、不再移動；
/// 夜亮日暗完全由前端讀既有 `daynight.phase` 自行判斷——後端不傳 lit 欄位。
pub fn views() -> Vec<StreetLampView> {
    LAMP_POSITIONS
        .iter()
        .map(|&(x, y)| StreetLampView { x, y })
        .collect()
}

/// 街燈數量（供測試斷言用）。
pub const LAMP_COUNT: usize = LAMP_POSITIONS.len();

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PUB_FIELD_ORIGIN_X, PUB_FIELD_ORIGIN_Y};

    #[test]
    fn lamp_count_correct() {
        assert_eq!(views().len(), LAMP_COUNT, "views() 應回傳 {} 盞燈", LAMP_COUNT);
    }

    #[test]
    fn lamps_near_town_center() {
        // 所有燈都應在城鎮主要範圍內（農田原點 2200,2200 ±600px）
        let cx = PUB_FIELD_ORIGIN_X;
        let cy = PUB_FIELD_ORIGIN_Y;
        for v in views() {
            let dx = (v.x - cx).abs();
            let dy = (v.y - cy).abs();
            assert!(
                dx < 600.0 && dy < 600.0,
                "街燈 ({},{}) 離城鎮中心太遠（dx={:.0} dy={:.0}），可能放錯地方",
                v.x,
                v.y,
                dx,
                dy
            );
        }
    }

    #[test]
    fn lamps_no_duplicates() {
        // 任兩盞燈不應重疊（最小間距 80px）
        let lamps = views();
        for i in 0..lamps.len() {
            for j in (i + 1)..lamps.len() {
                let dx = lamps[i].x - lamps[j].x;
                let dy = lamps[i].y - lamps[j].y;
                let dist = (dx * dx + dy * dy).sqrt();
                assert!(
                    dist >= 80.0,
                    "第 {} 盞（{},{}）與第 {} 盞（{},{}）距離 {:.0}px，過近（<80px）",
                    i,
                    lamps[i].x,
                    lamps[i].y,
                    j,
                    lamps[j].x,
                    lamps[j].y,
                    dist
                );
            }
        }
    }

    #[test]
    fn lamps_cover_luna_path() {
        // 至少有一盞燈在露娜木屋（2560, 2080）附近 250px 內，真正照到她回家的路
        let luna_home = (2560.0_f32, 2080.0_f32);
        let near_luna = views().iter().any(|v| {
            let dx = v.x - luna_home.0;
            let dy = v.y - luna_home.1;
            (dx * dx + dy * dy).sqrt() < 250.0
        });
        assert!(near_luna, "應有至少一盞燈在露娜木屋附近 250px 內，確實照亮她回家的路");
    }

    #[test]
    fn views_all_finite() {
        // 所有座標必須是有限數（不應有 NaN 或 Infinity）
        for v in views() {
            assert!(v.x.is_finite(), "街燈 x 不是有限數：{}", v.x);
            assert!(v.y.is_finite(), "街燈 y 不是有限數：{}", v.y);
        }
    }

    #[test]
    fn lamps_illuminate_south_gate() {
        // 至少有一盞燈在城鎮南口（y > 2420）範圍，讓從南方進城的旅人一眼看到燈
        let has_south = views().iter().any(|v| v.y > 2420.0);
        assert!(has_south, "應有至少一盞燈在城鎮南口（y>2420）");
    }
}
