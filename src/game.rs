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

            // 定期把「線上已登入玩家」的位置 + 乙太快照落地（每 ~10 秒一次）。
            // 先前只有玩家離線時才記,線上玩家撐不過 server 重啟（換版）——乙太會歸零。
            // 這裡讓線上玩家的狀態也持續落地,重啟後重連即帶回。
            // 只記已登入玩家（id 在 users 裡）；訪客 id 隨機、不記,避免 cache 無界成長。
            if tick % (TICK_HZ as u64 * 10) == 0 {
                let online: Vec<(uuid::Uuid, String, String, f32, f32, u32)> = {
                    let players = app.players.read().unwrap();
                    players
                        .values()
                        .filter(|p| app.users.get(p.id).is_some())
                        .map(|p| (p.id, p.name.clone(), p.species.clone(), p.x, p.y, p.ether))
                        .collect()
                };
                if !online.is_empty() {
                    // 先更新行程內 cache（同步,供重連 recall）,再非同步 upsert 到 Postgres。
                    app.positions.remember_all(
                        online.iter().map(|(id, _, _, x, y, e)| (*id, *x, *y, *e)),
                    );
                    app.positions.flush_online(&online).await;
                }
            }
        }
    });
}
