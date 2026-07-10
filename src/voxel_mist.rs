//! 乙太方界·晨霧 v1（ROADMAP 913）。
//!
//! 天氣（`voxel_weather`，700）至今只有晴／雨，季節（798）換的是色調，環境現象一路疊了
//! 雨（700）／初雪（900）／彩虹（780）／流星（904）——但它們全是「天上落下或掛著的東西」。
//! 本刀補上一條從沒碰過的軸：**大氣能見度**——每個清晨，整片乙太方界浮起一層薄霧，
//! 遠處的建築與樹在霧裡朦朧成剪影，等太陽升高再一點點散去（前端縮短 Three.js fog 的
//! 能見度、日出後漸還原，零協議破壞：純用既有廣播的 `time_of_day` 本地判定，不新增欄位）。
//! 而**每個清晨霧起的那一刻**，附近醒著的居民抬頭望向朦朧的世界，冒一句應景的晨霧感言、
//! 心情微亮，並把這一刻記進城鎮動態牆——不在線上的玩家回來也讀得到
//! 「清晨起了一層薄霧，露娜說走進霧裡像走進夢」（PLAN_ETHERVOX 北極星「日記／生命故事」）。
//!
//! **與既有環境事件 razor-sharp 區隔**：
//! - 與雨天反應（701）：雨是**降水**、每場雨都可能冒句；晨霧是**能見度**、綁定「清晨」時段、
//!   一天只在霧起那一刻觸發一次（且與是否下雨無關——晴天的清晨照樣起霧）。
//! - 與初雪（900）：初雪一冬僅一次、綁「冬季 ∧ 下雨」；晨霧**每天**清晨都來一次、四季皆有、
//!   不看季節不看天氣，是日復一日的清晨儀式感（療癒世界的作息節律），份量與稀有度都不同。
//! - 與黎明過渡台詞（`voxel_time` 的 Dawn 問候）：那是「時段切換」的問候；晨霧是「大氣現象」，
//!   前端有真實視覺（能見度縮短），且反應綁在霧起、非綁在單純的時段跳變。
//!
//! **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性狀態機／純函式（晨霧偵測、
//! 感言選句、Feed 摘要）；旗標消費、居民反應、Feed 廣播、前端霧效渲染都在 `voxel_ws.rs`／前端
//! （沿用初雪 900／季節反應 798 的短鎖循序手法）。

/// 每日晨霧的狀態機：記住「今天清晨的晨霧是否已播報過」，讓每個清晨的霧起只觸發一次居民反應。
///
/// 設計要點：
/// - **綁定清晨時段**：呼叫端傳入「當前是否為清晨（`TimePhase::Dawn`）」與「世界第幾天」。
///   進入清晨且今天尚未播過 → 觸發一次；同一個清晨後續 tick 不再重複。
/// - **每天重來**：用「世界天數」當去重鍵（`announced_day`）——天數一變（跨到隔天），
///   隔天清晨自然重新觸發一次。不需在非清晨時做任何重置，語意單純、無邊界抖動。
/// - 世界初始 `time_of_day ≈ 0.42`（白天、非清晨），故啟動當下不會誤觸發，第一縷晨霧
///   落在隔天清晨。
#[derive(Debug, Clone, Default)]
pub struct MistDayTracker {
    /// 最近一次已播報晨霧的世界天數（`None` = 從未播過）。
    announced_day: Option<u64>,
}

impl MistDayTracker {
    /// 建一個乾淨的追蹤器（尚未播過任何一天的晨霧）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 每輪 tick 呼叫。`is_dawn`＝當前是否為清晨時段；`day`＝世界第幾天（`days_elapsed`）。
    /// 回傳 `true` 若「這一刻是今天清晨第一次起霧」（呼叫端據此觸發居民反應＋Feed）。
    ///
    /// 確定性、可窮舉測試：同一狀態＋同一輸入永遠得同一結果與同一新狀態。
    pub fn update(&mut self, is_dawn: bool, day: u64) -> bool {
        if !is_dawn {
            return false;
        }
        if self.announced_day == Some(day) {
            return false; // 今天清晨已播過。
        }
        self.announced_day = Some(day);
        true
    }

    /// 今天（`day`）的晨霧是否已播報過（測試／狀態查詢用）。
    pub fn announced_today(&self, day: u64) -> bool {
        self.announced_day == Some(day)
    }
}

/// 今天清晨霧起那一刻，附近醒著的居民抬頭冒出的晨霧感言池（確定性選句，由呼叫端傳 `pick` 索引）。
/// 面向玩家字串（繁中；i18n 友善，集中此處）。調性守療癒底線：晨霧是安靜、朦朧、帶睡意的溫柔時刻。
const MORNING_MIST_LINES: [&str; 5] = [
    "晨霧裡，世界都變得朦朧又安靜了。",
    "這樣的霧，走著走著像走進夢裡。",
    "晨霧還沒散呢，空氣涼涼濕濕的，真舒服。",
    "霧裡的清晨最適合慢慢地醒來。",
    "等這層霧散了，今天大概會是個好天氣吧。",
];

/// 依 `pick` 索引（呼叫端傳任意數，內部取模）挑一句晨霧感言。確定性、可測。
pub fn morning_mist_line(pick: usize) -> &'static str {
    MORNING_MIST_LINES[pick % MORNING_MIST_LINES.len()]
}

/// 今天清晨霧起時，寫進城鎮動態牆的一則摘要（不在線上的玩家回來也讀得到）。
/// 面向玩家字串（繁中；i18n 友善）。
pub fn morning_mist_feed_detail() -> &'static str {
    "清晨的乙太方界起了一層薄霧，遠處的屋與樹都朦朧成了剪影。"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 非清晨永不觸發() {
        let mut t = MistDayTracker::new();
        assert!(!t.update(false, 0));
        assert!(!t.update(false, 1));
        assert!(!t.announced_today(0));
    }

    #[test]
    fn 清晨當天第一次觸發一次() {
        let mut t = MistDayTracker::new();
        assert!(t.update(true, 3)); // 第 3 天清晨霧起：觸發
        assert!(t.announced_today(3));
    }

    #[test]
    fn 同一天清晨後續tick不再觸發() {
        let mut t = MistDayTracker::new();
        assert!(t.update(true, 3)); // 霧起
        assert!(!t.update(true, 3)); // 同一個清晨後續輪：不再觸發
        assert!(!t.update(true, 3));
    }

    #[test]
    fn 跨到隔天清晨重新觸發() {
        let mut t = MistDayTracker::new();
        assert!(t.update(true, 3)); // 第 3 天
        // 第 3 天白天（非清晨）：不觸發也不重置。
        assert!(!t.update(false, 3));
        // 第 4 天清晨：重新觸發。
        assert!(t.update(true, 4));
        assert!(t.announced_today(4));
    }

    #[test]
    fn 清晨到白天再到隔天清晨的完整一輪() {
        let mut t = MistDayTracker::new();
        assert!(t.update(true, 0)); // 第 0 天清晨
        assert!(!t.update(true, 0)); // 還在清晨
        assert!(!t.update(false, 0)); // 白天
        assert!(!t.update(false, 0)); // 傍晚（仍第 0 天）
        assert!(t.update(true, 1)); // 第 1 天清晨：又一次晨霧
    }

    #[test]
    fn 感言選句循環且穩定() {
        // 同索引恆得同句（確定性）。
        assert_eq!(morning_mist_line(0), morning_mist_line(MORNING_MIST_LINES.len()));
        // 覆蓋整池不 panic、皆非空。
        for i in 0..MORNING_MIST_LINES.len() * 2 {
            assert!(!morning_mist_line(i).is_empty());
        }
    }

    #[test]
    fn feed摘要非空() {
        assert!(!morning_mist_feed_detail().is_empty());
    }
}
