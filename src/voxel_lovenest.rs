//! 乙太方界·戀人愛巢 v1（自主提案切片，承 PLAN_ETHERVOX §4「居民↔居民關係」＋§6「渴望蓋成真」）。
//!
//! **真缺口**：居民戀愛心動 v1（ROADMAP 846）讓一對老朋友並坐閒聊時偶爾擦出火花、締結成戀人，
//! 戀人牽掛（852）讓分開的戀人放下手邊的事走去相見——但這段全庫最深的羈絆至今**只活在資料與
//! 面板裡**：交情網面板上多一顆 ❤️、動態牆播一句話、記憶裡添一筆。放眼整片方塊天地，戀人在
//! 一起這件事**沒有留下任何實體痕跡**。村莊里程碑會立起村碑（885）、殖民奠基會落下村落殘核
//! （884）、居民自蓋的家會立牌署名（749）——唯獨「兩個人相愛、決定住在一起」這件人性裡最鮮明
//! 的事，世界看不出一絲一毫。
//!
//! **本刀**：一對戀人在一起一陣子後，會在村邊**合力蓋起一間亮著燈的小屋**——木板牆、一扇木門、
//! 屋頂中央一盞乙太燈（夜裡發著光的家），門前立一塊牌子寫著「露娜與奧瑞的愛巢」，並登記進世界
//! 的地標系統（比照 749/860 讓居民自蓋作品算進地標）。玩家探索走到村邊，第一次能親眼看見、親手
//! 讀到：這裡是誰和誰共同的家。戀愛第一次從面板上的一顆愛心，長成方塊天地裡站得住、走得到的實體。
//!
//! **與既有元素 razor-sharp 區隔（非同軸重複）**：
//! - **居民自蓋的家（749 nameplate）**＝**單獨一位**居民實現「想要一個家」的渴望、署自己的名；
//!   本刀＝**一對戀人合力**築的**共同**的家、署兩人的名，觸發源是「戀人關係」而非個人渴望。
//! - **村碑（885）／殖民殘核（884）**＝**集體**成就（全村里程碑／拓荒隊）的實體；本刀＝**兩人之間**
//!   私密羈絆的實體，尺度與觸發源皆不同。
//!
//! **純邏輯層鐵律**：本檔零 LLM、零鎖、零 async、零世界 IO——資格判定、選址、小屋方塊佈局、命名、
//! 文案全是確定性純函式，吃數字座標吐數字座標，方便單元測試釘死。真正把方塊放進世界（讀 delta、
//! set_block、落地 jsonl、廣播、記憶）全在 `voxel_ws.rs`，嚴守短鎖鐵律（比照殖民奠基 884 黃金
//! 安全模式：surface_y 鎖外算 → deltas 寫鎖批次即釋 → 鎖外廣播＋append-only 落地）。
//! **成本紀律**：零 LLM（命名/文案走確定性模板）、append-only 向後相容（新增獨立 jsonl 檔，不動
//! 既有欄位）、FPS 零影響（每 15 秒一次純比對，築巢是一次性事件，小屋約 26 格靜態方塊）。

use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

use crate::voxel::Block;

/// 持久化路徑（`data/` 已 gitignore）。
pub const NEST_PATH: &str = "data/voxel_lovenest.jsonl";

/// 記憶哨兵鍵：戀人「和另一半蓋了共同的家」這件事記在此分類下（比照 colony 的哨兵鍵，讓
/// 日記/回想能引用，且不與「對某位玩家的記憶」混淆）。
pub const NEST_MEMORY_PLAYER: &str = "__lovenest__";

/// 每一 tick、每一對「還沒築巢的戀人」真的動手築巢的機率。刻意偏低：戀人在一起一陣子後才會
/// 決定築巢（15 秒一 tick，~0.12 → 期望數分鐘後才發生），不是締結當下立刻蓋，讓它像「在一起
/// 久了、決定住在一起」的自然進展，而非機械的即時反應。
pub const NEST_CHANCE: f32 = 0.12;

// ── 選址常數 ─────────────────────────────────────────────────────────────────────────
/// 愛巢距村莊中心的基準距離（格）：蓋在「村邊」——夠遠不壓到廣場/村碑/街廓，又夠近仍屬村子。
pub const SITE_BASE_DIST: i32 = 12;
/// 每多一座愛巢、同方向再往外推這麼多格（讓同方向的多座愛巢天然拉開、不相疊）。
pub const SITE_DIST_STEP: i32 = 8;
/// 兩座愛巢中心至少相距這麼多格才准新築（雙保險，防選址巧合撞在一起）。
pub const MIN_NEST_SEPARATION: i32 = 5;

/// 八方位單位方向（選址用，seq % 8 取一個方向，讓不同戀人對的愛巢散佈村子四周）。
const DIRS: [(i32, i32); 8] = [
    (1, 0), (1, 1), (0, 1), (-1, 1),
    (-1, 0), (-1, -1), (0, -1), (1, -1),
];

// ── 一座愛巢（持久化單位）───────────────────────────────────────────────────────────
/// 一對戀人合築的共同的家。`a`/`b` 為兩位居民**顯示名**（比照 `voxel_romance::RomanceEntry`
/// 以顯示名記帳，避免系統 id 與顯示名兩套鍵值不一致的既有教訓）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Nest {
    /// 單調遞增序號（也用來確定性選址，重啟一致）。
    pub seq: u64,
    /// 戀人 A 顯示名。
    pub a: String,
    /// 戀人 B 顯示名。
    pub b: String,
    /// 愛巢（小屋中心）世界座標。
    pub cx: i32,
    pub cz: i32,
    /// 築巢的 unix 時間（秒）。
    pub built_unix: u64,
}

/// 正規化一對名字的鍵順序，讓 (a,b) 與 (b,a) 落在同一個鍵（比照 `voxel_romance::norm`）。
fn norm(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// 愛巢名冊：全世界已築的愛巢（append-only 落地、重啟後 replay）。
#[derive(Default)]
pub struct NestRegistry {
    nests: Vec<Nest>,
    next_seq: u64,
}

impl NestRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建（重啟後 replay）。
    pub fn from_entries(entries: Vec<Nest>) -> Self {
        let mut reg = Self::new();
        for n in entries {
            reg.next_seq = reg.next_seq.max(n.seq + 1);
            reg.nests.push(n);
        }
        reg
    }

    /// 目前有幾座愛巢。
    pub fn count(&self) -> usize {
        self.nests.len()
    }

    /// 下一個要用的序號（選址吃它）。
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// 全部愛巢（唯讀快照）。
    pub fn all(&self) -> &[Nest] {
        &self.nests
    }

    /// 這一對戀人是否已經築過巢（正規化查詢，(a,b)/(b,a) 同鍵）。
    pub fn has_nest(&self, a: &str, b: &str) -> bool {
        let (x, y) = norm(a, b);
        self.nests.iter().any(|n| norm(&n.a, &n.b) == (x.clone(), y.clone()))
    }

    /// 把一座新築的愛巢登記進名冊（記憶體）。呼叫端另外負責 append 落地。
    pub fn push(&mut self, nest: Nest) {
        self.next_seq = self.next_seq.max(nest.seq + 1);
        self.nests.push(nest);
    }

    /// 是否已有愛巢離 (cx,cz) 太近（< [`MIN_NEST_SEPARATION`]）——防選址巧合撞在一起。
    pub fn too_close_to_existing(&self, cx: i32, cz: i32) -> bool {
        let min2 = (MIN_NEST_SEPARATION as i64) * (MIN_NEST_SEPARATION as i64);
        self.nests.iter().any(|n| {
            let dx = (n.cx - cx) as i64;
            let dz = (n.cz - cz) as i64;
            dx * dx + dz * dz < min2
        })
    }
}

/// 是否在這一 tick 對這一對戀人擲中「動手築巢」（純函式、roll 由呼叫端 `rand::random::<f32>()`
/// 提供，確定可測）。
pub fn nest_roll(roll: f32) -> bool {
    roll < NEST_CHANCE
}

/// 依序號選址：以村莊中心 (vcx,vcz) 為原點，取第 `seq` 個方位往外推。同方向的多座愛巢隨序號
/// 每繞一圈（8 個）再往外一階，天然拉開不相疊。確定性、重啟一致。
pub fn pick_nest_site(vcx: i32, vcz: i32, seq: u64) -> (i32, i32) {
    let (dx, dz) = DIRS[(seq % 8) as usize];
    let ring = (seq / 8) as i32;
    let dist = SITE_BASE_DIST + ring * SITE_DIST_STEP;
    (vcx + dx * dist, vcz + dz * dist)
}

/// 愛巢牌面/地標用的名字：「{a}與{b}的愛巢」。兩名以正規化順序排列，讓 (a,b)/(b,a) 同名、
/// 重啟一致（不因觸發時傳入順序不同而生出兩個名字）。
pub fn nest_name(a: &str, b: &str) -> String {
    let (x, y) = norm(a, b);
    format!("{x}與{y}的愛巢")
}

/// 產生「一間小屋」該**新增**的方塊清單（絕對世界座標）。3×3 footprint、兩層木板牆、前方正中
/// 一扇木門（門楣封頂）、屋頂滿鋪木板、屋頂中央一盞乙太燈（夜裡發光的家）。
///
/// - `cx, cz`：小屋中心。
/// - `sy`：該格「地面正上方」的 y（`voxel_building::surface_y` 語意，即第一格空氣）——牆基站在
///   地表上、逐層往上疊，全部落在地表之上的空氣層，呼叫端只需 air-only 落子、絕不覆蓋既有方塊。
///
/// 佈局（相對中心，dx/dz∈[-1,1]）：
/// - 牆（周邊 8 格）y=sy、sy+1 兩層，木板；唯前方正中 (0,+1) 那一列改成 y=sy 木門＋y=sy+1 門楣。
/// - 屋頂 y=sy+2 滿鋪 3×3 木板。
/// - y=sy+3 中央一盞乙太燈。
/// 中央內部 (0,0) 各層留空氣（可站人）。
pub fn cottage_cells(cx: i32, cz: i32, sy: i32) -> Vec<(i32, i32, i32, Block)> {
    let mut out: Vec<(i32, i32, i32, Block)> = Vec::new();

    // ① 牆（周邊 8 格，兩層高）。前方正中 (0,+1) 為門。
    for dx in -1..=1 {
        for dz in -1..=1 {
            if dx == 0 && dz == 0 {
                continue; // 中央內部留空。
            }
            let is_door = dx == 0 && dz == 1; // 前方正中央＝門柱。
            if is_door {
                out.push((cx + dx, sy, cz + dz, Block::DoorClosed)); // 下層＝可開的木門。
                out.push((cx + dx, sy + 1, cz + dz, Block::Plank)); // 上層＝門楣封頂。
            } else {
                out.push((cx + dx, sy, cz + dz, Block::Plank));
                out.push((cx + dx, sy + 1, cz + dz, Block::Plank));
            }
        }
    }

    // ② 屋頂（3×3 滿鋪木板）。
    for dx in -1..=1 {
        for dz in -1..=1 {
            out.push((cx + dx, sy + 2, cz + dz, Block::Plank));
        }
    }

    // ③ 屋頂中央一盞乙太燈——夜裡發著光，一眼認出這是一戶有人住的家。
    out.push((cx, sy + 3, cz, Block::AetherLamp));

    out
}

/// 全村動態牆播報句：一對戀人合力築起了共同的家。確定性零 LLM。
pub fn nest_feed_line(a: &str, b: &str, name: &str) -> String {
    let (x, y) = norm(a, b);
    format!("💕 {x} 與 {y} 成了戀人，在村邊合力蓋起一間亮著燈的小屋——「{name}」，兩人從此有了共同的家。")
}

/// 築巢的戀人各自寫進記憶的一句（episodic、第一人稱內心，`partner` 為另一半的名字）。
/// 不含任何玩家名／私密渴望占位符，適用於配對中的任一位。
pub fn nest_memory_line(partner: &str) -> String {
    format!("今天我和{partner}一起，在村子邊上蓋起了屬於我們兩個人的家。以後就住在一起了，想到就覺得踏實。")
}

/// 築巢當下戀人冒的泡泡台詞（≤40 字不破泡泡框），`partner` 為另一半的名字，確定性零 LLM。
pub fn nest_say_line(partner: &str) -> String {
    let line = format!("我和{partner}的新家，蓋好啦！");
    line.chars().take(40).collect()
}

// ── 持久化（append-only、向後相容）───────────────────────────────────────────────────
/// 載回所有愛巢（伺服器啟動時呼叫一次）。檔不存在 / 壞行皆容忍（比照其餘 append-only store）。
pub fn load_nests() -> Vec<Nest> {
    let Ok(f) = fs::File::open(NEST_PATH) else { return vec![] };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<Nest>(&l).ok())
        .collect()
}

/// Append 一座新愛巢到 jsonl。append-only、絕不覆寫/刪除既有行；失敗只記 log 不 panic。
pub fn append_nest(nest: &Nest) {
    let Ok(line) = serde_json::to_string(nest) else { return };
    if let Some(parent) = std::path::Path::new(NEST_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(NEST_PATH) else {
        tracing::warn!("無法寫入愛巢名冊檔 {NEST_PATH}");
        return;
    };
    let _ = writeln!(f, "{line}");
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn nest_roll_respects_chance_boundary() {
        assert!(nest_roll(0.0));
        assert!(nest_roll(NEST_CHANCE - 0.001));
        assert!(!nest_roll(NEST_CHANCE)); // 邊界＝不中（< 嚴格小於）。
        assert!(!nest_roll(0.99));
    }

    #[test]
    fn name_is_symmetric_and_deterministic() {
        // (a,b) 與 (b,a) 生出同一個名字（正規化順序）。
        assert_eq!(nest_name("露娜", "奧瑞"), nest_name("奧瑞", "露娜"));
        // 兩名都嵌進去、含「愛巢」。
        let n = nest_name("露娜", "奧瑞");
        assert!(n.contains("露娜") && n.contains("奧瑞"));
        assert!(n.contains("愛巢"));
    }

    #[test]
    fn registry_has_nest_is_symmetric() {
        let reg = NestRegistry::from_entries(vec![Nest {
            seq: 0,
            a: "露娜".into(),
            b: "奧瑞".into(),
            cx: 10,
            cz: 20,
            built_unix: 100,
        }]);
        assert!(reg.has_nest("露娜", "奧瑞"));
        assert!(reg.has_nest("奧瑞", "露娜")); // 反序也認得。
        assert!(!reg.has_nest("露娜", "諾娃"));
        assert_eq!(reg.count(), 1);
    }

    #[test]
    fn registry_next_seq_is_monotonic_after_replay() {
        let reg = NestRegistry::from_entries(vec![
            Nest { seq: 0, a: "a".into(), b: "b".into(), cx: 0, cz: 0, built_unix: 1 },
            Nest { seq: 3, a: "c".into(), b: "d".into(), cx: 0, cz: 0, built_unix: 2 },
        ]);
        assert_eq!(reg.next_seq(), 4); // max(seq)+1。
    }

    #[test]
    fn registry_push_bumps_next_seq() {
        let mut reg = NestRegistry::new();
        assert_eq!(reg.next_seq(), 0);
        reg.push(Nest { seq: 0, a: "a".into(), b: "b".into(), cx: 0, cz: 0, built_unix: 1 });
        assert_eq!(reg.next_seq(), 1);
        assert_eq!(reg.count(), 1);
    }

    #[test]
    fn too_close_detects_nearby_and_allows_far() {
        let reg = NestRegistry::from_entries(vec![Nest {
            seq: 0,
            a: "a".into(),
            b: "b".into(),
            cx: 0,
            cz: 0,
            built_unix: 1,
        }]);
        assert!(reg.too_close_to_existing(2, 2)); // 距離 ~2.8 < 5。
        assert!(!reg.too_close_to_existing(10, 0)); // 距離 10 ≥ 5。
        assert!(!reg.too_close_to_existing(0, 6)); // 距離 6 ≥ 5。
    }

    #[test]
    fn pick_site_is_deterministic_and_offset_from_center() {
        // 同 seq 同輸入 → 同結果（重啟一致）。
        assert_eq!(pick_nest_site(100, 200, 0), pick_nest_site(100, 200, 0));
        // seq=0 用第一個方位 (1,0)、基準距離。
        assert_eq!(pick_nest_site(100, 200, 0), (100 + SITE_BASE_DIST, 200));
        // 選址不落在村中心正上（至少偏離基準距離）。
        let (x, z) = pick_nest_site(0, 0, 0);
        let d2 = (x * x + z * z) as i64;
        assert!(d2 >= (SITE_BASE_DIST as i64) * (SITE_BASE_DIST as i64));
    }

    #[test]
    fn pick_site_spreads_couples_around_and_rings_outward() {
        // 前 8 座（seq 0..8）用 8 個不同方位，彼此至少相距 MIN_NEST_SEPARATION（散佈村子四周）。
        let mut sites = Vec::new();
        for seq in 0..8u64 {
            sites.push(pick_nest_site(0, 0, seq));
        }
        for i in 0..sites.len() {
            for j in (i + 1)..sites.len() {
                let (ax, az) = sites[i];
                let (bx, bz) = sites[j];
                let d2 = ((ax - bx) as i64).pow(2) + ((az - bz) as i64).pow(2);
                assert!(
                    d2 >= (MIN_NEST_SEPARATION as i64).pow(2),
                    "seq {i} 與 {j} 的愛巢選址太近了"
                );
            }
        }
        // 第 9 座（seq=8）與第 1 座（seq=0）同方位、但往外一階（ring=1），距離更遠。
        let ring0 = pick_nest_site(0, 0, 0);
        let ring1 = pick_nest_site(0, 0, 8);
        let d0 = (ring0.0 * ring0.0 + ring0.1 * ring0.1) as i64;
        let d1 = (ring1.0 * ring1.0 + ring1.1 * ring1.1) as i64;
        assert!(d1 > d0, "同方位第二圈應更外圈");
    }

    #[test]
    fn cottage_is_air_only_above_surface_and_capped_by_lamp() {
        let sy = 20;
        let cells = cottage_cells(5, -3, sy);
        assert!(!cells.is_empty());
        // 全部落在地表之上（y ≥ sy），呼叫端 air-only 落子才安全、不覆蓋地表/既有方塊。
        assert!(cells.iter().all(|c| c.1 >= sy));
        // 座標不重複（同一格不被兩種方塊爭用）。
        let mut seen = HashSet::new();
        for c in &cells {
            assert!(seen.insert((c.0, c.1, c.2)), "座標 {:?} 被重複佔用", (c.0, c.1, c.2));
        }
        // 最高一格是乙太燈（夜裡發光的家）。
        let top = cells.iter().max_by_key(|c| c.1).unwrap();
        assert_eq!(top.3, Block::AetherLamp);
        assert_eq!((top.0, top.2), (5, -3)); // 燈在中心正上。
    }

    #[test]
    fn cottage_has_a_door_and_leaves_interior_open() {
        let sy = 10;
        let cells = cottage_cells(0, 0, sy);
        // 前方正中 (0, sy, +1) 是可開的木門。
        assert!(cells.iter().any(|c| *c == (0, sy, 1, Block::DoorClosed)));
        // 中央內部 (0,0) 各層都沒放方塊（可站人）。
        assert!(!cells.iter().any(|c| c.0 == 0 && c.2 == 0 && c.1 < sy + 2));
    }

    #[test]
    fn feed_and_memory_lines_are_clean_single_line() {
        let feed = nest_feed_line("露娜", "奧瑞", "露娜與奧瑞的愛巢");
        assert!(feed.contains("露娜") && feed.contains("奧瑞"));
        assert!(feed.contains("露娜與奧瑞的愛巢"));
        assert!(!feed.contains('\n') && !feed.is_empty());

        let mem = nest_memory_line("奧瑞");
        assert!(mem.contains("奧瑞"));
        assert!(!mem.contains('\n') && !mem.is_empty());
        assert!(!mem.contains('{')); // 不外洩占位符。

        let say = nest_say_line("露娜");
        assert!(say.contains("露娜"));
        assert!(say.chars().count() <= 40); // 不破泡泡框。
    }
}
