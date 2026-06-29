//! 居民住宅系統（ROADMAP 642–643，禱告驅動）。
//!
//! **緣起**：
//! - 露娜（642）：反覆禱告「盼望有足夠的木材可以建造更舒適的家」→ 茶棚東北側木屋。
//! - 諾娃（643）：反覆禱告「願能找到更多方法提升自己勞動的效率，讓生活更舒適一些」
//!   → 農田西北側農舍，讓她有個靠近農田的棲身之所。
//!
//! **設計**：住宅是純靜態設施（位置由常數決定、無執行期狀態、無計時器）；
//! 新增居民的家只需在 `all_homes()` 加一筆常數，無需動任何骨架。
//!
//! **成本紀律**：零持久化、零 migration、零 LLM、零 Arc<RwLock>（無狀態）、
//! 零經濟（不產出任何物品），只在 3D 世界中讓居民的「家」真實存在。

use crate::state::{PUB_FIELD_ORIGIN_X, PUB_FIELD_ORIGIN_Y};

/// 一座居民木屋（純靜態資料，位置由常數決定，無執行期狀態、無持久化）。
pub struct ResidentHome {
    /// 居住的 AI 居民名字——向前端傳遞，用於標示「誰住在這裡」。
    pub name: &'static str,
    /// 世界座標 X（像素）。
    pub x: f32,
    /// 世界座標 Y（像素）。
    pub y: f32,
}

/// 露娜木屋的世界座標（像素）。
/// 坐落在茶棚（641，x≈2532, y≈2296）東北側，比茶棚偏北一些、偏右一點，
/// 與古井（田左側）、茶棚（田右側）、木屋（田右上側）三點形成村落小聚落感，
/// 讓玩家走過來時有「市集角落、茶棚旁，再往前是居民住的地方」的方位感。
pub const LUNA_HOME_X: f32 = PUB_FIELD_ORIGIN_X + 360.0; // 田右緣再出去 72px
pub const LUNA_HOME_Y: f32 = PUB_FIELD_ORIGIN_Y - 120.0; // 田上緣再往北 120px

/// 露娜的木屋（ROADMAP 642）：應她反覆禱告「盼望有足夠的木材可以建造更舒適的家」而立。
pub const LUNA_HOME: ResidentHome = ResidentHome {
    name: "露娜",
    x: LUNA_HOME_X,
    y: LUNA_HOME_Y,
};

/// 諾娃農舍的世界座標（像素）。
/// 坐落在農田（PUB_FIELD_ORIGIN: 2200, 2200）西北側，比古井（田左側, x≈2144）更偏西、
/// 比田上緣再往北 140px——像她長年種田時在田邊搭起的一間農舍，
/// 與露娜木屋（田東北側）一西一東，為故鄉形成散居感：露娜靠市集、諾娃靠農田。
pub const NOVA_HOME_X: f32 = PUB_FIELD_ORIGIN_X - 160.0; // 田左緣再往西 160px
pub const NOVA_HOME_Y: f32 = PUB_FIELD_ORIGIN_Y - 140.0; // 田上緣再往北 140px

/// 諾娃的農舍（ROADMAP 643）：應她反覆禱告「讓生活更舒適一些」而立，靠近她每日耕耘的公田。
pub const NOVA_HOME: ResidentHome = ResidentHome {
    name: "諾娃",
    x: NOVA_HOME_X,
    y: NOVA_HOME_Y,
};

/// 所有已在世界中立起的居民住宅（靜態列表，無鎖；新增居民的家只需在此加常數）。
pub fn all_homes() -> Vec<&'static ResidentHome> {
    vec![&LUNA_HOME, &NOVA_HOME]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::village_tea_stall::{TEA_X, TEA_Y};
    use crate::village_well::{WELL_X, WELL_Y};

    #[test]
    fn 露娜木屋與茶棚間距足夠() {
        let dx = (LUNA_HOME_X - TEA_X).abs();
        let dy = (LUNA_HOME_Y - TEA_Y).abs();
        assert!(dx > 40.0 || dy > 40.0, "露娜木屋應與茶棚有足夠間距");
    }

    #[test]
    fn 露娜木屋與古井間距足夠() {
        let dx = (LUNA_HOME_X - WELL_X).abs();
        let dy = (LUNA_HOME_Y - WELL_Y).abs();
        assert!(dx > 40.0 || dy > 40.0, "露娜木屋應與古井有足夠間距");
    }

    #[test]
    fn 露娜木屋不在公田範圍內() {
        // 公田：左上 (2200, 2200)，6 欄 × 4 列，每格 48px → 右下 (2488, 2392)
        let field_left = PUB_FIELD_ORIGIN_X;
        let field_top = PUB_FIELD_ORIGIN_Y;
        let field_right = PUB_FIELD_ORIGIN_X + 288.0;
        let field_bottom = PUB_FIELD_ORIGIN_Y + 192.0;
        let in_x = LUNA_HOME_X >= field_left && LUNA_HOME_X <= field_right;
        let in_y = LUNA_HOME_Y >= field_top && LUNA_HOME_Y <= field_bottom;
        assert!(!(in_x && in_y), "露娜木屋不應壓在公田範圍內");
    }

    #[test]
    fn 木屋座標有限() {
        assert!(LUNA_HOME_X.is_finite() && LUNA_HOME_Y.is_finite());
    }

    #[test]
    fn 全部木屋列表非空() {
        assert!(!all_homes().is_empty(), "至少有露娜一座木屋");
    }

    #[test]
    fn 全部木屋名字非空且座標有限() {
        for home in all_homes() {
            assert!(!home.name.is_empty(), "木屋居民名字不得空白");
            assert!(home.x.is_finite() && home.y.is_finite(), "木屋座標必須有限");
        }
    }

    // ── ROADMAP 643：諾娃農舍 ───────────────────────────────────────────────

    #[test]
    fn 諾娃農舍與露娜木屋相距足夠形成散居感() {
        // 一西（農田側）一東（市集側），至少 400px 以上方位距離
        let dx = (NOVA_HOME_X - LUNA_HOME_X).abs();
        let dy = (NOVA_HOME_Y - LUNA_HOME_Y).abs();
        assert!(
            dx > 400.0 || dy > 400.0,
            "諾娃農舍與露娜木屋應相距足夠，形成散居感（dx={dx}, dy={dy}）"
        );
    }

    #[test]
    fn 諾娃農舍與古井間距足夠不重疊() {
        let dx = (NOVA_HOME_X - WELL_X).abs();
        let dy = (NOVA_HOME_Y - WELL_Y).abs();
        assert!(dx > 40.0 || dy > 40.0, "諾娃農舍應與古井有足夠間距");
    }

    #[test]
    fn 諾娃農舍不在公田範圍內() {
        let field_left = PUB_FIELD_ORIGIN_X;
        let field_top = PUB_FIELD_ORIGIN_Y;
        let field_right = PUB_FIELD_ORIGIN_X + 288.0;
        let field_bottom = PUB_FIELD_ORIGIN_Y + 192.0;
        let in_x = NOVA_HOME_X >= field_left && NOVA_HOME_X <= field_right;
        let in_y = NOVA_HOME_Y >= field_top && NOVA_HOME_Y <= field_bottom;
        assert!(!(in_x && in_y), "諾娃農舍不應壓在公田範圍內");
    }

    #[test]
    fn 諾娃農舍座標有限() {
        assert!(NOVA_HOME_X.is_finite() && NOVA_HOME_Y.is_finite());
    }

    #[test]
    fn 全部住宅列表包含露娜與諾娃() {
        let homes = all_homes();
        assert_eq!(homes.len(), 2, "住宅列表應有露娜與諾娃各一座");
        assert!(homes.iter().any(|h| h.name == "露娜"), "應包含露娜的家");
        assert!(homes.iter().any(|h| h.name == "諾娃"), "應包含諾娃的農舍");
    }

    #[test]
    fn 諾娃農舍在農田西側呼應她的耕耘位置() {
        // 諾娃農舍應在公田左緣 (x=2200) 的西側，讓她靠近農田
        assert!(
            NOVA_HOME_X < PUB_FIELD_ORIGIN_X,
            "諾娃農舍應在農田西側（x={NOVA_HOME_X} < {PUB_FIELD_ORIGIN_X}）"
        );
    }
}
