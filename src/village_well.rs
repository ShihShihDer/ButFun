//! 故鄉古井系統（ROADMAP 640，禱告驅動）。
//!
//! **緣起**：AI 居民「諾娃」反覆向世界禱告——「願農田旁能有水源」「願農田旁的水渠能順利
//! 修好，讓作物不再乾旱」（見 `data/prayers.jsonl`，是出現最多次的單一禱告）。造世界的 AI
//! 裁決這份願望合乎世界、對居民好、也療癒，於是在公共農田旁立起一口**故鄉古井**作為回應。
//!
//! **效用**：古井每隔 `WELL_INTERVAL` 秒自動替公共農田裡所有缺水的作物補滿濕度
//! （複用 `Field::water_all_planted`，與下雨澆田／灑水器同一套澆水介面），讓故鄉共享的田
//! 「不再乾旱」。純正向、零懲罰——只是把「逐格手動澆公田」的重複勞動免去，呼應 soil_vitality
//! 「純正向」療癒基調；作物成長仍走原本的節奏（古井只補水，不加速生長、不憑空生出收成）。
//!
//! **成本紀律**：古井是世界固定設施（位置由常數決定），**零持久化、零 migration、零 LLM、
//! 零經濟**（不產出任何物品／乙太，只滋潤玩家本來就種下的作物，與既有的雨水／灑水器同性質）。

use crate::field::Field;

/// 古井汲水間隔（秒）：每隔這麼久自動替公田缺水作物補一次水。
/// 刻意比灑水器（30s）慢一階——它免費、無人看管、覆蓋整塊公田，節律放緩較不失衡。
pub const WELL_INTERVAL: f32 = 40.0;

/// 汲水後「水波盪漾」的視覺窗（秒）：剛汲完水的這段時間，前端在井口畫一圈擴散水波。
/// 取一個略寬的窗（>一個廣播節律）讓水波在快照節流下仍穩定可見，不會一閃即逝。
pub const WELL_PULSE_SECS: f32 = 4.0;

/// 古井的世界座標（像素）。立在公共農田（origin 2200,2200，6×4 格 ×48px）左緣外側一點，
/// 像田邊真有一口井——`state::PUB_FIELD_ORIGIN_*` 是田左上角，這裡取左緣外、縱向置中。
pub const WELL_X: f32 = 2200.0 - 56.0;
pub const WELL_Y: f32 = 2200.0 + 96.0;

/// 一口故鄉古井的執行期狀態（記憶體模式，重啟由常數重新立起，無存檔）。
#[derive(Debug, Clone)]
pub struct VillageWell {
    /// 距下次汲水還剩幾秒。
    cooldown: f32,
    /// 距上次汲水已過幾秒（`f32::INFINITY` 表示從未汲過——剛開服時不畫水波）。
    since_watered: f32,
}

impl Default for VillageWell {
    fn default() -> Self {
        Self {
            // 開服即進入第一個間隔倒數（不一上線就汲水，給田一點自然乾濕節奏的起頭）。
            cooldown: WELL_INTERVAL,
            since_watered: f32::INFINITY,
        }
    }
}

impl VillageWell {
    pub fn new() -> Self {
        Self::default()
    }

    /// 推進 `dt` 秒；倒數到 0 時替 `field` 內所有缺水作物補水，重置倒數並記下「剛汲水」。
    /// 回傳「這次 tick 實際澆到幾格」（0＝還沒到汲水時刻，或到了但田裡沒有缺水的格）。
    /// 壞 dt（負/NaN）夾成 0 不推進，確定性、好測。
    pub fn tick(&mut self, dt: f32, field: &mut Field) -> u32 {
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
        // 累積「距上次汲水」（封頂於水波窗 + 1，避免浮點無限增長；只需區分窗內／窗外）。
        if self.since_watered.is_finite() {
            self.since_watered = (self.since_watered + dt).min(WELL_PULSE_SECS + 1.0);
        }
        self.cooldown -= dt;
        if self.cooldown > 0.0 {
            return 0;
        }
        self.cooldown = WELL_INTERVAL;
        self.since_watered = 0.0;
        field.water_all_planted()
    }

    /// 是否剛汲過水（位於水波視覺窗內）——供前端在井口畫擴散水波。純讀狀態、好測。
    pub fn recently_watered(&self) -> bool {
        self.since_watered.is_finite() && self.since_watered < WELL_PULSE_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::Field;
    use crate::season::Season;

    /// 種一格、翻土、播種、補一次水後讓它變乾，方便測井是否會補水。
    fn dry_field_with_crop() -> Field {
        let mut f = Field::new();
        // (0,0) 翻土→播種→澆水長一會兒讓水耗盡（needs_water 變 true）。
        f.till(0, 0);
        f.plant(0, 0);
        f.water(0, 0);
        // 推進到水耗盡但還沒成熟，使該格缺水。
        f.tick(crate::crops::MOISTURE_PER_WATER, Season::Summer);
        f
    }

    #[test]
    fn does_not_water_before_interval() {
        let mut well = VillageWell::new();
        let mut f = dry_field_with_crop();
        // 還沒到間隔：不汲水。
        let n = well.tick(WELL_INTERVAL - 1.0, &mut f);
        assert_eq!(n, 0, "未到汲水間隔不應澆水");
        assert!(!well.recently_watered(), "從未汲水不應顯示水波");
    }

    #[test]
    fn waters_dry_crops_at_interval() {
        let mut well = VillageWell::new();
        let mut f = dry_field_with_crop();
        let n = well.tick(WELL_INTERVAL, &mut f);
        assert_eq!(n, 1, "到汲水間隔應替缺水的 1 格補水");
        assert!(well.recently_watered(), "剛汲完水應在水波窗內");
    }

    #[test]
    fn pulse_window_expires() {
        let mut well = VillageWell::new();
        let mut f = dry_field_with_crop();
        well.tick(WELL_INTERVAL, &mut f); // 汲水
        assert!(well.recently_watered());
        // 過了水波窗：不再顯示水波（用乾田、避免又觸發下一輪汲水）。
        let mut empty = Field::new();
        well.tick(WELL_PULSE_SECS + 0.5, &mut empty);
        assert!(!well.recently_watered(), "超過水波窗應停止顯示水波");
    }

    #[test]
    fn waters_nothing_when_field_empty() {
        let mut well = VillageWell::new();
        let mut empty = Field::new();
        let n = well.tick(WELL_INTERVAL, &mut empty);
        assert_eq!(n, 0, "空田沒有缺水作物可澆");
        // 仍視為汲過水（井有運轉），但沒澆到格——水波仍顯示（井確實打了水上來）。
        assert!(well.recently_watered(), "井運轉了就有水波，即便田裡沒缺水的格");
    }

    #[test]
    fn bad_dt_does_not_advance() {
        let mut well = VillageWell::new();
        let mut f = dry_field_with_crop();
        let n1 = well.tick(f32::NAN, &mut f);
        let n2 = well.tick(-5.0, &mut f);
        assert_eq!(n1, 0);
        assert_eq!(n2, 0);
        assert!(!well.recently_watered(), "壞 dt 不推進、不汲水");
    }

    #[test]
    fn repeated_cycles_keep_field_watered() {
        // 連續兩個間隔：每輪都把又乾掉的作物補回來（井長期看顧公田）。
        let mut well = VillageWell::new();
        let mut f = dry_field_with_crop();
        let n1 = well.tick(WELL_INTERVAL, &mut f);
        assert_eq!(n1, 1, "第一輪補水");
        // 讓它再次乾掉。
        f.tick(crate::crops::MOISTURE_PER_WATER, Season::Summer);
        let n2 = well.tick(WELL_INTERVAL, &mut f);
        assert_eq!(n2, 1, "第二輪再次補水——井長期看顧公田");
    }
}
