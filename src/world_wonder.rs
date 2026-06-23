//! 世界奇觀首探（ROADMAP 524）。
//!
//! 五處隱藏秘境散落在廣闊世界的不同方位。旅人踏入任一奇觀的探索半徑後，
//! 若還沒有人發現過，立即觸發「全服首探公告 + 發放 20 乙太獎勵 + 小地圖永久標記」。
//! 奇觀本體始終存在（前端繪製），未探索時柔光加問號，已探索後加入發現者姓名與
//! 更明亮的光效。
//!
//! 設計鐵律：
//! - **純邏輯、零 IO、零 LLM**：定義與計算全部抽成純函式，可完整單元測試。
//! - **純記憶體、零 migration**：重啟後玩家重新探索（探索獎勵鼓勵每次上線都去冒險）。
//! - **共五處奇觀，方位各異**：激勵玩家往世界不同方向探索。

/// 探索接觸半徑（像素）：玩家走到此距離內視為「踏入奇觀」。
pub const DISCOVER_RADIUS: f32 = 120.0;

/// 首探乙太獎勵。
pub const DISCOVER_REWARD: u32 = 20;

/// 一處奇觀的靜態定義。
#[derive(Debug, Clone, Copy)]
pub struct WonderDef {
    /// 機器識別碼（snake_case），用於協議 / 去重判斷。
    pub key: &'static str,
    /// 世界 X 座標（像素）。
    pub wx: f32,
    /// 世界 Y 座標（像素）。
    pub wy: f32,
    /// 繁中名稱（面向玩家顯示，i18n 替換點）。
    pub name_zh: &'static str,
    /// 代表 emoji（前端顯示用）。
    pub emoji: &'static str,
}

/// 五處奇觀定義。座標以安全區中心（2344, 2296）為基準，向五個方位各放 3500~4000px。
/// 距離足夠遠以獎勵認真探索，方位互不重疊以鼓勵全方位探索。
pub const ALL_WONDERS: &[WonderDef] = &[
    WonderDef {
        key: "crystal_palace",
        wx: 5344.0,
        wy: 296.0,
        name_zh: "星核晶宮",
        emoji: "💎",
    },
    WonderDef {
        key: "jade_tree",
        wx: -656.0,
        wy: 296.0,
        name_zh: "翡翠古樹",
        emoji: "🌳",
    },
    WonderDef {
        key: "moon_temple",
        wx: 2344.0,
        wy: 5796.0,
        name_zh: "黃沙月神殿",
        emoji: "🏛️",
    },
    WonderDef {
        key: "coral_city",
        wx: -1656.0,
        wy: 2296.0,
        name_zh: "深洋珊瑚城",
        emoji: "🪸",
    },
    WonderDef {
        key: "steam_spring",
        wx: 5344.0,
        wy: 4796.0,
        name_zh: "蒸汽地熱泉",
        emoji: "♨️",
    },
];

/// 某奇觀已被首探的記錄。
#[derive(Debug, Clone)]
pub struct WonderDiscovery {
    /// 奇觀識別碼（對齊 `WonderDef::key`）。
    pub key: &'static str,
    /// 首探者顯示名稱。
    pub discoverer_name: String,
}

/// 全域奇觀探索狀態（記憶體前置，零持久化）。
#[derive(Debug, Default)]
pub struct WorldWonderState {
    /// 已完成首探的奇觀記錄。
    discoveries: Vec<WonderDiscovery>,
}

impl WorldWonderState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 玩家是否已踏入某奇觀的探索半徑。純函式，零副作用。
    pub fn is_near(px: f32, py: f32, def: &WonderDef) -> bool {
        if !px.is_finite() || !py.is_finite() {
            return false;
        }
        let dx = px - def.wx;
        let dy = py - def.wy;
        dx * dx + dy * dy <= DISCOVER_RADIUS * DISCOVER_RADIUS
    }

    /// 嘗試記錄首探。若該 `key` 尚未被首探，插入記錄並回傳 `true`（新發現）；
    /// 已有記錄則回傳 `false`（靜默忽略，不重複獎勵）。
    pub fn try_discover(&mut self, key: &'static str, discoverer_name: String) -> bool {
        if self.is_discovered(key) {
            return false;
        }
        self.discoveries.push(WonderDiscovery { key, discoverer_name });
        true
    }

    /// 某奇觀是否已被首探。純函式。
    pub fn is_discovered(&self, key: &'static str) -> bool {
        self.discoveries.iter().any(|d| d.key == key)
    }

    /// 取得某奇觀的首探記錄（若已首探）。純函式。
    pub fn discovery(&self, key: &'static str) -> Option<&WonderDiscovery> {
        self.discoveries.iter().find(|d| d.key == key)
    }

    /// 取得所有已首探的記錄，供 Snapshot 廣播。純函式。
    pub fn all_discoveries(&self) -> &[WonderDiscovery] {
        &self.discoveries
    }

    /// 全部五處奇觀是否都已首探。純函式。
    pub fn all_discovered(&self) -> bool {
        ALL_WONDERS.len() == self.discoveries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(idx: usize) -> &'static WonderDef {
        &ALL_WONDERS[idx]
    }

    #[test]
    fn is_near_center() {
        let d = def(0);
        // 中心點必定在範圍內
        assert!(WorldWonderState::is_near(d.wx, d.wy, d));
    }

    #[test]
    fn is_near_just_inside_boundary() {
        let d = def(0);
        let inside = DISCOVER_RADIUS - 1.0;
        assert!(WorldWonderState::is_near(d.wx + inside, d.wy, d));
        assert!(WorldWonderState::is_near(d.wx, d.wy + inside, d));
    }

    #[test]
    fn is_near_just_outside_boundary() {
        let d = def(0);
        let outside = DISCOVER_RADIUS + 1.0;
        assert!(!WorldWonderState::is_near(d.wx + outside, d.wy, d));
        assert!(!WorldWonderState::is_near(d.wx, d.wy + outside, d));
    }

    #[test]
    fn is_near_rejects_nan() {
        let d = def(0);
        assert!(!WorldWonderState::is_near(f32::NAN, d.wy, d));
        assert!(!WorldWonderState::is_near(d.wx, f32::NAN, d));
    }

    #[test]
    fn is_near_rejects_infinity() {
        let d = def(0);
        assert!(!WorldWonderState::is_near(f32::INFINITY, d.wy, d));
        assert!(!WorldWonderState::is_near(d.wx, f32::NEG_INFINITY, d));
    }

    #[test]
    fn try_discover_first_time_returns_true() {
        let mut s = WorldWonderState::new();
        assert!(s.try_discover(ALL_WONDERS[0].key, "旅人甲".to_string()));
        assert!(s.is_discovered(ALL_WONDERS[0].key));
    }

    #[test]
    fn try_discover_second_time_returns_false() {
        let mut s = WorldWonderState::new();
        s.try_discover(ALL_WONDERS[1].key, "旅人甲".to_string());
        // 第二次同一處，不管是誰，回傳 false
        assert!(!s.try_discover(ALL_WONDERS[1].key, "旅人乙".to_string()));
    }

    #[test]
    fn different_wonders_are_independent() {
        let mut s = WorldWonderState::new();
        s.try_discover(ALL_WONDERS[0].key, "旅人甲".to_string());
        // 第一處已探，第二處未探
        assert!(s.is_discovered(ALL_WONDERS[0].key));
        assert!(!s.is_discovered(ALL_WONDERS[1].key));
        // 第二處仍可探
        assert!(s.try_discover(ALL_WONDERS[1].key, "旅人乙".to_string()));
    }

    #[test]
    fn discovery_record_correct() {
        let mut s = WorldWonderState::new();
        s.try_discover(ALL_WONDERS[2].key, "旅人丙".to_string());
        let rec = s.discovery(ALL_WONDERS[2].key).unwrap();
        assert_eq!(rec.key, ALL_WONDERS[2].key);
        assert_eq!(rec.discoverer_name, "旅人丙");
    }

    #[test]
    fn all_discovered_false_initially() {
        let s = WorldWonderState::new();
        assert!(!s.all_discovered());
    }

    #[test]
    fn all_discovered_true_after_all_five() {
        let mut s = WorldWonderState::new();
        for (i, w) in ALL_WONDERS.iter().enumerate() {
            s.try_discover(w.key, format!("旅人{}", i));
        }
        assert!(s.all_discovered());
    }

    #[test]
    fn wonder_count_is_five() {
        assert_eq!(ALL_WONDERS.len(), 5);
    }

    #[test]
    fn all_keys_unique() {
        let keys: std::collections::HashSet<&str> = ALL_WONDERS.iter().map(|w| w.key).collect();
        assert_eq!(keys.len(), ALL_WONDERS.len());
    }

    #[test]
    fn all_wonders_far_from_town_center() {
        // 安全區中心 (2344, 2296)，奇觀應在 2500px 以上
        let (cx, cy) = (2344.0f32, 2296.0f32);
        for w in ALL_WONDERS {
            let dist = ((w.wx - cx).powi(2) + (w.wy - cy).powi(2)).sqrt();
            assert!(dist > 2500.0, "奇觀 {} 距城太近（{}px）", w.key, dist);
        }
    }
}
