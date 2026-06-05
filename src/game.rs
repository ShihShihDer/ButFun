//! 權威遊戲迴圈：固定 tick 整合所有玩家位置，廣播世界快照。

use std::time::Duration;

use crate::protocol::ServerMsg;
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

            // 推進農地成長（短暫持鎖，不跨 await）。
            let field_view = {
                let mut field = app.field.write().unwrap();
                field.tick(dt);
                field.view()
            };

            // 推進日夜時鐘（短暫持鎖，不跨 await）。
            let daynight_view = {
                let mut daynight = app.daynight.write().unwrap();
                daynight.advance(dt);
                daynight.view()
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
                    field: field_view,
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
