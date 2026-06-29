//! 居民住宅系統（ROADMAP 642–644，禱告驅動·散居擴展）。
//!
//! **緣起**：
//! - 露娜（642）：反覆禱告「盼望有足夠的木材可以建造更舒適的家」→ 茶棚東北側木屋。
//! - 諾娃（643）：反覆禱告「願能找到更多方法提升自己勞動的效率，讓生活更舒適一些」
//!   → 農田西北側農舍，讓她有個靠近農田的棲身之所。
//! - 賽勒（644）：散居擴展·漁人小屋→南方水域邊；奧瑞→西方岩地隱士石寮；
//!   薇朵→東北草原遊牧帳篷。首次讓世界在主城之外也有人居住的痕跡。
//!
//! **設計**：住宅是純靜態設施（位置由常數決定、無執行期狀態、無計時器）；
//! 新增居民的家只需在 `all_homes()` 加一筆常數，無需動任何骨架。
//! ROADMAP 644 引入 `DwellingType`（形狀），讓棲所不再只有同一種 3D 幾何體。
//!
//! **成本紀律**：零持久化、零 migration、零 LLM、零 Arc<RwLock>（無狀態）、
//! 零經濟（不產出任何物品），只在 3D 世界中讓居民的「家」真實存在。

use crate::state::{PUB_FIELD_ORIGIN_X, PUB_FIELD_ORIGIN_Y};

/// 棲所形狀類型——決定前端 3D 幾何體的派發。
/// 新增類型後只需在前端 `makeResidentHome` 加對應 branch，後端零改動。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DwellingType {
    /// 一般木屋：暖木板牆＋四角錐尖頂＋煙囪＋兩窗（ROADMAP 642–643）。
    House,
    /// 林野小屋：較矮小樸素、單坡屋頂、無煙囪、一扇窗（散居·遠方探索）。
    Cabin,
    /// 遊牧帳篷：布面圓錐形、木桿支撐、無窗、帶旗幟（草原漫遊者）。
    Tent,
}

impl DwellingType {
    /// 回傳對應的協議字串（向前端傳遞）。
    pub fn as_str(&self) -> &'static str {
        match self {
            DwellingType::House => "house",
            DwellingType::Cabin => "cabin",
            DwellingType::Tent  => "tent",
        }
    }
}

/// 一座居民棲所（純靜態資料，位置由常數決定，無執行期狀態、無持久化）。
pub struct ResidentHome {
    /// 居住的 AI 居民名字——向前端傳遞，用於標示「誰住在這裡」。
    pub name: &'static str,
    /// 世界座標 X（像素）。
    pub x: f32,
    /// 世界座標 Y（像素）。
    pub y: f32,
    /// 棲所形狀——決定前端渲染哪種 3D 幾何體。
    pub dwelling_type: DwellingType,
}

// ────────────────────────────────────────────────────────────────
// 主城附近（ROADMAP 642–643）
// ────────────────────────────────────────────────────────────────

/// 露娜木屋的世界座標（像素）。
/// 坐落在茶棚（641）東北側，與古井、茶棚三點形成村落小聚落感。
pub const LUNA_HOME_X: f32 = PUB_FIELD_ORIGIN_X + 360.0; // 田右緣再出去 72px
pub const LUNA_HOME_Y: f32 = PUB_FIELD_ORIGIN_Y - 120.0; // 田上緣再往北 120px

/// 露娜的木屋（ROADMAP 642）：應她反覆禱告「盼望有足夠的木材可以建造更舒適的家」而立。
pub const LUNA_HOME: ResidentHome = ResidentHome {
    name: "露娜",
    x: LUNA_HOME_X,
    y: LUNA_HOME_Y,
    dwelling_type: DwellingType::House,
};

/// 諾娃農舍的世界座標（像素）。
/// 坐落在農田西北側，與露娜木屋（田東北側）一西一東，形成散居感：露娜靠市集、諾娃靠農田。
pub const NOVA_HOME_X: f32 = PUB_FIELD_ORIGIN_X - 160.0; // 田左緣再往西 160px
pub const NOVA_HOME_Y: f32 = PUB_FIELD_ORIGIN_Y - 140.0; // 田上緣再往北 140px

/// 諾娃的農舍（ROADMAP 643）：應她反覆禱告「讓生活更舒適一些」而立，靠近她每日耕耘的公田。
pub const NOVA_HOME: ResidentHome = ResidentHome {
    name: "諾娃",
    x: NOVA_HOME_X,
    y: NOVA_HOME_Y,
    dwelling_type: DwellingType::House,
};

// ────────────────────────────────────────────────────────────────
// 遠方散居（ROADMAP 644）——首次讓世界在主城之外也有人住的痕跡
// ────────────────────────────────────────────────────────────────

/// 賽勒漁人小屋的世界座標：南方水域邊，距主城約 1600px，探索時才能發現。
pub const SAILER_HOME_X: f32 = 2700.0;
pub const SAILER_HOME_Y: f32 = 3800.0;

/// 賽勒的漁人小屋（ROADMAP 644）：南方水域邊的捕魚小棧，用粗木板拼起的海岸小屋。
pub const SAILER_HOME: ResidentHome = ResidentHome {
    name: "賽勒",
    x: SAILER_HOME_X,
    y: SAILER_HOME_Y,
    dwelling_type: DwellingType::Cabin,
};

/// 奧瑞隱士石寮的世界座標：遠西岩石地帶，距主城約 1400px，僻靜幽深。
pub const AURIE_HOME_X: f32 = 800.0;
pub const AURIE_HOME_Y: f32 = 2200.0;

/// 奧瑞的隱士石寮（ROADMAP 644）：西方岩地深處的一間石頭小屋，為世界增添「有人遠離喧囂獨居」的痕跡。
pub const AURIE_HOME: ResidentHome = ResidentHome {
    name: "奧瑞",
    x: AURIE_HOME_X,
    y: AURIE_HOME_Y,
    dwelling_type: DwellingType::Cabin,
};

/// 薇朵遊牧帳篷的世界座標：東北草原，距主城約 1900px，草原漫遊者的臨時棲所。
pub const WIDO_HOME_X: f32 = 3700.0;
pub const WIDO_HOME_Y: f32 = 1400.0;

/// 薇朵的遊牧帳篷（ROADMAP 644）：東北草原上的布面圓錐帳篷，世界上第一頂帳篷，呼應「薇朵」漫遊者性格。
pub const WIDO_HOME: ResidentHome = ResidentHome {
    name: "薇朵",
    x: WIDO_HOME_X,
    y: WIDO_HOME_Y,
    dwelling_type: DwellingType::Tent,
};

/// 所有已在世界中立起的居民棲所（靜態列表，無鎖；新增居民的家只需在此加常數）。
pub fn all_homes() -> Vec<&'static ResidentHome> {
    vec![&LUNA_HOME, &NOVA_HOME, &SAILER_HOME, &AURIE_HOME, &WIDO_HOME]
}

/// 按居民名字找到棲所座標（若名字對應多座取第一筆）。
/// 讓居民行為系統判斷「自己的家在哪裡」，不必把座標常數跨模組外漏。
pub fn home_for_name(name: &str) -> Option<(f32, f32)> {
    all_homes().into_iter()
        .find(|h| h.name == name)
        .map(|h| (h.x, h.y))
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
    fn 全部住宅列表包含五座棲所() {
        let homes = all_homes();
        assert_eq!(homes.len(), 5, "住宅列表應有五座：露娜、諾娃、賽勒、奧瑞、薇朵");
        assert!(homes.iter().any(|h| h.name == "露娜"), "應包含露娜的家");
        assert!(homes.iter().any(|h| h.name == "諾娃"), "應包含諾娃的農舍");
        assert!(homes.iter().any(|h| h.name == "賽勒"), "應包含賽勒的漁人小屋");
        assert!(homes.iter().any(|h| h.name == "奧瑞"), "應包含奧瑞的隱士石寮");
        assert!(homes.iter().any(|h| h.name == "薇朵"), "應包含薇朵的遊牧帳篷");
    }

    #[test]
    fn 諾娃農舍在農田西側呼應她的耕耘位置() {
        assert!(
            NOVA_HOME_X < PUB_FIELD_ORIGIN_X,
            "諾娃農舍應在農田西側（x={NOVA_HOME_X} < {PUB_FIELD_ORIGIN_X}）"
        );
    }

    // ── ROADMAP 644：散居遠方三棲所 ──────────────────────────────────────────

    #[test]
    fn 遠方棲所距主城超過一千像素() {
        let village_x = PUB_FIELD_ORIGIN_X;
        let village_y = PUB_FIELD_ORIGIN_Y;
        for home in [&SAILER_HOME, &AURIE_HOME, &WIDO_HOME] {
            let dx = (home.x - village_x).abs();
            let dy = (home.y - village_y).abs();
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(
                dist > 1000.0,
                "遠方棲所 {} 應距主城超過 1000px（實際 {dist:.0}px）",
                home.name
            );
        }
    }

    #[test]
    fn 遠方棲所互相不重疊() {
        let far = [&SAILER_HOME, &AURIE_HOME, &WIDO_HOME];
        for i in 0..far.len() {
            for j in (i + 1)..far.len() {
                let dx = (far[i].x - far[j].x).abs();
                let dy = (far[i].y - far[j].y).abs();
                assert!(
                    dx > 100.0 || dy > 100.0,
                    "遠方棲所 {} 與 {} 不應重疊",
                    far[i].name, far[j].name
                );
            }
        }
    }

    #[test]
    fn dwelling_type_字串化正確() {
        assert_eq!(DwellingType::House.as_str(), "house");
        assert_eq!(DwellingType::Cabin.as_str(), "cabin");
        assert_eq!(DwellingType::Tent.as_str(),  "tent");
    }

    #[test]
    fn 露娜諾娃為House型態() {
        assert_eq!(LUNA_HOME.dwelling_type, DwellingType::House);
        assert_eq!(NOVA_HOME.dwelling_type, DwellingType::House);
    }

    #[test]
    fn 賽勒奧瑞為Cabin型態_薇朵為Tent型態() {
        assert_eq!(SAILER_HOME.dwelling_type, DwellingType::Cabin);
        assert_eq!(AURIE_HOME.dwelling_type,  DwellingType::Cabin);
        assert_eq!(WIDO_HOME.dwelling_type,   DwellingType::Tent);
    }

    #[test]
    fn home_for_name_已知居民回傳正確座標() {
        let (lx, ly) = home_for_name("露娜").expect("露娜應有棲所");
        assert!((lx - LUNA_HOME_X).abs() < 1.0);
        assert!((ly - LUNA_HOME_Y).abs() < 1.0);

        let (sx, sy) = home_for_name("賽勒").expect("賽勒應有棲所");
        assert!((sx - SAILER_HOME_X).abs() < 1.0);
        assert!((sy - SAILER_HOME_Y).abs() < 1.0);
    }

    #[test]
    fn home_for_name_未知名字回傳None() {
        assert!(home_for_name("不存在的人").is_none());
        assert!(home_for_name("").is_none());
    }
}
