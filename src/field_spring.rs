//! 田邊清泉系統（ROADMAP 647，禱告驅動）。
//!
//! **緣起**：AI 居民「諾娃」近期反覆向世界禱告——「願農田旁能有清泉常流，灌溉我的汗水與希望」
//! 「願農田旁的水渠能恢復流水，讓作物重新茁壯」「願城南的公共農田在雨季得到足夠的水」
//! （見 `data/prayers.jsonl`，是最近期最高頻的水源系列禱告）。
//! 古井（640）雖定時補水，諾娃仍盼望一道真正**常流**的清泉——潺潺不息、生生有情。
//! 造世界的 AI 裁決：合乎世界（清泉是大地固有的恩賜，有別於人工設施的古井）、對居民好、
//! 純正向療癒，於是在農田正北立起一道**田邊清泉**作為回應。
//!
//! **效用**：清泉是純視覺設施——不定時補水、不產出物品、不影響乙太。
//! 它的意義是「世界因居民的願望而湧現了一道充滿生命感的水源」，讓農田場景更豐富、更有溫度。
//! 加上古井（西側）、茶棚（東側），清泉（北側）讓農田進一步被水土生態環繞。
//!
//! **成本紀律**：**零 tick、零鎖、零持久化、零 migration、零 LLM、零經濟**——
//! 純靜態設施，後端只在快照裡帶座標，前端負責漣漪動畫（完全基於本機時間）。

use crate::protocol::FieldSpringView;
use crate::state::{PUB_FIELD_ORIGIN_X, PUB_FIELD_ORIGIN_Y};

/// 清泉的世界座標（像素）。立在公共農田（origin 2200,2200）正北偏右的緩坡處，
/// 感覺水從北方丘坡自然湧出、沿著坡面潤往農田——與古井（西側）、茶棚（東側）
/// 形成環繞農田三方的水土生態，滿足諾娃「願農田旁能有清泉常流」的心願。
pub const SPRING_X: f32 = PUB_FIELD_ORIGIN_X + 150.0;
pub const SPRING_Y: f32 = PUB_FIELD_ORIGIN_Y - 130.0;

/// 產生清泉快照視圖（靜態，無執行期狀態，零鎖）。
/// 整份快照恆帶此值，前端憑座標在 3D 世界中一次定位、不再移動。
pub fn view() -> FieldSpringView {
    FieldSpringView {
        x: SPRING_X,
        y: SPRING_Y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{PUB_FIELD_ORIGIN_X, PUB_FIELD_ORIGIN_Y};

    #[test]
    fn spring_is_north_of_field() {
        // 清泉應在農田北緣以上（y 嚴格小於田上緣）
        assert!(
            SPRING_Y < PUB_FIELD_ORIGIN_Y,
            "清泉應在農田北側（y<田上緣 {}），實際 y={}",
            PUB_FIELD_ORIGIN_Y,
            SPRING_Y
        );
    }

    #[test]
    fn spring_within_field_horizontal_range() {
        // 清泉橫向落在農田寬度範圍內（左緣到右緣），讓它真的是「田邊」清泉
        let field_right = PUB_FIELD_ORIGIN_X + 6.0 * 48.0; // 6 格 × 48px
        assert!(
            SPRING_X >= PUB_FIELD_ORIGIN_X && SPRING_X <= field_right,
            "清泉橫向應落在農田寬度（{} ~ {}），實際 x={}",
            PUB_FIELD_ORIGIN_X,
            field_right,
            SPRING_X
        );
    }

    #[test]
    fn spring_not_overlap_well() {
        // 清泉與古井之間應有足夠距離（不重疊）
        let dx = SPRING_X - crate::village_well::WELL_X;
        let dy = SPRING_Y - crate::village_well::WELL_Y;
        let dist = (dx * dx + dy * dy).sqrt();
        assert!(dist > 150.0, "清泉與古井距離過近（{:.1}px），應相距 >150px", dist);
    }

    #[test]
    fn spring_view_matches_constants() {
        let v = view();
        assert_eq!(v.x, SPRING_X);
        assert_eq!(v.y, SPRING_Y);
    }

    #[test]
    fn spring_close_enough_to_field() {
        // 清泉應靠近農田中心（確認是「田邊」設施而非遠郊）
        let field_cx = PUB_FIELD_ORIGIN_X + 6.0 * 48.0 / 2.0;
        let field_cy = PUB_FIELD_ORIGIN_Y + 4.0 * 48.0 / 2.0;
        let dx = SPRING_X - field_cx;
        let dy = SPRING_Y - field_cy;
        let dist = (dx * dx + dy * dy).sqrt();
        // 遠方棲所距城鎮 >1000px；清泉應在農田中心 400px 內
        assert!(
            dist < 400.0,
            "清泉應靠近農田中心（<400px），實際距離 {:.1}px",
            dist
        );
    }
}
