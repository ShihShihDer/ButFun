//! 公會系統（ROADMAP 29）。
//!
//! 純邏輯層：公會資料結構 + 建立 / 加入 / 離開 / 捐獻，無 IO / 無 WebSocket。
//! 玩家可建立公會（花費 50 乙太）、加入其他公會、隨時離開；
//! 創始人離開後若還有成員，第一位留下的成員接任創始人。
//!
//! 限制：
//!   - 公會標籤（Tag）最多 3 個 Unicode 字元（顯示在名牌旁 [TAG]）。
//!   - 公會名稱最多 20 個字元。
//!   - 每個公會最多 20 名成員。
//!   - 整個伺服器最多 100 個公會（防止記憶體無限成長）。
//!   - 玩家同時只能屬於一個公會。
//!   - 重啟後公會資料清空（v1 記憶體前置；後續可接 Postgres）。

use std::collections::HashMap;
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

/// 公會管理器（記憶體，重啟清空）。
#[derive(Debug, Default)]
pub struct GuildStore {
    /// 公會主索引（by id）。
    guilds: HashMap<Uuid, Guild>,
    /// 玩家→公會索引，快速查詢「我屬於哪個公會」。
    member_to_guild: HashMap<Uuid, Uuid>,
    /// 標籤唯一性索引。
    tags: HashMap<String, Uuid>,
}

impl GuildStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 玩家目前所屬的公會 id。
    pub fn guild_of(&self, player_id: Uuid) -> Option<Uuid> {
        self.member_to_guild.get(&player_id).copied()
    }

    /// 玩家所屬公會的標籤（供 PlayerView 快速取用）。
    pub fn tag_of(&self, player_id: Uuid) -> Option<String> {
        let gid = self.guild_of(player_id)?;
        self.guilds.get(&gid).map(|g| g.tag.clone())
    }

    /// 依 id 取公會（唯讀）。
    pub fn get(&self, guild_id: Uuid) -> Option<&Guild> {
        self.guilds.get(&guild_id)
    }

    /// 傳回所有公會的簡介列表（供前端瀏覽）。
    pub fn brief_list(&self) -> Vec<GuildBrief> {
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

    /// 建立新公會。
    ///
    /// 回傳 `Ok(guild_id)` 代表建立成功；`Err(msg)` 說明失敗原因（前端顯示用）。
    /// 呼叫端負責在背包扣除 `GUILD_CREATE_COST` 乙太（此函式只做純邏輯驗證 + 狀態變更）。
    pub fn create(
        &mut self,
        founder_id: Uuid,
        name: String,
        tag: String,
    ) -> Result<Uuid, String> {
        // 已有公會不能再建。
        if self.member_to_guild.contains_key(&founder_id) {
            return Err("你已在一個公會中，請先離開再建立新公會".into());
        }
        // 上限檢查。
        if self.guilds.len() >= MAX_GUILDS {
            return Err("伺服器公會數已達上限".into());
        }
        // 名稱驗證。
        let name = sanitize_guild_text(&name, MAX_NAME_CHARS)
            .ok_or_else(|| "公會名稱不能為空或太長".to_string())?;
        // 標籤驗證。
        let tag = sanitize_tag(&tag)
            .ok_or_else(|| "公會標籤需為 1–3 個字元（英文會自動轉大寫）".to_string())?;
        // 標籤唯一性。
        if self.tags.contains_key(&tag) {
            return Err(format!("[{}] 標籤已被其他公會使用", tag));
        }
        // 名稱唯一性。
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

    /// 加入既有公會。
    ///
    /// 回傳 `Ok(())` 代表加入成功；`Err(msg)` 說明原因。
    pub fn join(&mut self, guild_id: Uuid, player_id: Uuid) -> Result<(), String> {
        // 已有公會。
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

    /// 玩家離開自己的公會。
    ///
    /// 若玩家是最後一名成員，公會自動解散；
    /// 若玩家是會長但還有其他成員，第一位剩餘成員接任會長。
    /// 回傳 `Ok(())` 成功；`Err` 表示不在任何公會。
    pub fn leave(&mut self, player_id: Uuid) -> Result<(), String> {
        let guild_id = self.member_to_guild.remove(&player_id)
            .ok_or_else(|| "你不在任何公會中".to_string())?;

        let dissolve = if let Some(guild) = self.guilds.get_mut(&guild_id) {
            guild.member_ids.retain(|&m| m != player_id);
            if guild.member_ids.is_empty() {
                // 最後一人，解散。
                true
            } else {
                // 若離開的是會長，接任。
                if guild.founder_id == player_id {
                    guild.founder_id = guild.member_ids[0];
                }
                false
            }
        } else {
            false
        };

        if dissolve {
            if let Some(guild) = self.guilds.remove(&guild_id) {
                self.tags.remove(&guild.tag);
            }
        }
        Ok(())
    }

    /// 玩家向公會金庫捐贈乙太。
    ///
    /// 回傳 `Ok(新金庫餘額)` 成功；`Err` 說明原因。
    /// 呼叫端負責驗證並扣除玩家身上的乙太。
    pub fn donate(&mut self, player_id: Uuid, amount: u32) -> Result<u32, String> {
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
        Ok(guild.treasury)
    }
}

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
        let mut store = GuildStore::new();
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
        let mut store = GuildStore::new();
        let gid = store.create(uid(), "測試公會".into(), "abc".into()).unwrap();
        assert_eq!(store.get(gid).unwrap().tag, "ABC");
    }

    #[test]
    fn create_duplicate_tag_fails() {
        let mut store = GuildStore::new();
        store.create(uid(), "公會甲".into(), "AAA".into()).unwrap();
        let err = store.create(uid(), "公會乙".into(), "AAA".into()).unwrap_err();
        assert!(err.contains("標籤已被"));
    }

    #[test]
    fn create_duplicate_name_fails() {
        let mut store = GuildStore::new();
        store.create(uid(), "星際探險家".into(), "AAA".into()).unwrap();
        let err = store.create(uid(), "星際探險家".into(), "BBB".into()).unwrap_err();
        assert!(err.contains("名稱已存在"));
    }

    #[test]
    fn join_and_leave() {
        let mut store = GuildStore::new();
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
        let mut store = GuildStore::new();
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
        let mut store = GuildStore::new();
        let founder = uid();
        let member = uid();
        let gid = store.create(founder, "接班公會".into(), "SUC".into()).unwrap();
        store.join(gid, member).unwrap();
        store.leave(founder).unwrap();
        assert_eq!(store.get(gid).unwrap().founder_id, member);
    }

    #[test]
    fn donate_updates_treasury() {
        let mut store = GuildStore::new();
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
        let mut store = GuildStore::new();
        let player = uid();
        let gid1 = store.create(uid(), "公會一".into(), "G01".into()).unwrap();
        let gid2 = store.create(uid(), "公會二".into(), "G02".into()).unwrap();
        store.join(gid1, player).unwrap();
        let err = store.join(gid2, player).unwrap_err();
        assert!(err.contains("已在一個公會"));
    }
}
