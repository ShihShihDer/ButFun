//! 臨時隊伍系統（ROADMAP 97）——純記憶體、臨時、零 migration。
//!
//! 設計：
//! - 一名玩家同一時間只能在一個隊伍。
//! - 任何玩家均可建隊（即成為隊長）；隊長邀請他人加入。
//! - 隊長離開 → 隊伍自動解散；一般成員離開 → 繼續存在（人數仍 ≥ 1）。
//! - 只剩 1 人也自動解散（不需要「1 人隊」）。
//! - 邀請為「待定」狀態：受邀者主動送 `JoinParty` 後才正式加入，可拒絕不理。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// 一支隊伍的狀態。
#[derive(Clone)]
pub struct Party {
    pub id: Uuid,
    pub leader: Uuid,
    /// 包含隊長在內的所有成員。
    pub members: Vec<Uuid>,
}

#[derive(Default)]
struct PartyInner {
    /// party_id → Party
    parties: HashMap<Uuid, Party>,
    /// player_id → party_id（已在隊）
    membership: HashMap<Uuid, Uuid>,
    /// player_id → party_id（待接受邀請，可覆蓋）
    invites: HashMap<Uuid, Uuid>,
}

/// 臨時隊伍管理器（Clone-safe via Arc<Mutex<...>>）。
#[derive(Clone, Default)]
pub struct PartyStore {
    inner: Arc<Mutex<PartyInner>>,
}

impl PartyStore {
    /// 建立新隊伍（隊長自動加入），回傳 party_id。
    /// 若隊長已在某隊，先移出再建新隊。
    pub fn create(&self, leader: Uuid) -> Uuid {
        let mut g = self.inner.lock().unwrap();
        // 先退出舊隊
        if let Some(old) = g.membership.remove(&leader) {
            if let Some(p) = g.parties.get_mut(&old) {
                p.members.retain(|&m| m != leader);
                // 若舊隊只剩其他人，留著（邊界：此處隊長是第一個離開的）
                if p.members.is_empty() {
                    g.parties.remove(&old);
                }
            }
        }
        let pid = Uuid::new_v4();
        g.parties.insert(pid, Party { id: pid, leader, members: vec![leader] });
        g.membership.insert(leader, pid);
        pid
    }

    /// 邀請目標加入指定隊伍。
    /// 回傳 `None` 若目標已在某隊（包括該隊）。
    pub fn invite(&self, party_id: Uuid, target: Uuid) -> Option<Uuid> {
        let mut g = self.inner.lock().unwrap();
        if g.membership.contains_key(&target) {
            return None; // 目標已在某隊
        }
        if !g.parties.contains_key(&party_id) {
            return None; // 隊伍不存在
        }
        g.invites.insert(target, party_id);
        Some(party_id)
    }

    /// 受邀者接受待定邀請。
    /// 成功回傳 `(party_id, leader_id, all_members)`；無邀請或邀請的隊伍已解散 → `None`。
    pub fn accept_invite(&self, player: Uuid) -> Option<(Uuid, Uuid, Vec<Uuid>)> {
        let mut g = self.inner.lock().unwrap();
        let pid = g.invites.remove(&player)?;
        if !g.parties.contains_key(&pid) {
            return None; // 隊伍已解散
        }
        g.membership.insert(player, pid);
        let party = g.parties.get_mut(&pid).unwrap();
        party.members.push(player);
        Some((pid, party.leader, party.members.clone()))
    }

    /// 玩家離開隊伍。
    /// 回傳 `Some((disbanded, remaining_members))`：
    /// - `disbanded=true`：隊長離開或隊伍人數不足 → 已解散，remaining 是被通知的其他成員。
    /// - `disbanded=false`：普通成員離開，remaining 繼續留在隊中。
    /// 玩家不在任何隊伍時回 `None`。
    pub fn leave(&self, player: Uuid) -> Option<(bool, Vec<Uuid>)> {
        let mut g = self.inner.lock().unwrap();
        let pid = g.membership.remove(&player)?;
        let party = g.parties.get_mut(&pid)?;
        party.members.retain(|&m| m != player);
        let remaining = party.members.clone();

        let is_leader = party.leader == player;
        let too_small = remaining.len() < 2; // 只剩 1 人的隊不值得留

        if is_leader || too_small {
            // 解散：清除所有成員的 membership + 待定邀請
            for &m in &remaining {
                g.membership.remove(&m);
            }
            g.parties.remove(&pid);
            g.invites.retain(|_, &mut p| p != pid);
            Some((true, remaining))
        } else {
            Some((false, remaining))
        }
    }

    /// 查某玩家所在隊伍 ID（None = 不在隊）。
    pub fn party_of(&self, player: Uuid) -> Option<Uuid> {
        self.inner.lock().unwrap().membership.get(&player).copied()
    }

    /// 取得隊伍所有成員。
    pub fn members(&self, party_id: Uuid) -> Vec<Uuid> {
        self.inner
            .lock()
            .unwrap()
            .parties
            .get(&party_id)
            .map(|p| p.members.clone())
            .unwrap_or_default()
    }

    /// 玩家是否為指定隊伍的隊長。
    pub fn is_leader(&self, player: Uuid, party_id: Uuid) -> bool {
        self.inner
            .lock()
            .unwrap()
            .parties
            .get(&party_id)
            .map(|p| p.leader == player)
            .unwrap_or(false)
    }

    /// 取得玩家的待定邀請 party_id（若有）。
    pub fn pending_invite(&self, player: Uuid) -> Option<Uuid> {
        self.inner.lock().unwrap().invites.get(&player).copied()
    }

    /// 撤銷某玩家的待定邀請（拒絕邀請時用）。
    pub fn decline_invite(&self, player: Uuid) {
        self.inner.lock().unwrap().invites.remove(&player);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_invite_join() {
        let store = PartyStore::default();
        let leader = Uuid::new_v4();
        let alice = Uuid::new_v4();

        let pid = store.create(leader);
        assert_eq!(store.party_of(leader), Some(pid));

        assert!(store.invite(pid, alice).is_some());
        assert_eq!(store.pending_invite(alice), Some(pid));

        let (pid2, ldr, members) = store.accept_invite(alice).unwrap();
        assert_eq!(pid2, pid);
        assert_eq!(ldr, leader);
        assert!(members.contains(&alice));
        assert!(members.contains(&leader));
        assert_eq!(store.party_of(alice), Some(pid));
        assert!(store.pending_invite(alice).is_none());
    }

    #[test]
    fn leader_leaves_disbands_all() {
        let store = PartyStore::default();
        let leader = Uuid::new_v4();
        let alice = Uuid::new_v4();

        let pid = store.create(leader);
        store.invite(pid, alice);
        store.accept_invite(alice);

        let (disbanded, remaining) = store.leave(leader).unwrap();
        assert!(disbanded);
        assert!(remaining.contains(&alice));
        // alice 應被清出
        assert_eq!(store.party_of(alice), None);
    }

    #[test]
    fn member_leaves_no_disband() {
        let store = PartyStore::default();
        let leader = Uuid::new_v4();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        let pid = store.create(leader);
        store.invite(pid, alice);
        store.accept_invite(alice);
        store.invite(pid, bob);
        store.accept_invite(bob);

        let (disbanded, remaining) = store.leave(alice).unwrap();
        assert!(!disbanded);
        assert!(remaining.contains(&leader));
        assert!(remaining.contains(&bob));
        assert_eq!(store.party_of(leader), Some(pid));
        assert_eq!(store.party_of(alice), None);
    }

    #[test]
    fn two_members_one_leaves_disbands() {
        let store = PartyStore::default();
        let leader = Uuid::new_v4();
        let alice = Uuid::new_v4();

        let pid = store.create(leader);
        store.invite(pid, alice);
        store.accept_invite(alice);

        // alice 離開 → 只剩隊長 1 人 → 解散
        let (disbanded, remaining) = store.leave(alice).unwrap();
        assert!(disbanded);
        assert!(remaining.contains(&leader));
        assert_eq!(store.party_of(leader), None);
    }

    #[test]
    fn invite_fails_if_already_in_party() {
        let store = PartyStore::default();
        let leader = Uuid::new_v4();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        let pid = store.create(leader);
        store.invite(pid, alice);
        store.accept_invite(alice);

        let pid2 = store.create(bob);
        // alice 已在隊，邀請失敗
        assert!(store.invite(pid2, alice).is_none());
    }

    #[test]
    fn decline_invite_removes_pending() {
        let store = PartyStore::default();
        let leader = Uuid::new_v4();
        let alice = Uuid::new_v4();

        let pid = store.create(leader);
        store.invite(pid, alice);
        store.decline_invite(alice);
        assert!(store.pending_invite(alice).is_none());
    }
}
