//! 乙太方界居民蓋家系統——渴望化為方塊 v1（ROADMAP 652）。
//!
//! 居民持有心願後（由 `voxel_desires` 管理），本模組把心願分類為「建物類型」，
//! 生成一份依序放置的方塊清單（`BuildPlan`），tick 每 8 秒放一塊——
//! 讓玩家親眼看到 AI 居民把夢想一磚一瓦蓋成真。
//!
//! **純邏輯層**：零 LLM、零鎖、零 IO 外包；`classify_desire` 與 `generate_blocks` 皆純函式。
//! 鎖 / 廣播 / 持久化觸發全在 `voxel_ws.rs`。

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::voxel::{biome_at_voxel, block_at, Block, VoxelBiome, BASE_HEIGHT};

// ── 建物類型 ──────────────────────────────────────────────────────────────────

/// 居民可蓋的建物種類（規則分類，零 LLM）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum BuildKind {
    /// 小木屋：Wood 牆 + Stone 頂，3×3×3。
    House,
    /// 水井：Stone 框 + Water 中心，3×3×2。
    Well,
    /// 瞭望台：Stone 柱身 + 頂台，3×3×6。
    Tower,
    /// 花圃：Grass 地 + Stone 邊框 + Leaves 中心，3×3×2。
    Garden,
    /// 涼亭：四柱撐頂、四面通透的遮蔭歇腳處，3×3×4。
    /// 刻意**不封牆**——與封閉的小木屋、高聳的瞭望台、種花的花圃、蓄水的水井都不同，
    /// 給世界第一座「開放式公共歇腳／遮雨」的建物，也是居民「想有個乘涼避雨的地方」渴望的落地。
    Pavilion,
    /// 工坊：比小木屋更大（5×4）的做工空間——三面牆＋正面敞口，露台一角擺工作台／熔爐，
    /// 讓「想有個能安心打鐵／做東西的地方」落地成一座真的能認得出是工坊的建物。
    Workshop,
    /// 磨坊：靠水而立（錨點旁 2 格內須有水才允許此 kind），石基木架撐起一座水輪造型，
    /// 是世界第一座「必須挑對地點才蓋得成」的建物——水邊限定，離水就蓋不了。
    Millhouse,
    /// 紀念碑：小佔地（3×3）卻高聳（5 層）的石柱，頂上一盞燈——不住人、不做工，
    /// 純為「想立個能被遠遠望見、記住某件事的地標」而生。
    Monument,
}

impl BuildKind {
    /// 顯示名（繁中，玩家看到的）。
    pub fn display_name(self) -> &'static str {
        match self {
            BuildKind::House => "小木屋",
            BuildKind::Well => "水井",
            BuildKind::Tower => "瞭望台",
            BuildKind::Garden => "花圃",
            BuildKind::Pavilion => "涼亭",
            BuildKind::Workshop => "工坊",
            BuildKind::Millhouse => "磨坊",
            BuildKind::Monument => "紀念碑",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            BuildKind::House => "house",
            BuildKind::Well => "well",
            BuildKind::Tower => "tower",
            BuildKind::Garden => "garden",
            BuildKind::Pavilion => "pavilion",
            BuildKind::Workshop => "workshop",
            BuildKind::Millhouse => "millhouse",
            BuildKind::Monument => "monument",
        }
    }

    /// 由 `as_str()` 字串反查 BuildKind（從持久化的目標記錄還原）；未知回 None。
    pub fn from_str(s: &str) -> Option<BuildKind> {
        match s {
            "house" => Some(BuildKind::House),
            "well" => Some(BuildKind::Well),
            "tower" => Some(BuildKind::Tower),
            "garden" => Some(BuildKind::Garden),
            "pavilion" => Some(BuildKind::Pavilion),
            "workshop" => Some(BuildKind::Workshop),
            "millhouse" => Some(BuildKind::Millhouse),
            "monument" => Some(BuildKind::Monument),
            _ => None,
        }
    }
}

/// 依心願文字規則分類建物種類（零 LLM、確定性、可測）。
/// 先比對「長/更具體」關鍵詞，避免短詞提早截斷。
pub fn classify_desire(desire: &str) -> Option<BuildKind> {
    // 先比對長詞
    if desire.contains("瞭望")
        || desire.contains("觀星")
        || desire.contains("高台")
    {
        return Some(BuildKind::Tower);
    }
    if desire.contains("水井") || desire.contains("水池") || desire.contains("水源") {
        return Some(BuildKind::Well);
    }
    if desire.contains("花圃")
        || desire.contains("花園")
        || desire.contains("種花")
        || desire.contains("植物")
    {
        return Some(BuildKind::Garden);
    }
    if desire.contains("涼亭")
        || desire.contains("亭子")
        || desire.contains("乘涼")
        || desire.contains("歇腳")
        || desire.contains("避雨")
        || desire.contains("遮雨")
        || desire.contains("遮陽")
        || desire.contains("遮蔭")
        || desire.contains("遮蔽")
    {
        return Some(BuildKind::Pavilion);
    }
    // 工坊：做工／打鐵／手作的地方（比 House 更具體，須排在 House 前）。
    if desire.contains("工坊")
        || desire.contains("作坊")
        || desire.contains("工作坊")
        || desire.contains("工作室")
        || desire.contains("打鐵")
        || desire.contains("鐵匠")
        || desire.contains("鍛造")
        || desire.contains("手作")
    {
        return Some(BuildKind::Workshop);
    }
    // 磨坊：靠水的水車磨坊（磨坊／水車／水磨）。刻意**只認「水」字頭**的關鍵詞：
    // 磨坊本就「靠水而立」（見 `millhouse_site_ok`），語意上與「風車」（陸上、靠風）無關；
    // 更關鍵的是「風車」會撞既有地標名池（`voxel_structure_name` 的「風車丘」）——居民
    // 見賢思齊（858）把地標名嵌進心願文字時，「看到『風車丘』…我也好想擁有一座小木屋」
    // 這種其實想要 House／Monument 的心願會被「風車」搶去誤分成磨坊。故此處不收「風車」，
    // 讓每種建物的心願文字都能正確 round-trip 回自己的 kind。
    if desire.contains("磨坊")
        || desire.contains("水車")
        || desire.contains("水磨")
    {
        return Some(BuildKind::Millhouse);
    }
    // 紀念碑：立碑紀念的地標。
    if desire.contains("紀念")
        || desire.contains("石碑")
        || desire.contains("碑")
        || desire.contains("方尖")
        || desire.contains("地標")
    {
        return Some(BuildKind::Monument);
    }
    if desire.contains("小屋")
        || desire.contains("家")
        || desire.contains("房子")
        || desire.contains("房屋")
        || desire.contains("住")
        || desire.contains("屋")
    {
        return Some(BuildKind::House);
    }
    // 單字再比對
    if desire.contains("塔") {
        return Some(BuildKind::Tower);
    }
    if desire.contains("亭") {
        return Some(BuildKind::Pavilion);
    }
    if desire.contains("泉") || desire.contains("井") {
        return Some(BuildKind::Well);
    }
    if desire.contains("花") || desire.contains("草") || desire.contains("種") {
        return Some(BuildKind::Garden);
    }
    // 通用建造意圖 → House
    if desire.contains("蓋") || desire.contains("建") || desire.contains("造") {
        return Some(BuildKind::House);
    }
    None
}

/// 心願閉環 v1（ROADMAP 859）：剛完工的建物是否正是「目前這份心願」所指、且這份心願
/// 還沒被實現過（純函式、可測）。呼叫端用來判斷完工那一刻要不要對 `voxel_desires` 的
/// `DesireStore` 補呼叫 `mark_fulfilled`——此前 `mark_fulfilled` 只在「玩家送禮」路徑
/// （722）被呼叫過，居民自己蓋出建物讓心願成真時從未呼叫，導致這份心願在 store 裡
/// 永遠停留在「未實現」，進而卡死 771（自我印象驅動自發追尋：`vacant` 判定要求
/// `fulfilled==true` 或 `None`）——同一格心願一旦被歸類成某種建物、蓋完後也不會清空，
/// 這位居民就再也種不出新的自發渴望。
pub fn build_fulfills_desire(desire_text: &str, already_fulfilled: bool, completed: BuildKind) -> bool {
    !already_fulfilled && classify_desire(desire_text) == Some(completed)
}

/// 磨坊靠水 gating（純函式、可測）：磨坊必須挑對地點——錨點 (cx,cz) 周圍水平 2 格內
/// （Chebyshev 距離 ≤ 2）、地面層 cy 或其下一層 cy-1，任一格是水（來源／流動／溫泉皆算）
/// 才允許蓋磨坊。用注入的 `is_water` 述詞查方塊（呼叫端傳 `|x,y,z| block_at(x,y,z).is_any_water()`），
/// 讓本判定完全純粹、可離線測試（不碰真實世界方塊）。
///
/// 只有 `BuildKind::Millhouse` 受此 gating 約束；其餘 kind 一律回 true（不挑地點）。
/// 呼叫端在選定 kind／錨點後、`new_plan` 之前呼叫；不通過就換地點或改蓋別的。
pub fn millhouse_site_ok<F: Fn(i32, i32, i32) -> bool>(
    kind: BuildKind,
    cx: i32,
    cy: i32,
    cz: i32,
    is_water: F,
) -> bool {
    if kind != BuildKind::Millhouse {
        return true; // 只有磨坊挑地點
    }
    for dx in -2i32..=2 {
        for dz in -2i32..=2 {
            for dy in [-1i32, 0] {
                if is_water(cx + dx, cy + dy, cz + dz) {
                    return true;
                }
            }
        }
    }
    false
}

/// 一句自主禱告（`npc_pray` 產出）要不要提升成持久渴望（純函式、可測）。
///
/// ROADMAP 6「禱告驅動蓋家」：居民每隔一段時間會自主許願（不像玩家聊天的心願，
/// 每次禱告幾乎都會成功產出一句），若照玩家心願路徑無條件覆蓋，會讓多數只是抒發
/// 心情的禱告（如「好想有個慶典熱鬧一下」）頻繁蓋掉真正具體的建造心願——因此**只有
/// 這句禱告本身能分類出具體建物種類時**才值得提升成持久渴望，其餘仍只是浮現又消失
/// 的一句心願泡泡（不覆蓋、不落地）。
pub fn prayer_promotable(prayer: &str) -> bool {
    classify_desire(prayer).is_some()
}

// ── 建造計畫 ──────────────────────────────────────────────────────────────────

/// 單一待放方塊（世界絕對座標 + 型別）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildBlock {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// `Block as u8`（serde 直接存數字，向後相容）。
    pub b: u8,
}

/// 一位居民的建造計畫（jsonl 落地單位）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildPlan {
    pub resident: String,
    /// BuildKind::as_str()，字串供 serde 向後相容。
    pub kind: String,
    /// 建物顯示名（玩家可讀）。
    pub kind_name: String,
    /// 建物中心世界座標（生成計畫時釘死，不隨居民移動）。
    pub cx: i32,
    pub cy: i32,
    pub cz: i32,
    /// 剩餘待放方塊（依序；每放一塊 pop_front）。
    pub remaining: VecDeque<BuildBlock>,
    /// 總方塊數（供進度計算；remaining.len() ≤ total）。
    pub total: u32,
    /// 單調遞增序號（越大越新；from_entries 用來取最新一份計畫）。
    pub seq: u64,
    /// 這是否為「擴建」（基礎四種都蓋完後再蓋的第 2 座）而非首次建造。
    /// 完工時決定要呼叫 `GoalStore::mark_done` 還是 `mark_expansion`。
    /// `#[serde(default)]` 供舊 jsonl 向後相容（舊行沒有這欄，一律視為 `false`）。
    #[serde(default)]
    pub expansion: bool,
    /// 心願真的成真 v1（ROADMAP 720）：若這座建物是某位玩家的話種下的心願所驅動
    /// （非居民自我啟發、非擴建），記下啟發者的玩家顯示名，完工時指名感謝。
    /// `#[serde(default)]` 供舊 jsonl 向後相容（舊行沒有這欄，一律視為 `None`，維持原通用完工行為）。
    #[serde(default)]
    pub inspired_by: Option<String>,
    /// 合力蓋家 v1（ROADMAP 834）：老朋友到訪順手幫忙推進（696）時記下的協力者名字（去重）。
    /// `#[serde(default)]` 供舊 jsonl 向後相容（舊行沒有這欄，一律視為空、維持原單人完工行為）。
    #[serde(default)]
    pub helpers: Vec<String>,
    /// 蓋家真的用材料 v3（回應 review #1283 v2 退回：每塊都缺料/復工兩則洗版動態牆）：
    /// 正卡關的那一種材料方塊 id；`None` = 沒有進行中的材料困擾。用來讓缺料/復工/將就
    /// 的泡泡與動態牆整份計畫對同一種材料只各講一次（材料換了種類、或這一種終於湊到了
    /// 才會變化），不影響 `remaining` 本身。`#[serde(default)]` 供舊 jsonl 向後相容
    /// （這個功能還沒上過 prod，純粹保留一貫的相容寫法）。
    #[serde(default)]
    pub stall_material: Option<u8>,
}

impl BuildPlan {
    pub fn is_done(&self) -> bool {
        self.remaining.is_empty()
    }

    /// 記一位協力者（去重、排除屋主本人）。合力蓋家 v1：讓完工功勞不再只算一人。
    pub fn add_helper(&mut self, name: &str) {
        if self.resident != name && !self.helpers.iter().any(|h| h == name) {
            self.helpers.push(name.to_string());
        }
    }

    /// 完成百分比 0..=100。
    pub fn progress_pct(&self) -> u32 {
        if self.total == 0 {
            return 100;
        }
        let placed = self.total.saturating_sub(self.remaining.len() as u32);
        placed * 100 / self.total
    }

    /// 取出下一個待放方塊（就地修改 remaining）。
    pub fn pop_next(&mut self) -> Option<BuildBlock> {
        self.remaining.pop_front()
    }

    /// 窺視下一個待放方塊，不取出（蓋家真的用材料 v1：放之前先看看要不要扣料）。
    pub fn peek_next(&self) -> Option<&BuildBlock> {
        self.remaining.front()
    }

    /// 把方塊放回最前面（撤銷剛才的 `pop_next`）：蓋家真的用材料 v3——判定材料不夠、
    /// 這塊這次不放時用，讓她下個 tick 還是先處理同一塊，不會憑空多算或漏算一塊
    /// （即使中途被別位居民互助蓋家插了一手，`remaining` 也不會壞帳，見模組檔頭鎖序說明）。
    pub fn push_front(&mut self, bb: BuildBlock) {
        self.remaining.push_front(bb);
    }

    /// 這份計畫剩下（含即將處理的這塊）還要幾塊某種材料——供批次備料抓「一次備多少」
    /// 的量，別讓她一塊只採一趟（蓋家真的用材料 v3，回應 review #1283 v2「動態牆洗版」退回）。
    pub fn count_material(&self, block_id: u8) -> u32 {
        self.remaining.iter().filter(|bb| bb.b == block_id).count() as u32
    }
}

/// 所有居民的建造計畫（每人至多一份 active plan）。
#[derive(Debug, Default, PartialEq)]
pub struct BuildStore {
    pub plans: HashMap<String, BuildPlan>,
    next_seq: u64,
}

impl BuildStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_plan(&self, resident: &str) -> bool {
        self.plans.contains_key(resident)
    }

    pub fn get_plan_mut(&mut self, resident: &str) -> Option<&mut BuildPlan> {
        self.plans.get_mut(resident)
    }

    /// ROADMAP 699：玩家協助居民蓋家。若玩家剛放的方塊（世界座標 + 類型）正好等於
    /// 某位居民建造計畫「下一塊待放」，判定為玩家幫了忙——彈掉該塊（居民之後 tick
    /// 不會重放這塊，接著蓋下一塊），回傳 `(resident_id, kind_name)` 供呼叫端道謝。
    /// 找不到符合的計畫回 `None`（多數放置與任何居民無關，屬正常情形）。
    pub fn try_player_help(&mut self, x: i32, y: i32, z: i32, b: u8) -> Option<(String, String)> {
        for (rid, plan) in self.plans.iter_mut() {
            let is_match = plan
                .remaining
                .front()
                .map_or(false, |front| front.x == x && front.y == y && front.z == z && front.b == b);
            if is_match {
                plan.pop_next();
                return Some((rid.clone(), plan.kind_name.clone()));
            }
        }
        None
    }

    /// 新建並插入計畫；回傳 clone 供呼叫端落地 jsonl。
    /// `expansion`：是否為擴建（基礎四種都蓋完後再蓋的第 2 座），影響完工時記錄方式。
    /// `inspired_by`：若這座建物是某位玩家的話種下的心願所驅動，記下玩家名（完工時指名感謝）。
    pub fn new_plan(
        &mut self,
        resident: &str,
        kind: BuildKind,
        cx: i32,
        cy: i32,
        cz: i32,
        expansion: bool,
        inspired_by: Option<String>,
    ) -> BuildPlan {
        // 建築風格依「居民 + 群系（由錨點座標查）+ 錨點座標」確定性決定，讓每間都不同。
        let biome = biome_at_voxel(cx, cz);
        let style = BuildStyle::for_resident(resident, biome, cx, cz);
        let blocks = generate_blocks(kind, cx, cy, cz, &style);
        let total = blocks.len() as u32;
        let plan = BuildPlan {
            resident: resident.to_string(),
            kind: kind.as_str().to_string(),
            kind_name: kind.display_name().to_string(),
            cx,
            cy,
            cz,
            remaining: blocks.into(),
            total,
            seq: self.next_seq,
            expansion,
            inspired_by,
            helpers: Vec::new(),
            stall_material: None,
        };
        self.next_seq += 1;
        self.plans.insert(resident.to_string(), plan.clone());
        plan
    }

    /// 若某居民計畫已完成（remaining 空），移除之。
    pub fn remove_if_done(&mut self, resident: &str) {
        if self.plans.get(resident).map_or(false, |p| p.is_done()) {
            self.plans.remove(resident);
        }
    }

    /// 從 jsonl 記錄還原（重啟後繼續未完成的建造）。**last-wins 語意**：一張計畫在 jsonl 裡
    /// 是「同 seq、`remaining` 逐塊遞減」的一長串 append，同居民最新的那一行才是真狀態。
    ///
    /// **churn 根治（prod 真 bug）**：舊版用 `e.seq > existing.seq`（嚴格大於）當去重閘——
    /// 對同一張計畫的多行（seq 相等）永遠留住**最先**掃到的那行（`remaining` 滿），
    /// 後面把它放到 `remaining=0`（done）的行因 `seq 不 > seq` 而**永遠蓋不掉它**。結果重啟後
    /// 一張早已蓋完的計畫被還原成「全新未蓋」→ 重蓋 → 完工落一筆 `anchor_only` 目標 → 再重啟
    /// 再重蓋……`voxel_goals.jsonl` 無界膨脹，且這張幽靈計畫讓 `has_plan` 恆真、卡死殖民者補蓋
    /// （露娜的家兩小時零進展的真兇）。改成 last-wins：同 seq 的後行覆蓋前行，且 done 的最新行
    /// **移除**先前留住的計畫——還原的永遠是「這位居民最後留下的真實狀態」：沒蓋完就續蓋、
    /// 蓋完了就沒計畫。對「同居民多張不同 seq 計畫」仍取最新（seq 較大者）。
    pub fn from_entries(entries: Vec<BuildPlan>) -> Self {
        let mut s = Self::default();
        // 記住每位居民目前依據的計畫序號（含 done 移除後的序號），確保 last-wins 不倒退。
        let mut kept_seq: HashMap<String, u64> = HashMap::new();
        for e in entries {
            if e.seq >= s.next_seq {
                s.next_seq = e.seq.wrapping_add(1);
            }
            // last-wins：seq ≥ 已記錄序號即以它為準（同 seq 後行覆蓋前行、新 seq 計畫覆蓋舊計畫）；
            // 比目前記錄還舊的行（seq 較小）忽略，避免亂序 jsonl 讓舊計畫復活。
            let is_newer = kept_seq.get(&e.resident).map_or(true, |&k| e.seq >= k);
            if !is_newer {
                continue;
            }
            kept_seq.insert(e.resident.clone(), e.seq);
            if e.is_done() {
                // 這位居民最新狀態是「蓋完了」→ 沒有進行中計畫（移除先前留下的那張）。
                s.plans.remove(&e.resident);
            } else {
                s.plans.insert(e.resident.clone(), e);
            }
        }
        s
    }
}

// ── 地表 y 計算 ────────────────────────────────────────────────────────────────

/// 在 (wx, wz) 找程序生成地形的表面 y（最高固體方塊的正上方一格）。
/// 掃描範圍 BASE_HEIGHT ± 12，足以涵蓋正常起伏地形；找不到回安全保底值。
pub fn surface_y(wx: i32, wz: i32) -> i32 {
    let hi = BASE_HEIGHT + 8;
    let lo = BASE_HEIGHT - 8;
    for y in (lo..=hi).rev() {
        if block_at(wx, y, wz).is_solid() {
            return y + 1; // 地面正上方（站立位置）
        }
    }
    BASE_HEIGHT + 1 // 保底（不應觸及）
}

/// 依居民 index（0..4）決定建造錨點偏移方向，讓 4 位居民朝不同方位蓋。
/// 偏移距離固定 6 方塊，確保不與出生點重疊。
pub fn build_anchor_offset(resident_idx: usize) -> (i32, i32) {
    match resident_idx % 4 {
        0 => (6, 0),
        1 => (0, 6),
        2 => (-6, 0),
        _ => (0, -6),
    }
}

// ── 建築風格：讓同種建物「依居民/群系」各有不同（純函式，確定性，可測）─────────
//
// 建築創作第一刀：此前 `generate_blocks` 對同一 `BuildKind` 永遠吐出一模一樣的方盒——
// 誰蓋幾次都是複製貼上。本層把「牆材質／屋頂形狀材質／尺寸／裝飾」抽成一份由
// 「**誰蓋的（居民 id）＋在哪個群系＋錨點座標**」確定性決定的 [`BuildStyle`]，
// 讓露娜的木屋、諾娃的石頂屋、沙漠居民的沙屋一眼看得出是不同人蓋的不同房子。
//
// **確定性鐵律**：同居民同錨點永遠算出同一份風格（可測、重啟一致、不會這次木下次石）；
// 風格在 `new_plan` 建計畫時算一次、烘進 `remaining` 方塊清單（jsonl 落地的是實際方塊，
// 之後重啟直接 replay，不重算風格）→ 與 `try_player_help` 的逐塊比對完全相容。

/// 門口點綴（每家不同的小細節；皆放在正面外側一格，不動地基/不與牆體重疊）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decor {
    /// 無點綴（素樸）。
    None,
    /// 門口一盞火把（暖光）。
    Torch,
    /// 門口一畦花圃（草地 + 葉片）。
    Flowerbed,
    /// 門口一根柱（兩格高，牆材質）。
    Pillar,
}

/// 附屬小棚（annex）貼在主屋的哪一側——建築創作第三刀：不只「有沒有」小棚，
/// 連小棚長在哪一側都因居民而異，同一種房子第一次有不只一種輪廓變化。
/// 三側皆貼在「背牆／側牆外一格」的後半部，數學上必與正面的門（z_max 側）分居兩端，
/// 且左右兩側的 x 座標永遠落在主屋佔地之外，恆不與主屋本體重疊。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnnexPos {
    /// 貼在背牆（z_min 側）外一排——建築創作第二刀的原始位置。
    Back,
    /// 貼在左牆（x_min 側）外一排。
    Left,
    /// 貼在右牆（x_max 側）外一排。
    Right,
}

/// 一座建物的樣式（由居民/群系/座標確定性決定）。純資料、無 IO。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildStyle {
    /// 牆體主建材（依群系：森林木、沙漠沙、雪原拋石/雪…）。
    pub wall: Block,
    /// 屋頂建材（石/木/葉/拋石）。
    pub roof: Block,
    /// 地板建材（由牆材質衍生）。
    pub floor: Block,
    /// 尖頂：平頂上再疊一層縮小的方塊，成斜頂感。
    pub peaked: bool,
    /// 側牆是否開玻璃窗。
    pub windows: bool,
    /// 牆高（層數，2 或 3）。
    pub wall_h: i32,
    /// 佔地範圍（相對中心，含界）；x_min/z_min 固定 -1，x_max/z_max 由尺寸決定。
    pub x_max: i32,
    pub z_max: i32,
    /// 門口點綴。
    pub decor: Decor,
    /// 建築創作第二刀：房子是否多長出一間小棚（annex）——第一次讓「同一種房子」
    /// 有不同的**形狀輪廓**，不只是換色/換尺寸（仍是既有零件：地板/牆/屋頂三種既有
    /// 建材再組一次，非新方塊種類）。
    pub annex: bool,
    /// 建築創作第三刀：annex 貼在主屋的哪一側，只有 `annex` 為真時才有意義。
    pub annex_pos: AnnexPos,
    /// 居家 v2·隔間 v1（舊欄位保留）：多層大宅生成器（見下）已讓每座家一律有房間概念，
    /// 此旗如今僅供 [`house_replay_layouts_at`] 辨識「隔間上線前的舊屋版型」時據以是否
    /// 補一份 legacy 候選（見該函式）；大宅結構本身不再讀它。
    pub room_split: bool,
    /// 多層大宅生成器（ROADMAP·把 3×3 方盒重寫成有房間概念的多層大宅）：地面以上樓層數
    /// （1~3，每層淨高 3 格）。與 `width`/`depth`/`basement` 一起把家撐成多層多房間大宅。
    pub stories: i32,
    /// 多層大宅生成器：是否在地基下挖一層地下酒窖（儲藏室）。3 層大宅不再加（控制方塊數上限）。
    pub basement: bool,
}

impl BuildStyle {
    pub const X_MIN: i32 = -1;
    pub const Z_MIN: i32 = -1;

    /// 依「居民 id + 群系 + 錨點座標」確定性推導一份樣式（同輸入永遠同輸出）。
    pub fn for_resident(resident: &str, biome: VoxelBiome, cx: i32, cz: i32) -> BuildStyle {
        let h = style_hash(resident, cx, cz);
        let wall = wall_palette(biome)[(h & 1) as usize];
        let roof = ROOF_PALETTE[((h >> 1) & 0b11) as usize];
        let peaked = (h >> 3) & 1 == 1;
        let windows = (h >> 4) & 1 == 1;
        let wall_h = 2 + ((h >> 5) & 1) as i32; // 舊欄位保留（大宅結構不讀，見 push_house）
        // decor / annex / annex_pos / room_split 皆為舊欄位：大宅結構不再讀它們，但
        // `house_replay_layouts_at` 仍靠 room_split（及 bits 6-7 重算的舊 footprint）辨識
        // 「隔間上線前的舊屋」候選，故照原 bit 保留計算、維持既有 prod 維修行為不回歸。
        let decor = match (h >> 8) & 0b11 {
            0 => Decor::None,
            1 => Decor::Torch,
            2 => Decor::Flowerbed,
            _ => Decor::Pillar,
        };
        let annex = (h >> 10) & 1 == 1;
        let annex_pos = match (h >> 11) & 0b11 {
            0 | 1 => AnnexPos::Back,
            2 => AnnexPos::Left,
            _ => AnnexPos::Right,
        };
        let room_split = (h >> 13) & 1 == 1;
        // ── 多層大宅維度（ROADMAP·多層大宅生成器）───────────────────────────────
        // 佔地一律拉大到 5~6 見方（比舊 3×3~4×4 方盒大得多，容得下多房間＋樓梯井）。
        // 佔地以 x0 = cx + X_MIN（= cx-1）起算，故 x_max = width-2 → footprint 恰為 width 格寬，
        // 與院牆/夜間補圈等消費端沿用的「min = 錨點-1、max = 錨點+x_max」footprint 慣例一致。
        let width = 5 + ((h >> 14) & 1) as i32; // 5 或 6
        let depth = 5 + ((h >> 15) & 1) as i32; // 5 或 6
        // 樓層：5×5 見方可達 1~3 層（1:2:3 比約 1:2:1，偶爾一層小巧、偶爾三層高聳）；
        // 較大佔地（6 見方）一律 2 層——多層為主，且避免 6 見方 ×3 層逼近上千塊。
        let stories = if width == 5 && depth == 5 {
            match (h >> 16) & 0b11 {
                0 => 1,
                3 => 3,
                _ => 2,
            }
        } else {
            2
        };
        let x_max = width - 2;
        let z_max = depth - 2;
        // 地下酒窖：只給 5×5 且非 3 層的家（避免 6 見方 + 多層 + 酒窖逼近上千塊、蓋數小時）。
        let basement = (h >> 18) & 1 == 1 && stories < 3 && width == 5 && depth == 5;
        // 地板由牆材質衍生（木系→木板、沙→沙、其餘→拋石），保持質感一致。
        let floor = match wall {
            Block::Wood | Block::Plank => Block::Plank,
            Block::Sand => Block::Sand,
            _ => Block::SmoothStone,
        };
        BuildStyle {
            wall, roof, floor, peaked, windows, wall_h, x_max, z_max, decor, annex, annex_pos,
            room_split, stories, basement,
        }
    }
}

/// 依「居民 id + 錨點座標」算出的穩定 64-bit 雜湊（FNV-1a）。
/// 純函式：居民 id 與座標都穩定 → 同居民同錨點永遠同雜湊（重啟一致）。
fn style_hash(resident: &str, cx: i32, cz: i32) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
    let mut mix = |b: u8| {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
    };
    for b in resident.bytes() {
        mix(b);
    }
    for v in [cx, cz] {
        for b in v.to_le_bytes() {
            mix(b);
        }
    }
    // 雪崩混合（splitmix64 finalizer）：讓各 bit 去相關——否則相近輸入（如 vox_res_0/1/2
    // 只差最後一個位元組）的低位 bit 容易撞在一起，害「牆材質」等只吃 1 bit 的維度失去變化。
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    h
}

/// 群系決定牆材質的兩個候選（再由雜湊選一，讓同群系兩位居民仍可能不同）。
fn wall_palette(biome: VoxelBiome) -> [Block; 2] {
    match biome {
        VoxelBiome::Forest => [Block::Wood, Block::Plank],
        VoxelBiome::Grassland => [Block::Wood, Block::StoneBrick],
        VoxelBiome::Desert => [Block::Sand, Block::SmoothStone],
        VoxelBiome::Snow => [Block::SmoothStone, Block::Snow],
    }
}

/// 屋頂材質候選（4 選 1）。
const ROOF_PALETTE: [Block; 4] = [Block::Stone, Block::Wood, Block::Leaves, Block::SmoothStone];

// ── 多層大宅生成器（ROADMAP·把 3×3 方盒重寫成有房間概念的多層大宅）──────────────

/// 生成一座「多層多房間大宅」的方塊清單並推入 `out`（純函式、確定性、可測）。
///
/// 取代舊的 3×3 單間方盒。佈局全由 `style` 確定性決定（同輸入永遠生出**逐塊相同**的清單，
/// 與 replay／玩家逐塊幫忙 `try_player_help` 完全相容）：
/// - **佔地**：footprint = `[cx+X_MIN ..= cx+x_max] × [cz+Z_MIN ..= cz+z_max]`
///   （`for_resident` 給 5~6 見方）。x0 固定在 `cx-1`，故 footprint 恰為 `width` 格寬，
///   與院牆/夜間補圈等消費端沿用的「min = 錨點-1、max = 錨點+x_max」慣例一致。
///   （`house_replay_layouts_at` 辨識隔間上線前舊屋時會傳入更小的 legacy footprint；本函式
///   對 3×3 起的任何尺寸都能生出合法不重疊、且含門與床的房子。）
/// - **樓層**：`stories` 層（每層淨高 3 格）＋可選地下酒窖 `basement`（地基下挖空一層儲藏室）。
/// - **樓梯**：一條 `Ladder`（非實心、可攀爬）垂直樓梯井貫穿各層，樓板在該欄位穿孔，真能上下走。
/// - **隔間**：夠大（≥5 見方）時每層以「內牆＋門(DoorClosed)」隔成前後兩間房；地面層＝起居
///   （桌/地毯）＋廚房角（爐）、樓上＝臥室（床/床頭櫃）、地下室＝儲藏（箱）。
/// - **窗**：每層兩側牆中段開玻璃窗採光（`windows` 風格再於背牆多開一扇）。
/// - **屋頂**：實心平頂，`peaked` 時再疊一圈縮小的脊成斜尖頂。
/// - **大門**：正面（z1 側）牆中央、地面層下兩層一扇木門。
fn push_house(out: &mut Vec<BuildBlock>, cx: i32, cy: i32, cz: i32, style: &BuildStyle) {
    let add = |out: &mut Vec<BuildBlock>, x: i32, y: i32, z: i32, b: Block| {
        out.push(BuildBlock { x, y, z, b: b as u8 });
    };
    let wall = style.wall;
    let roof = style.roof;
    let floor = style.floor;

    // 佔地（footprint）：x0..=x1 × z0..=z1。x0/z0 固定在錨點 -1（沿用 X_MIN/Z_MIN 慣例）。
    let x0 = cx + BuildStyle::X_MIN;
    let x1 = cx + style.x_max;
    let z0 = cz + BuildStyle::Z_MIN;
    let z1 = cz + style.z_max;
    let stories = style.stories.clamp(1, 3);

    // 夠大到值得隔間／地下室／獨立樓梯井嗎（內部至少 3×3、即寬深皆 ≥5）？
    let grand = (x1 - x0) >= 4 && (z1 - z0) >= 4;
    let basement = style.basement && grand;

    // 大門 x：對齊錨點欄位 cx（恆為正面牆的內部欄位、非轉角）——與院牆南面留的門口
    // 缺口（在 x=錨點）對齊，居民出門正對缺口不被圍籬擋住；也沿用舊屋門在 cx 的慣例。
    // 隔間內牆 z：footprint 中線，把每層分前後兩室。
    let door_x = cx;
    let mid_z = (z0 + z1) / 2;

    // 第 k 層可站立內部底面 y：sb(k)=cy+4k（牆 sb..sb+2）；其可走樓板 slab_y(k)=cy-1+4k。
    let story_base = |k: i32| cy + 4 * k;
    let slab_y = |k: i32| cy - 1 + 4 * k;
    let roof_y = slab_y(stories); // 頂層樓板之上即屋頂

    // 樓梯井欄位（lx,lz）：大宅放後排靠右內部欄位（恆非邊界、非隔間線、非大門、非床）；
    // 小屋退回「內部任一非床欄位」，找不到（3×3 內部只有錨點一格）就不設樓梯井。
    let bed_ground = (cx, cz); // 小屋的床落在錨點（恆為內部）
    let (lx, lz, has_shaft) = if grand {
        (x1 - 1, z0 + 1, stories >= 2 || basement)
    } else {
        let mut pick = None;
        'outer: for x in (x0 + 1)..=(x1 - 1) {
            for z in (z0 + 1)..=(z1 - 1) {
                if (x, z) != bed_ground {
                    pick = Some((x, z));
                    break 'outer;
                }
            }
        }
        match pick {
            Some((x, z)) => (x, z, stories >= 2),
            None => (cx, cz, false),
        }
    };
    // 樓梯井上下範圍：有地下室從 cy-4 起，否則從地面層站立面 cy 起；上抵頂層樓板。
    let shaft_bottom = if basement { cy - 4 } else { cy };
    let shaft_top = slab_y(stories - 1);
    // 某層樓板是否被樓梯井穿孔（井經過它）。
    let slab_holed = |k: i32| has_shaft && slab_y(k) >= shaft_bottom && slab_y(k) <= shaft_top;

    // ── 地下酒窖：挖空一間儲藏室（四壁＝天然泥石，天花＝地面層樓板），擺箱與火把 ──────
    let cellar_chest = (x0 + 1, cy - 4, z0 + 1);
    let cellar_torch = (x0 + 1, cy - 2, z1 - 1);
    if basement {
        for x in (x0 + 1)..=(x1 - 1) {
            for z in (z0 + 1)..=(z1 - 1) {
                for y in (cy - 4)..=(cy - 2) {
                    if x == lx && z == lz {
                        continue; // 井位稍後統一鋪梯
                    }
                    let b = if (x, y, z) == cellar_chest {
                        Block::Chest
                    } else if (x, y, z) == cellar_torch {
                        Block::Torch
                    } else {
                        Block::Air // 挖空成房（天然實心 → 空氣）
                    };
                    add(out, x, y, z, b);
                }
            }
        }
    }

    // ── 各層樓板（含地面層；樓梯井欄位穿孔）──────────────────────────────────────
    for k in 0..stories {
        let y = slab_y(k);
        let holed = slab_holed(k);
        for x in x0..=x1 {
            for z in z0..=z1 {
                if holed && x == lx && z == lz {
                    continue;
                }
                add(out, x, y, z, floor);
            }
        }
    }

    // ── 牆（每層 3 格高、只邊框）＋大門＋採光窗 ─────────────────────────────────
    for k in 0..stories {
        let sb = story_base(k);
        for dy in 0..3 {
            let y = sb + dy;
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if x != x0 && x != x1 && z != z0 && z != z1 {
                        continue; // 只邊框
                    }
                    // 正面大門（地面層中央下兩層）。
                    if k == 0 && x == door_x && z == z1 && dy < 2 {
                        add(out, x, y, z, Block::DoorClosed);
                        continue;
                    }
                    // 側牆採光窗（每層兩側牆中段）。
                    if dy == 1 && (x == x0 || x == x1) && z == mid_z {
                        add(out, x, y, z, Block::Glass);
                        continue;
                    }
                    // 背牆窗（開窗風格多一扇）。
                    if style.windows && dy == 1 && z == z0 && x == door_x {
                        add(out, x, y, z, Block::Glass);
                        continue;
                    }
                    add(out, x, y, z, wall);
                }
            }
        }
    }

    // ── 屋頂（實心平頂 + 可選斜尖頂）────────────────────────────────────────────
    for x in x0..=x1 {
        for z in z0..=z1 {
            add(out, x, roof_y, z, roof);
        }
    }
    if style.peaked {
        for x in (x0 + 1)..=(x1 - 1) {
            for z in (z0 + 1)..=(z1 - 1) {
                add(out, x, roof_y + 1, z, roof);
            }
        }
    }

    // ── 隔間內牆＋門（每層分前後兩室；大宅才做）─────────────────────────────────
    if grand {
        for k in 0..stories {
            let sb = story_base(k);
            for dy in 0..3 {
                let y = sb + dy;
                for x in (x0 + 1)..=(x1 - 1) {
                    if x == door_x {
                        // 房間之間留門：下層一扇門、中層通行留空、上層過梁。
                        if dy == 0 {
                            add(out, x, y, mid_z, Block::DoorClosed);
                        } else if dy == 2 {
                            add(out, x, y, mid_z, wall);
                        }
                    } else {
                        add(out, x, y, mid_z, wall);
                    }
                }
            }
        }
    }

    // ── 樓梯井（Ladder 貫穿，含地下室段）────────────────────────────────────────
    if has_shaft {
        for y in shaft_bottom..=shaft_top {
            add(out, lx, y, lz, Block::Ladder);
        }
    }

    // ── 傢俱（住人的痕跡）────────────────────────────────────────────────────────
    if grand {
        // 房間格清單（排除樓梯井格），供傢俱依序落位、保證不重疊、不越界、不擋門。
        let room_cells = |z_lo: i32, z_hi: i32| -> Vec<(i32, i32)> {
            let mut v = Vec::new();
            for x in (x0 + 1)..=(x1 - 1) {
                for z in z_lo..=z_hi {
                    if (x, z) != (lx, lz) {
                        v.push((x, z));
                    }
                }
            }
            v
        };
        let front = room_cells(mid_z + 1, z1 - 1); // 前室（近正門那半）
        let back = room_cells(z0 + 1, mid_z - 1); // 後室（樓梯井那半）
        let place = |out: &mut Vec<BuildBlock>, cells: &[(i32, i32)], y: i32, items: &[Block]| {
            for (i, b) in items.iter().enumerate() {
                if let Some(&(x, z)) = cells.get(i) {
                    add(out, x, y, z, *b);
                }
            }
        };
        if stories == 1 {
            // 單層大宅：前室兼臥室（床+桌+地毯）、後室廚房（爐+花盆）。
            place(out, &front, cy, &[Block::Bed, Block::Table, Block::Carpet]);
            place(out, &back, cy, &[Block::Furnace, Block::FlowerPot]);
        } else {
            // 地面層起居（桌+地毯）＋廚房角（爐）。
            place(out, &front, cy, &[Block::Table, Block::Carpet]);
            place(out, &back, cy, &[Block::Furnace]);
            // 樓上每層：前室臥室（床+床頭櫃）、後室書房（地毯+花盆）。
            for k in 1..stories {
                let sb = story_base(k);
                place(out, &front, sb, &[Block::Bed, Block::Table]);
                place(out, &back, sb, &[Block::Carpet, Block::FlowerPot]);
            }
        }
    } else {
        // 小屋（legacy footprint）：一張床落在錨點（恆為內部）。
        add(out, bed_ground.0, cy, bed_ground.1, Block::Bed);
    }
}

// ── 建物方塊生成（純函式，可測）────────────────────────────────────────────────

/// 生成建物的方塊清單（從底層往上，讓 tick 逐塊放置時玩家看到「由下往上長出」）。
/// `style`：由 [`BuildStyle::for_resident`] 依居民/群系確定性決定，讓每間都不同。
fn generate_blocks(kind: BuildKind, cx: i32, cy: i32, cz: i32, style: &BuildStyle) -> Vec<BuildBlock> {
    let mut out = Vec::new();

    let add = |out: &mut Vec<BuildBlock>, x: i32, y: i32, z: i32, b: Block| {
        out.push(BuildBlock { x, y, z, b: b as u8 });
    };

    match kind {
        BuildKind::House => {
            // 多層大宅生成器（ROADMAP·把 3×3 方盒重寫成有房間概念的多層大宅）：整段佈局
            // 抽到 `push_house`（純函式、確定性），依 style 生出多層多房間、樓梯貫通、
            // 可含地下酒窖、室內有傢俱的大宅。
            push_house(&mut out, cx, cy, cz, style);
        }

        BuildKind::Well => {
            // 底圈（cy-1 層，3×3 Stone 外框，中心空）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    if dx.abs() == 1 || dz.abs() == 1 {
                        add(&mut out, cx + dx, cy - 1, cz + dz, Block::Stone);
                    }
                }
            }
            // 水（中心 cy-1）
            add(&mut out, cx, cy - 1, cz, Block::Water);
            // 井壁（cy 層，同外框）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    if dx.abs() == 1 || dz.abs() == 1 {
                        add(&mut out, cx + dx, cy, cz + dz, Block::Stone);
                    }
                }
            }
            // 四角頂柱（井架感）——材質依居民/群系（style.wall），讓各家水井的井架不同。
            for &(dx, dz) in &[(-1i32, -1i32), (-1, 1), (1, -1), (1, 1)] {
                add(&mut out, cx + dx, cy + 1, cz + dz, style.wall);
            }
            // 共 8 + 1 + 8 + 4 = 21 塊
        }

        BuildKind::Tower => {
            // 塔身材質依居民/群系（style.wall），塔頂依 style.roof；地基維持 Stone（穩固）。
            // 塔身高 4 或 5 層（style.peaked 再拔高一層），讓各家瞭望台高矮不同。
            let body_h = if style.peaked { 5 } else { 4 };
            // 地基（cy-1 層，3×3 Stone 實心）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy - 1, cz + dz, Block::Stone);
                }
            }
            // 塔身（只邊框，style.wall，中心可穿行）
            for layer in 0..body_h {
                let y = cy + layer;
                for dx in -1i32..=1 {
                    for dz in -1i32..=1 {
                        if dx.abs() == 1 || dz.abs() == 1 {
                            add(&mut out, cx + dx, y, cz + dz, style.wall);
                        }
                    }
                }
            }
            // 瞭望台頂（cy+body_h 層，3×3 style.roof 實心）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy + body_h, cz + dz, style.roof);
                }
            }
            // 共 9 + 8×body_h + 9 塊
        }

        BuildKind::Garden => {
            // 花木中心：多數為 Leaves，開窗風格（style.windows）者改種樹苗，增添變化。
            let center = if style.windows { Block::Sapling } else { Block::Leaves };
            // 草地底（cy-1 層，3×3 Grass）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy - 1, cz + dz, Block::Grass);
                }
            }
            // 花壇邊框（cy 層，3×3 外框，材質依居民/群系 style.wall）
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    if dx.abs() == 1 || dz.abs() == 1 {
                        add(&mut out, cx + dx, cy, cz + dz, style.wall);
                    }
                }
            }
            // 中心裝飾（象徵花木）
            add(&mut out, cx, cy, cz, center);
            // 共 9 + 8 + 1 = 18 塊
        }

        BuildKind::Pavilion => {
            // 涼亭：四柱撐頂、四面通透。地板 style.floor、角柱 style.wall、頂蓋 style.roof，
            // 材質隨居民/群系變化（讓每座亭不同）；中心一盞燈給夜裡歇腳的暖意。
            // 亭身高 3 或 4 層（style.peaked 再拔高一層、頂上加尖脊）。
            let body_h = if style.peaked { 4 } else { 3 };
            // 地板（cy-1 層，3×3 style.floor 實心——抬出一方乾淨的歇腳台）。
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, cy - 1, cz + dz, style.floor);
                }
            }
            // 四角立柱（只四角、四面全開通透——與封牆的小木屋刻意區隔）。
            for layer in 0..body_h {
                let y = cy + layer;
                for &(dx, dz) in &[(-1i32, -1i32), (-1, 1), (1, -1), (1, 1)] {
                    add(&mut out, cx + dx, y, cz + dz, style.wall);
                }
            }
            // 頂蓋（cy+body_h 層，3×3 style.roof 實心，遮陽避雨）。
            let roof_y = cy + body_h;
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    add(&mut out, cx + dx, roof_y, cz + dz, style.roof);
                }
            }
            // 尖頂：中心再疊一塊（小尖脊感）。
            if style.peaked {
                add(&mut out, cx, roof_y + 1, cz, style.roof);
            }
            // 中心一盞燈（站立層 cy，火把——夜裡歇腳的暖光，也讓亭子在遠處認得出）。
            add(&mut out, cx, cy, cz, Block::Torch);
            // 共 9 + 4×body_h + 9 + 1（+1 尖頂）塊
        }

        BuildKind::Workshop => {
            // 工坊：比小木屋更大的 5×4 做工空間——三面封牆＋正面全敞（做工要透氣、進料方便），
            // 內部一角擺工作台與熔爐（皆既有方塊，零美術）。材質依居民/群系 style，牆高固定 3 層。
            // 佔地相對錨點：x ∈ [-2,2]（寬 5）、z ∈ [-1,2]（深 4）；正面 = z_max（最大 z）敞口。
            let (x0, x1) = (-2i32, 2i32);
            let (z0, z1) = (-1i32, 2i32);
            let wall_h = 3i32;
            let front_z = z1; // 正面敞口那一側
            // 地板（cy-1，實心填滿，style.floor）。
            for x in x0..=x1 {
                for z in z0..=z1 {
                    add(&mut out, cx + x, cy - 1, cz + z, style.floor);
                }
            }
            // 牆（三面封、正面敞口）：邊框才砌，且正面那一整排（z==front_z）不砌牆——敞口。
            for layer in 0..wall_h {
                let y = cy + layer;
                for x in x0..=x1 {
                    for z in z0..=z1 {
                        let border = x == x0 || x == x1 || z == z0 || z == z1;
                        if !border || z == front_z {
                            continue; // 非邊框、或正面敞口那排 → 不砌
                        }
                        // 側牆中點開窗（第 1 層、z 在中間、x 為左右牆），透光。
                        if style.windows && layer == 1 && z == 0 && (x == x0 || x == x1) {
                            add(&mut out, cx + x, y, cz + z, Block::Glass);
                            continue;
                        }
                        add(&mut out, cx + x, y, cz + z, style.wall);
                    }
                }
            }
            // 屋頂（cy+wall_h，實心填滿，style.roof）。
            let roof_y = cy + wall_h;
            for x in x0..=x1 {
                for z in z0..=z1 {
                    add(&mut out, cx + x, roof_y, cz + z, style.roof);
                }
            }
            // 做工設備（站立層 cy，內部後排——恆落在牆內、不與牆重疊、不擋正面敞口、彼此不重疊）：
            // 內部 x ∈ {-1,0,1}、z ∈ {0,1}（邊框才砌牆，內部這兩排是空的）；取內部後排 z==0
            // （最深、遠離正面敞口 z==2）的左右兩端 x=-1 / x=1 擺工作台與熔爐。
            add(&mut out, cx - 1, cy, cz, Block::Workbench);
            add(&mut out, cx + 1, cy, cz, Block::Furnace);
        }

        BuildKind::Millhouse => {
            // 磨坊：石基木架撐起一座「水輪」造型（皆既有方塊：石／木／拋光石），靠水而立。
            // 錨點自身是磨坊主體，水輪貼在正面（+z 側）向外一格垂直立起——刻意伸出主體之外，
            // 象徵「浸在旁邊水裡轉動」。3×3 主體 + 水輪，材質部分依 style 讓各座略有不同。
            let (x0, x1) = (-1i32, 1i32);
            let (z0, z1) = (-1i32, 1i32);
            let wall_h = 3i32;
            // 石基（cy-1，3×3 Stone 實心，穩固扎在水邊）。
            for dx in x0..=x1 {
                for dz in z0..=z1 {
                    add(&mut out, cx + dx, cy - 1, cz + dz, Block::Stone);
                }
            }
            // 磨坊主體牆（邊框，木架感 → 用 style.wall；正面中央下兩層留門洞可進出）。
            let front_z = z1;
            for layer in 0..wall_h {
                let y = cy + layer;
                for dx in x0..=x1 {
                    for dz in z0..=z1 {
                        let border = dx == x0 || dx == x1 || dz == z0 || dz == z1;
                        if !border {
                            continue;
                        }
                        if dx == 0 && dz == front_z && layer < 2 {
                            add(&mut out, cx + dx, y, cz + dz, Block::DoorClosed);
                            continue;
                        }
                        add(&mut out, cx + dx, y, cz + dz, style.wall);
                    }
                }
            }
            // 屋頂（cy+wall_h，3×3 style.roof 實心）。
            let roof_y = cy + wall_h;
            for dx in x0..=x1 {
                for dz in z0..=z1 {
                    add(&mut out, cx + dx, roof_y, cz + dz, style.roof);
                }
            }
            // 水輪：貼在正面外一格（z = z1+1 = 2）的垂直圓輪造型——用拋光石圈出輪框（四個
            // 上下左右點）＋木製輪軸中心，立在 cy..cy+2 三層高。恆落在主體佔地之外、彼此不重疊。
            let wheel_z = cz + z1 + 1;
            // 輪軸（中心，站立層 cy 起上下三格的中間 cy+1，木）。
            add(&mut out, cx, cy + 1, wheel_z, Block::Wood);
            // 輪框（上下左右四點，拋光石）：上 cy+2、下 cy、左 cx-1@cy+1、右 cx+1@cy+1。
            add(&mut out, cx, cy + 2, wheel_z, Block::SmoothStone);
            add(&mut out, cx, cy, wheel_z, Block::SmoothStone);
            add(&mut out, cx - 1, cy + 1, wheel_z, Block::SmoothStone);
            add(&mut out, cx + 1, cy + 1, wheel_z, Block::SmoothStone);
        }

        BuildKind::Monument => {
            // 紀念碑：小佔地（3×3 石基）卻高聳——一根 5 層石柱直上，頂上一盞燈遠遠可見。
            // 不住人、不做工，純地標。柱身用拋光石（比普通石更「碑」的質感），基座 Stone。
            let (x0, x1) = (-1i32, 1i32);
            let (z0, z1) = (-1i32, 1i32);
            let pillar_h = 5i32;
            // 基座（cy-1，3×3 Stone 實心）。
            for dx in x0..=x1 {
                for dz in z0..=z1 {
                    add(&mut out, cx + dx, cy - 1, cz + dz, Block::Stone);
                }
            }
            // 底座圈（cy 層，3×3 外框 style.wall，讓碑腳有一圈基座感、中心留給柱身）。
            for dx in x0..=x1 {
                for dz in z0..=z1 {
                    if dx.abs() == 1 || dz.abs() == 1 {
                        add(&mut out, cx + dx, cy, cz + dz, style.wall);
                    }
                }
            }
            // 中央石柱（cy..cy+pillar_h-1，拋光石，高聳）。
            for layer in 0..pillar_h {
                add(&mut out, cx, cy + layer, cz, Block::SmoothStone);
            }
            // 碑頂一盞燈（柱頂再上一格，火把——夜裡遠遠可見的地標光）。
            add(&mut out, cx, cy + pillar_h, cz, Block::Torch);
        }
    }

    out
}

// ── 居民搬新家（引導式都更）：舊家方塊集合重算 + 拆除安全過濾（純函式，可測）──────
//
// 搬家流程的拆除段**只准拆她自己舊家的方塊**。做法：用建當年同一套確定性函式
// （`BuildStyle::for_resident` + `generate_blocks`）就地重算舊家錨點的完整方塊集合，
// 逐格比對「現在世界上那格真的是計畫裡那塊」才拆——玩家平台 / 告示牌 / 箱子 / 鄰居的
// 建物 / 路面永遠不在集合裡或型別對不上，機制性一塊都碰不到。保守方向：舊版引擎蓋的
// 家若與今日重算略有出入，差異格會因比對不合而**留下不拆**（絕不誤拆，寧可留）。

/// 重算某位居民「當年在這個錨點蓋的家」的完整方塊集合（純函式、確定性、可測）。
/// 與 `new_plan` 用同一套 `BuildStyle::for_resident`（樣式 #1023）＋ `generate_blocks`，
/// 同居民同錨點永遠得到同一份清單（重啟一致 → 拆除中斷可冪等重算恢復，免存拆除游標）。
pub fn house_blocks_at(resident: &str, cx: i32, cy: i32, cz: i32) -> Vec<BuildBlock> {
    let biome = biome_at_voxel(cx, cz);
    let style = BuildStyle::for_resident(resident, biome, cx, cz);
    generate_blocks(BuildKind::House, cx, cy, cz, &style)
}

/// 一座家的可重放版型。`x_max`/`z_max` 是主屋 footprint；`blocks` 用來保守核對門床。
pub struct HouseReplayLayout {
    pub blocks: Vec<BuildBlock>,
    pub x_max: i32,
    pub z_max: i32,
}

/// 現行版型加上「居家 v2 隔間」上線前的舊版型候選。
///
/// prod 既有小屋不會因程式新增 `room_split` 就原地改建；只用今日的
/// [`house_blocks_at`] 重算會把約半數舊屋的門床及 footprint 認錯。呼叫端依現況門床挑中
/// 真正存在的候選，既保留新屋行為，也讓舊屋可被夜間補圈等保守維修辨識。
pub fn house_replay_layouts_at(resident: &str, cx: i32, cy: i32, cz: i32) -> Vec<HouseReplayLayout> {
    let biome = biome_at_voxel(cx, cz);
    let current = BuildStyle::for_resident(resident, biome, cx, cz);
    let mut out = vec![HouseReplayLayout {
        blocks: generate_blocks(BuildKind::House, cx, cy, cz, &current),
        x_max: current.x_max,
        z_max: current.z_max,
    }];
    if current.room_split {
        let h = style_hash(resident, cx, cz);
        let (x_max, z_max) = match (h >> 6) & 0b11 {
            0 => (1, 1),
            1 => (2, 1),
            2 => (1, 2),
            _ => (2, 2),
        };
        let mut legacy = current;
        legacy.x_max = x_max;
        legacy.z_max = z_max;
        legacy.room_split = false;
        legacy.annex = (h >> 10) & 1 == 1;
        out.push(HouseReplayLayout {
            blocks: generate_blocks(BuildKind::House, cx, cy, cz, &legacy),
            x_max,
            z_max,
        });
    }
    out
}

/// **拆除安全過濾**（純函式、可測）：舊家計畫裡的一格，現在世界上是 `current`——可以拆嗎？
/// 只有「現況方塊正是她當年放的那塊」才准拆；唯一的寬容是門的開關狀態（她蓋的是
/// `DoorClosed`，有人開過門變 `DoorOpen`，仍是同一扇她的門）。其餘任何出入
/// （空氣＝早拆過/被挖走、流動水、玩家後放的箱子/告示牌/任何別種方塊）一律不拆。
pub fn demolish_allowed(expected: u8, current: Block) -> bool {
    if current == Block::Air || current.is_flowing_water() {
        return false; // 已是空/水：沒東西可拆（也絕不把水「拆」成材料）
    }
    if current as u8 == expected {
        return true;
    }
    // 門的開關寬容：計畫是關著的門、現況是開著的同一扇門。
    expected == Block::DoorClosed as u8 && current == Block::DoorOpen
}

/// 拆下一格後，該格回復成什麼（純函式、可測）：自然程序基底是實心且可放置
/// （草/土/沙/石…）→ 回復基底（拆到地板層時地表恢復自然，不留一格深的坑）；
/// 否則（自然本就是空氣等）→ 空氣。與村莊大修復的回填精神一致。
pub fn demolition_restore(x: i32, y: i32, z: i32) -> Block {
    let base = block_at(x, y, z);
    if base.is_solid() && base.is_placeable() {
        base
    } else {
        Block::Air
    }
}

/// **拆除是否真有東西可拆**（純函式、可測）：`demolish_allowed` 之上再加一道「拆了會不會
/// 真的改變世界」的收斂閘。
///
/// **prod 真 bug（露娜等四位補蓋十幾小時零進展的第二層真因）**：`demolition_restore` 把拆掉的
/// 格回復成「該座標的自然地表方塊」。若居民當年放下的那塊「恰好等於自然地表方塊」——最典型是
/// **花圃裝飾（Flowerbed）在草原把 `Grass` 放在地表層**（`demolition_restore` 回復的也正是
/// `Grass`）——拆完之後現況方塊 == 她放的那塊，下一步 `demolish_allowed(Grass, Grass)` 仍為
/// true → 這一格**永遠被判定為「可拆」**，拆除迴圈每步都 `removed ≥ 1`、**永不收斂到收尾**、
/// active 名額永久不釋放，餓死全村的殖民補蓋掃描（米拉跨聚落 demolish 卡在 (-27,8,10) 一格
/// 花圃草地、被 append 一萬多次的真兇）。
///
/// 修法：一格只有在「拆了會真的改變世界」時才拆——`current != restore`。一旦拆到 `current`
/// 已等於自然回復值，就視為「這格已到位、無事可做」跳過，讓拆除確定性收斂到收尾。
/// 冪等且向後相容：門/牆/屋頂等回復成空氣或別種基底的格行為完全不變（`restore != current`）。
pub fn demolish_should_remove(expected: u8, current: Block, restore: Block) -> bool {
    demolish_allowed(expected, current) && current != restore
}

/// 拆下一格入包該記哪種材料（純函式、可測）：一律記「她當年放的那塊」——
/// 開著的門收回的是門（`DoorClosed`），不是「開著」這個狀態。
pub fn demolition_yield(expected: u8) -> u8 {
    expected
}

// ── 居民建造台詞（純函式，零 LLM）────────────────────────────────────────────

/// 生成居民在建造不同階段冒泡的台詞（進度百分比驅動）。
pub fn build_say_line(kind_name: &str, progress_pct: u32) -> String {
    if progress_pct == 0 {
        format!("我要開始蓋{}了！", kind_name)
    } else if progress_pct < 50 {
        format!("{}慢慢成形了……", kind_name)
    } else if progress_pct < 95 {
        format!("{}快蓋好了！", kind_name)
    } else {
        format!("{}蓋好了！✨", kind_name)
    }
}

// ── 蓋家真的用材料 v1（自主提案切片）──────────────────────────────────────────
//
// **真缺口**：蓋家（652）至今每塊方塊都是憑空生出——居民手邊的採集背包（`res_inv`）
// 與正在蓋的房子從沒有一絲關聯，就算她剛好一根木頭都沒有，牆照樣一塊塊冒出來。
// 「你送她的材料」也從沒真的變成她房子的一部分：送禮（gift）至今只換一句道謝與一筆
// 記憶，material 本身憑空消失，不像 670 交易、748 分享那樣真的進出居民的採集背包。
//
// **本刀**：牆與屋頂裡最基礎的四種原料——木頭／石頭／細沙／草皮（皆是居民自己採集
// 就會存進 `res_inv` 的資源，見 `voxel_skills::GatherResource`）——蓋下去時真的從她
// 的採集背包扣一份；手邊剛好沒有，這一塊就先不放（誠實暫停，不硬蓋），等她自己採到
// 或**你送她一份**，下一輪立刻接著蓋。門/床/火把/玻璃/拋光石等經加工或裝飾性方塊
// 刻意不列管（那些不是她自己隨手採得到的東西，管了只會變成無謂的卡關）。
//
// **與既有系統的分界**：不是 699/834 玩家協助蓋家（那是「你自己動手放一塊」）——本刀
// 是「你給她材料、她自己放」，方向不同、互不重疊，兩者可並存。
//
// **純邏輯層**：`is_raw_material` / `material_name_zh` 判斷與命名、缺料/復工的泡泡與
// 動態牆文案全是確定性純函式，零 LLM、零鎖、零 IO。res_inv 讀寫／`stall_material` 旗標
// 切換／批次備料執行全在 `voxel_ws.rs`。

/// 是否為「原料類」方塊——居民自己採集就會進 `res_inv` 的四種基礎資源。
/// 只有這四種蓋家時才真的扣材料；其餘（板材/拋光石/門/床/火把/玻璃……）維持既有
/// 免費放置，避免把還沒被發明引擎教會的加工品也卡進蓋家流程。
pub fn is_raw_material(block: Block) -> bool {
    matches!(block, Block::Wood | Block::Stone | Block::Sand | Block::Grass)
}

/// 蓋家真的用材料 v3（回應 review #1283 v2 退回：`GATHER_QUOTA=2` 讓她一塊採一趟、
/// 每塊都缺料/復工，動態牆被洗版）：缺料時一次備一批，批量對齊「這份計畫剩下還要
/// 幾塊同種材料」（見 `BuildPlan::count_material`），但設這個上限——不讓稀有材料的
/// 房子把居民綁在一趟過長的連續採集上（`GATHER_MAX_RADIUS` 加總可能要走很遠）。
pub const MATERIAL_BATCH_CAP: u32 = 6;

/// 原料方塊的中文顯示名（供缺料/復工文案使用）。非原料回傳空字串（呼叫端不該對非
/// 原料呼叫這個函式，回空字串比 panic 安全，且好測）。
pub fn material_name_zh(block: Block) -> &'static str {
    match block {
        Block::Wood => "木頭",
        Block::Stone => "石頭",
        Block::Sand => "細沙",
        Block::Grass => "草皮",
        _ => "",
    }
}

/// 缺料時冒出的泡泡（輪替數句，避免每次都同一句）。
pub fn material_stall_bubble(material: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "咦，手邊的{}用完了……去張羅張羅。",
        "還缺{}，先去找找看。",
        "{}不夠了，蓋到這邊得先停一下。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replacen("{}", material, 1)
}

/// 缺料暫停的動態牆文案。
pub fn material_stall_feed_line(resident: &str, kind_name: &str, material: &str) -> String {
    format!("{resident}蓋{kind_name}蓋到一半，手邊的{material}用完了，先停下來想辦法。")
}

/// 材料補上、接著蓋的泡泡（輪替數句）。
pub fn material_resume_bubble(material: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 3] = [
        "有{}了！接著蓋～",
        "太好了，湊到{}了，繼續！",
        "{}來了，可以接著蓋下去了！",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replacen("{}", material, 1)
}

/// 材料補上、接著蓋的動態牆文案。
pub fn material_resume_feed_line(resident: &str, kind_name: &str) -> String {
    format!("{resident}湊到了缺的材料，{kind_name}又接著蓋下去了。")
}

/// 蓋家真的用材料 v2（回應 review #1283 退回）：附近真的採不到缺的那種材料時的保底
/// 泡泡——誠實跳過材料要求、這塊照放，不讓居民永遠卡在建造狀態拖垮全村 agency。
pub fn material_waived_bubble(material: &str) -> String {
    format!("附近找不到{material}了，這塊先將就著放吧……")
}

/// 保底放置（附近採不到材料）的動態牆文案。
pub fn material_waived_feed_line(resident: &str, kind_name: &str, material: &str) -> String {
    format!("{resident}蓋{kind_name}時附近找不到{material}，將就放了這一塊。")
}

// ── 居民互助蓋家（純函式，零 LLM）──────────────────────────────────────────────
// ROADMAP 696：老朋友到訪時，若主人正在蓋家，順手幫忙推進一塊——讓情誼（672）不再只停在
// 問候與八卦（694），第一次外溢成「真的動手互相幫忙」的協力行為，小社會湧現再深一層。

/// 每次老朋友到訪，觸發一次幫忙推進的機率。
pub const HELP_CHANCE: f32 = 0.4;

/// 判斷這次到訪是否要伸出援手推進主人的建造計畫一塊。
///
/// `remaining_before_pop`：主人建造計畫目前還剩幾塊待放（0 = 沒有計畫或已完成）。
/// 要求至少剩 2 塊才幫忙，確保幫忙後計畫仍未完工——完工收尾（記錄已蓋種類/完工 Feed/
/// 廣播）只交給 `tick_residents` 第 6 節統一處理一次，本函式的呼叫端不重複那段邏輯。
pub fn should_help_build(remaining_before_pop: usize, roll: f32) -> bool {
    remaining_before_pop >= 2 && roll < HELP_CHANCE
}

/// 幫忙放了一塊後，幫忙者冒出的台詞。
pub fn help_say_line(helper: &str, kind_name: &str) -> String {
    format!("看到在蓋{kind_name}，{helper}順手也幫忙放了一塊！")
}

// ── 合力蓋家完工功勞（純函式，零 LLM）────────────────────────────────────────────
// ROADMAP 834：696 讓老朋友到訪時順手幫忙推進一塊，但完工那一刻（Feed／慶賀泡泡／廣播）
// 此前不管幫了幾次忙，功勞永遠只算屋主一人——協力者的付出在最有感的收尾時刻反而隱形。
// 本節補上「完工時列出所有協力者」的顯示文字，讓小社會的集體行動第一次在完工瞬間被看見。

/// 建物完工時的 Feed 詳情文字：有協力者就在建物名後標注「（與 X、Y 合力）」，沒有則原樣不變。
pub fn build_credit_detail(kind_name: &str, helpers: &[String]) -> String {
    if helpers.is_empty() {
        kind_name.to_string()
    } else {
        format!("{}（與{}合力）", kind_name, helpers.join("、"))
    }
}

// ── 玩家協助居民蓋家（純函式，零 LLM）──────────────────────────────────────────
// ROADMAP 699：居民互助蓋家（696）讓居民彼此的情誼外溢成動手幫忙，但玩家——那個一路
// 採礦砍樹合成工具的人——從沒能真正伸手參與居民蓋家。本節接上這個真缺口：玩家在正確的
// 座標放對方塊，就算幫了居民一把。

/// 居民收到玩家幫忙放塊後，冒出的道謝台詞。
pub fn player_help_say_line(player_name: &str, kind_name: &str) -> String {
    format!("謝謝{player_name}幫忙放的這塊，{kind_name}又更接近完工了！")
}

/// 玩家協助居民建造後，居民心中留下的**感激記憶**摘要（純函式、確定性、零 LLM）。
///
/// 「互動有後果」閉環的第一塊拼圖：此前玩家幫居民蓋家（ROADMAP 699）只化成一句道謝
/// 泡泡＋心情補助＋Feed，**從沒寫進記憶**——重啟即遺忘、不累積好感、也不驅動任何後續
/// 回報。這條摘要讓那份出力**真的被記住**：
/// - 含「幫我蓋」關鍵詞 → 供 [`crate::voxel_fond_greeting::detect_context`] 認出
///   「協助建造」情境，居民日後見你會說出提及這份情的專屬老友問候。
/// - **刻意避開** [`crate::voxel_memory::classify_importance`] 的目標/偏好/承諾關鍵詞
///   （不含「要蓋」「想要」「答應」等）→ 停在 episodic 層：計入好感（記憶筆數），
///   卻不佔用長期精華 12 條上限。
///
/// 面向玩家字串集中一處、留 i18n 空間。
pub fn player_help_memory_line(player_name: &str, kind_name: &str) -> String {
    format!("{player_name}幫我蓋{kind_name}，出了一份力，我記著這份情。")
}

// ── jsonl 持久化 ──────────────────────────────────────────────────────────────

/// 建造計畫落地路徑（`data/` 已 gitignore）。
const VOXEL_BUILDS_PATH: &str = "data/voxel_builds.jsonl";

/// Append 一筆計畫快照到 jsonl（每次放塊後更新 remaining）。
/// **鐵律**：只在不持任何鎖的情境呼叫（小檔同步寫，不 await）。
pub fn append_build(plan: &BuildPlan) {
    if let Ok(line) = serde_json::to_string(plan) {
        write_line(VOXEL_BUILDS_PATH, &line);
    }
}

/// 載回所有建造計畫記錄（伺服器啟動時呼叫一次）。檔不存在 / 壞行皆容忍。
pub fn load_builds() -> Vec<BuildPlan> {
    read_lines(VOXEL_BUILDS_PATH)
}

/// 啟動時先完整 replay 建造紀錄，再將已被覆蓋的舊快照安全壓縮。
/// replay 必須先完成；即使 compact 寫檔失敗，記憶體中的現況仍不受影響。
pub fn load_build_store_compacted() -> BuildStore {
    let store = BuildStore::from_entries(load_builds());
    compact_builds(VOXEL_BUILDS_PATH);
    store
}

// voxel_builds.jsonl 是「最終狀態型」append-only log：同居民取最大 seq，且同 seq
// 最後一行是現況。完成計畫的空 remaining 行是 tombstone，必須保留，否則舊計畫會復活。
// 因此每位居民只留 replay 會採用的最後一行，即可保留 plans 與 next_seq 的完整語意。

/// 將建造快照壓成「每位居民最後狀態一筆」，以同目錄 temp → rename 原子替換。
/// 任何讀取、序列化或寫入失敗都放棄替換，rename 前絕不改動原檔。
pub fn compact_builds(path: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return,
    };
    let mut entries = Vec::new();
    for line in content.lines().map(str::trim).filter(|line| !line.is_empty()) {
        match serde_json::from_str::<BuildPlan>(line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                tracing::warn!("[voxel_building] builds compact 遇到無法解析的原始行: {e}，保守放棄");
                return;
            }
        }
    }
    if entries.is_empty() {
        return;
    }

    let mut latest: HashMap<String, BuildPlan> = HashMap::new();
    for entry in entries {
        let replace = latest
            .get(&entry.resident)
            .map_or(true, |previous| entry.seq >= previous.seq);
        if replace {
            latest.insert(entry.resident.clone(), entry);
        }
    }

    let mut residents: Vec<_> = latest.keys().cloned().collect();
    residents.sort();
    let mut compacted = String::new();
    for resident in residents {
        match serde_json::to_string(&latest[&resident]) {
            Ok(line) => {
                compacted.push_str(&line);
                compacted.push('\n');
            }
            Err(e) => {
                tracing::warn!("[voxel_building] builds compact 序列化失敗: {e}，放棄");
                return;
            }
        }
    }

    let tmp = format!("{path}.compact.tmp");
    if let Err(e) = std::fs::write(&tmp, compacted) {
        tracing::warn!("[voxel_building] builds compact temp 寫入失敗: {e}，原檔保留");
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        tracing::warn!("[voxel_building] builds compact rename 失敗: {e}，原檔保留");
        let _ = std::fs::remove_file(&tmp);
    }
}

// ── 居民改動世界的方塊持久化（重啟後蓋的東西/挖的洞還在）──────────────────────
//
// hub 的 world delta 是記憶體層；居民蓋造放的每一塊、採集挖掉的每一格，都 append 到這份
// jsonl，啟動時 replay 套回 delta → **重啟後居民蓋的東西還在、挖的洞還在**（持久化）。
// append-only、向後相容（檔缺=空）。玩家自己 break/place 不走這裡（沿用原 session 內行為）。

/// 居民改動方塊落地路徑（`data/` 已 gitignore）。
pub const VOXEL_RES_BLOCKS_PATH: &str = "data/voxel_resident_blocks.jsonl";

/// Append 一筆「居民改了某方塊」記錄（放置或挖空都走這裡，b=0 即 Air）。
/// **鐵律**：只在不持任何鎖時呼叫（小檔同步寫，不 await）。
pub fn append_world_block(x: i32, y: i32, z: i32, b: u8) {
    let bb = BuildBlock { x, y, z, b };
    if let Ok(line) = serde_json::to_string(&bb) {
        write_world_line(VOXEL_RES_BLOCKS_PATH, &line);
    }
}

/// 載回所有居民方塊改動（伺服器啟動時呼叫一次，依序套回 delta）。檔不存在 / 壞行皆容忍。
pub fn load_world_blocks() -> Vec<BuildBlock> {
    let content = match std::fs::read_to_string(VOXEL_RES_BLOCKS_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() { None } else { serde_json::from_str::<BuildBlock>(l).ok() }
        })
        .collect()
}

fn write_world_line(path: &str, line: &str) {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{line}");
        }
        Err(e) => tracing::warn!("無法寫入居民方塊改動 {path}: {e}"),
    }
}

// ── Compaction（M4 防磁碟無界膨脹）─────────────────────────────────────────
//
// voxel_resident_blocks.jsonl 是「最終狀態型」append-only log：
// 每個 (x,y,z) 座標最後一次改動為現況（後蓋的覆蓋先蓋的）。
// 壓縮策略：對每個座標只保留最後一筆（最新的），原子重寫替換原檔。
//
// 鐵律：
//   1. replay(原始) 與 replay(compact) 後的 delta map 完全等價。
//   2. rename 前原檔不動；rename 失敗保住原檔。
//   3. 向後相容：serde default，不 drop 任何欄位。

/// 把 `path` 對應的 `voxel_resident_blocks.jsonl` 壓縮成「每座標最後一筆」最小序列
/// （原子 rename 替換原檔）。失敗時只記 log、保住原檔、不 panic。
/// 呼叫時機：伺服器啟動時（replay 完成後）或定期排程（鎖外呼叫）。
pub fn compact_world_blocks(path: &str) {
    let entries = load_world_blocks_from(path);
    if entries.is_empty() {
        return; // 空檔不必壓縮
    }

    // 每個座標只保留最後一筆（last-write-wins）
    // 走線性掃描：後出現的覆蓋先前的。
    use std::collections::HashMap;
    let mut last: HashMap<(i32, i32, i32), BuildBlock> = HashMap::new();
    let mut order: Vec<(i32, i32, i32)> = Vec::new();
    for bb in entries {
        let key = (bb.x, bb.y, bb.z);
        if !last.contains_key(&key) {
            order.push(key);
        }
        last.insert(key, bb);
    }

    // 按首次出現順序輸出（保持 replay 的 delta 套用語意）
    let mut content = String::new();
    for key in &order {
        let bb = &last[key];
        match serde_json::to_string(bb) {
            Ok(line) => {
                content.push_str(&line);
                content.push('\n');
            }
            Err(e) => {
                tracing::warn!("[voxel_building] compact 序列化失敗: {e}，放棄");
                return;
            }
        }
    }

    // 原子替換：temp → rename
    let tmp = format!("{path}.compact.tmp");
    if std::fs::write(&tmp, &content).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        tracing::warn!("[voxel_building] compact rename 失敗: {e}，原檔保留");
        let _ = std::fs::remove_file(&tmp);
    }
}

/// 從指定路徑讀回所有 BuildBlock（供 compact 使用；壞行略過）。
pub fn load_world_blocks_from(path: &str) -> Vec<BuildBlock> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() { None } else { serde_json::from_str::<BuildBlock>(l).ok() }
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
        Err(e) => tracing::warn!("無法寫入居民建造記錄 {path}: {e}"),
    }
}

fn read_lines(path: &str) -> Vec<BuildPlan> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() { None } else { serde_json::from_str::<BuildPlan>(l).ok() }
        })
        .collect()
}

// ── 單元測試 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── classify_desire 純函式 ────────────────────────────────────────────────

    #[test]
    fn classify_house_keywords() {
        assert_eq!(classify_desire("我想蓋一間小屋"), Some(BuildKind::House));
        assert_eq!(classify_desire("我想有一個家"), Some(BuildKind::House));
        assert_eq!(classify_desire("我想建一棟房子"), Some(BuildKind::House));
        assert_eq!(classify_desire("我想住在這裡"), Some(BuildKind::House));
    }

    #[test]
    fn classify_well_keywords() {
        assert_eq!(classify_desire("我想蓋一口水井"), Some(BuildKind::Well));
        assert_eq!(classify_desire("我想要一個水池"), Some(BuildKind::Well));
        assert_eq!(classify_desire("我夢想有個清泉"), Some(BuildKind::Well));
    }

    #[test]
    fn classify_tower_keywords() {
        assert_eq!(classify_desire("我想蓋一座塔"), Some(BuildKind::Tower));
        assert_eq!(classify_desire("我想建一個瞭望台"), Some(BuildKind::Tower));
        assert_eq!(classify_desire("我想在觀星台看星"), Some(BuildKind::Tower));
    }

    #[test]
    fn classify_garden_keywords() {
        assert_eq!(classify_desire("我想種花"), Some(BuildKind::Garden));
        assert_eq!(classify_desire("我想有個花圃"), Some(BuildKind::Garden));
        assert_eq!(classify_desire("我想在花園裡種草"), Some(BuildKind::Garden));
    }

    #[test]
    fn classify_pavilion_keywords() {
        // 「想有個乘涼避雨的地方」這類渴望此前落空（→ None），現在能蓋成一座涼亭。
        assert_eq!(classify_desire("好想有座涼亭"), Some(BuildKind::Pavilion));
        assert_eq!(classify_desire("想搭個亭子乘涼"), Some(BuildKind::Pavilion));
        assert_eq!(classify_desire("盼有個能歇腳的地方"), Some(BuildKind::Pavilion));
        assert_eq!(classify_desire("下雨天想有處避雨"), Some(BuildKind::Pavilion));
        assert_eq!(classify_desire("想要遮陽的涼快處"), Some(BuildKind::Pavilion));
        // 單字「亭」也認得。
        assert_eq!(classify_desire("蓋座亭"), Some(BuildKind::Pavilion));
    }

    #[test]
    fn classify_pavilion_does_not_steal_other_kinds() {
        // 新增涼亭關鍵詞不得誤搶既有分類（家/塔/井/花仍各歸各的）。
        assert_eq!(classify_desire("我想有一個家"), Some(BuildKind::House));
        assert_eq!(classify_desire("我想蓋一座塔"), Some(BuildKind::Tower));
        assert_eq!(classify_desire("我想要一口水井"), Some(BuildKind::Well));
        assert_eq!(classify_desire("我想種花"), Some(BuildKind::Garden));
    }

    // ── 新建物種類分類（M4-B1：工坊 / 磨坊 / 紀念碑）──────────────────────────────

    #[test]
    fn classify_workshop() {
        assert_eq!(classify_desire("我想蓋一間工坊"), Some(BuildKind::Workshop));
        assert_eq!(classify_desire("好想有個能打鐵的作坊"), Some(BuildKind::Workshop));
        assert_eq!(classify_desire("我夢想有座工作坊"), Some(BuildKind::Workshop));
        assert_eq!(classify_desire("想要一個手作的地方"), Some(BuildKind::Workshop));
    }

    #[test]
    fn classify_millhouse() {
        assert_eq!(classify_desire("我想蓋一座磨坊"), Some(BuildKind::Millhouse));
        assert_eq!(classify_desire("好想看水車轉動"), Some(BuildKind::Millhouse));
        assert_eq!(classify_desire("我想要一台水磨"), Some(BuildKind::Millhouse));
    }

    #[test]
    fn classify_monument() {
        assert_eq!(classify_desire("我想立一座紀念碑"), Some(BuildKind::Monument));
        assert_eq!(classify_desire("想蓋個石碑記住這件事"), Some(BuildKind::Monument));
        assert_eq!(classify_desire("盼有個能被遠遠望見的地標"), Some(BuildKind::Monument));
    }

    #[test]
    fn classify_new_kinds_do_not_steal_existing() {
        // 三種新建物關鍵詞不得誤搶既有分類（家/塔/井/花/涼亭仍各歸各的）。
        assert_eq!(classify_desire("我想有一個家"), Some(BuildKind::House));
        assert_eq!(classify_desire("我想蓋一座塔"), Some(BuildKind::Tower));
        assert_eq!(classify_desire("我想要一口水井"), Some(BuildKind::Well));
        assert_eq!(classify_desire("我想種花"), Some(BuildKind::Garden));
        assert_eq!(classify_desire("好想有座涼亭"), Some(BuildKind::Pavilion));
    }

    #[test]
    fn build_kind_str_roundtrips_for_all_kinds() {
        // as_str ↔ from_str 對所有 kind 皆可往返（持久化向後相容的地基）。
        for kind in [
            BuildKind::House,
            BuildKind::Well,
            BuildKind::Tower,
            BuildKind::Garden,
            BuildKind::Pavilion,
            BuildKind::Workshop,
            BuildKind::Millhouse,
            BuildKind::Monument,
        ] {
            assert_eq!(BuildKind::from_str(kind.as_str()), Some(kind), "{kind:?} 往返失敗");
            assert!(!kind.display_name().is_empty(), "{kind:?} 應有顯示名");
        }
        // 未知字串仍回 None。
        assert_eq!(BuildKind::from_str("不存在的種類"), None);
    }

    #[test]
    fn classify_generic_build_falls_to_house() {
        assert_eq!(classify_desire("我想蓋些什麼"), Some(BuildKind::House));
        assert_eq!(classify_desire("我想建一個東西"), Some(BuildKind::House));
    }

    #[test]
    fn classify_no_match_returns_none() {
        assert!(classify_desire("我想和旅人聊天").is_none());
        assert!(classify_desire("我想學習新事物").is_none());
        assert!(classify_desire("").is_none());
    }

    // ── build_fulfills_desire 純函式（ROADMAP 859 心願閉環）──────────────────────

    #[test]
    fn build_fulfills_desire_matches_kind_and_unfulfilled() {
        assert!(build_fulfills_desire("我想有一個家", false, BuildKind::House));
        assert!(build_fulfills_desire("好想有座涼亭", false, BuildKind::Pavilion));
    }

    #[test]
    fn build_fulfills_desire_wrong_kind_returns_false() {
        // 蓋出來的是水井，但心願指的是家——不該誤標成這份心願被實現。
        assert!(!build_fulfills_desire("我想有一個家", false, BuildKind::Well));
    }

    #[test]
    fn build_fulfills_desire_already_fulfilled_returns_false() {
        // 已經標記過的心願不重複觸發（冪等，避免 append_desire 洗版）。
        assert!(!build_fulfills_desire("我想有一個家", true, BuildKind::House));
    }

    #[test]
    fn build_fulfills_desire_unclassifiable_desire_returns_false() {
        // 心願本身分類不到任何建物種類（例如純聊天話題）→ 恆不算實現。
        assert!(!build_fulfills_desire("我想和旅人聊天", false, BuildKind::House));
    }

    #[test]
    fn classify_longer_keyword_wins_over_shorter() {
        // 「瞭望台」應比單字「塔」更優先比對
        assert_eq!(classify_desire("我想建一個瞭望台"), Some(BuildKind::Tower));
        // 「水井」應比「井」更優先
        assert_eq!(classify_desire("我想要一口水井"), Some(BuildKind::Well));
    }

    // ── prayer_promotable（禱告→持久渴望的提升閘）────────────────────────────

    #[test]
    fn prayer_promotable_true_for_concrete_build_wish() {
        // 建造 prompt 範例句本身即可分類，理當提升。
        assert!(prayer_promotable("願農田旁能有水源"));
        assert!(prayer_promotable("好想蓋一座能眺望遠方的瞭望台"));
    }

    #[test]
    fn prayer_promotable_false_for_vague_mood_wish() {
        // 純抒發心情、沒有具體建物種類，不該提升（避免頻繁蓋掉真正的建造心願）。
        assert!(!prayer_promotable("好想有個慶典熱鬧一下"));
        assert!(!prayer_promotable("這一帶夜裡不安全，盼有人守望"));
        assert!(!prayer_promotable(""));
    }

    // ── generate_blocks 方塊數（用「最小樣式」＝退化回原本 3×3 方盒，維持穩定基準）──

    /// 最小樣式：3×3、牆高 2、平頂、無窗、無點綴 → 等同建築創作前的原始小木屋，
    /// 讓既有方塊數/門洞的穩定基準測試仍成立（其餘測試才去驗「變化」）。
    fn style_small() -> BuildStyle {
        BuildStyle {
            wall: Block::Wood,
            roof: Block::Stone,
            floor: Block::Plank,
            peaked: false,
            windows: false,
            wall_h: 2,
            x_max: 1,
            z_max: 1,
            decor: Decor::None,
            annex: false,
            annex_pos: AnnexPos::Back,
            room_split: false,
            stories: 1,
            basement: false,
        }
    }

    #[test]
    fn house_block_count_small_style() {
        // style_small 是退化的 3×3 單層小屋（legacy footprint，走 push_house 的小屋分支）：
        // 地板 9 + 牆 3 層×8=24（含 2 門洞 + 2 側窗，皆為替換不改總數）+ 屋頂 9 + 床 1 = 43。
        let blocks = generate_blocks(BuildKind::House, 0, 5, 0, &style_small());
        assert_eq!(blocks.len(), 43);
    }

    #[test]
    fn house_has_beds_inside_never_on_wall_or_door() {
        // 多層大宅生成器：每座家都有臥室，至少一張床，且每張床都落在室內（非邊界牆、
        // 非門格），並落在某層可站立面（y = cy + 4k）。讓蓋好的家真的有人住的痕跡。
        for rid in ["vox_res_0", "vox_res_1", "露娜", "諾娃"] {
            for biome in [
                VoxelBiome::Grassland,
                VoxelBiome::Forest,
                VoxelBiome::Desert,
                VoxelBiome::Snow,
            ] {
                let (cx, cy, cz) = (3, 5, 9);
                let style = BuildStyle::for_resident(rid, biome, cx, cz);
                let blocks = generate_blocks(BuildKind::House, cx, cy, cz, &style);
                let beds: Vec<_> = blocks.iter().filter(|b| b.b == Block::Bed as u8).collect();
                assert!(!beds.is_empty(), "{rid}/{biome:?} 大宅應至少一張床");
                let (x0, x1) = (cx + BuildStyle::X_MIN, cx + style.x_max);
                let (z0, z1) = (cz + BuildStyle::Z_MIN, cz + style.z_max);
                for bed in beds {
                    // 落在某層可站立面（cy、cy+4、cy+8…）。
                    assert_eq!((bed.y - cy).rem_euclid(4), 0, "{rid}/{biome:?} 床應落在某層站立面");
                    assert!(bed.y >= cy, "{rid}/{biome:?} 床不應在地下室");
                    // 內部：不在邊界牆、不在正面門那一排。
                    assert!(bed.x > x0 && bed.x < x1, "{rid}/{biome:?} 床 x 應落在內部");
                    assert!(bed.z > z0 && bed.z < z1, "{rid}/{biome:?} 床 z 應落在內部");
                }
            }
        }
    }

    // ── 多層大宅結構（ROADMAP·多層大宅生成器）────────────────────────────────────

    #[test]
    fn grand_house_is_multi_floor_with_rooms_stairs_and_furniture() {
        // 掃過多位居民×四群系，逐一驗大宅該有的結構：多層（多個不同 y 的樓板）、
        // 內牆隔間門＋正面大門、樓梯（Ladder）貫通、傢俱（床/桌），有地下室的還要真的
        // 挖空成房（地面層以下有空氣）並有一口箱子。
        let mut saw_multi_story = false;
        let mut saw_basement = false;
        let mut saw_three = false;
        for i in 0..24 {
            let rid = format!("vox_res_{i}");
            for biome in [
                VoxelBiome::Grassland,
                VoxelBiome::Forest,
                VoxelBiome::Desert,
                VoxelBiome::Snow,
            ] {
                let (cx, cy, cz) = (0, 64, 0);
                let s = BuildStyle::for_resident(&rid, biome, cx, cz);
                let blocks = generate_blocks(BuildKind::House, cx, cy, cz, &s);
                // 正面大門（≥1 扇 DoorClosed）。
                assert!(
                    blocks.iter().any(|b| b.b == Block::DoorClosed as u8),
                    "{rid}/{biome:?} 大宅應有門"
                );
                // 傢俱：至少一張床、一張桌。
                assert!(blocks.iter().any(|b| b.b == Block::Bed as u8), "{rid}/{biome:?} 應有床");
                assert!(blocks.iter().any(|b| b.b == Block::Table as u8), "{rid}/{biome:?} 應有桌");
                // 側牆採光窗。
                assert!(blocks.iter().any(|b| b.b == Block::Glass as u8), "{rid}/{biome:?} 應有窗");
                if s.stories >= 2 {
                    saw_multi_story = true;
                    if s.stories == 3 {
                        saw_three = true;
                    }
                    // 樓層之間確有樓梯（Ladder）真的能上下。
                    assert!(
                        blocks.iter().any(|b| b.b == Block::Ladder as u8),
                        "{rid}/{biome:?} 多層大宅應有樓梯（Ladder）"
                    );
                    // 多個不同的樓板 y（地面層 + 樓上）。
                    let floor_ys: std::collections::HashSet<i32> = blocks
                        .iter()
                        .filter(|b| b.b == s.floor as u8)
                        .map(|b| b.y)
                        .collect();
                    assert!(floor_ys.len() >= 2, "{rid}/{biome:?} 多層應有 ≥2 個樓板高度");
                    // 隔間門（門洞在 z=footprint 中線，非正面 z_max 那排）。
                    let z1 = cz + s.z_max;
                    assert!(
                        blocks.iter().any(|b| b.b == Block::DoorClosed as u8 && b.z != z1),
                        "{rid}/{biome:?} 應有隔間內門"
                    );
                }
                if s.basement {
                    saw_basement = true;
                    // 地下室真的挖空成房（地面層以下 y<cy-1 有空氣格）。
                    assert!(
                        blocks.iter().any(|b| b.b == Block::Air as u8 && b.y < cy - 1),
                        "{rid}/{biome:?} 地下室應挖空（有空氣格）"
                    );
                    // 儲藏：一口箱子。
                    assert!(
                        blocks.iter().any(|b| b.b == Block::Chest as u8),
                        "{rid}/{biome:?} 地下室應有箱子"
                    );
                    // 樓梯井延伸到地下（有 y<cy 的 Ladder）。
                    assert!(
                        blocks.iter().any(|b| b.b == Block::Ladder as u8 && b.y < cy),
                        "{rid}/{biome:?} 樓梯井應下探地下室"
                    );
                }
            }
        }
        assert!(saw_multi_story, "24 位居民中應有人蓋出 ≥2 層大宅");
        assert!(saw_three, "24 位居民中應有人蓋出 3 層大宅");
        assert!(saw_basement, "24 位居民中應有人蓋出含地下室的大宅");
    }

    #[test]
    fn grand_house_is_deterministic_blockwise() {
        // 確定性鐵律：同居民同錨點兩次生成 → 逐塊完全相同（replay／玩家逐塊幫忙安全）。
        for rid in ["vox_res_0", "vox_res_5", "露娜"] {
            for &(cx, cy, cz) in &[(0, 64, 0), (-150, 9, 80), (37, 8, -12)] {
                let biome = biome_at_voxel(cx, cz);
                let s = BuildStyle::for_resident(rid, biome, cx, cz);
                let a = generate_blocks(BuildKind::House, cx, cy, cz, &s);
                let b = generate_blocks(BuildKind::House, cx, cy, cz, &s);
                assert_eq!(a, b, "{rid}@({cx},{cy},{cz}) 大宅應逐塊確定性一致");
            }
        }
    }

    #[test]
    fn house_has_two_layer_door_at_front() {
        let blocks = generate_blocks(BuildKind::House, 10, 5, 20, &style_small());
        // 正面（dx=0, z=z_max=+1）兩層都應是門，讓居民蓋的家真的能被打開走進去。
        let door_layer0 = blocks.iter().find(|b| b.x == 10 && b.y == 5 && b.z == 21);
        let door_layer1 = blocks.iter().find(|b| b.x == 10 && b.y == 6 && b.z == 21);
        assert_eq!(door_layer0.map(|b| b.b), Some(Block::DoorClosed as u8));
        assert_eq!(door_layer1.map(|b| b.b), Some(Block::DoorClosed as u8));
        // 其餘牆體邊框仍是 Wood，沒有被誤改。
        let corner = blocks.iter().find(|b| b.x == 9 && b.y == 5 && b.z == 19).unwrap();
        assert_eq!(corner.b, Block::Wood as u8);
    }

    #[test]
    fn well_block_count_small_style() {
        let blocks = generate_blocks(BuildKind::Well, 0, 5, 0, &style_small());
        // 底圈 8 + 水 1 + 井壁 8 + 角柱 4 = 21
        assert_eq!(blocks.len(), 21);
    }

    #[test]
    fn tower_block_count_small_style() {
        let blocks = generate_blocks(BuildKind::Tower, 0, 5, 0, &style_small());
        // 地基 9 + 塔身 8×4 + 頂台 9 = 50（peaked=false → body_h=4）
        assert_eq!(blocks.len(), 50);
    }

    #[test]
    fn garden_block_count_small_style() {
        let blocks = generate_blocks(BuildKind::Garden, 0, 5, 0, &style_small());
        // 草地 9 + 邊框 8 + 中心 1 = 18
        assert_eq!(blocks.len(), 18);
    }

    #[test]
    fn pavilion_block_count_small_style() {
        let blocks = generate_blocks(BuildKind::Pavilion, 0, 5, 0, &style_small());
        // 地板 9 + 四角柱 4×3 + 頂蓋 9 + 中心燈 1 = 31（peaked=false → body_h=3、無尖頂）
        assert_eq!(blocks.len(), 31);
    }

    #[test]
    fn pavilion_is_open_sided_with_central_lantern() {
        // 涼亭刻意四面通透：站立層 cy 的邊中點（非角）不得有牆，只有四角立柱擋著。
        let blocks = generate_blocks(BuildKind::Pavilion, 0, 5, 0, &style_small());
        // 邊中點（如 dx=0,dz=-1）站立層應是空的（開放通行）。
        let edge_mid = blocks.iter().find(|b| b.x == 0 && b.y == 5 && b.z == -1);
        assert!(edge_mid.is_none(), "涼亭側邊中點不該有牆（應四面通透）");
        // 四角立柱存在（如 dx=-1,dz=-1 站立層）。
        let corner = blocks.iter().find(|b| b.x == -1 && b.y == 5 && b.z == -1);
        assert_eq!(corner.map(|b| b.b), Some(Block::Wood as u8), "四角應有立柱");
        // 中心一盞火把（歇腳暖光）。
        let lantern = blocks.iter().find(|b| b.x == 0 && b.y == 5 && b.z == 0);
        assert_eq!(lantern.map(|b| b.b), Some(Block::Torch as u8), "涼亭中心應有一盞燈");
    }

    // ── 新建物：工坊 / 磨坊 / 紀念碑（M4-B1）──────────────────────────────────────

    #[test]
    fn workshop_block_count_and_bounds_small_style() {
        let blocks = generate_blocks(BuildKind::Workshop, 0, 5, 0, &style_small());
        // 佔地 5×4=20。地板 20；牆三面（正面 z=z1 那排敞口）3 層；屋頂 20；設備 2。
        // 牆邊框格數（含正面）：周長 = 2*(5)+2*(4)-4 = 14；扣掉正面那排 5 格 → 9 格/層 × 3 層 = 27。
        // 但正面兩角本已含在正面 5 格內；其餘三面邊框恰 9 格。20 + 27 + 20 + 2 = 69。
        assert_eq!(blocks.len(), 69, "工坊方塊數應為 69");
        // 佔地邊界：x ∈ [-2,2]、z ∈ [-1,2]。
        for b in &blocks {
            assert!(b.x >= -2 && b.x <= 2, "x 越界: {}", b.x);
            assert!(b.z >= -1 && b.z <= 2, "z 越界: {}", b.z);
        }
        // 正面敞口：z==2（front）站立層邊框處不得有牆（只有地板/屋頂那層才在該排放方塊）。
        let front_wall = blocks
            .iter()
            .any(|b| b.z == 2 && b.y >= 5 && b.y <= 7 && b.b == Block::Wood as u8);
        assert!(!front_wall, "工坊正面應敞口，不砌牆");
        // 內部有恰好一個工作台與一個熔爐。
        assert_eq!(
            blocks.iter().filter(|b| b.b == Block::Workbench as u8).count(),
            1,
            "應恰一座工作台"
        );
        assert_eq!(
            blocks.iter().filter(|b| b.b == Block::Furnace as u8).count(),
            1,
            "應恰一座熔爐"
        );
    }

    #[test]
    fn millhouse_block_count_and_wheel_offset() {
        let blocks = generate_blocks(BuildKind::Millhouse, 0, 5, 0, &style_small());
        // 石基 9 + 牆 8×3（含門洞取代不改總數）+ 屋頂 9 + 水輪 5 = 9+24+9+5 = 47。
        assert_eq!(blocks.len(), 47, "磨坊方塊數應為 47");
        // 水輪貼在正面外一格（z=2），共 5 塊、全在主體佔地（z ≤ 1）之外。
        let wheel: Vec<_> = blocks.iter().filter(|b| b.z == 2).collect();
        assert_eq!(wheel.len(), 5, "水輪應為 5 塊，位於正面外一格");
        // 主體帶門洞（正面中央下兩層）。
        assert!(
            blocks.iter().any(|b| b.x == 0 && b.z == 1 && b.y == 5 && b.b == Block::DoorClosed as u8),
            "磨坊正面應有門洞"
        );
    }

    #[test]
    fn monument_is_tall_and_small_footprint() {
        let blocks = generate_blocks(BuildKind::Monument, 0, 5, 0, &style_small());
        // 基座 9 + 底座圈 8 + 石柱 5 + 頂燈 1 = 23。
        assert_eq!(blocks.len(), 23, "紀念碑方塊數應為 23");
        // 佔地小（3×3），但高聳：最高一塊（頂燈）落在 cy+5=10。
        let max_y = blocks.iter().map(|b| b.y).max().unwrap();
        assert_eq!(max_y, 10, "紀念碑應高聳至 cy+5");
        for b in &blocks {
            assert!(b.x >= -1 && b.x <= 1 && b.z >= -1 && b.z <= 1, "紀念碑佔地應為 3×3");
        }
        // 頂端一盞燈。
        assert!(
            blocks.iter().any(|b| b.x == 0 && b.z == 0 && b.y == 10 && b.b == Block::Torch as u8),
            "碑頂應有一盞燈"
        );
    }

    #[test]
    fn new_kinds_are_deterministic_and_overlap_free() {
        // 建築生成鐵律：同居民同錨點恆生出逐塊相同、且無重疊的藍圖。
        for kind in [BuildKind::Workshop, BuildKind::Millhouse, BuildKind::Monument] {
            for rid in ["vox_res_0", "vox_res_1", "露娜", "諾娃"] {
                for biome in [
                    VoxelBiome::Grassland,
                    VoxelBiome::Forest,
                    VoxelBiome::Desert,
                    VoxelBiome::Snow,
                ] {
                    let s = BuildStyle::for_resident(rid, biome, 12, 34);
                    let a = generate_blocks(kind, 12, 5, 34, &s);
                    let b = generate_blocks(kind, 12, 5, 34, &s);
                    assert_eq!(a, b, "{kind:?}/{rid}/{biome:?} 應逐塊確定性一致");
                    let mut seen = std::collections::HashSet::new();
                    for bb in &a {
                        assert!(
                            seen.insert((bb.x, bb.y, bb.z)),
                            "{kind:?}/{rid}/{biome:?} 方塊重疊於 ({},{},{})",
                            bb.x, bb.y, bb.z
                        );
                    }
                }
            }
        }
    }

    // ── 磨坊靠水 gating（millhouse_site_ok）──────────────────────────────────────

    #[test]
    fn millhouse_site_ok_requires_water_within_two() {
        // 只有磨坊受約束：錨點旁 2 格內（cy 或 cy-1）有水才准蓋。
        // 造一個「(2,4,0) 有水」的世界（正好落在 dx=2、dy=-1 範圍內）。
        let has_water = |x: i32, y: i32, z: i32| (x, y, z) == (2, 4, 0);
        assert!(
            millhouse_site_ok(BuildKind::Millhouse, 0, 5, 0, has_water),
            "旁 2 格內有水應允許蓋磨坊"
        );
        // 完全沒水 → 不准。
        assert!(
            !millhouse_site_ok(BuildKind::Millhouse, 0, 5, 0, |_, _, _| false),
            "無水應不准蓋磨坊"
        );
        // 水在 3 格外（dx=3）→ 超出範圍，不准。
        let far_water = |x: i32, y: i32, z: i32| (x, y, z) == (3, 5, 0);
        assert!(
            !millhouse_site_ok(BuildKind::Millhouse, 0, 5, 0, far_water),
            "水在 3 格外應不准"
        );
    }

    #[test]
    fn millhouse_site_ok_other_kinds_unconstrained() {
        // 其餘建物種類不挑地點，無水也恆允許（gating 只約束磨坊）。
        for kind in [
            BuildKind::House,
            BuildKind::Well,
            BuildKind::Tower,
            BuildKind::Garden,
            BuildKind::Pavilion,
            BuildKind::Workshop,
            BuildKind::Monument,
        ] {
            assert!(
                millhouse_site_ok(kind, 0, 5, 0, |_, _, _| false),
                "{kind:?} 不該受靠水約束"
            );
        }
    }

    #[test]
    fn all_blocks_have_valid_block_type_across_residents() {
        // 掃過多位居民 × 四群系 × 四種建物，任何組合生出的方塊都必須是合法方塊 id。
        for rid in ["vox_res_0", "vox_res_1", "vox_res_2", "vox_res_3", "vox_res_7"] {
            for biome in [
                VoxelBiome::Grassland,
                VoxelBiome::Forest,
                VoxelBiome::Desert,
                VoxelBiome::Snow,
            ] {
                let style = BuildStyle::for_resident(rid, biome, 7, 0);
                for kind in [
                    BuildKind::House,
                    BuildKind::Well,
                    BuildKind::Tower,
                    BuildKind::Garden,
                    BuildKind::Pavilion,
                    BuildKind::Workshop,
                    BuildKind::Millhouse,
                    BuildKind::Monument,
                ] {
                    let blocks = generate_blocks(kind, 0, 5, 0, &style);
                    for bb in &blocks {
                        assert!(
                            Block::from_u8(bb.b).is_some(),
                            "無效方塊 id={} 在 {:?}/{:?}/{rid}",
                            bb.b, kind, biome
                        );
                    }
                }
            }
        }
    }

    // ── 建築風格：確定性 + 變化 + 合法性（建築創作第一刀）──────────────────────

    #[test]
    fn style_is_deterministic_for_same_resident_and_anchor() {
        // 同居民、同群系、同錨點 → 永遠同一份風格（重啟一致、不會這次木下次石）。
        let a = BuildStyle::for_resident("vox_res_0", VoxelBiome::Forest, 7, 0);
        let b = BuildStyle::for_resident("vox_res_0", VoxelBiome::Forest, 7, 0);
        assert_eq!(a, b);
        // 連方塊清單也逐塊一致。
        let ba = generate_blocks(BuildKind::House, 100, 5, 100, &a);
        let bb = generate_blocks(BuildKind::House, 100, 5, 100, &b);
        assert_eq!(ba, bb, "同輸入應生出逐塊相同的藍圖");
    }

    #[test]
    fn wall_material_follows_biome() {
        // 牆材質限定在該群系的候選內（森林木系、沙漠沙/拋石、雪原拋石/雪）。
        for rid in ["vox_res_0", "vox_res_1", "vox_res_2", "vox_res_3"] {
            let forest = BuildStyle::for_resident(rid, VoxelBiome::Forest, 7, 0).wall;
            assert!(
                matches!(forest, Block::Wood | Block::Plank),
                "森林牆材應為木系：{forest:?}"
            );
            let desert = BuildStyle::for_resident(rid, VoxelBiome::Desert, 7, 0).wall;
            assert!(
                matches!(desert, Block::Sand | Block::SmoothStone),
                "沙漠牆材應為沙/拋石：{desert:?}"
            );
            let snow = BuildStyle::for_resident(rid, VoxelBiome::Snow, 7, 0).wall;
            assert!(
                matches!(snow, Block::SmoothStone | Block::Snow),
                "雪原牆材應為拋石/雪：{snow:?}"
            );
        }
    }

    #[test]
    fn different_residents_get_varied_houses() {
        // 走進村子：不同居民（同群系同座標）蓋出的房子藍圖應當彼此不同，不再是複製貼上。
        // 收集多位居民的房子「特徵指紋」（材質/尺寸/屋頂/裝飾），要求有足夠多樣。
        let biome = VoxelBiome::Grassland;
        let mut fingerprints = std::collections::HashSet::new();
        let mut blueprints = std::collections::HashSet::new();
        for i in 0..8 {
            let rid = format!("vox_res_{i}");
            let s = BuildStyle::for_resident(&rid, biome, 7, 0);
            fingerprints.insert((
                s.wall as u8,
                s.roof as u8,
                s.peaked,
                s.windows,
                s.stories,
                s.x_max,
                s.z_max,
                s.basement,
            ));
            let blocks = generate_blocks(BuildKind::House, 0, 5, 0, &s);
            // 以「排序後的方塊清單」當整棟房子的指紋。
            let mut v: Vec<(i32, i32, i32, u8)> =
                blocks.iter().map(|b| (b.x, b.y, b.z, b.b)).collect();
            v.sort();
            blueprints.insert(v);
        }
        // 8 位居民至少要有 4 種不同的樣式指紋（實務上遠超過），證明「各有不同」。
        assert!(
            fingerprints.len() >= 4,
            "8 位居民的房子樣式太雷同（只有 {} 種）",
            fingerprints.len()
        );
        assert!(
            blueprints.len() >= 4,
            "8 位居民的房子藍圖太雷同（只有 {} 種）",
            blueprints.len()
        );
    }

    #[test]
    fn house_never_has_overlapping_blocks() {
        // 合法性：同一棟房子不得有兩塊落在同一格（重疊＝壞掉的藍圖）。掃過所有變化維度。
        for rid in ["vox_res_0", "vox_res_1", "vox_res_2", "vox_res_3", "vox_res_5"] {
            for biome in [
                VoxelBiome::Grassland,
                VoxelBiome::Forest,
                VoxelBiome::Desert,
                VoxelBiome::Snow,
            ] {
                let s = BuildStyle::for_resident(rid, biome, 7, 0);
                let blocks = generate_blocks(BuildKind::House, 0, 5, 0, &s);
                let mut seen = std::collections::HashSet::new();
                for b in &blocks {
                    assert!(
                        seen.insert((b.x, b.y, b.z)),
                        "方塊重疊於 ({},{},{})：{rid}/{biome:?}",
                        b.x, b.y, b.z
                    );
                }
            }
        }
    }

    #[test]
    fn house_foundation_floor_is_always_solid_fill() {
        // 防破地基：cy-1 地面層樓板必須把整個佔地填滿——除了樓梯井下探地下室時在該欄位
        // 留一格供 Ladder 通行的孔（那格是可攀爬的樓梯，非破洞）。故每格非實心即為 Ladder。
        for i in 0..8 {
            let rid = format!("vox_res_{i}");
            let s = BuildStyle::for_resident(&rid, VoxelBiome::Grassland, 7, 0);
            let (cx, cy, cz) = (0, 5, 0);
            let blocks = generate_blocks(BuildKind::House, cx, cy, cz, &s);
            for x in BuildStyle::X_MIN..=s.x_max {
                for z in BuildStyle::Z_MIN..=s.z_max {
                    let cell = blocks
                        .iter()
                        .find(|b| b.x == cx + x && b.y == cy - 1 && b.z == cz + z)
                        .and_then(|b| Block::from_u8(b.b));
                    let ok = cell.map_or(false, |bl| bl.is_solid() || bl == Block::Ladder);
                    assert!(ok, "地面層 ({x},{z}) 應為實心地基或樓梯井格：{rid}");
                }
            }
        }
    }

    #[test]
    fn house_always_has_two_layer_front_door() {
        // 防回歸：無論尺寸/材質怎麼變，正面牆中央（door_x, z=z_max）下兩層永遠是門，
        // 讓每間家都「打得開走得進」（完工錨點 #967 不受影響）。
        for i in 0..8 {
            let rid = format!("vox_res_{i}");
            let s = BuildStyle::for_resident(&rid, VoxelBiome::Grassland, 7, 0);
            let (cx, cy, cz) = (10, 5, 20);
            let blocks = generate_blocks(BuildKind::House, cx, cy, cz, &s);
            let door_x = cx; // 大門對齊錨點欄位（與院牆門口缺口對齊）。
            let door_z = cz + s.z_max;
            for layer in 0..2 {
                let d = blocks
                    .iter()
                    .find(|b| b.x == door_x && b.y == cy + layer && b.z == door_z);
                assert_eq!(
                    d.map(|b| b.b),
                    Some(Block::DoorClosed as u8),
                    "正面第 {layer} 層應為門：{rid}"
                );
            }
        }
    }

    #[test]
    fn house_block_count_stays_bounded() {
        // 效能：多層大宅方塊數要落在合理範圍（標準約 150~230、幸運的 5×5 三層/含地下室
        // 上限 ~294，蓋起來約 20~40 分鐘）；設 90 ≤ n ≤ 300，別暴增成蓋數小時的摩天樓。
        for rid in ["vox_res_0", "vox_res_1", "vox_res_2", "vox_res_3", "vox_res_9"] {
            for biome in [
                VoxelBiome::Grassland,
                VoxelBiome::Forest,
                VoxelBiome::Desert,
                VoxelBiome::Snow,
            ] {
                let s = BuildStyle::for_resident(rid, biome, 7, 0);
                let n = generate_blocks(BuildKind::House, 0, 5, 0, &s).len();
                assert!(n <= 300, "房子方塊數暴增（{n}）：{rid}/{biome:?}");
                assert!(n >= 90, "房子方塊數異常過少（{n}）：{rid}/{biome:?}");
            }
        }
    }

    // ── （舊 annex 測試已隨多層大宅生成器取代 3×3 方盒＋小棚而移除；
    //     大宅結構改由上方 grand_house_* 系列測試把關）─────────────────────────────

    // ── BuildStore 純函式 ─────────────────────────────────────────────────────

    #[test]
    fn store_has_plan_after_new() {
        let mut s = BuildStore::new();
        assert!(!s.has_plan("vox_res_0"));
        s.new_plan("vox_res_0", BuildKind::House, 10, 5, 20, false, None);
        assert!(s.has_plan("vox_res_0"));
        assert!(!s.has_plan("vox_res_1"));
    }

    #[test]
    fn store_pop_next_reduces_remaining() {
        let mut s = BuildStore::new();
        s.new_plan("vox_res_0", BuildKind::Well, 0, 5, 0, false, None);
        let before = s.plans["vox_res_0"].remaining.len();
        let b = s.get_plan_mut("vox_res_0").unwrap().pop_next();
        assert!(b.is_some());
        assert_eq!(s.plans["vox_res_0"].remaining.len(), before - 1);
    }

    #[test]
    fn store_remove_if_done_works() {
        let mut s = BuildStore::new();
        s.new_plan("vox_res_0", BuildKind::Well, 0, 5, 0, false, None);
        // drain all blocks
        while s.get_plan_mut("vox_res_0").and_then(|p| p.pop_next()).is_some() {}
        assert!(s.plans["vox_res_0"].is_done());
        s.remove_if_done("vox_res_0");
        assert!(!s.has_plan("vox_res_0"));
    }

    #[test]
    fn from_entries_restores_incomplete_plan() {
        let plan = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: 5,
            cy: 3,
            cz: 5,
            remaining: vec![BuildBlock { x: 5, y: 3, z: 5, b: Block::Wood as u8 }].into(),
            total: 34,
            seq: 0,
            expansion: false,
            inspired_by: None,
            helpers: Vec::new(),
            stall_material: None,
        };
        let s = BuildStore::from_entries(vec![plan]);
        assert!(s.has_plan("vox_res_0"));
    }

    #[test]
    fn from_entries_skips_done_plan() {
        let plan = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: 5,
            cy: 3,
            cz: 5,
            remaining: VecDeque::new(), // done
            total: 34,
            seq: 0,
            expansion: false,
            inspired_by: None,
            helpers: Vec::new(),
            stall_material: None,
        };
        let s = BuildStore::from_entries(vec![plan]);
        assert!(!s.has_plan("vox_res_0"), "已完成的計畫不應載回");
    }

    #[test]
    fn from_entries_keeps_latest_seq() {
        let old = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: 0,
            cy: 0,
            cz: 0,
            remaining: vec![BuildBlock { x: 0, y: 0, z: 0, b: Block::Wood as u8 }].into(),
            total: 10,
            seq: 0,
            expansion: false,
            inspired_by: None,
            helpers: Vec::new(),
            stall_material: None,
        };
        let new = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "tower".into(),
            kind_name: "瞭望台".into(),
            cx: 5,
            cy: 3,
            cz: 5,
            remaining: vec![BuildBlock { x: 5, y: 3, z: 5, b: Block::Stone as u8 }].into(),
            total: 50,
            seq: 5,
            expansion: false,
            inspired_by: None,
            helpers: Vec::new(),
            stall_material: None,
        };
        let s = BuildStore::from_entries(vec![old, new]);
        assert_eq!(s.plans["vox_res_0"].kind, "tower", "應保留 seq 較大的計畫");
    }

    #[test]
    fn from_entries_last_wins_within_same_seq_drops_completed_plan() {
        // churn 根治回歸（prod 真 bug）：一張計畫的 jsonl 是「同 seq、remaining 逐塊遞減」
        // 的一長串 append。舊版嚴格 `>` 去重永遠留住**最先**那行（remaining 滿），把它放到
        // remaining=0（done）的後續行永遠蓋不掉它 → 重啟把早已蓋完的計畫還原成全新未蓋，
        // 重蓋→落 anchor_only→再重啟再重蓋，goals.jsonl 無界膨脹、has_plan 恆真卡死補蓋。
        let mk = |rem: usize| BuildPlan {
            resident: "vox_res_0".into(),
            kind: "pavilion".into(),
            kind_name: "涼亭".into(),
            cx: -6,
            cy: 7,
            cz: 22,
            remaining: (0..rem)
                .map(|i| BuildBlock { x: -6 + i as i32, y: 7, z: 22, b: Block::Wood as u8 })
                .collect::<VecDeque<_>>(),
            total: 31,
            seq: 1973, // 同一張計畫全程同 seq
            expansion: false,
            inspired_by: None,
            helpers: Vec::new(),
            stall_material: None,
        };
        // 檔序：滿 → 遞減 → 0（done）。最新（最後）一行才是真狀態＝蓋完了。
        let entries = vec![mk(31), mk(20), mk(10), mk(1), mk(0)];
        let s = BuildStore::from_entries(entries);
        assert!(
            !s.has_plan("vox_res_0"),
            "同 seq 最後一行是 done → 不該還原成進行中的幽靈計畫（churn 根因）"
        );
        // next_seq 仍接在最大 seq 之後（新計畫不撞號）。
        let mut s = s;
        assert_eq!(s.new_plan("vox_res_0", BuildKind::Well, 0, 5, 0, false, None).seq, 1974);
    }

    #[test]
    fn from_entries_same_seq_resumes_partial_when_last_line_incomplete() {
        // 對稱：若最新一行還沒蓋完（例如放到一半時當機），仍以最新進度續蓋、不倒退回滿。
        let mk = |rem: usize| BuildPlan {
            resident: "vox_res_1".into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: 0,
            cy: 5,
            cz: 0,
            remaining: (0..rem)
                .map(|i| BuildBlock { x: i as i32, y: 5, z: 0, b: Block::Wood as u8 })
                .collect::<VecDeque<_>>(),
            total: 30,
            seq: 42,
            expansion: false,
            inspired_by: None,
            helpers: Vec::new(),
            stall_material: None,
        };
        let s = BuildStore::from_entries(vec![mk(30), mk(15), mk(7)]);
        assert!(s.has_plan("vox_res_1"), "最新一行未蓋完 → 該續蓋");
        assert_eq!(s.plans["vox_res_1"].remaining.len(), 7, "以最新進度續蓋、不倒退回滿");
    }

    #[test]
    fn compact_builds_replay_state_equivalent() {
        use std::io::Write;

        let mk = |resident: &str, seq: u64, remaining: usize| BuildPlan {
            resident: resident.into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: seq as i32,
            cy: 5,
            cz: 0,
            remaining: (0..remaining)
                .map(|x| BuildBlock { x: x as i32, y: 5, z: 0, b: Block::Wood as u8 })
                .collect(),
            total: 3,
            seq,
            expansion: false,
            inspired_by: None,
            helpers: Vec::new(),
            stall_material: None,
        };
        // 同 seq 後行覆蓋、較小 seq 亂序舊行忽略；done tombstone 也必須留下。
        let entries = vec![
            mk("luna", 7, 3),
            mk("nova", 9, 2),
            mk("luna", 7, 1),
            mk("luna", 6, 2),
            mk("nova", 9, 0),
        ];
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for entry in &entries {
            writeln!(file, "{}", serde_json::to_string(entry).unwrap()).unwrap();
        }
        let path = file.path().to_str().unwrap();

        let state_a = BuildStore::from_entries(read_lines(path));
        compact_builds(path);
        let compacted = read_lines(path);
        let state_b = BuildStore::from_entries(compacted.clone());

        assert_eq!(state_a, state_b, "compact 前後 replay 現狀必須完全等價（A==B）");
        assert_eq!(compacted.len(), 2, "每位居民只應保留最後狀態一行");
        assert!(!state_b.has_plan("nova"), "done tombstone replay 後不可復活舊計畫");
    }

    // ── build_say_line 純函式 ─────────────────────────────────────────────────

    #[test]
    fn say_line_stages() {
        let line0 = build_say_line("小木屋", 0);
        assert!(line0.contains("開始蓋"), "進度 0 應包含「開始蓋」：{line0}");

        let line30 = build_say_line("小木屋", 30);
        assert!(line30.contains("成形"), "進度 30 應包含「成形」：{line30}");

        let line70 = build_say_line("水井", 70);
        assert!(line70.contains("快蓋好"), "進度 70 應包含「快蓋好」：{line70}");

        let line99 = build_say_line("瞭望台", 99);
        assert!(line99.contains("蓋好了"), "進度 99 應包含「蓋好了」：{line99}");
    }

    // ── 蓋家真的用材料 v1：is_raw_material / 缺料復工文案 純函式 ─────────────────

    #[test]
    fn only_four_raw_materials_are_gated() {
        // 只有這四種——居民自己採集就會進 res_inv 的基礎資源——蓋下去才扣料。
        assert!(is_raw_material(Block::Wood));
        assert!(is_raw_material(Block::Stone));
        assert!(is_raw_material(Block::Sand));
        assert!(is_raw_material(Block::Grass));
        // 加工/裝飾類方塊維持既有免費放置，避免無謂卡關。
        assert!(!is_raw_material(Block::Plank));
        assert!(!is_raw_material(Block::SmoothStone));
        assert!(!is_raw_material(Block::StoneBrick));
        assert!(!is_raw_material(Block::Leaves));
        assert!(!is_raw_material(Block::Glass));
        assert!(!is_raw_material(Block::DoorClosed));
        assert!(!is_raw_material(Block::Torch));
        assert!(!is_raw_material(Block::Water));
        assert!(!is_raw_material(Block::Air));
    }

    #[test]
    fn material_name_zh_covers_all_gated_blocks() {
        assert_eq!(material_name_zh(Block::Wood), "木頭");
        assert_eq!(material_name_zh(Block::Stone), "石頭");
        assert_eq!(material_name_zh(Block::Sand), "細沙");
        assert_eq!(material_name_zh(Block::Grass), "草皮");
        // 非原料方塊回傳空字串（呼叫端本就不該對它們求名）。
        assert_eq!(material_name_zh(Block::Plank), "");
    }

    #[test]
    fn material_stall_bubble_mentions_material_and_varies() {
        let lines: Vec<String> = (0..3).map(|i| material_stall_bubble("木頭", i)).collect();
        for l in &lines {
            assert!(l.contains("木頭"), "缺料泡泡應提到材料名：{l}");
            assert!(!l.contains('\n'), "泡泡不應含換行：{l}");
        }
        // 三句彼此不同，避免每次卡關都看到一模一樣的台詞。
        assert_ne!(lines[0], lines[1]);
        assert_ne!(lines[1], lines[2]);
        // 任意大的 pick 取模有界，不 panic。
        let _ = material_stall_bubble("石頭", 9999);
    }

    #[test]
    fn material_resume_bubble_mentions_material_and_varies() {
        let lines: Vec<String> = (0..3).map(|i| material_resume_bubble("石頭", i)).collect();
        for l in &lines {
            assert!(l.contains("石頭"), "復工泡泡應提到材料名：{l}");
            assert!(!l.contains('\n'), "泡泡不應含換行：{l}");
        }
        assert_ne!(lines[0], lines[1]);
        assert_ne!(lines[1], lines[2]);
        let _ = material_resume_bubble("細沙", 9999);
    }

    #[test]
    fn material_stall_and_resume_feed_lines_mention_resident_and_kind() {
        let stall = material_stall_feed_line("露娜", "小木屋", "木頭");
        assert!(stall.contains("露娜") && stall.contains("小木屋") && stall.contains("木頭"));
        assert!(!stall.contains('\n'));

        let resume = material_resume_feed_line("露娜", "小木屋");
        assert!(resume.contains("露娜") && resume.contains("小木屋"));
        assert!(!resume.contains('\n'));
    }

    #[test]
    fn material_waived_bubble_and_feed_line_mention_material_resident_and_kind() {
        let bubble = material_waived_bubble("木頭");
        assert!(bubble.contains("木頭"));
        assert!(!bubble.contains('\n'));

        let feed = material_waived_feed_line("露娜", "小木屋", "木頭");
        assert!(feed.contains("露娜") && feed.contains("小木屋") && feed.contains("木頭"));
        assert!(!feed.contains('\n'));
    }

    #[test]
    fn peek_next_does_not_remove_block() {
        let mut plan = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: 0,
            cy: 0,
            cz: 0,
            remaining: vec![BuildBlock { x: 0, y: 0, z: 0, b: Block::Wood as u8 }].into(),
            total: 1,
            seq: 0,
            expansion: false,
            inspired_by: None,
            helpers: Vec::new(),
            stall_material: None,
        };
        assert_eq!(plan.peek_next().map(|b| b.b), Some(Block::Wood as u8));
        // peek 兩次結果一致（不像 pop_next 會消耗掉）。
        assert_eq!(plan.peek_next().map(|b| b.b), Some(Block::Wood as u8));
        assert_eq!(plan.remaining.len(), 1, "peek 不應改動 remaining");
    }

    // ── push_front / count_material 純函式（蓋家真的用材料 v3）─────────────────

    #[test]
    fn push_front_undoes_pop_next_without_losing_or_duplicating() {
        let mut plan = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: 0,
            cy: 0,
            cz: 0,
            remaining: vec![
                BuildBlock { x: 0, y: 0, z: 0, b: Block::Wood as u8 },
                BuildBlock { x: 1, y: 0, z: 0, b: Block::Stone as u8 },
            ]
            .into(),
            total: 2,
            seq: 0,
            expansion: false,
            inspired_by: None,
            helpers: Vec::new(),
            stall_material: None,
        };
        let bb = plan.pop_next().expect("應彈出第一塊");
        assert_eq!(bb.b, Block::Wood as u8);
        assert_eq!(plan.remaining.len(), 1, "pop 後只剩一塊");
        assert_eq!(plan.progress_pct(), 50, "pop 後進度應計入");

        plan.push_front(bb);
        assert_eq!(plan.remaining.len(), 2, "push_front 撤銷 pop，數量應還原");
        assert_eq!(plan.peek_next().map(|b| b.b), Some(Block::Wood as u8), "順序應還原到最前面");
        assert_eq!(plan.progress_pct(), 0, "撤銷後不應誤算已放置");
    }

    #[test]
    fn count_material_counts_only_matching_remaining_blocks() {
        let plan = BuildPlan {
            resident: "vox_res_0".into(),
            kind: "house".into(),
            kind_name: "小木屋".into(),
            cx: 0,
            cy: 0,
            cz: 0,
            remaining: vec![
                BuildBlock { x: 0, y: 0, z: 0, b: Block::Wood as u8 },
                BuildBlock { x: 1, y: 0, z: 0, b: Block::Stone as u8 },
                BuildBlock { x: 2, y: 0, z: 0, b: Block::Wood as u8 },
                BuildBlock { x: 3, y: 0, z: 0, b: Block::Plank as u8 },
            ]
            .into(),
            total: 4,
            seq: 0,
            expansion: false,
            inspired_by: None,
            helpers: Vec::new(),
            stall_material: None,
        };
        assert_eq!(plan.count_material(Block::Wood as u8), 2);
        assert_eq!(plan.count_material(Block::Stone as u8), 1);
        assert_eq!(plan.count_material(Block::Glass as u8), 0, "沒出現過的材料應回 0");
    }

    // ── should_help_build / help_say_line 純函式 ─────────────────────────────

    #[test]
    fn help_needs_at_least_two_remaining() {
        // 只剩 1 塊或 0 塊（沒計畫/已完成）：就算擲骰中獎也不該幫忙，避免搶在
        // tick_residents 完工收尾之前把計畫清空。
        assert!(!should_help_build(0, 0.0));
        assert!(!should_help_build(1, 0.0));
        // 剩 2 塊以上、擲骰命中 → 幫忙。
        assert!(should_help_build(2, 0.0));
        assert!(should_help_build(10, 0.39));
    }

    #[test]
    fn help_respects_chance_roll() {
        // 擲骰值 >= HELP_CHANCE 不幫忙（就算剩很多塊）。
        assert!(!should_help_build(10, HELP_CHANCE));
        assert!(!should_help_build(10, 0.99));
    }

    #[test]
    fn help_say_line_mentions_helper_and_kind() {
        let line = help_say_line("諾娃", "小木屋");
        assert!(line.contains("諾娃"), "應提到幫忙者：{line}");
        assert!(line.contains("小木屋"), "應提到建物種類：{line}");
    }

    // ── try_player_help 純邏輯（ROADMAP 699）────────────────────────────────────

    fn store_with_plan(resident: &str, kind: BuildKind) -> BuildStore {
        let mut s = BuildStore::new();
        s.new_plan(resident, kind, 0, 64, 0, false, None);
        s
    }

    #[test]
    fn player_help_pops_matching_front_block() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        let front = s.plans["vox_res_0"].remaining.front().cloned().unwrap();
        let before_len = s.plans["vox_res_0"].remaining.len();

        let result = s.try_player_help(front.x, front.y, front.z, front.b);

        assert_eq!(result, Some(("vox_res_0".to_string(), "小木屋".to_string())));
        assert_eq!(s.plans["vox_res_0"].remaining.len(), before_len - 1, "應彈掉一塊");
    }

    #[test]
    fn player_help_ignores_wrong_position() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        let before_len = s.plans["vox_res_0"].remaining.len();

        let result = s.try_player_help(9999, 9999, 9999, Block::Wood as u8);

        assert_eq!(result, None, "座標不符不算幫忙");
        assert_eq!(s.plans["vox_res_0"].remaining.len(), before_len, "沒有計畫被更動");
    }

    #[test]
    fn player_help_ignores_wrong_block_type() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        let front = s.plans["vox_res_0"].remaining.front().cloned().unwrap();
        let before_len = s.plans["vox_res_0"].remaining.len();
        let wrong_block = if front.b == Block::Stone as u8 { Block::Wood as u8 } else { Block::Stone as u8 };

        let result = s.try_player_help(front.x, front.y, front.z, wrong_block);

        assert_eq!(result, None, "座標對但方塊類型不符不算幫忙");
        assert_eq!(s.plans["vox_res_0"].remaining.len(), before_len);
    }

    #[test]
    fn player_help_no_plans_returns_none() {
        let mut s = BuildStore::new();
        assert_eq!(s.try_player_help(0, 64, 0, Block::Wood as u8), None);
    }

    #[test]
    fn player_help_picks_correct_resident_among_many() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        s.new_plan("vox_res_1", BuildKind::Well, 20, 64, 20, false, None);
        let front1 = s.plans["vox_res_1"].remaining.front().cloned().unwrap();

        let result = s.try_player_help(front1.x, front1.y, front1.z, front1.b);

        assert_eq!(result, Some(("vox_res_1".to_string(), "水井".to_string())));
        // 另一位居民的計畫不受影響。
        assert!(!s.plans["vox_res_0"].remaining.is_empty());
    }

    #[test]
    fn player_help_say_line_mentions_player_and_kind() {
        let line = player_help_say_line("小明", "小木屋");
        assert!(line.contains("小明"), "應提到玩家名：{line}");
        assert!(line.contains("小木屋"), "應提到建物種類：{line}");
    }

    #[test]
    fn player_help_memory_line_mentions_player_kind_and_has_helpbuild_keyword() {
        let line = player_help_memory_line("小明", "小木屋");
        assert!(line.contains("小明"), "應提到玩家名：{line}");
        assert!(line.contains("小木屋"), "應提到建物種類：{line}");
        // 含「幫我蓋」→ 供 fond_greeting 認出「協助建造」情境。
        assert!(line.contains("幫我蓋"), "應含 HelpedBuild 關鍵詞「幫我蓋」：{line}");
    }

    #[test]
    fn player_help_memory_line_avoids_semantic_keywords() {
        // 感激記憶必須停在 episodic 層（計入好感、不佔長期精華）——
        // 不可含 classify_importance 的目標/偏好/承諾關鍵詞，否則會被誤升成語意事實。
        for kind in ["小木屋", "水井", "瞭望台", "花圃"] {
            let line = player_help_memory_line("旅人", kind);
            for kw in ["要蓋", "要建", "想要", "打算", "要把", "我要", "最喜歡", "喜歡", "答應", "承諾"] {
                assert!(!line.contains(kw), "感激記憶不該含語意關鍵詞「{kw}」：{line}");
            }
        }
    }

    // ── 合力蓋家完工功勞（ROADMAP 834）────────────────────────────────────────────

    #[test]
    fn new_plan_starts_with_no_helpers() {
        let s = store_with_plan("vox_res_0", BuildKind::House);
        assert!(s.plans["vox_res_0"].helpers.is_empty());
    }

    #[test]
    fn add_helper_records_name() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        s.get_plan_mut("vox_res_0").unwrap().add_helper("賽勒");
        assert_eq!(s.plans["vox_res_0"].helpers, vec!["賽勒".to_string()]);
    }

    #[test]
    fn add_helper_dedupes_same_name() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        let plan = s.get_plan_mut("vox_res_0").unwrap();
        plan.add_helper("賽勒");
        plan.add_helper("賽勒");
        assert_eq!(s.plans["vox_res_0"].helpers.len(), 1, "同一人重複幫忙不應重複記名");
    }

    #[test]
    fn add_helper_ignores_owner_self() {
        let mut s = store_with_plan("露娜", BuildKind::House);
        s.get_plan_mut("露娜").unwrap().add_helper("露娜");
        assert!(s.plans["露娜"].helpers.is_empty(), "屋主本人不算協力者");
    }

    #[test]
    fn add_helper_supports_multiple_distinct_names() {
        let mut s = store_with_plan("vox_res_0", BuildKind::House);
        let plan = s.get_plan_mut("vox_res_0").unwrap();
        plan.add_helper("賽勒");
        plan.add_helper("諾娃");
        assert_eq!(s.plans["vox_res_0"].helpers, vec!["賽勒".to_string(), "諾娃".to_string()]);
    }

    #[test]
    fn build_credit_detail_unchanged_when_no_helpers() {
        assert_eq!(build_credit_detail("小木屋", &[]), "小木屋");
    }

    #[test]
    fn build_credit_detail_lists_single_helper() {
        let helpers = vec!["賽勒".to_string()];
        assert_eq!(build_credit_detail("小木屋", &helpers), "小木屋（與賽勒合力）");
    }

    #[test]
    fn build_credit_detail_lists_multiple_helpers_joined() {
        let helpers = vec!["賽勒".to_string(), "諾娃".to_string()];
        assert_eq!(build_credit_detail("水井", &helpers), "水井（與賽勒、諾娃合力）");
    }

    #[test]
    fn build_plan_deserializes_without_helpers_field() {
        // 向後相容：舊 jsonl 沒有 helpers 欄位，載回應預設空陣列、不 panic。
        let old_json = r#"{"resident":"vox_res_0","kind":"house","kind_name":"小木屋",
            "cx":0,"cy":64,"cz":0,"remaining":[],"total":0,"seq":1}"#;
        let plan: BuildPlan = serde_json::from_str(old_json).expect("舊格式應可解析");
        assert!(plan.helpers.is_empty());
    }

    // ── 居民搬新家：舊家方塊集合重算 + 拆除安全過濾 ─────────────────────────────

    #[test]
    fn grand_house_placed_equals_natural_cells_never_cause_churn() {
        // prod 真 bug（demolish churn）承接：多層大宅生成器取代舊 3×3 方盒＋花圃裝飾後，
        // 那格「花圃在草地上放草」的自我還原churn 已不復存在；但沙漠沙地板鋪在天然沙上等
        // 情形仍可能讓某格「放的＝天然回復值」。鐵律（承接 `demolish_should_remove` 收斂修）：
        // 任何這種格拆了世界不變 → 必被判為「不必拆」、拆除迴圈確定性收斂、永不churn。
        // 掃過多位居民×多錨點逐格驗證（涵蓋各群系與有無地下室）。
        for rid in ["vox_res_0", "vox_res_3", "vox_res_4", "vox_res_5", "米拉"] {
            for &(cx, cy, cz) in &[(-27, 9, 8), (0, 64, 0), (300, 9, 300), (-500, 9, 120)] {
                let blocks = house_blocks_at(rid, cx, cy, cz);
                for bb in &blocks {
                    let restore = demolition_restore(bb.x, bb.y, bb.z);
                    // 「放的＝天然回復值」的churn候選格：收斂閘必須把它判為「不必拆」。
                    if demolish_allowed(bb.b, restore) {
                        assert!(
                            !demolish_should_remove(bb.b, restore, restore),
                            "{rid}@({},{},{}) 放的＝天然值卻仍被判可拆 → 會churn",
                            bb.x, bb.y, bb.z
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn e2e_mira_cross_colony_demolish_converges_not_churns() {
        // ── 端到端拆除迴圈重放（prod 米拉實況 fixture）──────────────────────────────
        // 把 `relocation_demolish_step` 的**逐步決策**照搬到隔離的模擬世界：世界先鋪上米拉真的
        // 舊家（house_blocks_at 的每一塊），再逐 tick 跑「算現況 → demolish_should_remove →
        // 拆一塊 → set 成 demolition_restore」的同一套邏輯，驗證迴圈**確定性收斂**（有限步內
        // 再也沒有可拆的格 → 收尾），而非像 prod 那樣卡在花圃草地一格永不收尾。
        //
        // 這正是前三修「只綠純函式卻 prod 不生效」的補課：#1223 只驗了 walk_back gate，沒重放
        // 拆除迴圈本身，於是漏掉了 restore==placed 的churn。此測直接重放迴圈到收斂。
        use std::collections::HashMap;
        let rid = "vox_res_4";
        let (ox, oy, oz) = (-27, 9, 8); // prod 米拉跨聚落 demolish 的舊家錨點
        let expected = house_blocks_at(rid, ox, oy, oz);
        assert!(!expected.is_empty(), "米拉舊家應有方塊");

        // 模擬世界：座標 → 現況方塊。先鋪上她當年蓋的家（現況 == 她放的那塊）。
        // 未在 map 裡的座標＝自然地表（用 demolition_restore 代表「若拆此格會落到的自然值」）。
        let mut world: HashMap<(i32, i32, i32), u8> = HashMap::new();
        for bb in &expected {
            world.insert((bb.x, bb.y, bb.z), bb.b);
        }
        let current_at = |world: &HashMap<(i32, i32, i32), u8>, x, y, z| -> Block {
            match world.get(&(x, y, z)) {
                Some(&b) => Block::from_u8(b).unwrap_or(Block::Air),
                None => demolition_restore(x, y, z), // 家以外＝自然地表
            }
        };

        // 逐步重放拆除迴圈（每步最多拆 per_step 塊，與 reloc_demolish_per_step 同量級）。
        let per_step = 6usize;
        let mut total_removed = 0usize;
        let mut converged_step: Option<usize> = None;
        // 上限遠大於方塊數；若沒收斂會撞到上限 → 測試失敗（正是舊churn的病徵）。
        let max_steps = expected.len() * 4 + 50;
        for step in 0..max_steps {
            let mut removed_this = 0usize;
            for bb in &expected {
                if removed_this >= per_step {
                    break;
                }
                let current = current_at(&world, bb.x, bb.y, bb.z);
                let restore = demolition_restore(bb.x, bb.y, bb.z);
                if !demolish_should_remove(bb.b, current, restore) {
                    continue; // 不是她放的、或拆了不變 → 跳過（收斂關鍵）
                }
                // 拆：世界改成自然回復值（與 relocation_demolish_step 一致）。
                world.insert((bb.x, bb.y, bb.z), restore as u8);
                removed_this += 1;
                total_removed += 1;
            }
            if removed_this == 0 {
                converged_step = Some(step); // 本步再也沒得拆 → 收尾（active 名額釋放）
                break;
            }
        }

        let converged = converged_step.expect("拆除迴圈必須在有限步內收斂（否則就是 prod 的churn）");
        assert!(
            converged < max_steps,
            "收斂步數 {converged} 應遠小於上限 {max_steps}"
        );
        // 拆掉的塊數＝她真放下、且拆了會改變世界的那些（花圃草地那格因拆了不變、被正確跳過，
        // 所以拆的塊數 < 全部塊數；這正是修好後該有的結果）。
        assert!(total_removed > 0, "應真的拆掉了牆/門/屋頂等塊");
        // 注意：多層大宅生成器取代 3×3 方盒＋花圃後，舊那格「花圃草地」churn 已不存在；
        // 此 e2e 的核心保證是「拆除迴圈確定性收斂、不無限churn」（converged_step 為 Some），
        // 而非特定一格被跳過。地下室的空氣格與沙漠沙地板等「放的＝天然值」的格仍會被
        // demolish_should_remove 正確跳過，不影響收斂。

        // 收斂後世界的最終狀態：門(43)/床(45)/牆等都回復成自然（不再是她放的那塊）。
        let door_gone = expected
            .iter()
            .filter(|bb| bb.b == Block::DoorClosed as u8)
            .all(|bb| current_at(&world, bb.x, bb.y, bb.z) != Block::DoorClosed);
        assert!(door_gone, "舊家的門應已全部拆除回復");
        let bed_gone = expected
            .iter()
            .filter(|bb| bb.b == Block::Bed as u8)
            .all(|bb| current_at(&world, bb.x, bb.y, bb.z) != Block::Bed);
        assert!(bed_gone, "舊家的床應已拆除回復");
    }

    #[test]
    fn demolish_should_remove_converges_on_placed_equals_natural() {
        // 收斂閘純函式驗證：拆了會改變世界才拆；拆完（current==restore）即視為已了。
        // 情境一：牆磚放在自然草地上——拆完回復成草（restore=Grass≠StoneBrick）。
        assert!(
            demolish_should_remove(Block::StoneBrick as u8, Block::StoneBrick, Block::Grass),
            "牆磚現況正是她放的、拆了會變草 → 拆"
        );
        // 拆完現況已是草（==restore）→ 不再拆（收斂）。
        assert!(
            !demolish_should_remove(Block::StoneBrick as u8, Block::Grass, Block::Grass),
            "已回復成草 → 不再拆"
        );
        // 情境二（churn 病灶）：她放的就是草、restore 也是草——第一次就不拆（拆了世界不變）。
        assert!(
            !demolish_should_remove(Block::Grass as u8, Block::Grass, Block::Grass),
            "花圃草地放在自然草地上：拆了不變 → 一開始就不該拆（根治churn）"
        );
        // 空氣 / 別人的東西照舊不拆（沿用 demolish_allowed 的既有保護）。
        assert!(!demolish_should_remove(Block::Plank as u8, Block::Air, Block::Grass));
        assert!(!demolish_should_remove(Block::Plank as u8, Block::Chest, Block::Grass));
    }

    #[test]
    fn e2e_luna_repair_house_plan_lands_door_and_bed() {
        // ── 補蓋端到端閉環（承接米拉 demolish 收斂→active 釋放→輪到露娜）──────────────
        // prod：露娜（vox_res_0）已歸屬殖民地、家卻不在地塊上（house_missing_near）→ 一旦米拉的
        // demolish 收斂、active 名額釋放，`colonist_house_repair` 就會為她在殖民地地塊上開一張
        // House BuildPlan。本測驗證那張 plan 走**既有建造引擎唯一路徑**（new_plan→generate_blocks）
        // 落下的方塊集合，真的含門(43)+床(45)——即「露娜家終於有門有床」的可觀察閘。
        let mut s = BuildStore::new();
        // 露娜的殖民地小地塊上一處補蓋錨點（座標取自風禾屯地塊帶；new_plan 依錨點確定性生成）。
        let plan = s.new_plan("vox_res_0", BuildKind::House, 469, 8, 173, false, None);
        let has_door = plan.remaining.iter().any(|b| b.b == Block::DoorClosed as u8);
        let has_bed = plan.remaining.iter().any(|b| b.b == Block::Bed as u8);
        assert!(has_door, "補蓋的家必須含門(43)——露娜的家能被走進去");
        assert!(has_bed, "補蓋的家必須含床(45)——露娜真的有得住");
        // 且 House 型態的門/床座標可由 house_blocks_at 確定性重算（完工後拆/搬皆冪等）。
        let recomputed = house_blocks_at("vox_res_0", 469, 8, 173);
        assert!(recomputed.iter().any(|b| b.b == Block::DoorClosed as u8));
        assert!(recomputed.iter().any(|b| b.b == Block::Bed as u8));
    }

    #[test]
    fn house_blocks_at_matches_what_new_plan_built() {
        // 拆除清單必須與「當年建造引擎放下的那份清單」逐塊一致——這是「只拆她自己家」的根基。
        let mut s = BuildStore::new();
        let plan = s.new_plan("vox_res_0", BuildKind::House, -150, 9, 80, false, None);
        let built: Vec<BuildBlock> = plan.remaining.iter().cloned().collect();
        let recomputed = house_blocks_at("vox_res_0", -150, 9, 80);
        assert_eq!(recomputed, built, "重算的舊家方塊集合應與建造計畫逐塊一致");
    }

    #[test]
    fn house_blocks_at_is_deterministic_and_owner_specific() {
        let a = house_blocks_at("vox_res_0", 30, 9, -40);
        let b = house_blocks_at("vox_res_0", 30, 9, -40);
        assert_eq!(a, b, "同居民同錨點重算永遠一致（中斷可冪等恢復）");
        // 換一位居民（同錨點）通常是另一份藍圖——集合綁定「誰的家」，不是「哪裡有房」。
        let c = house_blocks_at("vox_res_3", 30, 9, -40);
        assert_ne!(a, c, "不同居民的家藍圖應不同（樣式因人而異）");
    }

    #[test]
    fn demolish_allowed_only_exact_match() {
        // 現況正是她放的那塊 → 准拆。
        assert!(demolish_allowed(Block::Plank as u8, Block::Plank));
        assert!(demolish_allowed(Block::Bed as u8, Block::Bed));
        // 型別不符（玩家/鄰居後放的東西、路面）→ 一律不拆。
        assert!(!demolish_allowed(Block::Plank as u8, Block::Chest));
        assert!(!demolish_allowed(Block::Plank as u8, Block::Sign));
        assert!(!demolish_allowed(Block::Plank as u8, Block::SmoothStone));
        assert!(!demolish_allowed(Block::Wood as u8, Block::Plank));
    }

    #[test]
    fn demolish_allowed_skips_air_and_water() {
        // 已是空氣（早拆過/被挖走）或流動水 → 沒東西可拆。
        assert!(!demolish_allowed(Block::Plank as u8, Block::Air));
        assert!(!demolish_allowed(Block::Plank as u8, Block::WaterFlow3));
        assert!(!demolish_allowed(Block::Plank as u8, Block::Water));
    }

    #[test]
    fn demolish_allowed_tolerates_opened_door() {
        // 她蓋的是關著的門，有人開過變 DoorOpen——仍是她的門，准拆。
        assert!(demolish_allowed(Block::DoorClosed as u8, Block::DoorOpen));
        // 但反向不寬容：計畫不是門的格，現況是門 → 不拆（那是別人的門）。
        assert!(!demolish_allowed(Block::Plank as u8, Block::DoorOpen));
        assert!(!demolish_allowed(Block::Plank as u8, Block::DoorClosed));
    }

    #[test]
    fn demolition_yield_returns_expected_material() {
        assert_eq!(demolition_yield(Block::Plank as u8), Block::Plank as u8);
        // 開著的門收回的是門本身。
        assert_eq!(demolition_yield(Block::DoorClosed as u8), Block::DoorClosed as u8);
    }

    #[test]
    fn demolition_restore_returns_solid_base_or_air() {
        // 地表層（surface_y-1）自然基底是實心 → 回復基底（地表復原、不留坑）。
        let sy = surface_y(0, 0);
        let ground = demolition_restore(0, sy - 1, 0);
        assert!(ground.is_solid() && ground.is_placeable(), "地表層應回復實心基底");
        assert_eq!(ground, block_at(0, sy - 1, 0), "回復的正是自然程序基底");
        // 地表之上自然是空氣 → 回 Air（牆/屋頂拆掉就是空）。
        assert_eq!(demolition_restore(0, sy + 2, 0), Block::Air);
    }

    // ── surface_y 純函式 ──────────────────────────────────────────────────────

    #[test]
    fn surface_y_above_solid() {
        // 程序地形在 BASE_HEIGHT 附近有固體，surface_y 應回 > 0
        let sy = surface_y(0, 0);
        assert!(sy > 0, "surface_y 應在地表以上");
        // sy-1 應是固體方塊
        assert!(block_at(0, sy - 1, 0).is_solid(), "surface_y-1 應是固體");
        // sy 本身應是空氣（站立處）
        assert!(!block_at(0, sy, 0).is_solid(), "surface_y 本身應是空氣");
    }

    #[test]
    fn build_anchor_offset_covers_four_directions() {
        let offsets: Vec<(i32, i32)> = (0..4).map(build_anchor_offset).collect();
        let mut xs: Vec<i32> = offsets.iter().map(|&(x, _)| x).collect();
        let mut zs: Vec<i32> = offsets.iter().map(|&(_, z)| z).collect();
        xs.sort();
        zs.sort();
        // 四個方向應有 -6, 0, 0, 6 各一
        assert_eq!(xs, vec![-6, 0, 0, 6]);
        assert_eq!(zs, vec![-6, 0, 0, 6]);
    }

    // ── compact_world_blocks：A==B 等價驗證（M4 資料安全閘）────────────────

    use std::collections::HashMap;

    /// 把 entries 轉成 「(x,y,z) → b」的最終狀態 map（last-write-wins）。
    fn final_block_map(entries: &[BuildBlock]) -> HashMap<(i32, i32, i32), u8> {
        let mut m = HashMap::new();
        for bb in entries {
            m.insert((bb.x, bb.y, bb.z), bb.b);
        }
        m
    }

    /// 把 BuildBlock slice 序列化成 jsonl tempfile。
    fn write_blocks_tempfile(entries: &[BuildBlock]) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        for bb in entries {
            writeln!(f, "{}", serde_json::to_string(bb).unwrap()).unwrap();
        }
        f
    }

    #[test]
    fn compact_world_blocks_state_equivalent_simple() {
        // 同一座標被改動兩次：只有最後一筆應留存
        let entries = vec![
            BuildBlock { x: 0, y: 0, z: 0, b: 1 },
            BuildBlock { x: 0, y: 0, z: 0, b: 3 }, // 覆蓋
            BuildBlock { x: 1, y: 0, z: 0, b: 2 },
        ];
        let tf = write_blocks_tempfile(&entries);
        let path = tf.path().to_str().unwrap().to_string();

        // 狀態 A：直接 replay 原始 entries
        let state_a = final_block_map(&load_world_blocks_from(&path));

        compact_world_blocks(&path);

        // 狀態 B：replay compact 後的 entries
        let state_b = final_block_map(&load_world_blocks_from(&path));

        assert_eq!(state_a, state_b, "compact 前後現狀應完全等價（A==B）");
    }

    #[test]
    fn compact_world_blocks_reduces_line_count() {
        // 100 筆寫同一座標 → compact 後只剩 1 行
        let entries: Vec<BuildBlock> = (0u8..100)
            .map(|i| BuildBlock { x: 5, y: 5, z: 5, b: i % 8 })
            .collect();
        let tf = write_blocks_tempfile(&entries);
        let path = tf.path().to_str().unwrap().to_string();

        let state_a = final_block_map(&load_world_blocks_from(&path));
        compact_world_blocks(&path);
        let state_b = final_block_map(&load_world_blocks_from(&path));

        assert_eq!(state_a, state_b, "A==B");
        let after = load_world_blocks_from(&path);
        assert_eq!(after.len(), 1, "同一座標 compact 後應只剩 1 行");
    }

    #[test]
    fn compact_world_blocks_many_coords_preserved() {
        // 多個不同座標 + 部分重複 → compact 後現狀完整保留
        let mut entries = Vec::new();
        for x in 0i32..5 {
            // 每個座標寫三次，最後一次決定 b 值
            for round in 0u8..3 {
                entries.push(BuildBlock { x, y: 0, z: 0, b: round + 1 });
            }
        }
        let tf = write_blocks_tempfile(&entries);
        let path = tf.path().to_str().unwrap().to_string();

        let state_a = final_block_map(&load_world_blocks_from(&path));
        compact_world_blocks(&path);
        let state_b = final_block_map(&load_world_blocks_from(&path));

        assert_eq!(state_a, state_b, "A==B");
        let after = load_world_blocks_from(&path);
        // 5 個不同座標 → compact 後剛好 5 行
        assert_eq!(after.len(), 5, "5 個不同座標 compact 後應剩 5 行");
    }
}
