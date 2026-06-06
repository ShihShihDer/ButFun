//! 玩家個人資料編輯端點(目前只有改顯示名)。
//!
//! 對齊 PLAN.md 高優先薄切片「玩家改顯示名」(owner 親自點名,玩家建議多筆求改名與隱私訴求)。
//! 刻意**獨立成新檔、不碰 ws / 遊戲迴圈 / 廣播 shape**:本切片只做「改名 API + 持久化」這半。
//! 流程是玩家 PATCH 自己的顯示名,過 `sanitize_name`,再更新 `UserStore`(append 持久化)。
//! 因 `ws.rs` 連線時即時讀 `UserStore`(authed 路徑 `user.name`),改名後**重連**即生效、
//! 重啟也還在(append last-wins,見 `users::index_users`)。
//!
//! 「目前線上、不重連就即時反映 HUD / 聊天 / 快照」的 live 廣播刻意留給 backend lane,
//! 那會動 live 廣播 shape、屬架構級接線(依 PLAN.md 分工),本 lane 不搶那段。

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::patch;
use axum::Router;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::user_id_from_cookies;
use crate::state::AppState;

pub fn profile_router() -> Router<AppState> {
    Router::new().route("/api/profile", patch(patch_profile))
}

/// 改名請求 body:`{"name": "新顯示名"}`。
#[derive(Deserialize)]
struct ProfileUpdate {
    name: String,
}

/// 改名成功的回應(回出清理後的實際結果,讓前端直接拿乾淨值更新 HUD)。
#[derive(Serialize)]
struct ProfileResponse {
    id: Uuid,
    name: String,
    species: String,
}

/// `PATCH /api/profile` — 改自己的顯示名。
///
/// - 未設 OAuth(訪客模式,無 session)→ 503。
/// - 未登入 / session cookie 無效 → 401。
/// - cookie 對得上但帳號已不在 → 401。
///
/// 名字一律過 `sanitize_name`(濾控制字元、截 24 字、空退「拓荒者」),不信任客戶端。
/// 只能改**自己**的名:`uid` 取自呼叫者自己的簽章 session,無法指定改別人。
async fn patch_profile(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<ProfileUpdate>,
) -> Response {
    let Some(cfg) = app.auth.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth 尚未設定").into_response();
    };
    let Some(uid) = user_id_from_cookies(&headers, &cfg.session_secret) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match app.users.rename(uid, &update.name).await {
        Some(u) => {
            // 即時反映到線上世界:若該玩家此刻在線,更新其權威 Player 的 name,下一張快照就帶新名
            // (HUD / 世界名牌 / 聊天 from 都讀這個)——不必重連。離線玩家下次進場時 ws.rs 從
            // UserStore 讀到新名,同樣生效。
            if let Some(p) = app.players.write().unwrap().get_mut(&uid) {
                p.name = u.name.clone();
            }
            Json(ProfileResponse {
                id: u.id,
                name: u.name,
                species: u.species,
            })
            .into_response()
        }
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}
