//! 回訪鉤子（ROADMAP 374）：登入玩家進場時，讀取「等你的東西」並回傳摘要。
//!
//! 純邏輯層（無 IO、無 WebSocket）：只讀現有記憶體狀態，不新增經濟、不修改任何值。

use std::collections::HashMap;
use uuid::Uuid;

use crate::daily_quest::{PlayerDailyState, DAILY_TASK_COUNT};
use crate::farm_crops::FarmCropRegistry;
use crate::land_plot::LandPlotRegistry;
use crate::ranching::RanchRegistry;

/// 玩家進場時「等你的東西」摘要（純讀，不修改任何狀態）。
#[derive(Debug, Clone, Default)]
pub struct ReturnSummaryData {
    /// 個人農田地塊裡已成熟、待收割的作物數（伺服器重啟後歸零）。
    pub ripe_crops: u32,
    /// 個人牧場地塊裡已累積、待領取的蛋數（伺服器重啟後歸零）。
    pub eggs_ready: u32,
    /// 今日任務已完成條數（尚無任務記錄時回 0）。
    pub daily_quests_done: u32,
    /// 今日任務總條數（尚無任務記錄時回 0；有記錄時為 DAILY_TASK_COUNT）。
    pub daily_quests_total: u32,
}

/// 為指定玩家建立回訪摘要。全部唯讀，不動任何狀態。
pub fn build_return_summary(
    uid: Uuid,
    land_plots: &LandPlotRegistry,
    farm_crops: &FarmCropRegistry,
    ranch: &RanchRegistry,
    daily_quests: &HashMap<Uuid, PlayerDailyState>,
) -> ReturnSummaryData {
    // 找到玩家的城外地塊（每人至多一塊）。
    let plot_id = land_plots.plot_of(uid);

    // 計算成熟作物數（無地塊或未種植時為 0）。
    let ripe_crops = plot_id
        .map(|pid| farm_crops.state_of(pid).iter().filter(|c| c.ripe).count() as u32)
        .unwrap_or(0);

    // 計算待領蛋數。
    let eggs_ready = plot_id
        .map(|pid| ranch.state_of(pid).1)
        .unwrap_or(0);

    // 每日任務完成度（尚無記錄 = 0/0）。
    let (daily_quests_done, daily_quests_total) = match daily_quests.get(&uid) {
        Some(state) => (state.done_count() as u32, DAILY_TASK_COUNT as u32),
        None => (0, 0),
    };

    ReturnSummaryData { ripe_crops, eggs_ready, daily_quests_done, daily_quests_total }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::farm_crops::CropKind;
    use crate::land_plot::PlotPurpose;

    #[test]
    fn no_plot_returns_zeros() {
        let uid = Uuid::new_v4();
        let land_plots = LandPlotRegistry::new();
        let farm_crops = FarmCropRegistry::new();
        let ranch = RanchRegistry::new();
        let daily_quests = HashMap::new();
        let s = build_return_summary(uid, &land_plots, &farm_crops, &ranch, &daily_quests);
        assert_eq!(s.ripe_crops, 0);
        assert_eq!(s.eggs_ready, 0);
        assert_eq!(s.daily_quests_done, 0);
        assert_eq!(s.daily_quests_total, 0);
    }

    #[test]
    fn counts_ripe_crops() {
        let uid = Uuid::new_v4();
        let mut land_plots = LandPlotRegistry::new();
        land_plots.buy(0, uid, PlotPurpose::Farm);
        let mut farm_crops = FarmCropRegistry::new();
        farm_crops.plant(0, CropKind::Wheat);
        farm_crops.plant(0, CropKind::Carrot);
        // 時間推到成熟（GROW_TIME_SECS=90，推 200 秒確保已過）。
        farm_crops.tick(200.0, false);
        let ranch = RanchRegistry::new();
        let daily_quests = HashMap::new();
        let s = build_return_summary(uid, &land_plots, &farm_crops, &ranch, &daily_quests);
        assert_eq!(s.ripe_crops, 2, "應有兩株成熟作物");
        assert_eq!(s.eggs_ready, 0);
    }

    #[test]
    fn unripe_crops_not_counted() {
        let uid = Uuid::new_v4();
        let mut land_plots = LandPlotRegistry::new();
        land_plots.buy(2, uid, PlotPurpose::Farm);
        let mut farm_crops = FarmCropRegistry::new();
        farm_crops.plant(2, CropKind::Potato);
        // 只走 10 秒，還沒熟（需要 90 秒）。
        farm_crops.tick(10.0, false);
        let ranch = RanchRegistry::new();
        let daily_quests = HashMap::new();
        let s = build_return_summary(uid, &land_plots, &farm_crops, &ranch, &daily_quests);
        assert_eq!(s.ripe_crops, 0, "未成熟不應計入");
    }

    #[test]
    fn counts_eggs_ready() {
        let uid = Uuid::new_v4();
        let mut land_plots = LandPlotRegistry::new();
        land_plots.buy(3, uid, PlotPurpose::Farm);
        let farm_crops = FarmCropRegistry::new();
        let mut ranch = RanchRegistry::new();
        ranch.buy_chicken(3);
        // 走夠久讓蛋出現（EGG_INTERVAL_SECS 約 60 秒）。
        ranch.tick(200.0);
        let daily_quests = HashMap::new();
        let s = build_return_summary(uid, &land_plots, &farm_crops, &ranch, &daily_quests);
        assert!(s.eggs_ready > 0, "應有蛋可收");
    }

    #[test]
    fn daily_quest_done_count() {
        use crate::daily_quest::PlayerDailyState;
        let uid = Uuid::new_v4();
        let land_plots = LandPlotRegistry::new();
        let farm_crops = FarmCropRegistry::new();
        let ranch = RanchRegistry::new();
        let mut daily_quests = HashMap::new();
        // 插入一個已完成 0 條的每日任務記錄。
        let state = PlayerDailyState::new(42, 0);
        daily_quests.insert(uid, state);
        let s = build_return_summary(uid, &land_plots, &farm_crops, &ranch, &daily_quests);
        assert_eq!(s.daily_quests_done, 0);
        assert_eq!(s.daily_quests_total, 3, "應有 3 條每日任務");
    }
}
