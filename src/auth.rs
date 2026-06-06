//! Google OAuth 流程 + 簽章式 session cookie。
//!
//! 設計重點:
//! - 身份模型 **provider 無關**(見 `users.rs`),這裡只是 Google 這個 provider 的接線。
//! - Session 為 **stateless 簽章 cookie**:`{user_id}.{HMAC-SHA256(user_id, secret)}`。
//!   伺服器不存 session 表,純驗 HMAC,簡單可橫向擴展。
//! - CSRF 防護:OAuth 起始時種一個短期 `state` cookie,callback 對齊。

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum::Router;
use axum::routing::{get, post};
use base64::Engine;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

const SESSION_COOKIE: &str = "butfun_session";
const STATE_COOKIE: &str = "butfun_oauth_state";

/// 啟動 / callback 用的設定(讀環境變數)。
#[derive(Clone)]
pub struct AuthConfig {
    pub google_client_id: String,
    pub google_client_secret: String,
    pub google_redirect_uri: String,
    pub session_secret: Vec<u8>,
    /// AI 居民自助註冊的共用金鑰(`AI_REGISTER_KEY`)。`None` = 未啟用,`/auth/ai/register` 回 503。
    /// 持有這把鑰匙才能建 AI 帳號——否則公開端點等於讓任何人無限生帳號。
    pub ai_register_key: Option<String>,
}

impl AuthConfig {
    pub fn from_env() -> Option<Self> {
        Some(Self {
            google_client_id: std::env::var("GOOGLE_CLIENT_ID").ok()?,
            google_client_secret: std::env::var("GOOGLE_CLIENT_SECRET").ok()?,
            google_redirect_uri: std::env::var("GOOGLE_REDIRECT_URI").ok()?,
            session_secret: std::env::var("BUTFUN_SESSION_SECRET").ok()?.into_bytes(),
            // 選用:沒設就不開放 AI 註冊。去頭尾空白後為空也視為沒設。
            ai_register_key: std::env::var("AI_REGISTER_KEY")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        })
    }
}

/// 已驗身的使用者抓取(從 cookie),供其他處共用。
pub fn user_id_from_cookies(headers: &HeaderMap, secret: &[u8]) -> Option<Uuid> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    let token = read_cookie(cookie_header, SESSION_COOKIE)?;
    verify_session(token, secret)
}

pub fn auth_router() -> Router<AppState> {
    Router::new()
        .route("/auth/google/start", get(google_start))
        .route("/auth/google/callback", get(google_callback))
        .route("/auth/me", get(me))
        .route("/auth/logout", post(logout))
        .route("/auth/ai/register", post(ai_register))
}

// ---- /auth/ai/register ----
// AI 居民自助註冊:持金鑰 → 建一個 provider="ai" 的帳號 → 回 session token。
// 讓 AI 測試員/居民有固定身分、進度持久化、可無上限擴增(見 docs/PLAN 的 AI 居民方向)。

#[derive(Deserialize)]
struct AiRegisterReq {
    key: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    species: Option<String>,
}

#[derive(Serialize)]
struct AiRegisterResp {
    user_id: Uuid,
    /// 可直接當 Cookie header 用:`butfun_session=<token>`。
    session: String,
}

async fn ai_register(State(app): State<AppState>, Json(req): Json<AiRegisterReq>) -> Response {
    let Some(cfg) = app.auth.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "auth 尚未設定").into_response();
    };
    let Some(expected) = cfg.ai_register_key.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "AI 註冊未啟用(伺服器未設 AI_REGISTER_KEY)",
        )
            .into_response();
    };
    // 常數時間比對金鑰,避免時序側信道。
    if !constant_time_eq(req.key.as_bytes(), expected.as_bytes()) {
        return (StatusCode::FORBIDDEN, "註冊金鑰錯誤").into_response();
    }
    // 沒給名字就配一個主題隨機代號;沒給物種就用預設。一律過既有 sanitizer(create_ai 內處理)。
    let name = req
        .name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(crate::users::random_codename);
    let species = req
        .species
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| crate::users::DEFAULT_SPECIES.to_string());
    let user = app.users.create_ai(&name, &species).await;
    let token = sign_session(&user.id, &cfg.session_secret);
    Json(AiRegisterResp {
        user_id: user.id,
        session: format!("{SESSION_COOKIE}={token}"),
    })
    .into_response()
}

// ---- /auth/google/start ----
async fn google_start(State(app): State<AppState>) -> Response {
    let Some(cfg) = app.auth.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth 尚未設定").into_response();
    };
    let state = random_b64(16);
    let scopes = "openid email profile";
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?response_type=code&client_id={cid}&redirect_uri={ru}&scope={sc}&state={st}&access_type=online&prompt=select_account",
        cid = urlencoding::encode(&cfg.google_client_id),
        ru = urlencoding::encode(&cfg.google_redirect_uri),
        sc = urlencoding::encode(scopes),
        st = urlencoding::encode(&state),
    );

    let mut resp = Redirect::temporary(&auth_url).into_response();
    // state cookie,5 分鐘,SameSite=Lax 才能在從 Google 轉回來時帶回。
    let cookie = format!(
        "{STATE_COOKIE}={state}; Path=/; Max-Age=300; HttpOnly; Secure; SameSite=Lax"
    );
    resp.headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
    resp
}

// ---- /auth/google/callback ----
#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn google_callback(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let Some(cfg) = app.auth.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth 尚未設定").into_response();
    };

    if let Some(err) = q.error {
        return (StatusCode::BAD_REQUEST, format!("Google 回傳錯誤: {err}")).into_response();
    }
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return (StatusCode::BAD_REQUEST, "缺少 code 或 state").into_response();
    };

    // 驗 state(對齊 cookie 中先前種下的值)。
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    match read_cookie(cookie_header, STATE_COOKIE) {
        Some(saved) if saved == state => {}
        _ => return (StatusCode::BAD_REQUEST, "state 對不上(防 CSRF 機制)").into_response(),
    }

    // 1) 用 code 換 access_token + id_token
    let token: TokenResponse = match exchange_code(cfg, &code).await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("換 token 失敗: {e}"),
            )
                .into_response();
        }
    };
    // 2) 用 access_token 取 userinfo
    let info: GoogleUserInfo = match fetch_userinfo(&token.access_token).await {
        Ok(i) => i,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("取 userinfo 失敗: {e}"),
            )
                .into_response();
        }
    };

    // 3) find-or-create user
    //    刻意不拿 Google 回傳的真實姓名（info.name）當顯示名——那會把本名公開廣播給所有
    //    玩家（聊天 from / HUD），是隱私問題（玩家建議 at=1780631336007）。新帳號改配一個
    //    隨機角色代號；既有帳號 find_or_create 命中即早回，不會走到這個名字。
    let user = app
        .users
        .find_or_create(
            "google",
            &info.sub,
            info.email.clone(),
            &crate::users::random_codename(),
        )
        .await;

    // 4) 種 session cookie + 清掉 state cookie + 導回 /
    let session_token = sign_session(&user.id, &cfg.session_secret);
    let session_cookie = format!(
        "{SESSION_COOKIE}={session_token}; Path=/; Max-Age=2592000; HttpOnly; Secure; SameSite=Lax"
    );
    let clear_state = format!(
        "{STATE_COOKIE}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax"
    );

    let mut resp = Redirect::temporary("/").into_response();
    resp.headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(&session_cookie).unwrap());
    resp.headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(&clear_state).unwrap());
    resp
}

// ---- /auth/me ----
#[derive(Serialize)]
struct MeResponse {
    id: Uuid,
    name: String,
    species: String,
    email: Option<String>,
}

async fn me(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(cfg) = app.auth.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OAuth 尚未設定").into_response();
    };
    let Some(uid) = user_id_from_cookies(&headers, &cfg.session_secret) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(u) = app.users.get(uid) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    Json(MeResponse {
        id: u.id,
        name: u.name,
        species: u.species,
        email: u.email,
    })
    .into_response()
}

// ---- /auth/logout ----
async fn logout() -> Response {
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let clear = format!(
        "{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax"
    );
    resp.headers_mut()
        .append(header::SET_COOKIE, HeaderValue::from_str(&clear).unwrap());
    resp
}

// ============= Google token + userinfo =============

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[allow(dead_code)]
    id_token: Option<String>,
}

#[derive(Deserialize, Debug)]
struct GoogleUserInfo {
    sub: String,
    email: Option<String>,
    /// Google 會回傳真實姓名,但我們刻意不拿來當顯示名(隱私,見上方 find-or-create
    /// 註解);保留欄位是為了文件化「收得到、但不採用」這件事。
    #[allow(dead_code)]
    name: Option<String>,
}

async fn exchange_code(cfg: &AuthConfig, code: &str) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();
    let params = [
        ("code", code),
        ("client_id", cfg.google_client_id.as_str()),
        ("client_secret", cfg.google_client_secret.as_str()),
        ("redirect_uri", cfg.google_redirect_uri.as_str()),
        ("grant_type", "authorization_code"),
    ];
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!(
            "token endpoint 回 {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    resp.json::<TokenResponse>()
        .await
        .map_err(|e| e.to_string())
}

async fn fetch_userinfo(access_token: &str) -> Result<GoogleUserInfo, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://www.googleapis.com/oauth2/v3/userinfo")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("userinfo 回 {}", resp.status()));
    }
    resp.json::<GoogleUserInfo>()
        .await
        .map_err(|e| e.to_string())
}

// ============= session 簽章 + cookie 解析 =============

fn sign_session(user_id: &Uuid, secret: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let uid = user_id.to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC 任意長度");
    mac.update(uid.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);
    format!("{uid}.{sig_b64}")
}

fn verify_session(token: &str, secret: &[u8]) -> Option<Uuid> {
    let (uid_str, _sig) = token.split_once('.')?;
    let uid = Uuid::parse_str(uid_str).ok()?;
    let expected = sign_session(&uid, secret);
    if constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        Some(uid)
    } else {
        None
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut x = 0u8;
    for i in 0..a.len() {
        x |= a[i] ^ b[i];
    }
    x == 0
}

fn read_cookie<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    for part in header.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(&format!("{name}=")) {
            return Some(v);
        }
    }
    None
}

fn random_b64(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

// ============= 純邏輯單元測試(無 IO) =============
#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-secret-do-not-leak";

    // ---- /auth/ai/register 請求合約 ----

    #[test]
    fn ai_register_req_name_species_optional() {
        // 只給 key 也要能解析(name/species 省略 → None,端點會配隨機代號 + 預設物種)。
        let r: AiRegisterReq = serde_json::from_str(r#"{"key":"abc"}"#).unwrap();
        assert_eq!(r.key, "abc");
        assert!(r.name.is_none() && r.species.is_none());
        let r2: AiRegisterReq =
            serde_json::from_str(r#"{"key":"abc","name":"居民阿一","species":"aurelian"}"#).unwrap();
        assert_eq!(r2.name.as_deref(), Some("居民阿一"));
        assert_eq!(r2.species.as_deref(), Some("aurelian"));
    }

    #[test]
    fn ai_register_wrong_key_rejected_by_constant_time_eq() {
        // 金鑰比對是這個公開端點的唯一守門——錯的金鑰一定要被擋(常數時間比對)。
        assert!(constant_time_eq(b"right-key", b"right-key"));
        assert!(!constant_time_eq(b"right-key", b"wrong-key"));
        assert!(!constant_time_eq(b"right-key", b"right-key-extra"));
    }

    // ---- sign_session / verify_session ----

    #[test]
    fn sign_then_verify_round_trip() {
        let uid = Uuid::new_v4();
        let token = sign_session(&uid, SECRET);
        // token 形如 {uuid}.{sig};驗章後應拿回同一個 uid。
        assert!(token.starts_with(&uid.to_string()));
        assert_eq!(verify_session(&token, SECRET), Some(uid));
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        let uid = Uuid::new_v4();
        let token = sign_session(&uid, SECRET);
        // 把簽章尾巴改掉一個字元 → 驗章失敗。
        let mut bad = token.clone();
        let last = bad.pop().unwrap();
        bad.push(if last == 'A' { 'B' } else { 'A' });
        assert_eq!(verify_session(&bad, SECRET), None);
    }

    #[test]
    fn verify_rejects_forged_uid_with_unknown_secret() {
        // 攻擊者自行替某個 uid 簽章,但用錯誤(不知道的)secret。
        let uid = Uuid::new_v4();
        let forged = sign_session(&uid, b"attacker-secret");
        assert_eq!(verify_session(&forged, SECRET), None);
    }

    #[test]
    fn verify_rejects_uid_swap_under_same_secret() {
        // 即使握有合法 secret,也不能把別人的 uid 套上自己的簽章。
        let real = Uuid::new_v4();
        let other = Uuid::new_v4();
        let token = sign_session(&real, SECRET);
        let (_, sig) = token.split_once('.').unwrap();
        let swapped = format!("{other}.{sig}");
        assert_eq!(verify_session(&swapped, SECRET), None);
    }

    #[test]
    fn verify_rejects_malformed_tokens() {
        assert_eq!(verify_session("沒有點號", SECRET), None);
        assert_eq!(verify_session("not-a-uuid.sig", SECRET), None);
        assert_eq!(verify_session("", SECRET), None);
    }

    // ---- constant_time_eq ----

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab")); // 長度不同
        assert!(constant_time_eq(b"", b""));
    }

    // ---- read_cookie ----

    #[test]
    fn read_cookie_single() {
        assert_eq!(read_cookie("foo=bar", "foo"), Some("bar"));
    }

    #[test]
    fn read_cookie_multiple_with_whitespace() {
        let header = "a=1; butfun_session=tok123;  b=2";
        assert_eq!(read_cookie(header, "butfun_session"), Some("tok123"));
        assert_eq!(read_cookie(header, "a"), Some("1"));
        assert_eq!(read_cookie(header, "b"), Some("2"));
    }

    #[test]
    fn read_cookie_missing_returns_none() {
        assert_eq!(read_cookie("a=1; b=2", "c"), None);
        assert_eq!(read_cookie("", "a"), None);
    }

    #[test]
    fn read_cookie_does_not_match_prefix() {
        // 找 "ab" 不應誤中 "abc=x"。
        assert_eq!(read_cookie("abc=x", "ab"), None);
    }
}

