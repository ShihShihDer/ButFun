//! 公會系統（ROADMAP 29）+ 持久化（ROADMAP 113）。
//!
//! 純邏輯層（GuildInner）：公會資料結構 + 建立 / 加入 / 離開 / 捐獻，無 IO / 無 WebSocket。
//! 持久化層（GuildStore）：包裝 GuildInner，Postgres 模式下 fire-and-forget 落地；
//!   記憶體模式（無 DATABASE_URL / 測試）重啟後清空，行為正確但不跨重啟。
//!
//! 限制：
//!   - 公會標籤（Tag）最多 3 個 Unicode 字元（顯示在名牌旁 [TAG]）。
//!   - 公會名稱最多 20 個字元。
//!   - 每個公會最多 20 名成員。
//!   - 整個伺服器最多 100 個公會（防止記憶體無限成長）。
//!   - 玩家同時只能屬於一個公會。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// 建立公會所需乙太。
pub const GUILD_CREATE_COST: u32 = 50;
/// 公會最大成員數。
pub const MAX_GUILD_MEMBERS: usize = 20;
/// 伺服器最多公會數。
pub const MAX_GUILDS: usize = 100;
/// 公會標籤最大字元數（顯示友善：最多 3 字）。
pub const MAX_TAG_CHARS: usize = 3;
/// 公會名稱最大字元數。
pub const MAX_NAME_CHARS: usize = 20;

/// 一個公會的完整資料。
#[derive(Debug, Clone)]
pub struct Guild {
    /// 唯一識別碼。
    pub id: Uuid,
    /// 公會名稱（最多 20 字）。
    pub name: String,
    /// 公會標籤（最多 3 字，全大寫 ASCII 或繁中均可）。
    pub tag: String,
    /// 創始人（目前會長）玩家 id。
    pub founder_id: Uuid,
    /// 所有成員（含創始人），用有序 Vec 保證接任順序穩定。
    pub member_ids: Vec<Uuid>,
    /// 公會金庫（成員捐贈累積的乙太）。
    pub treasury: u32,
}

impl Guild {
    /// 成員人數。
    pub fn member_count(&self) -> usize {
        self.member_ids.len()
    }

    /// 是否已達人數上限。
    pub fn is_full(&self) -> bool {
        self.member_ids.len() >= MAX_GUILD_MEMBERS
    }

    /// 是否有此成員。
    pub fn has_member(&self, player_id: Uuid) -> bool {
        self.member_ids.contains(&player_id)
    }
}

/// 公會簡介（供瀏覽清單使用，不含完整成員列表）。
#[derive(Debug, Clone)]
pub struct GuildBrief {
    pub id: Uuid,
    pub name: String,
    pub tag: String,
    pub member_count: usize,
    pub treasury: u32,
}

/// 離開操作的內部效果（供持久化層判斷需要哪些 DB 操作）。
enum LeaveEffect {
    /// 最後一名成員離開，公會已解散。
    Dissolved { guild_id: Uuid },
    /// 會長離開，由新會長接班。
    FounderChanged { guild_id: Uuid, new_founder: Uuid },
    /// 普通成員離開。
    MemberLeft { guild_id: Uuid },
}

// ── 純記憶體邏輯層 ────────────────────────────────────────────────────────────

/// 純記憶體公會邏輯層（不含 IO，供 GuildStore 包裝）。
#[derive(Debug, Default)]
struct GuildInner {
    guilds: HashMap<Uuid, Guild>,
    member_to_guild: HashMap<Uuid, Uuid>,
    tags: HashMap<String, Uuid>,
}

impl GuildInner {
    fn new() -> Self {
        Self::default()
    }

    fn guild_of(&self, player_id: Uuid) -> Option<Uuid> {
        self.member_to_guild.get(&player_id).copied()
    }

    fn tag_of(&self, player_id: Uuid) -> Option<String> {
        let gid = self.guild_of(player_id)?;
        self.guilds.get(&gid).map(|g| g.tag.clone())
    }

    fn get(&self, guild_id: Uuid) -> Option<&Guild> {
        self.guilds.get(&guild_id)
    }

    fn brief_list(&self) -> Vec<GuildBrief> {
        let mut list: Vec<GuildBrief> = self.guilds.values().map(|g| GuildBrief {
            id: g.id,
            name: g.name.clone(),
            tag: g.tag.clone(),
            member_count: g.member_count(),
            treasury: g.treasury,
        }).collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    fn create(
        &mut self,
        founder_id: Uuid,
        name: String,
        tag: String,
    ) -> Result<Uuid, String> {
        if self.member_to_guild.contains_key(&founder_id) {
            return Err("你已在一個公會中，請先離開再建立新公會".into());
        }
        if self.guilds.len() >= MAX_GUILDS {
            return Err("伺服器公會數已達上限".into());
        }
        let name = sanitize_guild_text(&name, MAX_NAME_CHARS)
            .ok_or_else(|| "公會名稱不能為空或太長".to_string())?;
        let tag = sanitize_tag(&tag)
            .ok_or_else(|| "公會標籤需為 1–3 個字元（英文會自動轉大寫）".to_string())?;
        if self.tags.contains_key(&tag) {
            return Err(format!("[{}] 標籤已被其他公會使用", tag));
        }
        if self.guilds.values().any(|g| g.name == name) {
            return Err(format!("「{}」公會名稱已存在", name));
        }
        let id = Uuid::new_v4();
        let guild = Guild {
            id,
            name,
            tag: tag.clone(),
            founder_id,
            member_ids: vec![founder_id],
            treasury: 0,
        };
        self.guilds.insert(id, guild);
        self.member_to_guild.insert(founder_id, id);
        self.tags.insert(tag, id);
        Ok(id)
    }

    fn join(&mut self, guild_id: Uuid, player_id: Uuid) -> Result<(), String> {
        if self.member_to_guild.contains_key(&player_id) {
            return Err("你已在一個公會中，請先離開再加入".into());
        }
        let guild = self.guilds.get_mut(&guild_id)
            .ok_or_else(|| "找不到該公會".to_string())?;
        if guild.is_full() {
            return Err(format!("「{}」公會已達人數上限（{}人）", guild.name, MAX_GUILD_MEMBERS));
        }
        guild.member_ids.push(player_id);
        self.member_to_guild.insert(player_id, guild_id);
        Ok(())
    }

    fn leave(&mut self, player_id: Uuid) -> Result<LeaveEffect, String> {
        let guild_id = self.member_to_guild.remove(&player_id)
            .ok_or_else(|| "你不在任何公會中".to_string())?;

        let effect = if let Some(guild) = self.guilds.get_mut(&guild_id) {
            guild.member_ids.retain(|&m| m != player_id);
            if guild.member_ids.is_empty() {
                LeaveEffect::Dissolved { guild_id }
            } else if guild.founder_id == player_id {
                let new_founder = guild.member_ids[0];
                guild.founder_id = new_founder;
                LeaveEffect::FounderChanged { guild_id, new_founder }
            } else {
                LeaveEffect::MemberLeft { guild_id }
            }
        } else {
            LeaveEffect::Dissolved { guild_id }
        };

        if let LeaveEffect::Dissolved { guild_id } = &effect {
            if let Some(guild) = self.guilds.remove(guild_id) {
                self.tags.remove(&guild.tag);
            }
        }
        Ok(effect)
    }

    /// 捐贈，回傳 (guild_id, 新金庫餘額) 供持久化層使用。
    fn donate(&mut self, player_id: Uuid, amount: u32) -> Result<(Uuid, u32), String> {
        if amount == 0 {
            return Err("捐贈金額需大於 0".into());
        }
        let guild_id = self.guild_of(player_id)
            .ok_or_else(|| "你不在任何公會中".to_string())?;
        let guild = match self.guilds.get_mut(&guild_id) {
            Some(g) => g,
            None => return Err("公會資料異常，請重試".into()),
        };
        guild.treasury = guild.treasury.saturating_add(amount);
        Ok((guild_id, guild.treasury))
    }
}

// ── 持久化後端 ────────────────────────────────────────────────────────────────

#[derive(Clone)]
enum Backend {
    Memory,
    Postgres(sqlx::postgres::PgPool),
}

/// 公會管理器（持久化包裝層，Clone 安全，內含 Arc）。
///
/// Postgres 模式：啟動時從 DB 載回全部公會；create / join / leave / donate 時 fire-and-forget 落地。
/// 記憶體模式：重啟後清空，行為正確但不跨重啟（測試 / 無 DATABASE_URL）。
#[derive(Clone)]
pub struct GuildStore {
    inner: Arc<Mutex<GuildInner>>,
    backend: Backend,
}

impl Default for GuildStore {
    fn default() -> Self {
        Self::new()
    }
}

impl GuildStore {
    /// 記憶體模式（測試 / 無 DB）。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(GuildInner::new())),
            backend: Backend::Memory,
        }
    }

    /// Postgres 模式：啟動時從 DB 載回全部公會資料。
    pub async fn from_pool(pool: sqlx::postgres::PgPool) -> Self {
        let mut inner = GuildInner::new();
        load_from_db(&pool, &mut inner).await;
        Self {
            inner: Arc::new(Mutex::new(inner)),
            backend: Backend::Postgres(pool),
        }
    }

    /// 玩家目前所屬的公會 id。
    pub fn guild_of(&self, player_id: Uuid) -> Option<Uuid> {
        self.inner.lock().unwrap().guild_of(player_id)
    }

    /// 玩家所屬公會的標籤（供 PlayerView 快速取用）。
    pub fn tag_of(&self, player_id: Uuid) -> Option<String> {
        self.inner.lock().unwrap().tag_of(player_id)
    }

    /// 取得公會資料複本（避免持有鎖跨越 await 點）。
    pub fn get(&self, guild_id: Uuid) -> Option<Guild> {
        self.inner.lock().unwrap().get(guild_id).cloned()
    }

    /// 傳回所有公會的簡介列表（供前端瀏覽）。
    pub fn brief_list(&self) -> Vec<GuildBrief> {
        self.inner.lock().unwrap().brief_list()
    }

    /// 建立新公會。回傳 `Ok(guild_id)` 代表成功；`Err(msg)` 說明失敗原因。
    /// 呼叫端負責在背包扣除 `GUILD_CREATE_COST` 乙太。
    pub fn create(
        &self,
        founder_id: Uuid,
        name: String,
        tag: String,
    ) -> Result<Uuid, String> {
        let result = self.inner.lock().unwrap().create(founder_id, name.clone(), tag.clone());
        if let Ok(guild_id) = result {
            if let Backend::Postgres(pool) = &self.backend {
                let pool = pool.clone();
                tokio::spawn(async move {
                    // 建會 + 寫入創始成員包在單一交易：避免中途失敗留下「有公會無創始成員」的孤兒態。
                    if let Err(e) = db_create_guild_atomic(&pool, guild_id, &name, &tag, founder_id).await {
                        tracing::error!(%e, %guild_id, "工會建立交易失敗");
                    }
                });
            }
        }
        result
    }

    /// 加入既有公會。回傳 `Ok(())` 代表成功；`Err(msg)` 說明原因。
    pub fn join(&self, guild_id: Uuid, player_id: Uuid) -> Result<(), String> {
        let result = self.inner.lock().unwrap().join(guild_id, player_id);
        if result.is_ok() {
            if let Backend::Postgres(pool) = &self.backend {
                let pool = pool.clone();
                tokio::spawn(async move {
                    if let Err(e) = db_insert_member(&pool, guild_id, player_id).await {
                        tracing::error!(%e, %guild_id, %player_id, "工會成員 INSERT 失敗");
                    }
                });
            }
        }
        result
    }

    /// 玩家離開自己的公會。若玩家是最後一名成員，公會自動解散。
    pub fn leave(&self, player_id: Uuid) -> Result<(), String> {
        let result = self.inner.lock().unwrap().leave(player_id);
        match result {
            Ok(effect) => {
                if let Backend::Postgres(pool) = &self.backend {
                    let pool = pool.clone();
                    tokio::spawn(async move {
                        match effect {
                            LeaveEffect::Dissolved { guild_id } => {
                                if let Err(e) = db_delete_guild(&pool, guild_id).await {
                                    tracing::error!(%e, %guild_id, "公會解散 DELETE 失敗");
                                }
                            }
                            LeaveEffect::FounderChanged { guild_id, new_founder } => {
                                // 刪離開成員 + 換會長包在單一交易：避免「成員已刪但會長沒換」的不一致。
                                if let Err(e) = db_change_founder_atomic(&pool, guild_id, player_id, new_founder).await {
                                    tracing::error!(%e, %guild_id, "公會換會長交易失敗");
                                }
                            }
                            LeaveEffect::MemberLeft { guild_id } => {
                                if let Err(e) = db_delete_member(&pool, guild_id, player_id).await {
                                    tracing::error!(%e, "公會成員 DELETE 失敗");
                                }
                            }
                        }
                    });
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// 玩家向公會金庫捐贈乙太。回傳 `Ok(新金庫餘額)` 成功；`Err` 說明原因。
    /// 呼叫端負責驗證並扣除玩家身上的乙太。
    pub fn donate(&self, player_id: Uuid, amount: u32) -> Result<u32, String> {
        let result = self.inner.lock().unwrap().donate(player_id, amount);
        match result {
            Ok((guild_id, new_treasury)) => {
                if let Backend::Postgres(pool) = &self.backend {
                    let pool = pool.clone();
                    tokio::spawn(async move {
                        if let Err(e) = db_update_treasury(&pool, guild_id, new_treasury).await {
                            tracing::error!(%e, %guild_id, "公會金庫 UPDATE 失敗");
                        }
                    });
                }
                Ok(new_treasury)
            }
            Err(e) => Err(e),
        }
    }
}

// ── DB 輔助函式 ──────────────────────────────────────────────────────────────

/// 啟動時從 Postgres 載回全部公會資料。
async fn load_from_db(pool: &sqlx::postgres::PgPool, inner: &mut GuildInner) {
    // 先載公會主體。
    let guilds_res = sqlx::query("SELECT id, name, tag, founder_id, treasury FROM guilds")
        .fetch_all(pool)
        .await;
    match guilds_res {
        Ok(rows) => {
            for r in &rows {
                use sqlx::Row;
                let id: Uuid = match r.try_get("id") { Ok(v) => v, Err(_) => continue };
                let name: String = match r.try_get("name") { Ok(v) => v, Err(_) => continue };
                let tag: String = match r.try_get("tag") { Ok(v) => v, Err(_) => continue };
                let founder_id: Uuid = match r.try_get("founder_id") { Ok(v) => v, Err(_) => continue };
                let treasury: i32 = r.try_get("treasury").unwrap_or(0);
                inner.guilds.insert(id, Guild {
                    id,
                    name,
                    tag: tag.clone(),
                    founder_id,
                    member_ids: vec![],
                    treasury: treasury as u32,
                });
                inner.tags.insert(tag, id);
            }
        }
        Err(e) => {
            tracing::error!(%e, "載入 guilds 失敗");
            return;
        }
    }

    // 再載成員清單，補進各公會的 member_ids。
    let members_res = sqlx::query("SELECT guild_id, player_id FROM guild_members")
        .fetch_all(pool)
        .await;
    match members_res {
        Ok(rows) => {
            for r in &rows {
                use sqlx::Row;
                let guild_id: Uuid = match r.try_get("guild_id") { Ok(v) => v, Err(_) => continue };
                let player_id: Uuid = match r.try_get("player_id") { Ok(v) => v, Err(_) => continue };
                // 只有 guild 真實存在時才建 member→guild 映射：否則孤兒 guild_members 列
                // （例如公會刪除後殘留）會把玩家綁到一個查不到、退不掉的「幽靈公會」，
                // 從此無法加入/建立公會（線上鎖死）。
                if let Some(guild) = inner.guilds.get_mut(&guild_id) {
                    if !guild.member_ids.contains(&player_id) {
                        guild.member_ids.push(player_id);
                    }
                    inner.member_to_guild.insert(player_id, guild_id);
                }
            }
        }
        Err(e) => {
            tracing::error!(%e, "載入 guild_members 失敗");
        }
    }

    let guild_count = inner.guilds.len();
    let member_count = inner.member_to_guild.len();
    tracing::info!(%guild_count, %member_count, "公會資料從 DB 載回完成");
}

/// 建立公會 + 寫入創始成員：兩語句包在單一交易，任一失敗整筆回滾，
/// 不留下「有公會無成員」或「有成員無公會」的不一致列。
async fn db_create_guild_atomic(
    pool: &sqlx::postgres::PgPool,
    id: Uuid,
    name: &str,
    tag: &str,
    founder_id: Uuid,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO guilds (id, name, tag, founder_id) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(name)
    .bind(tag)
    .bind(founder_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO guild_members (guild_id, player_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(founder_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

async fn db_insert_member(
    pool: &sqlx::postgres::PgPool,
    guild_id: Uuid,
    player_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO guild_members (guild_id, player_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(guild_id)
    .bind(player_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn db_delete_member(
    pool: &sqlx::postgres::PgPool,
    guild_id: Uuid,
    player_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM guild_members WHERE guild_id = $1 AND player_id = $2")
        .bind(guild_id)
        .bind(player_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 公會解散：先刪成員，再刪公會主體——包在單一交易，避免「成員刪了但公會還在」
/// （或反之）的不一致殘留。
async fn db_delete_guild(
    pool: &sqlx::postgres::PgPool,
    guild_id: Uuid,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM guild_members WHERE guild_id = $1")
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM guilds WHERE id = $1")
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// 換會長：刪去離開的成員 + 更新 founder，包在單一交易，避免
/// 「成員已刪但會長沒換」這種會讓公會頓失會長的不一致。
async fn db_change_founder_atomic(
    pool: &sqlx::postgres::PgPool,
    guild_id: Uuid,
    leaving_player: Uuid,
    new_founder: Uuid,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM guild_members WHERE guild_id = $1 AND player_id = $2")
        .bind(guild_id)
        .bind(leaving_player)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE guilds SET founder_id = $1 WHERE id = $2")
        .bind(new_founder)
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

async fn db_update_treasury(
    pool: &sqlx::postgres::PgPool,
    guild_id: Uuid,
    treasury: u32,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE guilds SET treasury = $1 WHERE id = $2")
        .bind(treasury as i32)
        .bind(guild_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── 文字清理工具 ─────────────────────────────────────────────────────────────

/// 清理公會名稱 / 標籤：移除控制字元、截斷長度。
/// 若清理後為空則回 `None`。
fn sanitize_guild_text(s: &str, max_chars: usize) -> Option<String> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect();
    let trimmed = cleaned.trim().to_string();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

/// 清理並驗證公會標籤：ASCII 英文轉大寫、去空白、驗長度 1–3。
fn sanitize_tag(s: &str) -> Option<String> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .take(MAX_TAG_CHARS)
        .map(|c| if c.is_ascii_alphabetic() { c.to_ascii_uppercase() } else { c })
        .collect();
    let char_count = cleaned.chars().count();
    if char_count == 0 || char_count > MAX_TAG_CHARS {
        None
    } else {
        Some(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid() -> Uuid { Uuid::new_v4() }

    #[test]
    fn create_basic() {
        let store = GuildStore::new();
        let founder = uid();
        let gid = store.create(founder, "蒸汽聯盟".into(), "STA".into()).unwrap();
        let g = store.get(gid).unwrap();
        assert_eq!(g.name, "蒸汽聯盟");
        assert_eq!(g.tag, "STA");
        assert_eq!(g.member_count(), 1);
        assert_eq!(store.guild_of(founder), Some(gid));
    }

    #[test]
    fn tag_lowercased_to_upper() {
        let store = GuildStore::new();
        let gid = store.create(uid(), "測試公會".into(), "abc".into()).unwrap();
        assert_eq!(store.get(gid).unwrap().tag, "ABC");
    }

    #[test]
    fn create_duplicate_tag_fails() {
        let store = GuildStore::new();
        store.create(uid(), "公會甲".into(), "AAA".into()).unwrap();
        let err = store.create(uid(), "公會乙".into(), "AAA".into()).unwrap_err();
        assert!(err.contains("標籤已被"));
    }

    #[test]
    fn create_duplicate_name_fails() {
        let store = GuildStore::new();
        store.create(uid(), "星際探險家".into(), "AAA".into()).unwrap();
        let err = store.create(uid(), "星際探險家".into(), "BBB".into()).unwrap_err();
        assert!(err.contains("名稱已存在"));
    }

    #[test]
    fn join_and_leave() {
        let store = GuildStore::new();
        let founder = uid();
        let member = uid();
        let gid = store.create(founder, "聯合公會".into(), "UNI".into()).unwrap();
        store.join(gid, member).unwrap();
        assert_eq!(store.get(gid).unwrap().member_count(), 2);
        store.leave(member).unwrap();
        assert_eq!(store.get(gid).unwrap().member_count(), 1);
        assert_eq!(store.guild_of(member), None);
    }

    #[test]
    fn leave_last_member_dissolves_guild() {
        let store = GuildStore::new();
        let founder = uid();
        let gid = store.create(founder, "孤狼公會".into(), "WLF".into()).unwrap();
        store.leave(founder).unwrap();
        assert!(store.get(gid).is_none());
        assert!(store.guild_of(founder).is_none());
        // 標籤應已釋放，可重用。
        store.create(uid(), "新公會".into(), "WLF".into()).unwrap();
    }

    #[test]
    fn founder_succession_on_leave() {
        let store = GuildStore::new();
        let founder = uid();
        let member = uid();
        let gid = store.create(founder, "接班公會".into(), "SUC".into()).unwrap();
        store.join(gid, member).unwrap();
        store.leave(founder).unwrap();
        assert_eq!(store.get(gid).unwrap().founder_id, member);
    }

    #[test]
    fn donate_updates_treasury() {
        let store = GuildStore::new();
        let player = uid();
        let gid = store.create(player, "金庫公會".into(), "TSY".into()).unwrap();
        let new_bal = store.donate(player, 100).unwrap();
        assert_eq!(new_bal, 100);
        assert_eq!(store.get(gid).unwrap().treasury, 100);
    }

    #[test]
    fn tag_of_non_member_is_none() {
        let store = GuildStore::new();
        assert_eq!(store.tag_of(uid()), None);
    }

    #[test]
    fn cannot_join_twice() {
        let store = GuildStore::new();
        let player = uid();
        let gid1 = store.create(uid(), "公會一".into(), "G01".into()).unwrap();
        let gid2 = store.create(uid(), "公會二".into(), "G02".into()).unwrap();
        store.join(gid1, player).unwrap();
        let err = store.join(gid2, player).unwrap_err();
        assert!(err.contains("已在一個公會"));
    }
}
