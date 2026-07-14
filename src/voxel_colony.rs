//! 乙太方界·分村殖民 v1——拓荒立村（自主提案切片，承 PLAN_ETHERVOX §7「居民散佈世界各處住」
//! 與維護者 2026-07-10 IDEAS_ETHERVOX「分村(殖民)機制——活的野外村落」）。
//!
//! **真缺口**：主村已長成一座有廣場有路的村莊（835 村莊系統），人口也靠世代傳承（819 誕辰）
//! 慢慢變多，但無論村子多成熟、居民多擁擠，**世界永遠只有這一座村**。麥塊那種「探索遠方
//! 撞見一座村落」的驚喜，這片天地至今沒有——而且維護者點明：麥塊的預生成村莊是「死殼」
//! （憑空出現、無來歷），違和這個「每一磚都有來歷」的世界。**正解＝分村**：主村夠成熟時，
//! 派幾位居民組成拓荒隊，長途跋涉到另一片群系，親手奠下第二座村的雛形——它**有名字、有拓荒
//! 故事、記得是誰在哪一天建立**。玩家遠行探索撞見它，能讀到這段來歷。
//!
//! **本刀範圍（v1，刻意有界）**：只做**奠基那一刻**——主村成熟 → 選址遠方異群系 → 選出拓荒者
//! → 生成村名與立村故事 → 在遠方落下一座**奠基殘核**（小廣場鋪面＋中央水井＋四角燈＋一座立村
//! 碑）→ 記進持久殖民地名冊 → 世界公告 → 拓荒者記得這件事 → 玩家走近可發現、記進探索紀事。
//! **不做**（明確排在未來、屬架構級，見 ROADMAP 條目）：居民真的搬過去定居、跨村的名冊/技能/
//! 交情分域、第二村自己長路長地塊。那些要先把「單一村莊中心」的資料模型一般化，不在本刀。
//!
//! **與既有地標的區隔**：遺跡/溫泉（838/839）是**程序生成、噪聲決定、無來歷**的天然地標；
//! 殖民地是**事件驅動、執行期由居民奠基、有名字有故事**的人為聚落——這正是維護者要的「活的
//! 野外村落」與「死殼」的分野。
//!
//! **純邏輯層鐵律**：本檔零 LLM、零鎖、零 async、零世界 IO——選址/選人/命名/立村故事/奠基
//! 方塊佈局/成熟度閘全是確定性純函式，吃數字座標吐數字座標，方便單元測試釘死。真正把方塊放進
//! 世界（讀 delta、set_block、落地 jsonl、廣播、記憶）全在 `voxel_ws.rs`，嚴守短鎖鐵律。
//! **成本紀律**：零 LLM（村名/故事走策展模板＋確定性挑選）、append-only 向後相容、FPS 零影響
//! （每 15 秒一次純比對，奠基是一次性事件）。

use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

use serde::{Deserialize, Serialize};

use crate::voxel::{Block, VoxelBiome};

/// 持久化路徑（`data/` 已 gitignore）。
pub const COLONY_PATH: &str = "data/voxel_colonies.jsonl";

/// 記憶哨兵鍵：拓荒者「奠基了某座殖民地」這件事記在此分類下（比照 expedition 的哨兵鍵，
/// 讓日記/回想能引用，且不與「對某位玩家的記憶」混淆）。
pub const COLONY_MEMORY_PLAYER: &str = "__colony__";

// ── 成熟度 / 節流常數 ─────────────────────────────────────────────────────────────────
/// 觸發第一次奠基所需的最低人口（含世代傳承出生的居民）。小到主村真的「擠」了才外派。
pub const MIN_POP_TO_FOUND: usize = 6;
/// 觸發奠基所需的最低「已認領地塊數」——代表主村街廓已鋪開、住得夠滿。
pub const MIN_CLAIMED_TO_FOUND: usize = 4;
/// 世界最多幾座殖民地（含奠基殘核）。有界成長，避免無限外擴刷爆世界/名冊。
pub const MAX_COLONIES: usize = 4;
/// 兩次奠基之間的最短間隔（秒）：一座村站穩再談下一座，拓荒是稀有大事。
pub const FOUND_INTERVAL_SECS: u64 = 6 * 60 * 60; // 6 小時（乙太世界的一次遠征級大事）

// ── 選址常數 ─────────────────────────────────────────────────────────────────────────
/// 第一座殖民地距主村中心的基準距離（格）：夠遠才有「遠方村落」的探索感，且遠離主村街廓。
pub const SITE_BASE_DIST: i32 = 520;
/// 每多一座殖民地，選址距離再往外推這麼多格（讓各殖民地天然拉開、彼此不擠）。
pub const SITE_DIST_STEP: i32 = 180;
/// 兩座殖民地中心至少相距這麼多格才准新奠基（雙保險，防選址巧合太近）。
pub const MIN_COLONY_SEPARATION: i32 = 240;

// ── 立村奠基殘核尺寸 ─────────────────────────────────────────────────────────────────
/// 奠基小廣場半徑（格）：(2R+1)×(2R+1) 鋪面。R=2 → 5×5，小而聚，像剛站穩腳跟的拓荒營地。
pub const NUCLEUS_PLAZA_RADIUS: i32 = 2;

/// 玩家走近殖民地中心這麼多格內＝「發現」它（記進探索紀事）。夠寬，讓遠行撞見不易錯過。
pub const DISCOVER_RADIUS: f32 = 44.0;

// ── 一座殖民地（持久化單位）─────────────────────────────────────────────────────────
/// 一座殖民地。`biome` 存成穩定字串 wire（[`biome_wire`]）而非 enum——`VoxelBiome` 未 derive
/// Serialize，且字串 wire 讓內部 enum 改名不牽動已落地的歷史檔（向後相容）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Colony {
    /// 單調遞增序號（也用來確定性選址/命名，重啟一致）。
    pub seq: u64,
    /// 村名（策展模板＋確定性挑選，如「霜語屯」）。
    pub name: String,
    /// 村中心世界座標。
    pub cx: i32,
    pub cz: i32,
    /// 所在群系 wire（"grassland"/"forest"/"desert"/"snow"）。
    pub biome: String,
    /// 拓荒者名字（奠基那一刻外派的居民；v1 純敘事，不代表真的搬過去住）。
    pub founders: Vec<String>,
    /// 立村故事（人類可讀的一句來歷）。
    pub story: String,
    /// 奠基的 unix 時間（秒）。
    pub founded_unix: u64,
}

/// 殖民地名冊：全世界已奠基的殖民地（有界成長、append-only 落地）。
#[derive(Default)]
pub struct ColonyRegistry {
    colonies: Vec<Colony>,
    next_seq: u64,
}

impl ColonyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 由載入的歷史事件重建（重啟後 replay）。
    pub fn from_entries(entries: Vec<Colony>) -> Self {
        let mut reg = Self::new();
        for c in entries {
            reg.next_seq = reg.next_seq.max(c.seq + 1);
            reg.colonies.push(c);
        }
        reg
    }

    /// 目前有幾座殖民地。
    pub fn count(&self) -> usize {
        self.colonies.len()
    }

    /// 下一個要用的序號（選址/命名/落地都吃它）。
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// 全部殖民地（唯讀快照，供廣播/玩家發現比距）。
    pub fn all(&self) -> &[Colony] {
        &self.colonies
    }

    /// 把一座新奠基的殖民地登記進名冊（記憶體）。呼叫端另外負責 append 落地。
    pub fn push(&mut self, colony: Colony) {
        self.next_seq = self.next_seq.max(colony.seq + 1);
        self.colonies.push(colony);
    }

    /// 是否已有殖民地離 (cx,cz) 太近（< [`MIN_COLONY_SEPARATION`]）——防選址巧合撞在一起。
    pub fn too_close_to_existing(&self, cx: i32, cz: i32) -> bool {
        let min2 = (MIN_COLONY_SEPARATION as i64) * (MIN_COLONY_SEPARATION as i64);
        self.colonies.iter().any(|c| {
            let dx = (c.cx - cx) as i64;
            let dz = (c.cz - cz) as i64;
            dx * dx + dz * dz < min2
        })
    }

    /// 找出離 (px,pz) 在 `radius` 格內、最近的一座殖民地（供玩家發現）。回傳其參照。
    pub fn nearest_within(&self, px: f32, pz: f32, radius: f32) -> Option<&Colony> {
        let r2 = radius * radius;
        self.colonies
            .iter()
            .filter_map(|c| {
                let dx = c.cx as f32 - px;
                let dz = c.cz as f32 - pz;
                let d2 = dx * dx + dz * dz;
                if d2 <= r2 { Some((d2, c)) } else { None }
            })
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(_, c)| c)
    }
}

// ── 群系 wire 互轉（不動 voxel.rs 的 enum derive）─────────────────────────────────────
/// 群系 → 穩定字串 wire。
pub fn biome_wire(b: VoxelBiome) -> &'static str {
    match b {
        VoxelBiome::Grassland => "grassland",
        VoxelBiome::Forest => "forest",
        VoxelBiome::Desert => "desert",
        VoxelBiome::Snow => "snow",
    }
}

/// 群系面向玩家的繁中名（i18n 集中此處）。
pub fn biome_label(b: VoxelBiome) -> &'static str {
    match b {
        VoxelBiome::Grassland => "草原",
        VoxelBiome::Forest => "森林",
        VoxelBiome::Desert => "沙漠",
        VoxelBiome::Snow => "雪原",
    }
}

// ── 成熟度閘（純函式）────────────────────────────────────────────────────────────────
/// 主村是否「夠成熟、該外派拓荒隊」了。全部條件皆須滿足：
/// 人口達門檻 + 已認領地塊達門檻 + 主村街廓已鋪好(`village_done`) +
/// 殖民地數未達上限 + 距上次奠基已過最短間隔。純函式、可測。
#[allow(clippy::too_many_arguments)]
pub fn should_found(
    pop: usize,
    claimed: usize,
    village_done: bool,
    colonies_count: usize,
    elapsed_since_last: u64,
) -> bool {
    pop >= MIN_POP_TO_FOUND
        && claimed >= MIN_CLAIMED_TO_FOUND
        && village_done
        && colonies_count < MAX_COLONIES
        && elapsed_since_last >= FOUND_INTERVAL_SECS
}

// ── 選址（純函式、確定性）────────────────────────────────────────────────────────────
/// 依序號 `seq` 與主村中心 (mx,mz) 確定性挑一處遠方選址。距離隨 seq 往外推、方位以 seq 均分
/// 一圈，讓歷來各殖民地天然散在主村四周不同方向、彼此拉開。純函式、可測（同輸入同輸出）。
pub fn pick_site(seq: u64, mx: i32, mz: i32) -> (i32, i32) {
    let dist = SITE_BASE_DIST + (seq as i32) * SITE_DIST_STEP;
    // 用一組固定「黃金角」步進的方位表，讓相鄰序號的方向差得夠開（非同一直線一路往外）。
    // 8 個大致均分又錯開的方位（避免全落在正東正北，看起來像放射狀拓荒）。
    const BEARINGS: [(f32, f32); 8] = [
        (1.0, 0.30),
        (-0.40, 0.92),
        (0.55, -0.84),
        (-0.95, -0.32),
        (0.25, 0.97),
        (-0.80, 0.60),
        (0.90, -0.44),
        (-0.30, -0.95),
    ];
    let (ux, uz) = BEARINGS[(seq as usize) % BEARINGS.len()];
    // 單位化（表內向量非嚴格單位長，正規化確保距離感一致）。
    let len = (ux * ux + uz * uz).sqrt().max(0.0001);
    let cx = mx + (ux / len * dist as f32).round() as i32;
    let cz = mz + (uz / len * dist as f32).round() as i32;
    (cx, cz)
}

/// 方位的繁中概略描述（給立村故事/公告用「往西北方」這種人味方向）。
pub fn bearing_label(mx: i32, mz: i32, cx: i32, cz: i32) -> &'static str {
    let dx = (cx - mx) as f32;
    let dz = (cz - mz) as f32;
    // 世界座標：+x 東、+z 南（沿用 expedition::bearing_label 慣例）。
    let ew = if dx.abs() < dz.abs() * 0.5 {
        ""
    } else if dx >= 0.0 {
        "東"
    } else {
        "西"
    };
    let ns = if dz.abs() < dx.abs() * 0.5 {
        ""
    } else if dz >= 0.0 {
        "南"
    } else {
        "北"
    };
    match (ns, ew) {
        ("", "") => "遠方",
        ("", e) => match e {
            "東" => "東方",
            _ => "西方",
        },
        (n, "") => match n {
            "南" => "南方",
            _ => "北方",
        },
        ("北", "東") => "東北方",
        ("北", "西") => "西北方",
        ("南", "東") => "東南方",
        ("南", "西") => "西南方",
        _ => "遠方",
    }
}

// ── 村莊地圖標記（純函式，供地圖面板顯示「還有其他村子」）───────────────────────────────
/// 一座殖民地在村莊地圖面板上的摘要條目：方位＋直線距離＋現居人口，不含世界 IO。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ColonyMapEntry {
    pub name: String,
    pub bearing: &'static str,
    pub dist: i64,
    pub population: usize,
}

/// 由一座殖民地與村莊中心座標算出地圖摘要條目。人口數由呼叫端另外查好傳入
/// （殖民地名冊與聚落歸屬名冊是兩本不同的鎖，本函式保持零 IO／零鎖）。純函式、可測。
pub fn colony_map_entry(colony: &Colony, vcx: i32, vcz: i32, population: usize) -> ColonyMapEntry {
    let dx = (colony.cx - vcx) as f64;
    let dz = (colony.cz - vcz) as f64;
    ColonyMapEntry {
        name: colony.name.clone(),
        bearing: bearing_label(vcx, vcz, colony.cx, colony.cz),
        dist: (dx * dx + dz * dz).sqrt().round() as i64,
        population,
    }
}

// ── 選拓荒者（純函式、確定性）────────────────────────────────────────────────────────
/// 從現有居民名字裡確定性挑出這次的拓荒隊（v1 派 2 位）。用 seq 決定起點，避免每次都同一批。
/// 名單少於 2 人時就給幾位算幾位（呼叫端在人口門檻已擋住太少的情形）。純函式、可測。
pub fn pick_founders(names: &[String], seq: u64) -> Vec<String> {
    if names.is_empty() {
        return Vec::new();
    }
    let want = 2.min(names.len());
    let start = (seq as usize) % names.len();
    let mut out = Vec::with_capacity(want);
    for k in 0..want {
        out.push(names[(start + k) % names.len()].clone());
    }
    out
}

// ── 命名（純函式、確定性、零 LLM）────────────────────────────────────────────────────
/// 依群系確定性挑一個詩意村名詞根（每群系一組策展詞根，扣住那片土地的氣味）。
fn name_roots(biome: VoxelBiome) -> &'static [&'static str] {
    match biome {
        VoxelBiome::Snow => &["霜語", "雪誓", "凜光", "白河", "寒星"],
        VoxelBiome::Desert => &["日灼", "流沙", "金曦", "旱井", "曜石"],
        VoxelBiome::Forest => &["翠濤", "綠蔭", "林語", "苔痕", "深樾"],
        VoxelBiome::Grassland => &["風禾", "草浪", "晴野", "牧歌", "初穗"],
    }
}

/// 村名後綴（聚落規模感，由小到有村的樣子）。
const NAME_SUFFIX: [&str; 3] = ["屯", "隅", "村"];

/// 確定性生成村名：群系詞根 + 後綴，用 seq 挑選（同 seq 同群系永遠同名，重啟一致）。
pub fn colony_name(biome: VoxelBiome, seq: u64) -> String {
    let roots = name_roots(biome);
    let root = roots[(seq as usize) % roots.len()];
    let suffix = NAME_SUFFIX[((seq as usize) / roots.len()) % NAME_SUFFIX.len()];
    format!("{root}{suffix}")
}

/// 立村故事（人類可讀的一句來歷）：誰、往哪個方向、在哪片群系、建立了哪座村。
pub fn founding_story(name: &str, founders: &[String], biome: VoxelBiome, bearing: &str) -> String {
    let who = match founders.len() {
        0 => "一群拓荒者".to_string(),
        1 => founders[0].clone(),
        _ => format!("{}與{}", founders[0], founders[1]),
    };
    format!(
        "{who}離開擁擠的主村，往{bearing}跋涉，在這片{biome}親手奠下了「{name}」——一座從零攢起、每一磚都記得來歷的野外村落。",
        biome = biome_label(biome),
    )
}

/// 奠基當下、外派居民頭上冒的泡泡（面向所有人、有拓荒的莊重感）。
pub fn embark_bubble(name: &str, bearing: &str) -> String {
    format!("主村住滿了…我往{bearing}去，替我們開一座新村——就叫「{name}」吧。")
}

/// 世界動態 feed 一句（非同步層可回看）。
pub fn founding_feed_line(name: &str, biome: VoxelBiome, bearing: &str) -> String {
    format!("拓荒隊往{bearing}的{}奠下了新村「{name}」", biome_label(biome))
}

/// 拓荒者記憶摘要（記在 [`COLONY_MEMORY_PLAYER`] 分類下，日記/回想可引用）。
pub fn founder_memory_summary(name: &str, biome: VoxelBiome) -> String {
    format!("我遠行到{}，替大家奠下了新村「{name}」，這片天地第一次有了第二座村。", biome_label(biome))
}

/// 玩家走近、第一次發現一座殖民地時單播給他看的來歷（含村名與立村故事）。
pub fn discover_line(colony: &Colony) -> String {
    format!("你發現了野外村落「{}」🏘️——{}", colony.name, colony.story)
}

// ── 奠基殘核方塊佈局（純函式、確定性、可測）──────────────────────────────────────────
/// 依村中心 (cx,cz) 與「錨點地表 y」`sy`（呼叫端用 `surface_y(cx,cz)` 就地算好傳入，比照
/// expedition 營火/小棚：小尺寸殘核假設選址地表大致平坦）生成奠基殘核的方塊清單。
///
/// 回傳 `(x, y, z, block, ground_level)`：`ground_level=true` 的方塊是「地表重鋪」層
/// （y = sy-1，呼叫端只在該格目前是自然地表時覆蓋，絕不拆作品）；`false` 的是「地表之上」層
/// （y ≥ sy，呼叫端只在該格目前是空氣時放，絕不覆蓋任何既有方塊）。這條分層規則讓呼叫端能
/// 用單一守則安全落地，不必知道每塊的語意。
///
/// 殘核組成：小廣場鋪面（plaza_surface）＋中央水井（Water）＋四角燈（Torch）＋一座立村碑
/// （StoneBrick 柱 + 頂 Torch，立在廣場北緣，遠遠一眼認得出這是座有人奠基的村）。
pub fn nucleus_blocks(cx: i32, cz: i32, sy: i32, biome: VoxelBiome) -> Vec<(i32, i32, i32, Block, bool)> {
    let mut out: Vec<(i32, i32, i32, Block, bool)> = Vec::new();
    let plaza = crate::voxel_village::plaza_surface(biome);
    let gy = sy - 1; // 地表方塊本身那一層。

    // ① 小廣場鋪面（中央除外——留給水井）。地表重鋪層。
    let r = NUCLEUS_PLAZA_RADIUS;
    for dx in -r..=r {
        for dz in -r..=r {
            if dx == 0 && dz == 0 {
                continue; // 中央留給水井。
            }
            out.push((cx + dx, gy, cz + dz, plaza, true));
        }
    }
    // ② 中央水井：把中央地表格換成水（一口井的意象）。地表重鋪層。
    out.push((cx, gy, cz, Block::Water, true));

    // ③ 四角燈（火把插在廣場四角、鋪面之上一格）。地表之上層。
    for &(dx, dz) in &[(-r, -r), (-r, r), (r, -r), (r, r)] {
        out.push((cx + dx, sy, cz + dz, Block::Torch, false));
    }

    // ④ 立村碑：廣場北緣（-z 方向）外一格，一柱兩層石磚 + 頂端火把。地表之上層。
    let mx = cx;
    let mz = cz - (r + 1);
    out.push((mx, sy, mz, Block::StoneBrick, false));
    out.push((mx, sy + 1, mz, Block::StoneBrick, false));
    out.push((mx, sy + 2, mz, Block::Torch, false));

    out
}

// ── 持久化（append-only、向後相容）───────────────────────────────────────────────────
/// 載回所有殖民地（伺服器啟動時呼叫一次）。檔不存在 / 壞行皆容忍（比照其餘 append-only store）。
pub fn load_colonies() -> Vec<Colony> {
    let Ok(f) = fs::File::open(COLONY_PATH) else { return vec![] };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<Colony>(&l).ok())
        .collect()
}

/// Append 一座新殖民地到 jsonl。append-only、絕不覆寫/刪除既有行；失敗只記 log 不 panic。
pub fn append_colony(colony: &Colony) {
    let Ok(line) = serde_json::to_string(colony) else { return };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(COLONY_PATH) else {
        tracing::warn!("無法寫入殖民地名冊檔 {COLONY_PATH}");
        return;
    };
    let _ = writeln!(f, "{line}");
}

// ── 上次奠基時間戳（跨重啟節流基準，比照 voxel_roster 的 last_birth）─────────────────
/// 覆寫式單值檔（非 append）：只保留最新一次奠基的 unix 秒，讓奠基間隔跨重啟累積（修正
/// prod 每 15 分重啟會讓「距上次夠久」永遠重置的 bug——比照 811 誕生節流）。
const LAST_FOUND_PATH: &str = "data/voxel_last_colony";

/// 載回上次奠基的 unix 秒（檔缺 = 從沒奠過 → None）。
pub fn load_last_found_unix() -> Option<u64> {
    std::fs::read_to_string(LAST_FOUND_PATH).ok().and_then(|s| s.trim().parse::<u64>().ok())
}

/// 覆寫存下最新一次奠基的 unix 秒。IO 失敗只忽略、不 panic（寧可下次早一點再判）。
pub fn save_last_found_unix(unix: u64) {
    if let Some(parent) = std::path::Path::new(LAST_FOUND_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(LAST_FOUND_PATH, format!("{unix}\n"));
}

// ── 單元測試 ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_found_all_conditions_must_hold() {
        // 全部滿足 → true。
        assert!(should_found(6, 4, true, 0, FOUND_INTERVAL_SECS));
        // 人口不足。
        assert!(!should_found(5, 4, true, 0, FOUND_INTERVAL_SECS));
        // 地塊不足。
        assert!(!should_found(6, 3, true, 0, FOUND_INTERVAL_SECS));
        // 主村街廓還沒鋪好。
        assert!(!should_found(6, 4, false, 0, FOUND_INTERVAL_SECS));
        // 已達殖民地上限。
        assert!(!should_found(9, 9, true, MAX_COLONIES, FOUND_INTERVAL_SECS));
        // 距上次奠基還不夠久。
        assert!(!should_found(6, 4, true, 0, FOUND_INTERVAL_SECS - 1));
    }

    #[test]
    fn pick_site_is_deterministic_and_far() {
        let a = pick_site(0, 100, 200);
        let b = pick_site(0, 100, 200);
        assert_eq!(a, b, "同輸入必同址（重啟一致）");
        let dx = (a.0 - 100) as f32;
        let dz = (a.1 - 200) as f32;
        let dist = (dx * dx + dz * dz).sqrt();
        assert!(dist >= SITE_BASE_DIST as f32 - 2.0, "第一座選址該離主村約基準距離，實得 {dist}");
    }

    #[test]
    fn pick_site_later_colonies_push_further_and_apart() {
        let s0 = pick_site(0, 0, 0);
        let s1 = pick_site(1, 0, 0);
        let d0 = ((s0.0 * s0.0 + s0.1 * s0.1) as f32).sqrt();
        let d1 = ((s1.0 * s1.0 + s1.1 * s1.1) as f32).sqrt();
        assert!(d1 > d0, "後來的殖民地應被推得更遠");
        // 不同 seq 的方位該錯開，兩址不會撞在一起。
        let sep = (((s0.0 - s1.0).pow(2) + (s0.1 - s1.1).pow(2)) as f32).sqrt();
        assert!(sep > MIN_COLONY_SEPARATION as f32, "相鄰序號選址應天然拉開，實得 {sep}");
    }

    #[test]
    fn registry_dedup_and_separation() {
        let mut reg = ColonyRegistry::new();
        assert_eq!(reg.count(), 0);
        assert_eq!(reg.next_seq(), 0);
        reg.push(Colony {
            seq: 0,
            name: "霜語屯".into(),
            cx: 500,
            cz: 0,
            biome: "snow".into(),
            founders: vec!["露娜".into()],
            story: "…".into(),
            founded_unix: 100,
        });
        assert_eq!(reg.count(), 1);
        assert_eq!(reg.next_seq(), 1, "next_seq 應接續在已用序號之後");
        // 太近 → 擋；夠遠 → 放行。
        assert!(reg.too_close_to_existing(500 + MIN_COLONY_SEPARATION - 10, 0));
        assert!(!reg.too_close_to_existing(500 + MIN_COLONY_SEPARATION + 10, 0));
    }

    #[test]
    fn registry_from_entries_rebuilds_next_seq() {
        let entries = vec![
            Colony { seq: 0, name: "a".into(), cx: 0, cz: 0, biome: "snow".into(), founders: vec![], story: "".into(), founded_unix: 1 },
            Colony { seq: 3, name: "b".into(), cx: 900, cz: 0, biome: "desert".into(), founders: vec![], story: "".into(), founded_unix: 2 },
        ];
        let reg = ColonyRegistry::from_entries(entries);
        assert_eq!(reg.count(), 2);
        assert_eq!(reg.next_seq(), 4, "next_seq 應為歷來最大序號 + 1");
    }

    #[test]
    fn nearest_within_finds_closest_in_radius() {
        let mut reg = ColonyRegistry::new();
        reg.push(Colony { seq: 0, name: "近".into(), cx: 10, cz: 0, biome: "snow".into(), founders: vec![], story: "".into(), founded_unix: 1 });
        reg.push(Colony { seq: 1, name: "遠".into(), cx: 1000, cz: 0, biome: "desert".into(), founders: vec![], story: "".into(), founded_unix: 2 });
        // 玩家站在 (20,0)：只有「近」在半徑內。
        let hit = reg.nearest_within(20.0, 0.0, DISCOVER_RADIUS);
        assert_eq!(hit.map(|c| c.name.as_str()), Some("近"));
        // 站在半徑外：找不到。
        assert!(reg.nearest_within(500.0, 0.0, DISCOVER_RADIUS).is_none());
    }

    #[test]
    fn pick_founders_deterministic_and_bounded() {
        let names: Vec<String> = ["露娜", "諾娃", "賽勒", "奧瑞"].iter().map(|s| s.to_string()).collect();
        let f0 = pick_founders(&names, 0);
        assert_eq!(f0.len(), 2, "v1 派 2 位");
        assert_eq!(f0, pick_founders(&names, 0), "同輸入同拓荒隊");
        // 不同 seq 起點不同。
        assert_ne!(pick_founders(&names, 0), pick_founders(&names, 1));
        // 邊界：0/1 人。
        assert!(pick_founders(&[], 5).is_empty());
        assert_eq!(pick_founders(&["獨行".to_string()], 3).len(), 1);
    }

    #[test]
    fn colony_name_deterministic_per_biome_and_seq() {
        let n = colony_name(VoxelBiome::Snow, 0);
        assert_eq!(n, colony_name(VoxelBiome::Snow, 0), "同輸入同名");
        assert!(!n.is_empty());
        // 群系不同 → 詞根不同（雪原不會叫「日灼」）。
        assert_ne!(colony_name(VoxelBiome::Snow, 0), colony_name(VoxelBiome::Desert, 0));
        // 名字帶其中一個後綴。
        assert!(NAME_SUFFIX.iter().any(|s| n.ends_with(s)), "村名應以某後綴結尾，實得 {n}");
    }

    #[test]
    fn founding_story_mentions_name_and_founders() {
        let founders = vec!["露娜".to_string(), "諾娃".to_string()];
        let story = founding_story("霜語屯", &founders, VoxelBiome::Snow, "西北方");
        assert!(story.contains("霜語屯"));
        assert!(story.contains("露娜"));
        assert!(story.contains("諾娃"));
        assert!(story.contains("雪原"));
        assert!(story.contains("西北方"));
        // 0 拓荒者也不 panic、有合理敘述。
        let s2 = founding_story("孤村", &[], VoxelBiome::Grassland, "遠方");
        assert!(s2.contains("孤村") && !s2.is_empty());
    }

    #[test]
    fn bearing_label_cardinals() {
        // +x 東、+z 南。
        assert_eq!(bearing_label(0, 0, 500, 0), "東方");
        assert_eq!(bearing_label(0, 0, -500, 0), "西方");
        assert_eq!(bearing_label(0, 0, 0, 500), "南方");
        assert_eq!(bearing_label(0, 0, 0, -500), "北方");
        assert_eq!(bearing_label(0, 0, 400, -400), "東北方");
        assert_eq!(bearing_label(0, 0, -400, 400), "西南方");
    }

    #[test]
    fn nucleus_blocks_layers_and_wellcenter() {
        let blocks = nucleus_blocks(100, 200, 65, VoxelBiome::Snow);
        assert!(!blocks.is_empty());
        // 中央 (100,200) 該是水井、且是地表重鋪層。
        let center: Vec<_> = blocks.iter().filter(|(x, _, z, _, _)| *x == 100 && *z == 200).collect();
        assert_eq!(center.len(), 1, "中央只該有一塊（水井），不與鋪面重複");
        assert_eq!(center[0].3, Block::Water);
        assert!(center[0].4, "水井屬地表重鋪層");
        // 地表重鋪層一律在 y = sy-1；地表之上層一律在 y ≥ sy。
        for &(_, y, _, _, ground) in &blocks {
            if ground {
                assert_eq!(y, 64, "地表重鋪層該在 sy-1");
            } else {
                assert!(y >= 65, "地表之上層該在 sy 或更高");
            }
        }
        // 該有四角火把 + 立村碑頂火把 = 至少 5 個 Torch。
        let torches = blocks.iter().filter(|(_, _, _, b, _)| *b == Block::Torch).count();
        assert!(torches >= 5, "四角燈 + 立村碑頂燈，實得 {torches}");
        // 立村碑在北緣（z 更小處）該有兩層石磚。廣場鋪面（plaza_surface）恰也是石磚但屬
        // 地表重鋪層（ground=true），故只數「地表之上層」（ground=false）的石磚，才是立村碑本身。
        let bricks = blocks
            .iter()
            .filter(|(_, _, _, b, ground)| *b == Block::StoneBrick && !ground)
            .count();
        assert_eq!(bricks, 2, "立村碑兩層石磚（地表之上層，不含廣場鋪面）");
    }

    #[test]
    fn nucleus_blocks_deterministic() {
        assert_eq!(
            nucleus_blocks(10, 20, 65, VoxelBiome::Desert),
            nucleus_blocks(10, 20, 65, VoxelBiome::Desert),
            "同輸入同佈局（重啟一致、可落地冪等）"
        );
    }

    #[test]
    fn biome_wire_roundtrip_labels_distinct() {
        for b in [VoxelBiome::Grassland, VoxelBiome::Forest, VoxelBiome::Desert, VoxelBiome::Snow] {
            assert!(!biome_wire(b).is_empty());
            assert!(!biome_label(b).is_empty());
        }
        assert_ne!(biome_wire(VoxelBiome::Snow), biome_wire(VoxelBiome::Desert));
        assert_ne!(biome_label(VoxelBiome::Snow), biome_label(VoxelBiome::Desert));
    }

    fn sample_colony(cx: i32, cz: i32) -> Colony {
        Colony {
            seq: 0,
            name: "風禾屯".to_string(),
            cx,
            cz,
            biome: biome_wire(VoxelBiome::Grassland).to_string(),
            founders: vec!["露娜".to_string()],
            story: "測試故事".to_string(),
            founded_unix: 0,
        }
    }

    #[test]
    fn colony_map_entry_computes_distance_and_bearing() {
        // 正東方 300 格：距離即 300，方位「東方」。
        let c = sample_colony(400, 100);
        let e = colony_map_entry(&c, 100, 100, 3);
        assert_eq!(e.dist, 300);
        assert_eq!(e.bearing, "東方");
        assert_eq!(e.population, 3);
        assert_eq!(e.name, "風禾屯");
    }

    #[test]
    fn colony_map_entry_pythagorean_distance() {
        // 3-4-5 三角形：東 300、南 400 → 距離 500。
        let c = sample_colony(400, 500);
        let e = colony_map_entry(&c, 100, 100, 0);
        assert_eq!(e.dist, 500);
        assert_eq!(e.bearing, "東南方");
        assert_eq!(e.population, 0);
    }

    #[test]
    fn colony_map_entry_same_point_zero_distance() {
        let c = sample_colony(100, 100);
        let e = colony_map_entry(&c, 100, 100, 1);
        assert_eq!(e.dist, 0);
    }
}
