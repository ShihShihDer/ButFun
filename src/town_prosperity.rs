//! 城鎮繁榮儀（ROADMAP 128）。
//!
//! 把所有路人居民的平均快樂值匯聚成四段繁榮等級，等級改變時廣播至世界聊天，
//! 快照帶 `town_prosperity_level: u8` 讓前端 HUD 即時顯示城鎮狀態。
//!
//! 純記憶體、零 migration、零 LLM。

/// 城鎮繁榮等級（0-3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProsperityLevel {
    /// 0：凋零——居民普遍低落（平均快樂值 < 30）。
    Withered = 0,
    /// 1：平靜——居民生活尚可（平均快樂值 30-54）。
    Calm = 1,
    /// 2：生機——居民多半快樂（平均快樂值 55-74）。
    Thriving = 2,
    /// 3：繁盛——居民一片喜悅（平均快樂值 ≥ 75）。
    Vibrant = 3,
}

impl ProsperityLevel {
    /// 轉成 u8（用於快照傳輸）。
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// 等級名稱（前端/廣播用）。
    pub fn name(self) -> &'static str {
        match self {
            ProsperityLevel::Withered => "凋零",
            ProsperityLevel::Calm     => "平靜",
            ProsperityLevel::Thriving => "生機",
            ProsperityLevel::Vibrant  => "繁盛",
        }
    }

    /// 等級 emoji（前端 HUD 用）。
    pub fn emoji(self) -> &'static str {
        match self {
            ProsperityLevel::Withered => "🥀",
            ProsperityLevel::Calm     => "🌿",
            ProsperityLevel::Thriving => "🌻",
            ProsperityLevel::Vibrant  => "🎉",
        }
    }
}

/// 由居民平均快樂值計算繁榮等級。
pub fn prosperity_from_avg(avg_happiness: u8) -> ProsperityLevel {
    match avg_happiness {
        0..=29  => ProsperityLevel::Withered,
        30..=54 => ProsperityLevel::Calm,
        55..=74 => ProsperityLevel::Thriving,
        _       => ProsperityLevel::Vibrant,
    }
}

/// 由 u8 還原等級（快照解碼用；超出範圍視為 Calm）。
pub fn level_from_u8(v: u8) -> ProsperityLevel {
    match v {
        0 => ProsperityLevel::Withered,
        2 => ProsperityLevel::Thriving,
        3 => ProsperityLevel::Vibrant,
        _ => ProsperityLevel::Calm,
    }
}

/// 等級變化廣播文字。
pub fn prosperity_changed_msg(old: ProsperityLevel, new: ProsperityLevel) -> String {
    let dir = if (new as u8) > (old as u8) { "提升" } else { "降低" };
    format!(
        "{} 城鎮繁榮{}至【{}】（{}→{}）",
        new.emoji(), dir, new.name(), old.name(), new.name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_withered_threshold() {
        assert_eq!(prosperity_from_avg(0), ProsperityLevel::Withered);
        assert_eq!(prosperity_from_avg(29), ProsperityLevel::Withered);
    }

    #[test]
    fn test_calm_threshold() {
        assert_eq!(prosperity_from_avg(30), ProsperityLevel::Calm);
        assert_eq!(prosperity_from_avg(54), ProsperityLevel::Calm);
    }

    #[test]
    fn test_thriving_threshold() {
        assert_eq!(prosperity_from_avg(55), ProsperityLevel::Thriving);
        assert_eq!(prosperity_from_avg(74), ProsperityLevel::Thriving);
    }

    #[test]
    fn test_vibrant_threshold() {
        assert_eq!(prosperity_from_avg(75), ProsperityLevel::Vibrant);
        assert_eq!(prosperity_from_avg(100), ProsperityLevel::Vibrant);
    }

    #[test]
    fn test_level_as_u8() {
        assert_eq!(ProsperityLevel::Withered.as_u8(), 0);
        assert_eq!(ProsperityLevel::Calm.as_u8(), 1);
        assert_eq!(ProsperityLevel::Thriving.as_u8(), 2);
        assert_eq!(ProsperityLevel::Vibrant.as_u8(), 3);
    }

    #[test]
    fn test_level_from_u8_roundtrip() {
        for v in 0u8..=3 {
            assert_eq!(level_from_u8(v).as_u8(), v);
        }
    }

    #[test]
    fn test_name_not_empty() {
        for lv in [ProsperityLevel::Withered, ProsperityLevel::Calm, ProsperityLevel::Thriving, ProsperityLevel::Vibrant] {
            assert!(!lv.name().is_empty());
        }
    }

    #[test]
    fn test_prosperity_changed_msg_upgrade() {
        let msg = prosperity_changed_msg(ProsperityLevel::Calm, ProsperityLevel::Thriving);
        assert!(msg.contains("提升"));
        assert!(msg.contains("生機"));
    }

    #[test]
    fn test_prosperity_changed_msg_downgrade() {
        let msg = prosperity_changed_msg(ProsperityLevel::Thriving, ProsperityLevel::Calm);
        assert!(msg.contains("降低"));
        assert!(msg.contains("平靜"));
    }

    #[test]
    fn test_level_ordering() {
        assert!((ProsperityLevel::Vibrant as u8) > (ProsperityLevel::Withered as u8));
        assert!((ProsperityLevel::Thriving as u8) > (ProsperityLevel::Calm as u8));
    }
}
