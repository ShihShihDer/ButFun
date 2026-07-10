//! 乙太方界·冬季飄雪 v1（ROADMAP 900）。
//!
//! 天氣（`voxel_weather`，700）至今永遠只有雨；季節（`voxel_season`，798）換到冬天也只是
//! 整片天地換上冷色調。本模組把兩者第一次扣在一起：**冬天下雨即飄雪**——前端把同一場雨
//! 渲染成白、較大、飄落更慢、左右輕飄的雪花（零協議破壞：前端用既有廣播的 `season`＋`raining`
//! 本地判定，不必新增欄位）；而**每個冬天第一次飄雪的那一刻**，附近醒著的居民抬頭冒一句
//! 「初雪」感言、心情微亮，並把這一刻記進城鎮動態牆——不在線上的玩家回來也讀得到
//! 「初雪落下，露娜伸手接了一片」（PLAN_ETHERVOX 北極星「日記／生命故事」）。
//!
//! **與既有環境事件的區隔**：季節輪替（798）在**換季那一刻**觸發一次反應（偏「時間流轉」的感受）；
//! 本刀觸發在**冬天內第一次下雨（飄雪）那一刻**——換季進冬天時未必正在下雨，初雪往往晚幾分鐘才來，
//! 是一個獨立於換季、專屬於冬天的天氣時刻。與雨天反應（701）也區隔：雨天反應每場雨都可能冒句，
//! 本刀一個冬天只觸發一次（罕見、有份量），且離開冬天會重置、下一個冬天再落一次初雪。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性狀態機／純函式（初雪偵測、
//! 感言選句、Feed 摘要）；旗標消費、居民反應、Feed 廣播、前端渲染都在 `voxel_ws.rs` / 前端
//! （沿用雨天反應 701 / 季節反應 798 的短鎖循序手法）。

/// 冬季初雪的季內狀態機：追蹤「是否正在飄雪」與「本冬是否已落過初雪」，
/// 每輪 tick 由呼叫端傳入當前季節是否為冬、當前是否下雨，回傳這一刻是否應觸發「初雪」播報。
///
/// 設計要點：
/// - **飄雪 = 冬季 ∧ 下雨**：雨可能在秋天就開始、延續進冬天——用「飄雪狀態」的 false→true 轉換
///   （而非「下雨」的轉換）當觸發點，才能正確涵蓋「雨跨季延續到冬天」這個邊界（跨進冬天的那一刻
///   飄雪由 false 轉 true）。
/// - **一冬僅一次**：`first_snow_announced` 記住本冬已播過初雪；離開冬季即重置，下個冬天再落一次。
#[derive(Debug, Clone, Default)]
pub struct SnowSeasonTracker {
    /// 上一輪是否正在飄雪（冬季 ∧ 下雨），用來偵測 false→true 轉換。
    was_snowing: bool,
    /// 本冬是否已播過初雪（離開冬季即重置）。
    first_snow_announced: bool,
}

impl SnowSeasonTracker {
    /// 建一個乾淨的追蹤器（尚未飄雪、本冬未播初雪）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 每輪 tick 呼叫。`is_winter`＝當前是否冬季；`raining`＝當前是否下雨。
    /// 回傳 `true` 若「這一刻是本冬第一次飄雪」（呼叫端據此觸發居民反應＋Feed）。
    ///
    /// 確定性、可窮舉測試：同一狀態＋同一輸入永遠得同一結果與同一新狀態。
    pub fn update(&mut self, is_winter: bool, raining: bool) -> bool {
        // 非冬季：重置本冬旗標與飄雪狀態，讓下一個冬天能重新落一次初雪。
        if !is_winter {
            self.was_snowing = false;
            self.first_snow_announced = false;
            return false;
        }
        // 冬季下雨即飄雪。
        let snowing = raining;
        let just_started = snowing && !self.was_snowing;
        self.was_snowing = snowing;
        if just_started && !self.first_snow_announced {
            self.first_snow_announced = true;
            return true;
        }
        false
    }

    /// 目前是否正在飄雪（測試／狀態查詢用）。
    pub fn is_snowing(&self) -> bool {
        self.was_snowing
    }
}

/// 本冬第一次飄雪時，附近醒著的居民抬頭冒出的「初雪」感言池（確定性選句，由呼叫端傳 `pick` 索引）。
/// 面向玩家字串（繁中；i18n 友善，集中此處）。調性守療癒底線：初雪是溫柔、帶點小雀躍的時刻。
const FIRST_SNOW_LINES: [&str; 5] = [
    "初雪落下來了，好美啊……",
    "今年第一場雪呢，我伸手接了一片。",
    "下雪了！冬天真的來了。",
    "雪花軟軟的，落在肩上都捨不得拍掉。",
    "第一場雪總是最讓人安靜下來。",
];

/// 依 `pick` 索引（呼叫端傳任意數，內部取模）挑一句初雪感言。確定性、可測。
pub fn first_snow_line(pick: usize) -> &'static str {
    FIRST_SNOW_LINES[pick % FIRST_SNOW_LINES.len()]
}

/// 本冬第一次飄雪時，寫進城鎮動態牆的一則摘要（不在線上的玩家回來也讀得到）。
/// 面向玩家字串（繁中；i18n 友善）。
pub fn first_snow_feed_detail() -> &'static str {
    "今年第一場雪落下了，乙太方界披上薄薄一層白。"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 非冬季永不飄雪也不觸發() {
        let mut t = SnowSeasonTracker::new();
        assert!(!t.update(false, true)); // 秋天下雨：不算飄雪、不觸發
        assert!(!t.is_snowing());
        assert!(!t.update(false, false));
        assert!(!t.is_snowing());
    }

    #[test]
    fn 冬季首次下雨觸發初雪一次() {
        let mut t = SnowSeasonTracker::new();
        // 冬天沒下雨：不飄雪、不觸發。
        assert!(!t.update(true, false));
        assert!(!t.is_snowing());
        // 冬天第一次下雨：觸發初雪。
        assert!(t.update(true, true));
        assert!(t.is_snowing());
    }

    #[test]
    fn 同一場雪只觸發一次初雪() {
        let mut t = SnowSeasonTracker::new();
        assert!(t.update(true, true)); // 初雪觸發
        assert!(!t.update(true, true)); // 雪還在下：不再觸發
        assert!(!t.update(true, true));
    }

    #[test]
    fn 本冬第二場雪不再算初雪() {
        let mut t = SnowSeasonTracker::new();
        assert!(t.update(true, true)); // 第一場雪：初雪
        assert!(!t.update(true, false)); // 雪停
        assert!(!t.is_snowing());
        assert!(!t.update(true, true)); // 本冬第二場雪：不再是初雪
    }

    #[test]
    fn 跨到下一個冬天重新落初雪() {
        let mut t = SnowSeasonTracker::new();
        assert!(t.update(true, true)); // 這個冬天的初雪
        // 春夏秋（離開冬季）：重置。
        assert!(!t.update(false, true));
        assert!(!t.update(false, false));
        assert!(!t.update(false, true));
        // 又一個冬天下雪：重新算初雪。
        assert!(t.update(true, true));
    }

    #[test]
    fn 雨跨季延續進冬天也算初雪() {
        let mut t = SnowSeasonTracker::new();
        // 秋天就開始下雨（不算飄雪）。
        assert!(!t.update(false, true));
        assert!(!t.is_snowing());
        // 雨一路下到冬天：跨進冬天那一刻，飄雪由 false→true，算初雪。
        assert!(t.update(true, true));
        assert!(t.is_snowing());
    }

    #[test]
    fn 感言選句循環且穩定() {
        // 同索引恆得同句（確定性）。
        assert_eq!(first_snow_line(0), first_snow_line(FIRST_SNOW_LINES.len()));
        // 覆蓋整池不 panic。
        for i in 0..FIRST_SNOW_LINES.len() * 2 {
            assert!(!first_snow_line(i).is_empty());
        }
    }

    #[test]
    fn feed摘要非空() {
        assert!(!first_snow_feed_detail().is_empty());
    }
}
