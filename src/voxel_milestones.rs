//! 乙太方界·玩家里程碑 v1（成就徽章，ROADMAP 724）。
//!
//! **設計依據**：`docs/PLAN_ETHERVOX.md`「玩家遊玩」節——
//! 「進度/目標：療癒循環（採集→合成→蓋造→與居民同住）給人想一直玩下去的理由」。
//! 這條軸線至今從未被實作：玩家的合成/交易/建造/耕種一次次成功，卻從沒有任何
//! 「回頭看看自己走了多遠」的管道——不像居民有技能簿（719）、交情網（708）能查閱，
//! 玩家自己的成長軌跡完全沒有被看見。
//!
//! **換維度**：671~723 疊的是「居民↔居民」到訪劇本（問候/八卦/互助/拌嘴/傳授/交易），
//! 本切片換到全新角度——**玩家自己的旅程**，把療癒循環裡每個「第一次」變成一枚
//! 可回頭翻閱、達成當下有小小慶祝感的徽章。
//!
//! **里程碑追上新系統 v1（自主提案切片，接續 828）**：724 上線後，箱子儲存(692)、植樹
//! 造林(738)、建築藍圖(826)、漂流瓶(825)、並肩協作(827)、掉落物轉手(828) 六個系統陸續
//! 上線，玩家做這些系統裡的「第一次」卻從未被里程碑牆看見——本刀補上對應六枚徽章，讓
//! 里程碑牆跟上世界已經長出的新內容，不留下「做過了卻沒被看見」的縫。零新架構、只是
//! append 定義 + 在既有成功路徑接上 `try_unlock_milestone`（沿用同一套冪等機制）。
//!
//! 純邏輯層（`MilestoneStore` 同步資料結構，無鎖/IO/async），IO 與觸發點在 `voxel_ws.rs`。
//! 持久化格式：`data/voxel_milestones.jsonl`（每行一筆 `(player, id)`，append-only、
//! `unlock` 本身冪等——已達成的里程碑重複呼叫不會重複寫檔，因為呼叫端只在 `unlock` 回
//! `true`（本次才第一次達成）時才 append）。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// 持久化路徑（`data/` 已 gitignore）。
const MILESTONES_PATH: &str = "data/voxel_milestones.jsonl";

/// 一枚里程碑的靜態定義。
pub struct MilestoneDef {
    /// 穩定 id（wire 契約，前後端與持久化皆用此鍵）。
    pub id: &'static str,
    /// 繁中名稱。
    pub name_zh: &'static str,
    /// 繁中說明（達成條件白話文）。
    pub desc_zh: &'static str,
    /// emoji 圖示。
    pub icon: &'static str,
}

/// 全部里程碑，依「採集→建造→合成→耕種→贈禮→交易→熟識→安眠」療癒循環順序排列。
/// 新增里程碑只准往後 append（順序即是玩家旅程的敘事順序，別中途插隊重排）。
pub const MILESTONES: &[MilestoneDef] = &[
    MilestoneDef { id: "first_mine",  name_zh: "初次採集", desc_zh: "挖出人生第一塊方塊", icon: "⛏️" },
    MilestoneDef { id: "first_place", name_zh: "初次建造", desc_zh: "在世界放下第一塊方塊", icon: "🧱" },
    MilestoneDef { id: "first_craft", name_zh: "初次合成", desc_zh: "合成出第一件成品", icon: "🔨" },
    MilestoneDef { id: "first_farm",  name_zh: "初次耕種", desc_zh: "種下人生第一顆種子", icon: "🌱" },
    MilestoneDef { id: "first_gift",  name_zh: "初次贈禮", desc_zh: "送給居民第一份禮物", icon: "🎁" },
    MilestoneDef { id: "first_trade", name_zh: "初次交易", desc_zh: "與居民完成第一筆以物易物", icon: "⇌" },
    MilestoneDef { id: "first_bond",  name_zh: "初次熟識", desc_zh: "和一位居民混熟了", icon: "💛" },
    MilestoneDef { id: "first_sleep", name_zh: "初次安眠", desc_zh: "在床上一覺睡到天亮", icon: "🛌" },
    MilestoneDef { id: "first_fish",  name_zh: "初次垂釣", desc_zh: "在水邊釣起第一尾魚", icon: "🎣" },
    MilestoneDef { id: "first_taste", name_zh: "初次品嚐", desc_zh: "嚐一口自己親手煮的料理", icon: "🍲" },
    MilestoneDef { id: "first_firework", name_zh: "初次煙火", desc_zh: "朝夜空施放第一束乙太煙火", icon: "🎆" },
    MilestoneDef { id: "first_chest", name_zh: "初次收納", desc_zh: "把多餘的材料收進第一座箱子", icon: "📦" },
    MilestoneDef { id: "first_grove", name_zh: "初次造林", desc_zh: "親手種下第一株樹苗", icon: "🌳" },
    MilestoneDef { id: "first_blueprint", name_zh: "初次藍圖", desc_zh: "把建築藍圖交給居民、指定她蓋什麼", icon: "📐" },
    MilestoneDef { id: "first_bottle", name_zh: "初次寄信", desc_zh: "把寫著心裡話的漂流瓶丟進水裡", icon: "🍾" },
    MilestoneDef { id: "first_coop", name_zh: "初次協作", desc_zh: "和其他真人玩家一起採集、多得了一份默契", icon: "🤝" },
    MilestoneDef { id: "first_dropitem", name_zh: "初次轉手", desc_zh: "把手上一件材料親手交給另一位真人", icon: "🤲" },
    MilestoneDef { id: "first_market", name_zh: "初次擺攤", desc_zh: "在自由市集和另一位真人玩家議定成一筆以物易物", icon: "🏪" },
    // 探索紀事 v1（自主提案切片，接續 828）：838 遺跡／839 溫泉上線時漏補對應里程碑，本刀補齊。
    MilestoneDef { id: "first_ruin", name_zh: "初探遺跡", desc_zh: "走遠找到一處古代遺跡，敲下柱頂裸露的乙太礦", icon: "🏛️" },
    MilestoneDef { id: "first_hotspring", name_zh: "初次泡湯", desc_zh: "巧遇一泓暖泉，泡進去歇了口氣", icon: "♨️" },
    // 餵野兔馴服 v1（自主提案切片）：世界環境軸線（847/848）第一次能被玩家親手觸碰。
    MilestoneDef { id: "first_tame", name_zh: "初次馴服", desc_zh: "用一根胡蘿蔔馴服了一隻野兔", icon: "🐇" },
    // 地標旅人留言 v1（自主提案切片，ROADMAP 862）：地標第一次擁有共同的旅人留言簿。
    MilestoneDef { id: "first_landmark_note", name_zh: "初次留言", desc_zh: "在一處地標留下一句話給後來的旅人", icon: "📜" },
    // 個人路標 v1（自主提案切片，ROADMAP 869）：世界第一次能被玩家自己標記、導航回去。
    MilestoneDef { id: "first_waypoint", name_zh: "初次插旗", desc_zh: "在世界插下第一支屬於自己的路標", icon: "🚩" },
    // 放養雞 v1（自主提案切片，ROADMAP 870）：wildlife 系統第二種可馴服動物。
    MilestoneDef { id: "first_chicken_tame", name_zh: "初次養雞", desc_zh: "用一把種子馴服了一隻雞", icon: "🐔" },
    // 深層寶藏 v1（自主提案切片）：天然礦脈裡的秘密驚喜，挖礦第一次有機會巧遇乙太幣。
    MilestoneDef { id: "first_treasure", name_zh: "初次尋寶", desc_zh: "在天然礦脈裡意外挖到一座藏寶", icon: "💎" },
    // 邊陲營地探索 v1（自主提案切片，接續 881 立牌）：荒野裡居民親手搭起的據點，玩家視角
    // 第一次也被世界記住——地標系統從「居民立牌」延伸出「玩家探索紀事」的另一半。
    MilestoneDef { id: "first_outpost_discover", name_zh: "覓得營地", desc_zh: "循著居民的足跡走進荒野，找到了她親手搭起的邊陲營地", icon: "⛺" },
    // 為馴服的動物取名 v1（自主提案切片，ROADMAP 895）：馴服的羈絆第一次有了署名。
    MilestoneDef { id: "first_pet_name", name_zh: "初次命名", desc_zh: "替一隻已馴服、跟著你走的小夥伴取了名字", icon: "🐾" },
    // 寵愛你的夥伴 v1（自主提案切片，ROADMAP 899）：馴養羈絆線第一次有了「疼牠」的日常暖收尾。
    MilestoneDef { id: "first_treat", name_zh: "初次寵愛", desc_zh: "遞一份零食給已馴服的小夥伴，換來牠一次撒嬌", icon: "💕" },
    // 世界奇觀·乙太世界樹 v1（ROADMAP 940）：跋涉到世界邊陲，撞見全世界唯一一座天然大奇觀。
    MilestoneDef { id: "first_wonder", name_zh: "初見奇觀", desc_zh: "跋涉到世界盡頭，撞見獨一無二的乙太世界樹，仰望那團泛著幽光的巨大花冠", icon: "🌳" },
    // 眾力共築·乙太燈塔 v1（自主提案切片）：玩家之間第一件共同蓋起來的公共工程。
    MilestoneDef { id: "first_lighthouse_gift", name_zh: "初獻磚石", desc_zh: "為全世界共築的乙太燈塔獻上第一份材料", icon: "🗼" },
    // 地底遺跡神殿 v1（自主提案切片，ROADMAP 975）：世界第一座人工鑿建的地底密室，
    // 得先挖穿深層石牆才看得見裡頭，垂直/室內探索第一次被打開。
    MilestoneDef { id: "first_dungeon", name_zh: "初闖遺跡", desc_zh: "鑿穿深層岩壁，找到了一座人工鑿建的地底遺跡神殿", icon: "🏺" },
    // 玩家羈絆帳本 v1（自主提案切片，ROADMAP 985）：玩家↔玩家的真實互動第一次被世界記住。
    MilestoneDef { id: "first_player_bond", name_zh: "初結旅伴", desc_zh: "和另一位真人玩家處出了交情——旅伴", icon: "🚶" },
    // 圓夢地標 v1.1（自主提案切片，接續 `voxel_lifeproject` v1）：居民親手圓滿的個人大夢，
    // 第一次也被世界記住——地標系統從「天然生成／居民立牌」延伸出「她默默做完的事」。
    MilestoneDef { id: "first_dream_landmark", name_zh: "初見圓夢", desc_zh: "路過，撞見了一位居民親手圓滿的夢想角落", icon: "🌟" },
];

/// 查表確認是否為已知里程碑 id（守 store 資料乾淨，未知 id 不寫入）。
pub fn is_known(id: &str) -> bool {
    MILESTONES.iter().any(|m| m.id == id)
}

// ── 持久化格式 ────────────────────────────────────────────────────────────────

/// 一筆持久化記錄：某玩家達成某項里程碑。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MilestoneEntry {
    pub player: String,
    pub id: String,
}

// ── 里程碑 Store ─────────────────────────────────────────────────────────────

/// 玩家里程碑帳本（純同步資料結構，由呼叫端包進 `RwLock`）。
#[derive(Default, Debug)]
pub struct MilestoneStore {
    /// key = 玩家名 → 已達成的里程碑 id 集合。
    earned: HashMap<String, HashSet<String>>,
}

impl MilestoneStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 從持久化記錄還原（啟動時呼叫一次）。未知 id（例如舊版留下的壞資料）安全略過。
    pub fn from_entries(entries: impl IntoIterator<Item = MilestoneEntry>) -> Self {
        let mut s = Self::new();
        for e in entries {
            if is_known(&e.id) {
                s.earned.entry(e.player).or_default().insert(e.id);
            }
        }
        s
    }

    /// 標記玩家達成一項里程碑。回傳 `true` 代表「這次才第一次達成」——
    /// 呼叫端只在回 `true` 時才 append 持久化 + 廣播慶祝；已達成過再呼叫安全回 `false`，冪等。
    /// 未知 id 一律不寫入、回 `false`（防呆，不污染 store）。
    pub fn unlock(&mut self, player: &str, id: &str) -> bool {
        if !is_known(id) {
            return false;
        }
        self.earned.entry(player.to_string()).or_default().insert(id.to_string())
    }

    /// 玩家是否已達成指定里程碑。
    pub fn has(&self, player: &str, id: &str) -> bool {
        self.earned.get(player).is_some_and(|s| s.contains(id))
    }

    /// 玩家已達成的里程碑 id 清單（不保證順序，呼叫端可依 `MILESTONES` 順序重排顯示）。
    pub fn earned_ids(&self, player: &str) -> Vec<String> {
        self.earned.get(player).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }
}

// ── jsonl 持久化（append-only，比照 voxel_invent::append_invented_skill 慣例）──────

/// Append 一筆里程碑記錄。**鐵律**：只在不持任何鎖時呼叫（同步小檔寫，不 await）。
pub fn append_milestone(entry: &MilestoneEntry) {
    if let Ok(line) = serde_json::to_string(entry) {
        write_line(MILESTONES_PATH, &line);
    }
}

/// 載回所有里程碑記錄（啟動時呼叫一次）。檔不存在／壞行皆容忍。
pub fn load_milestones() -> Vec<MilestoneEntry> {
    let content = match std::fs::read_to_string(MILESTONES_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                None
            } else {
                serde_json::from_str::<MilestoneEntry>(l).ok()
            }
        })
        .collect()
}

fn write_line(path: &str, line: &str) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("無法寫入里程碑記錄 {path}: {e}"),
    }
}

// ── 測試 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlock_first_time_true_second_time_false() {
        let mut s = MilestoneStore::new();
        assert!(s.unlock("nova", "first_mine"), "第一次達成應回 true");
        assert!(!s.unlock("nova", "first_mine"), "重複達成應冪等回 false");
    }

    #[test]
    fn unknown_id_never_unlocks() {
        let mut s = MilestoneStore::new();
        assert!(!s.unlock("nova", "not_a_real_id"));
        assert!(!s.has("nova", "not_a_real_id"));
        assert!(s.earned_ids("nova").is_empty());
    }

    #[test]
    fn players_independent() {
        let mut s = MilestoneStore::new();
        s.unlock("alice", "first_mine");
        assert!(s.has("alice", "first_mine"));
        assert!(!s.has("bob", "first_mine"));
    }

    #[test]
    fn has_false_before_unlock() {
        let s = MilestoneStore::new();
        assert!(!s.has("nova", "first_craft"));
    }

    #[test]
    fn earned_ids_reflects_unlocks() {
        let mut s = MilestoneStore::new();
        s.unlock("nova", "first_mine");
        s.unlock("nova", "first_craft");
        let mut ids = s.earned_ids("nova");
        ids.sort();
        assert_eq!(ids, vec!["first_craft".to_string(), "first_mine".to_string()]);
    }

    #[test]
    fn from_entries_rebuilds_state() {
        let entries = vec![
            MilestoneEntry { player: "nova".into(), id: "first_mine".into() },
            MilestoneEntry { player: "nova".into(), id: "first_place".into() },
            MilestoneEntry { player: "luna".into(), id: "first_farm".into() },
        ];
        let s = MilestoneStore::from_entries(entries);
        assert!(s.has("nova", "first_mine"));
        assert!(s.has("nova", "first_place"));
        assert!(s.has("luna", "first_farm"));
        assert!(!s.has("luna", "first_mine"));
    }

    #[test]
    fn from_entries_skips_unknown_ids() {
        let entries = vec![MilestoneEntry { player: "nova".into(), id: "bogus".into() }];
        let s = MilestoneStore::from_entries(entries);
        assert!(s.earned_ids("nova").is_empty());
    }

    #[test]
    fn is_known_matches_static_list() {
        assert!(is_known("first_mine"));
        assert!(is_known("first_sleep"));
        assert!(is_known("first_treasure"));
        assert!(!is_known(""));
        assert!(!is_known("first_win_lottery"));
    }

    #[test]
    fn catchup_milestones_known() {
        for id in [
            "first_chest", "first_grove", "first_blueprint",
            "first_bottle", "first_coop", "first_dropitem",
        ] {
            assert!(is_known(id), "追上新系統的里程碑 {id} 應已登記");
        }
    }

    #[test]
    fn all_milestone_ids_unique() {
        let mut ids: Vec<&str> = MILESTONES.iter().map(|m| m.id).collect();
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), before, "里程碑 id 不應重複");
    }

    #[test]
    fn all_milestones_have_nonempty_fields() {
        for m in MILESTONES {
            assert!(!m.id.is_empty());
            assert!(!m.name_zh.is_empty());
            assert!(!m.desc_zh.is_empty());
            assert!(!m.icon.is_empty());
        }
    }

    #[test]
    fn empty_store_earned_ids_empty() {
        let s = MilestoneStore::new();
        assert!(s.earned_ids("nobody").is_empty());
    }
}
