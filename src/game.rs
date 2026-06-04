//! 權威遊戲迴圈：固定 tick 整合所有玩家位置，廣播世界快照。

use std::time::Duration;

use crate::protocol::ServerMsg;
use crate::state::{AppState, Player};

/// 每秒 tick 數（伺服器模擬頻率）。
const TICK_HZ: f32 = 15.0;

/// 定期把所有在線玩家位置寫回持久層的間隔(tick 數)。約 30 秒 @15Hz。
const SAVE_EVERY_TICKS: u64 = 450;

/// 啟動遊戲迴圈，常駐執行。
pub fn spawn(app: AppState) {
    tokio::spawn(async move {
        let dt = 1.0 / TICK_HZ;
        let mut interval = tokio::time::interval(Duration::from_secs_f32(dt));
        let mut tick: u64 = 0;

        loop {
            interval.tick().await;
            tick += 1;

            // 整合位置並建立快照（短暫持鎖，不跨 await）。
            let snapshot = {
                let mut players = app.players.write().unwrap();
                for p in players.values_mut() {
                    p.step(dt);
                }

                ServerMsg::Snapshot {
                    tick,
                    players: players.values().map(|p| p.view()).collect(),
                }
            };

            // 沒有訂閱者時 send 會回 Err，忽略即可。
            if let Ok(json) = serde_json::to_string(&snapshot) {
                let _ = app.tx.send(json);
            }

            // 定期把所有玩家位置寫回(防伺服器非正常結束時 cleanup 沒跑到)。
            // 在鎖內快照後丟鎖,寫回丟到背景任務做,不拖慢 tick 節奏;無 DB 時為 no-op。
            if tick % SAVE_EVERY_TICKS == 0 {
                let to_save: Vec<Player> = {
                    let players = app.players.read().unwrap();
                    players.values().cloned().collect()
                };
                if !to_save.is_empty() {
                    let store = app.store.clone();
                    tokio::spawn(async move {
                        for p in &to_save {
                            store.save(p).await;
                        }
                    });
                }
            }
        }
    });
}
