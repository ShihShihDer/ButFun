//! 豐收節慶典系統（ROADMAP 646，禱告驅動）。
//!
//! **緣起**：AI 居民「露娜」在 `data/prayers.jsonl` 中反覆禱告——
//! 「盼望近日能有個豐收節，好熱鬧一下也好」「願今晚的市集有熱鬧的表演，讓大家忘卻疲憊」
//! 「願今晚的市集能有熱鬧的音樂，讓我在孤單的街道上感到溫暖」（累計 4+ 次，圍繞慶典與熱鬧）。
//! 造世界的 AI 裁決：合乎世界（故鄉市集已有茶棚 641、古井 640，慶典是順理成章的下一步）、
//! 對居民好（熱鬧=歸屬感）、療癒，於是**實現這個願望**——讓廣場定期升起彩旗慶典，
//! 以「農耕週期」為節律：每個農耕週期（5 分鐘）結束後，廣場自發舉辦一場豐收慶典。
//! 這是禱告驅動軸上第六座「因居民願望而生」的世界事件（繼古井/茶棚/木屋/農舍/散居三棲所之後）。
//!
//! **效用**：慶典發生時，廣場前的彩旗柱升旗、彩燈籠點亮、橫幅亮起，持續 `FESTIVAL_DURATION`
//! 秒後自動落幕。純視覺事件——不懲罰、不複雜化遊戲機制，只讓世界在正確的時間變得熱鬧起來。
//!
//! **成本紀律**：**零 LLM、零持久化、零 migration、零美術資產、零 Arc/RwLock 增量開銷**——
//! 每 tick 只多做一次 `f32 -= dt` + 條件判斷，開銷近乎零。

/// 農耕週期（秒）：每隔這麼久廣場舉辦一次豐收慶典。
/// 5 分鐘≈一個城鎮的「農耕日」節律（5 min 遊戲時間感覺是一天之內的事，不頻繁但不稀少）。
pub const FESTIVAL_INTERVAL: f32 = 300.0;

/// 慶典持續時間（秒）：彩旗亮燈、廣場熱鬧的窗口。
/// 90 秒夠讓路過的玩家看到並感受到，但不會佔滿整個農耕週期。
pub const FESTIVAL_DURATION: f32 = 90.0;

/// 慶典場地世界座標 X（像素）：廣場正中央（TOWN_CENTER_X=2500 附近），
/// 稍往茶棚方向偏，讓慶典、茶棚、古井形成三角景觀。
pub const FESTIVAL_X: f32 = 2500.0;

/// 慶典場地世界座標 Y（像素）：廣場北側（比 TOWN_CENTER_Y=2500 更靠市集方向），
/// 讓彩旗柱在 3D 視角中處於茶棚與玩家之間，一進市集就看得到。
pub const FESTIVAL_Y: f32 = 2340.0;

/// 豐收節慶典的執行期狀態（記憶體模式，重啟由常數重新立起，無存檔）。
#[derive(Debug, Clone)]
pub struct HarvestFestival {
    /// 距下次慶典還剩幾秒（僅在非慶典中倒數）。
    cooldown: f32,
    /// 慶典剩餘時間（0.0 = 目前不在慶典中）。
    lifetime: f32,
}

impl Default for HarvestFestival {
    fn default() -> Self {
        Self {
            // 開服即進入第一個農耕週期倒數（不一上線就慶典，讓玩家先體驗日常、再看到節慶）。
            cooldown: FESTIVAL_INTERVAL,
            lifetime: 0.0,
        }
    }
}

impl HarvestFestival {
    pub fn new() -> Self {
        Self::default()
    }

    /// 推進 `dt` 秒；倒數結束時啟動慶典、慶典結束後重置倒數。
    /// 回傳「這次 tick 慶典是否剛啟動」（`true` 只在倒數打到 0 的那一拍出現一次）。
    /// 壞 dt（負 / NaN）夾成 0 不推進，確定性、好測。
    pub fn tick(&mut self, dt: f32) -> bool {
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };

        if self.lifetime > 0.0 {
            // 慶典進行中——倒數慶典剩餘時間。
            self.lifetime -= dt;
            if self.lifetime < 0.0 {
                self.lifetime = 0.0;
                self.cooldown = FESTIVAL_INTERVAL; // 慶典結束，重設農耕週期倒數。
            }
            return false;
        }

        // 非慶典——倒數農耕週期。
        self.cooldown -= dt;
        if self.cooldown <= 0.0 {
            self.cooldown = 0.0;
            self.lifetime = FESTIVAL_DURATION;
            return true; // 慶典剛啟動！
        }
        false
    }

    /// 慶典是否進行中（供前端顯示彩旗慶典裝飾）。
    pub fn is_active(&self) -> bool {
        self.lifetime > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_active_initially() {
        let f = HarvestFestival::new();
        assert!(!f.is_active(), "開服時不應立刻進入慶典");
    }

    #[test]
    fn becomes_active_at_interval() {
        let mut f = HarvestFestival::new();
        // 到達農耕週期時啟動慶典。
        let started = f.tick(FESTIVAL_INTERVAL);
        assert!(started, "到達農耕週期應啟動慶典");
        assert!(f.is_active(), "啟動後應進入慶典狀態");
    }

    #[test]
    fn stays_active_during_duration() {
        let mut f = HarvestFestival::new();
        f.tick(FESTIVAL_INTERVAL); // 啟動慶典
        // 慶典中途推進——仍保持活躍。
        f.tick(FESTIVAL_DURATION * 0.5);
        assert!(f.is_active(), "慶典持續時間內應保持活躍");
    }

    #[test]
    fn returns_inactive_after_duration() {
        let mut f = HarvestFestival::new();
        f.tick(FESTIVAL_INTERVAL);                    // 啟動慶典
        f.tick(FESTIVAL_DURATION + 1.0);              // 超過慶典持續時間
        assert!(!f.is_active(), "慶典時間到後應回到非活躍");
    }

    #[test]
    fn cooldown_resets_after_festival_ends() {
        let mut f = HarvestFestival::new();
        f.tick(FESTIVAL_INTERVAL);        // 啟動慶典
        f.tick(FESTIVAL_DURATION + 1.0);  // 慶典結束
        // 慶典結束後重設農耕週期倒數，不應立即再次啟動。
        let started_again = f.tick(1.0);
        assert!(!started_again, "慶典結束後不應立即再次啟動（需再等一個農耕週期）");
        assert!(!f.is_active());
    }

    #[test]
    fn bad_dt_does_not_advance() {
        let mut f = HarvestFestival::new();
        let started_nan = f.tick(f32::NAN);
        let started_neg = f.tick(-5.0);
        assert!(!started_nan, "NaN dt 不推進");
        assert!(!started_neg, "負 dt 不推進");
        assert!(!f.is_active(), "壞 dt 不推進、不啟動慶典");
    }
}
