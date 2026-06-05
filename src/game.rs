//! 權威遊戲迴圈：固定 tick 整合所有玩家位置，廣播世界快照。

use std::time::Duration;

use crate::protocol::{FieldView, ServerMsg};
use crate::state::AppState;

/// 每秒 tick 數（伺服器模擬頻率）。
const TICK_HZ: f32 = 15.0;

/// 啟動遊戲迴圈，常駐執行。
pub fn spawn(app: AppState) {
    tokio::spawn(async move {
        let dt = 1.0 / TICK_HZ;
        let mut interval = tokio::time::interval(Duration::from_secs_f32(dt));
        let mut tick: u64 = 0;

        loop {
            interval.tick().await;
            tick += 1;

            // 先推進日夜時鐘，取得當下亮度決定作物成長速度（短暫持鎖，不跨 await）。
            let (daynight_view, growth_rate) = {
                let mut daynight = app.daynight.write().unwrap();
                daynight.advance(dt);
                (daynight.view(), daynight.growth_rate())
            };

            // 推進所有玩家農地的成長：依日夜成長倍率縮放 dt——白天亮、長得快，夜裡暗、
            // 放慢（0-G「隨日夜成長」）。濕度也一併縮放，故每次澆水的總成長量不變、
            // 只有牆鐘速度隨日夜變化。同時把每塊地轉成快照、並戳上擁有者 id（`Field`
            // 自己不知道屬於誰，由這層持有的 `user_id → Field` 對映補上）。短暫持鎖，不跨 await。
            let field_views: Vec<FieldView> = {
                let mut fields = app.fields.write().unwrap();
                fields
                    .iter_mut()
                    .map(|(owner, field)| {
                        field.tick(dt * growth_rate);
                        let mut v = field.view();
                        v.owner = *owner;
                        v
                    })
                    .collect()
            };

            // 整合位置並建立快照（短暫持鎖，不跨 await）。
            let snapshot = {
                let mut players = app.players.write().unwrap();
                for p in players.values_mut() {
                    p.step(dt);
                }

                ServerMsg::Snapshot {
                    tick,
                    players: players.values().map(|p| p.view()).collect(),
                    fields: field_views,
                    daynight: daynight_view,
                }
            };

            // 沒有訂閱者時 send 會回 Err，忽略即可。
            if let Ok(json) = serde_json::to_string(&snapshot) {
                let _ = app.tx.send(json);
            }
        }
    });
}
