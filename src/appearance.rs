//! 外觀自訂端點（ROADMAP 98 捏臉系統）。
//!
//! `PATCH /api/appearance`：已登入玩家更新帽型 / 膚色 / 護目鏡鏡片色。
//! 三個選項各 0~4 共五選；伺服器截到合法值（min(v, 4)）後持久化到 users 表，
//! 並即時更新在線玩家的 Player 狀態，讓下一幀快照就帶新外觀——不必重連。

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::patch;
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::auth::user_id_from_cookies;
use crate::state::AppState;

pub fn appearance_router() -> Router<AppState> {
    Router::new().route("/api/appearance", patch(patch_appearance))
}

/// 外觀更新請求 body。
#[derive(Deserialize)]
struct AppearanceUpdate {
    hair_style: u8,
    skin_tone: u8,
    goggle_color: u8,
}

/// 成功回應：回傳更新後的值（前端可直接用這個更新 UI 而不必等下一幀快照）。
#[derive(Serialize)]
struct AppearanceResponse {
    hair_style: u8,
    skin_tone: u8,
    goggle_color: u8,
}

/// `PATCH /api/appearance` — 更新自己的角色外觀。
///
/// - 未設 OAuth → 503。
/// - 未登入 / session 無效 → 401。
/// - 查無帳號 → 401。
/// - 更新成功 → 200 JSON `AppearanceResponse`。
async fn patch_appearance(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<AppearanceUpdate>,
) -> Response {
    let Some(cfg) = app.auth.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth 尚未設定").into_response();
    };
    let Some(uid) = user_id_from_cookies(&headers, &cfg.session_secret) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(user) = app
        .users
        .update_appearance(uid, update.hair_style, update.skin_tone, update.goggle_color)
        .await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    // 即時反映到在線玩家的 Player 狀態，下一幀快照就帶新外觀。
    {
        if let Ok(mut ps) = app.players.write() {
            if let Some(p) = ps.get_mut(&uid) {
                p.hair_style = user.hair_style;
                p.skin_tone = user.skin_tone;
                p.goggle_color = user.goggle_color;
            }
        }
    }
    Json(AppearanceResponse {
        hair_style: user.hair_style,
        skin_tone: user.skin_tone,
        goggle_color: user.goggle_color,
    })
    .into_response()
}
