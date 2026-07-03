//! 乙太方界·居民收成你送的種子長成的菜園、當面回贈第一把收穫給你 v1（ROADMAP 755）。
//!
//! **核心信念**：「你的互動有後果」＋ PLAN_ETHERVOX 反覆點名的「交織點」——人類種田的
//! 樂趣與 AI 居民的生活在同一片方塊天地交織。754 讓「已和你要好的居民，會把你送的種子
//! 真的種進家旁的土裡」——你的餽贈第一次在世界裡生根長大；但那畦菜園長成後就沒了下文，
//! 收成只能你自己回頭去收。本切片把這條線走完最後一步、把它接成一個**完整的閉環**：
//!
//! **當那畦因你而生的菜園熟了，種下它的居民會親手收成、把第一把收穫當面回贈給你**——
//! 你送出一把種子 → 她種下 → 世界長出作物 → 她收成 → 又回到你手裡。餽贈第一次在世界裡
//! 走了一整圈才回來，而且回來時已經不是你送出去的那把種子，而是它結出的果實。居民不再
//! 只是「收下你心意」的一方，她成了會用你的餽贈生產、再懂得回饋的鄰居。
//!
//! **與既有系統的分界**：這不是 667「居民回禮」（憑空／從採集背包挑一份小禮，一生一次）；
//! 本切片的禮物**確定性地就是那畦田結的果**（你送小麥種子→回小麥，胡蘿蔔→胡蘿蔔），且
//! 可以隨你一次次送種子而一次次上演（每畦田一次）。也不是 753「照料菜園」（幫你**顧**你自己
//! 種的、還沒熟的作物）——這是她收成**她自己**種的、因你而生的那畦，果實回到你手裡。
//!
//! **持久化**：座標鍵 → 一畦禮物菜園（居民 id、送種子的玩家名、作物種類），比照告示牌
//! （740）／箱子（692）的「每座標側資料 + append-only JSONL」範式；收成回贈後標記移除。
//! 重啟後 replay 取每座標「最新一筆」重建現況（`removed` ＝已收成回贈、replay 時剔除）。
//!
//! 純邏輯層：零 async、零鎖、零 IO 外包、零 LLM；鎖 / IO / 廣播 / 方塊查詢全在 `voxel_ws.rs`。

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

use crate::voxel_farm::{CropKind, CARROT_ID, POTATO_ID, WHEAT_ID};

/// 持久化路徑。
pub const GIFT_GARDEN_PATH: &str = "data/voxel_gift_gardens.jsonl";

/// 每 tick（10Hz）觸發「檢查是否有熟的禮物菜園可收成回贈」的機率（低頻節流）。
/// 實際還要層層過閘（送種子的那位玩家就在旁邊、真有一畦掛她名下的田、且已成熟），
/// 故有感頻率遠低於此；這道機率只是避免玩家在旁久站時每 tick 都白做一次 delta 讀。
pub const HARVEST_CHANCE_PER_TICK: f32 = 0.05;

/// 作物種類代碼（序列化用的穩定 u8；與 `CropKind` 一對一，但不依賴其 repr）。
pub const CROP_WHEAT: u8 = 0;
pub const CROP_CARROT: u8 = 1;
pub const CROP_POTATO: u8 = 2;

/// `CropKind` → 穩定代碼（存進 JSONL）。
pub fn crop_code(kind: CropKind) -> u8 {
    match kind {
        CropKind::Wheat => CROP_WHEAT,
        CropKind::Carrot => CROP_CARROT,
        CropKind::Potato => CROP_POTATO,
    }
}

/// 作物中文名（供台詞／記憶／Feed）。未知代碼回退「作物」（防呆，不 panic）。
pub fn crop_name(code: u8) -> &'static str {
    match code {
        CROP_WHEAT => "小麥",
        CROP_CARROT => "胡蘿蔔",
        CROP_POTATO => "馬鈴薯",
        _ => "作物",
    }
}

/// 這畦田收成、回贈給玩家的果實（物品 id, 數量）——確定性地就是那畦田結的果。
/// 數量沿用玩家親手收成的產量精神（馬鈴薯量大是特色，見 `voxel_farm` 收穫規則）。
pub fn produce_gift(code: u8) -> (u8, u32) {
    match code {
        CROP_WHEAT => (WHEAT_ID, 1),
        CROP_CARROT => (CARROT_ID, 1),
        CROP_POTATO => (POTATO_ID, 2),
        _ => (WHEAT_ID, 1),
    }
}

/// 世界座標鍵（"wx,wy,wz"，與告示牌／箱子同格式）。
pub fn pos_key(wx: i32, wy: i32, wz: i32) -> String {
    format!("{wx},{wy},{wz}")
}

/// 反解座標鍵。格式不符回 None。確定性、可測。
pub fn parse_key(k: &str) -> Option<(i32, i32, i32)> {
    let mut it = k.split(',');
    let wx = it.next()?.parse::<i32>().ok()?;
    let wy = it.next()?.parse::<i32>().ok()?;
    let wz = it.next()?.parse::<i32>().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((wx, wy, wz))
}

/// 居民收成時頭頂的暖句（四句輪替）。`crop`＝作物名、`player`＝送種子的玩家名（空則用「你」）。
pub fn harvest_say_line(player: &str, crop: &str, pick: usize) -> String {
    let who = if player.is_empty() { "你" } else { player };
    let lines = [
        format!("{who}送的{crop}種子，我這畦田熟啦！第一把收成該還給你～"),
        format!("你看這{crop}長得多好！這是你的種子結的果，收下嘛！"),
        format!("多虧{who}的{crop}種子，我也有收成了——這頭一把留給你！"),
        format!("{crop}種子是{who}給的，果實也該有你一份，來、拿去～"),
    ];
    lines[pick % lines.len()].clone()
}

/// 居民記憶摘要（掛玩家名下，`🌾` 前綴供日記／回想歸類）。
pub fn harvest_memory_line(player: &str, crop: &str) -> String {
    let who = if player.is_empty() { "你" } else { player };
    format!("🌾收成了{who}送我種子長成的那畦{crop}，把第一把收穫回贈給{who}")
}

/// 動態牆一行。
pub fn harvest_feed_line(rname: &str, player: &str, crop: &str) -> String {
    let who = if player.is_empty() { "有人" } else { player };
    format!("{rname}收成了{who}送的種子長成的{crop}，把第一把收穫回贈給{who}～")
}

/// 一畦禮物菜園的側資料。
#[derive(Debug, Clone, PartialEq)]
pub struct GiftGarden {
    /// 種下這畦田的居民 id。
    pub resident_id: String,
    /// 送出這把種子的玩家名（收成回贈的對象）。
    pub player: String,
    /// 作物種類代碼（`CROP_*`）。
    pub crop: u8,
}

/// 一筆禮物菜園事件（append-only JSONL 最小單元）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GiftGardenEntry {
    /// 作物方塊世界座標鍵。
    pub pos: String,
    /// 種下的居民 id。
    pub resident_id: String,
    /// 送種子的玩家名。
    pub player: String,
    /// 作物種類代碼。
    pub crop: u8,
    /// true ＝已收成回贈／失效，replay 時剔除該座標。
    pub removed: bool,
    /// 單調遞增序號（replay 取每座標最大 seq 者為現況）。
    pub seq: u64,
}

/// 全局禮物菜園 store：pos_key → 側資料（只存未收成的）。
#[derive(Default)]
pub struct GiftGardenStore {
    plots: HashMap<String, GiftGarden>,
    next_seq: u64,
}

impl GiftGardenStore {
    /// 空 store（測試 / 首次啟動）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建（每座標取最新 seq；`removed` 最新者剔除）。
    pub fn from_entries(entries: Vec<GiftGardenEntry>) -> Self {
        let mut latest: HashMap<String, &GiftGardenEntry> = HashMap::new();
        let mut max_seq = 0u64;
        for e in &entries {
            max_seq = max_seq.max(e.seq);
            match latest.get(&e.pos) {
                Some(prev) if prev.seq >= e.seq => {}
                _ => {
                    latest.insert(e.pos.clone(), e);
                }
            }
        }
        let mut plots = HashMap::new();
        for (pos, e) in latest {
            if !e.removed {
                plots.insert(
                    pos,
                    GiftGarden {
                        resident_id: e.resident_id.clone(),
                        player: e.player.clone(),
                        crop: e.crop,
                    },
                );
            }
        }
        Self { plots, next_seq: max_seq.saturating_add(1) }
    }

    /// 目前是否一畦禮物菜園都沒有（供 tick 前 O(1) 早退整段功能，no-op 世界不白鎖）。
    pub fn is_empty(&self) -> bool {
        self.plots.is_empty()
    }

    /// 登記一畦新的禮物菜園（居民種下你送的種子時）。回傳持久化事件供 append。
    pub fn record(&mut self, pos: &str, resident_id: &str, player: &str, crop: u8) -> GiftGardenEntry {
        self.plots.insert(
            pos.to_string(),
            GiftGarden {
                resident_id: resident_id.to_string(),
                player: player.to_string(),
                crop,
            },
        );
        let seq = self.next_seq;
        self.next_seq += 1;
        GiftGardenEntry {
            pos: pos.to_string(),
            resident_id: resident_id.to_string(),
            player: player.to_string(),
            crop,
            removed: false,
            seq,
        }
    }

    /// 移除一畦（已收成回贈，或作物被別的方式收走／破壞而失效）。
    /// 有這畦才回傳移除事件（供 append）；沒有回 None（不產生多餘事件）。
    pub fn remove(&mut self, pos: &str) -> Option<GiftGardenEntry> {
        let g = self.plots.remove(pos)?;
        let seq = self.next_seq;
        self.next_seq += 1;
        Some(GiftGardenEntry {
            pos: pos.to_string(),
            resident_id: g.resident_id,
            player: g.player,
            crop: g.crop,
            removed: true,
            seq,
        })
    }

    /// 這位居民名下、且送種子的玩家是 `player` 的所有禮物菜園座標與作物代碼。
    /// 供 tick 篩「該回贈給眼前這位玩家的田」。已按座標鍵排序求確定性。
    pub fn plots_for(&self, resident_id: &str, player: &str) -> Vec<(String, u8)> {
        let mut v: Vec<(String, u8)> = self
            .plots
            .iter()
            .filter(|(_, g)| g.resident_id == resident_id && g.player == player)
            .map(|(k, g)| (k.clone(), g.crop))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }
}

// ── 持久化 IO（在 voxel_ws.rs 的鎖外呼叫）────────────────────────────────────────────

/// 從磁碟載入所有事件（啟動時呼叫一次）。
pub fn load_gift_gardens() -> Vec<GiftGardenEntry> {
    let Ok(f) = fs::File::open(GIFT_GARDEN_PATH) else {
        return vec![];
    };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<GiftGardenEntry>(&l).ok())
        .collect()
}

/// Append 單筆事件。
pub fn append_gift_garden(entry: &GiftGardenEntry) {
    let Ok(line) = serde_json::to_string(entry) else {
        return;
    };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(GIFT_GARDEN_PATH) else {
        return;
    };
    let _ = writeln!(f, "{line}");
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    // ── 作物映射 ──────────────────────────────────────────────────────────
    #[test]
    fn crop_code_maps_all_kinds() {
        assert_eq!(crop_code(CropKind::Wheat), CROP_WHEAT);
        assert_eq!(crop_code(CropKind::Carrot), CROP_CARROT);
        assert_eq!(crop_code(CropKind::Potato), CROP_POTATO);
    }

    #[test]
    fn crop_name_covers_and_defaults() {
        assert_eq!(crop_name(CROP_WHEAT), "小麥");
        assert_eq!(crop_name(CROP_CARROT), "胡蘿蔔");
        assert_eq!(crop_name(CROP_POTATO), "馬鈴薯");
        assert_eq!(crop_name(99), "作物"); // 未知代碼防呆
    }

    #[test]
    fn produce_gift_matches_crop() {
        assert_eq!(produce_gift(CROP_WHEAT), (WHEAT_ID, 1));
        assert_eq!(produce_gift(CROP_CARROT), (CARROT_ID, 1));
        assert_eq!(produce_gift(CROP_POTATO), (POTATO_ID, 2)); // 馬鈴薯量大
    }

    // ── 座標鍵 ────────────────────────────────────────────────────────────
    #[test]
    fn pos_key_roundtrip() {
        assert_eq!(pos_key(1, -2, 300), "1,-2,300");
        assert_eq!(parse_key("1,-2,300"), Some((1, -2, 300)));
        assert_eq!(parse_key("1,2"), None);
        assert_eq!(parse_key("1,2,3,4"), None);
        assert_eq!(parse_key("a,b,c"), None);
    }

    // ── store ─────────────────────────────────────────────────────────────
    #[test]
    fn record_and_plots_for() {
        let mut s = GiftGardenStore::new();
        assert!(s.is_empty());
        s.record("1,4,1", "res-luna", "旅人", CROP_WHEAT);
        assert!(!s.is_empty());
        let plots = s.plots_for("res-luna", "旅人");
        assert_eq!(plots, vec![("1,4,1".to_string(), CROP_WHEAT)]);
        // 別的居民 / 別的玩家撈不到。
        assert!(s.plots_for("res-nova", "旅人").is_empty());
        assert!(s.plots_for("res-luna", "路人").is_empty());
    }

    #[test]
    fn plots_for_sorted_and_multi() {
        let mut s = GiftGardenStore::new();
        s.record("5,4,5", "r", "p", CROP_CARROT);
        s.record("1,4,1", "r", "p", CROP_WHEAT);
        s.record("3,4,3", "r", "other", CROP_POTATO); // 別的玩家
        let plots = s.plots_for("r", "p");
        // 只回這位玩家的兩畦、且按座標鍵排序。
        assert_eq!(
            plots,
            vec![("1,4,1".to_string(), CROP_WHEAT), ("5,4,5".to_string(), CROP_CARROT)]
        );
    }

    #[test]
    fn remove_returns_event_and_clears() {
        let mut s = GiftGardenStore::new();
        s.record("2,4,2", "r", "p", CROP_POTATO);
        let ev = s.remove("2,4,2").expect("有這畦應回移除事件");
        assert!(ev.removed);
        assert_eq!(ev.crop, CROP_POTATO);
        assert!(s.plots_for("r", "p").is_empty());
        // 再移除同座標回 None（不產生多餘事件）。
        assert!(s.remove("2,4,2").is_none());
    }

    #[test]
    fn record_returns_persist_event() {
        let mut s = GiftGardenStore::new();
        let ev = s.record("0,4,0", "res-x", "阿明", CROP_CARROT);
        assert!(!ev.removed);
        assert_eq!(ev.pos, "0,4,0");
        assert_eq!(ev.resident_id, "res-x");
        assert_eq!(ev.player, "阿明");
        assert_eq!(ev.crop, CROP_CARROT);
    }

    #[test]
    fn from_entries_takes_latest_seq() {
        let entries = vec![
            GiftGardenEntry {
                pos: "0,4,0".into(),
                resident_id: "r".into(),
                player: "p".into(),
                crop: CROP_WHEAT,
                removed: false,
                seq: 0,
            },
            // 收成回贈：移除事件（seq 較大）。
            GiftGardenEntry {
                pos: "0,4,0".into(),
                resident_id: "r".into(),
                player: "p".into(),
                crop: CROP_WHEAT,
                removed: true,
                seq: 2,
            },
            // 亂序的舊事件，不該蓋掉最新。
            GiftGardenEntry {
                pos: "0,4,0".into(),
                resident_id: "r".into(),
                player: "p".into(),
                crop: CROP_WHEAT,
                removed: false,
                seq: 1,
            },
        ];
        let s = GiftGardenStore::from_entries(entries);
        assert!(s.plots_for("r", "p").is_empty(), "最新是 removed＝已收成回贈");
        assert_eq!(s.next_seq, 3);
    }

    #[test]
    fn from_entries_keeps_active_plot() {
        let entries = vec![GiftGardenEntry {
            pos: "7,4,7".into(),
            resident_id: "res-luna".into(),
            player: "旅人".into(),
            crop: CROP_POTATO,
            removed: false,
            seq: 5,
        }];
        let s = GiftGardenStore::from_entries(entries);
        assert_eq!(s.plots_for("res-luna", "旅人"), vec![("7,4,7".to_string(), CROP_POTATO)]);
        assert_eq!(s.next_seq, 6);
    }

    // ── 文案 ──────────────────────────────────────────────────────────────
    #[test]
    fn say_line_rotates_and_names() {
        let a = harvest_say_line("露娜", "小麥", 0);
        let b = harvest_say_line("露娜", "小麥", 1);
        assert_ne!(a, b, "不同 pick 給不同台詞");
        for p in 0..8 {
            let s = harvest_say_line("露娜", "馬鈴薯", p);
            assert!(s.contains("馬鈴薯"), "台詞要提到作物");
        }
        assert!(harvest_say_line("諾娃", "胡蘿蔔", 0).contains("諾娃"));
    }

    #[test]
    fn say_line_empty_player_uses_you() {
        assert!(harvest_say_line("", "小麥", 0).contains("你"));
    }

    #[test]
    fn memory_line_has_prefix_and_names() {
        let m = harvest_memory_line("露娜", "小麥");
        assert!(m.starts_with("🌾"), "記憶要帶前綴供日記歸類");
        assert!(m.contains("露娜") && m.contains("小麥"));
    }

    #[test]
    fn feed_line_names_both_and_crop() {
        let f = harvest_feed_line("諾娃", "旅人", "胡蘿蔔");
        assert!(f.contains("諾娃") && f.contains("旅人") && f.contains("胡蘿蔔"));
    }
}
