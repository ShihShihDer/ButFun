// ============================================================
// voxel_player_stats.rs — 玩家生存指標第一階段（溫和版）
// ============================================================
// 「乙太方界」的玩家飢餓度＋血量：跟著真系統走（食物已真實、未來怪物需要 HP 地基），
// 但這是**療癒世界不是硬核生存**——懲罰溫和、UI 極簡、重生溫柔。
//
// 本模組是**純邏輯**：飢餓衰減、吃回復＋扣背包、溺水／跌落傷害計算、飽食回血、
// 重生點選擇、持久化往返，全部抽成可測函式，不碰網路/鎖/世界。
// voxel_ws.rs 負責把這些函式接上 tick／訊息／廣播／持久化。
//
// **後端權威**：飢餓/血量/傷害/吃全在伺服器算，客戶端只顯示＋發「吃」請求；
// 客戶端無法自報血量或飢餓。
//
// 別碰居民系統（voxel_feed / npc_needs 是居民的，本模組只管玩家）。

use serde::{Deserialize, Serialize};

/// 血量上限（0~20 半顆制：20 = 10 顆心，對齊 MC 語彙，前端可畫成 10 顆心）。
pub const MAX_HEALTH: u32 = 20;
/// 飢餓度上限（0~100）。
pub const MAX_HUNGER: f32 = 100.0;

/// 飢餓衰減速率（每秒）：節奏抓「遊戲內幾十分鐘才見底」——
/// 100 / 0.055 ≈ 1818 秒 ≈ 30 分鐘從全飽掉到 0。不煩人。
pub const HUNGER_DECAY_PER_SEC: f32 = 0.055;

/// 飽食回血門檻：飢餓 > 此值時緩慢回血（療癒世界，吃飽就自癒）。
pub const REGEN_HUNGER_THRESHOLD: f32 = 70.0;
/// 飽食回血間隔（秒）：每隔這麼久回 1 點血。
pub const REGEN_INTERVAL_SECS: f32 = 4.0;

/// 餓到 0 的移動懲罰係數（移動變慢到 70%）——溫和，不扣血不死。
pub const STARVING_MOVE_MULT: f32 = 0.70;
/// 「飢餓過低」判定門檻：低於此值前端顯示提示、套用移動懲罰。
pub const STARVING_THRESHOLD: f32 = 0.5;

/// 溺水：頭泡在水裡連續多久（秒）後開始扣血。給緩衝、別一沾水就扣。
pub const DROWN_GRACE_SECS: f32 = 6.0;
/// 溺水扣血間隔（秒）：撐過緩衝後，每隔這麼久扣 1 點血。
pub const DROWN_INTERVAL_SECS: f32 = 2.0;

/// 跌落傷害的安全高度（格）：落差 ≤ 此值不痛（療癒世界，日常跳躍/下坡免傷）。
pub const FALL_SAFE_BLOCKS: f32 = 4.0;
/// 跌落每超出安全高度 1 格扣的血（半顆＝1）。溫和：從 8 格高落地約扣 4 點（2 顆心）。
pub const FALL_DAMAGE_PER_BLOCK: f32 = 1.0;

/// 玩家生存指標（後端權威）。持久化跨重登（登入玩家）；訪客 session 內有效。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayerStats {
    /// 血量 0~MAX_HEALTH（半顆制）。
    pub health: u32,
    /// 飢餓度 0~MAX_HUNGER。
    pub hunger: f32,
    /// 飽食回血計時累加器（秒，不廣播、不持久化語意上可省）。
    #[serde(default, skip)]
    pub regen_acc: f32,
    /// 溺水累計秒（頭在水中連續時間，離水歸零；不持久化）。
    #[serde(default, skip)]
    pub drown_acc: f32,
    /// 上次溺水扣血後的計時（不持久化）。
    #[serde(default, skip)]
    pub drown_tick_acc: f32,
}

impl Default for PlayerStats {
    fn default() -> Self {
        Self {
            health: MAX_HEALTH,
            hunger: MAX_HUNGER,
            regen_acc: 0.0,
            drown_acc: 0.0,
            drown_tick_acc: 0.0,
        }
    }
}

impl PlayerStats {
    /// 是否處於「飢餓過低」狀態（前端提示＋移動懲罰）。
    pub fn is_starving(&self) -> bool {
        self.hunger <= STARVING_THRESHOLD
    }

    /// 目前移動速度倍率（餓到 0 慢下來，否則正常）。給伺服器決定是否夾玩家速度／或前端顯示用。
    pub fn move_mult(&self) -> f32 {
        if self.is_starving() {
            STARVING_MOVE_MULT
        } else {
            1.0
        }
    }

    /// 是否已倒下（血歸零 → 觸發溫柔重生）。
    pub fn is_down(&self) -> bool {
        self.health == 0
    }
}

/// 飢餓隨時間衰減 `dt` 秒（夾在 0 以上）。純函式：回傳新飢餓值。
pub fn decay_hunger(hunger: f32, dt: f32) -> f32 {
    (hunger - HUNGER_DECAY_PER_SEC * dt).max(0.0)
}

/// 每個食物的營養值（回復多少飢餓度）。回 None＝非食物、不可吃。
///
/// 對齊 #1089 食物清單＋既有 `voxel_gift::is_food_gift`（實際 id：67=野菜暖湯 STEW、78=莓果醬 JAM）：
///   小麥18（生穀粒，回一點點）、麵包19、胡蘿蔔49、馬鈴薯53、烤地薯64、
///   小魚61、乙太魚62、烤魚63、野菜暖湯67、莓果77、果醬78。
/// 熟食／加工品回復多；生食回復少——鼓勵烹飪但生吃也管一點用。
/// 註：68=乙太煙火（非食物，不可吃）。
pub fn food_nutrition(block_id: u8) -> Option<f32> {
    let v = match block_id {
        18 => 8.0,  // 小麥（生穀粒，勉強充飢）
        19 => 25.0, // 麵包（主食·熟食）
        49 => 14.0, // 胡蘿蔔（生蔬）
        53 => 12.0, // 馬鈴薯（生）
        64 => 28.0, // 烤地薯（熟食，最飽）
        61 => 10.0, // 小魚（生）
        62 => 16.0, // 乙太魚（生，稀有）
        63 => 26.0, // 烤魚（熟食）
        67 => 30.0, // 野菜暖湯（三種作物慢燉的熟食，最暖最飽）
        77 => 10.0, // 莓果（生，#1089 清單）
        78 => 22.0, // 果醬（加工·甜點，#1089 清單）
        105 => 16.0, // 南瓜（季限作物·秋南瓜 v1；生食料裡最沉甸甸，飽足高於馬鈴薯/胡蘿蔔）
        _ => return None,
    };
    Some(v)
}

/// 是否為可吃的食物。
pub fn is_edible(block_id: u8) -> bool {
    food_nutrition(block_id).is_some()
}

/// 吃一份食物的判定（後端權威）：只有「是食物 ＆ 背包有 ＆ 沒吃飽」才允許扣背包。
///
/// - `have`：玩家背包該食物的存量（呼叫端從 InvStore 讀）。
/// - 回 `Some(new_hunger)`：吃成功，新飢餓值；呼叫端據此扣背包 1 個、更新飢餓、廣播。
/// - 回 `None`：不可吃（非食物／背包沒有／已全飽），呼叫端不動背包。
///
/// **濫用防護**：飢餓已滿（MAX）就不給吃——避免無限吃刷、也符合直覺（飽了吃不下）。
pub fn try_eat(block_id: u8, have: u32, hunger: f32) -> Option<f32> {
    let nutrition = food_nutrition(block_id)?;
    if have == 0 {
        return None;
    }
    // 已全飽就吃不下（不浪費食物）。用小 epsilon：飢餓顯示會四捨五入成整數，
    // 若只差不到 0.5 就當作滿了——顯示 100 時吃不下，與畫面一致（不造成「明明滿卻能吃」的困惑）。
    if hunger >= MAX_HUNGER - 0.5 {
        return None;
    }
    Some((hunger + nutrition).min(MAX_HUNGER))
}

/// 跌落傷害計算（純函式）：落差 `fall_blocks`（峰值 y − 落地 y，格）→ 扣血點數。
/// 安全高度內回 0。溫和：超出安全高度的每格扣 FALL_DAMAGE_PER_BLOCK。
pub fn fall_damage(fall_blocks: f32) -> u32 {
    if fall_blocks <= FALL_SAFE_BLOCKS {
        return 0;
    }
    let over = fall_blocks - FALL_SAFE_BLOCKS;
    (over * FALL_DAMAGE_PER_BLOCK).round() as u32
}

/// 套用傷害（夾在 0 以上），回新血量。傷害受傷時清 regen 累加器由呼叫端處理。
pub fn apply_damage(health: u32, dmg: u32) -> u32 {
    health.saturating_sub(dmg)
}

/// 溺水推進一 tick（純函式）：更新溺水累加器＋判定這 tick 是否該扣血。
///
/// - `head_in_water`：這 tick 頭是否在水裡（呼叫端採樣頭部方塊）。
/// - 回 `(new_drown_acc, new_tick_acc, damage)`：新累加器們＋這 tick 要扣的血（0 或 1）。
/// - 撐過 DROWN_GRACE_SECS 後，每 DROWN_INTERVAL_SECS 扣 1。離水立即歸零（給喘息）。
pub fn tick_drown(head_in_water: bool, drown_acc: f32, tick_acc: f32, dt: f32) -> (f32, f32, u32) {
    if !head_in_water {
        return (0.0, 0.0, 0);
    }
    let new_drown = drown_acc + dt;
    if new_drown < DROWN_GRACE_SECS {
        return (new_drown, 0.0, 0);
    }
    let new_tick = tick_acc + dt;
    if new_tick >= DROWN_INTERVAL_SECS {
        (new_drown, new_tick - DROWN_INTERVAL_SECS, 1)
    } else {
        (new_drown, new_tick, 0)
    }
}

/// 飽食回血推進一 tick（純函式）：飢餓 > 門檻且未滿血 → 累加，每 REGEN_INTERVAL_SECS 回 1。
///
/// - 回 `(new_regen_acc, heal)`：新累加器＋這 tick 要回的血（0 或 1）。
pub fn tick_regen(hunger: f32, health: u32, regen_acc: f32, dt: f32) -> (f32, u32) {
    if hunger <= REGEN_HUNGER_THRESHOLD || health >= MAX_HEALTH {
        return (0.0, 0); // 沒飽或已滿血：不回、歸零累加器
    }
    let acc = regen_acc + dt;
    if acc >= REGEN_INTERVAL_SECS {
        (acc - REGEN_INTERVAL_SECS, 1)
    } else {
        (acc, 0)
    }
}

/// 重生點候選：床頭優先，否則村莊廣場（spawn）。純選擇邏輯。
///
/// - `bed`：玩家最近睡過的床座標（若有，`Some((x,y,z))`）。
/// - `plaza`：村莊廣場／預設出生點座標。
/// - 回實際重生座標。療癒世界：**背包不掉落**（由呼叫端保證不動背包）。
pub fn respawn_point(
    bed: Option<(f32, f32, f32)>,
    plaza: (f32, f32, f32),
) -> (f32, f32, f32) {
    bed.unwrap_or(plaza)
}

/// 溫柔重生後的滿血滿飢狀態（血飢回滿、清所有累加器）。
pub fn revived_stats() -> PlayerStats {
    PlayerStats::default()
}

// ── 溫泉遺跡 v1（世界第二種可探索地標，自主提案切片）─────────────────────────
// 走遠巧遇溫泉、泡進去有實際功能回饋：不必等飽食就能回血、回得更快，飢餓也消耗得慢——
// 像在休息。與一般飽食回血（療癒但被動）刻意區隔：這是「走遠探索換來的主動獎賞」。

/// 泡溫泉時的回血門檻：比一般飽食回血（[`REGEN_HUNGER_THRESHOLD`]=70）寬鬆得多——
/// 只要沒餓到瀕臨見底就能回，不必特地先吃飽才能去泡。
pub const HOT_SPRING_REGEN_HUNGER_THRESHOLD: f32 = 20.0;
/// 泡溫泉時的回血間隔（秒）：比一般（[`REGEN_INTERVAL_SECS`]=4.0）快上不少，泡一下子就有感。
pub const HOT_SPRING_REGEN_INTERVAL_SECS: f32 = 1.5;
/// 泡溫泉時飢餓消耗的倍率：只剩正常速率的 35%，像在暖泉裡歇著、不太耗體力。
pub const HOT_SPRING_HUNGER_DECAY_MULT: f32 = 0.35;

/// 飢餓衰減（泡溫泉版）：`soaking=true` 時消耗速率打折（見 [`HOT_SPRING_HUNGER_DECAY_MULT`]），
/// 否則與 [`decay_hunger`] 完全一致（零回歸）。
pub fn decay_hunger_soaking(hunger: f32, dt: f32, soaking: bool) -> f32 {
    if !soaking {
        return decay_hunger(hunger, dt);
    }
    (hunger - HUNGER_DECAY_PER_SEC * HOT_SPRING_HUNGER_DECAY_MULT * dt).max(0.0)
}

/// 飽食回血推進一 tick（泡溫泉版）：`soaking=true` 時門檻更寬鬆、回血更快
/// （見 [`HOT_SPRING_REGEN_HUNGER_THRESHOLD`]／[`HOT_SPRING_REGEN_INTERVAL_SECS`]），
/// 否則與 [`tick_regen`] 完全一致（零回歸）。
pub fn tick_regen_soaking(hunger: f32, health: u32, regen_acc: f32, dt: f32, soaking: bool) -> (f32, u32) {
    if !soaking {
        return tick_regen(hunger, health, regen_acc, dt);
    }
    if hunger <= HOT_SPRING_REGEN_HUNGER_THRESHOLD || health >= MAX_HEALTH {
        return (0.0, 0);
    }
    let acc = regen_acc + dt;
    if acc >= HOT_SPRING_REGEN_INTERVAL_SECS {
        (acc - HOT_SPRING_REGEN_INTERVAL_SECS, 1)
    } else {
        (acc, 0)
    }
}

/// 溫暖的重生提示語（i18n：目前繁中，字串集中此處便於日後在地化）。
/// `pick` 由呼叫端提供（確定性、不走 random），輪替幾句避免每次一樣。
pub fn respawn_message(pick: usize) -> &'static str {
    const POOL: &[&str] = &[
        "你在溫暖的爐火邊醒來，身上的疲憊都散了。",
        "一陣暖意把你喚醒——你安然回到了村莊，什麼都沒少。",
        "你在柔軟的被褥裡睜開眼，星光正好，該重新出發了。",
        "爐火劈啪作響，你被暖醒了；行囊還在身邊，一切安好。",
    ];
    POOL[pick % POOL.len()]
}

// ============================================================
// 持久化往返（比照 #1024 位置持久化風格：jsonl 一行一玩家）
// ============================================================

/// 持久化到磁碟的一行（玩家名 → 血/飢）。只存需要跨重登的欄位（血、飢），
/// 累加器（regen/drown）是 session 內暫態，不持久化（重登從 0 累）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsRow {
    /// 玩家顯示名（索引鍵；登入玩家綁帳號名，穩定）。
    pub player: String,
    /// 血量。
    pub health: u32,
    /// 飢餓度。
    pub hunger: f32,
}

impl StatsRow {
    /// 從 PlayerStats + 玩家名組一行（存檔用）。
    pub fn from_stats(player: &str, s: &PlayerStats) -> Self {
        Self {
            player: player.to_string(),
            health: s.health,
            hunger: s.hunger,
        }
    }

    /// 還原成 PlayerStats（載入用；累加器歸零，血/飢夾在合法範圍內防髒資料）。
    pub fn to_stats(&self) -> PlayerStats {
        PlayerStats {
            health: self.health.min(MAX_HEALTH),
            hunger: self.hunger.clamp(0.0, MAX_HUNGER),
            regen_acc: 0.0,
            drown_acc: 0.0,
            drown_tick_acc: 0.0,
        }
    }
}

/// 把一列 StatsRow 序列化成 jsonl 文字（存檔用，一行一玩家）。純函式便於測往返。
pub fn serialize_rows(rows: &[StatsRow]) -> String {
    let mut out = String::new();
    for r in rows {
        if let Ok(line) = serde_json::to_string(r) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// 從 jsonl 文字解析回 StatsRow（載入用）。壞行略過（韌性：髒資料不 panic）。
pub fn parse_rows(text: &str) -> Vec<StatsRow> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                return None;
            }
            serde_json::from_str::<StatsRow>(l).ok()
        })
        .collect()
}

// ============================================================
// 測試
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_full() {
        let s = PlayerStats::default();
        assert_eq!(s.health, MAX_HEALTH);
        assert_eq!(s.hunger, MAX_HUNGER);
        assert!(!s.is_starving());
        assert!(!s.is_down());
        assert_eq!(s.move_mult(), 1.0);
    }

    #[test]
    fn hunger_decays_slowly_over_tens_of_minutes() {
        // 30 分鐘（1800 秒）約掉到 0，但 5 分鐘還遠沒見底（不煩人）。
        let after_5min = {
            let mut h = MAX_HUNGER;
            for _ in 0..(5 * 60 * 10) {
                h = decay_hunger(h, 0.1);
            }
            h
        };
        assert!(after_5min > 60.0, "5 分鐘後仍應相當飽：{after_5min}");

        let after_30min = {
            let mut h = MAX_HUNGER;
            for _ in 0..(30 * 60 * 10) {
                h = decay_hunger(h, 0.1);
            }
            h
        };
        assert!(after_30min <= 5.0, "30 分鐘後應接近見底：{after_30min}");
    }

    #[test]
    fn hunger_never_below_zero() {
        assert_eq!(decay_hunger(0.0, 100.0), 0.0);
        assert_eq!(decay_hunger(1.0, 10000.0), 0.0);
    }

    #[test]
    fn starving_slows_movement_but_no_death() {
        let s = PlayerStats { hunger: 0.0, ..Default::default() };
        assert!(s.is_starving());
        assert_eq!(s.move_mult(), STARVING_MOVE_MULT);
        assert!(!s.is_down(), "餓到 0 不死（溫和）");
    }

    #[test]
    fn food_ids_align_with_1089_list() {
        // #1089 清單：小麥18/麵包19/胡蘿蔔49/馬鈴薯53/烤地薯64/莓果77/果醬78。
        for id in [18u8, 19, 49, 53, 64, 77, 78] {
            assert!(is_edible(id), "{id} 應可吃");
            assert!(food_nutrition(id).unwrap() > 0.0);
        }
        // 魚類與野菜暖湯也是食物（對齊 is_food_gift：61/62/63 魚、67 暖湯）。
        for id in [61u8, 62, 63, 67] {
            assert!(is_edible(id), "食物 {id} 應可吃");
        }
        // 非食物不可吃（含 68 乙太煙火——它接在 STEW_ID=67 之後但不是食物）。
        for id in [0u8, 1, 3, 5, 15, 16, 45, 68] {
            assert!(!is_edible(id), "{id} 不該可吃");
            assert!(food_nutrition(id).is_none());
        }
    }

    #[test]
    fn cooked_food_more_filling_than_raw() {
        // 熟食比生食更飽（鼓勵烹飪）。
        assert!(food_nutrition(64).unwrap() > food_nutrition(53).unwrap(), "烤地薯 > 生馬鈴薯");
        assert!(food_nutrition(63).unwrap() > food_nutrition(61).unwrap(), "烤魚 > 生魚");
        assert!(food_nutrition(67).unwrap() > food_nutrition(49).unwrap(), "野菜暖湯 > 生胡蘿蔔");
    }

    #[test]
    fn eat_restores_hunger_and_requires_inventory() {
        // 有背包＋沒吃飽 → 吃成功，飢餓回復（不超上限）。
        let after = try_eat(19, 3, 50.0).unwrap();
        assert_eq!(after, 50.0 + 25.0);

        // 回復不超過上限。
        let capped = try_eat(19, 1, 90.0).unwrap();
        assert_eq!(capped, MAX_HUNGER);

        // 背包沒有 → 不可吃（後端權威：不信客戶端）。
        assert!(try_eat(19, 0, 50.0).is_none());

        // 非食物 → 不可吃。
        assert!(try_eat(5, 3, 50.0).is_none());

        // 已全飽 → 吃不下（防無限吃刷）。
        assert!(try_eat(19, 3, MAX_HUNGER).is_none());
    }

    #[test]
    fn fall_damage_safe_within_threshold() {
        // 安全高度內不痛（日常跳躍/下坡）。
        assert_eq!(fall_damage(0.0), 0);
        assert_eq!(fall_damage(4.0), 0);
        assert_eq!(fall_damage(FALL_SAFE_BLOCKS), 0);
    }

    #[test]
    fn fall_damage_scales_above_threshold() {
        // 8 格高落地：超出 4 格 → 扣 4 點（2 顆心），溫和。
        assert_eq!(fall_damage(8.0), 4);
        // 6 格：超出 2 → 扣 2。
        assert_eq!(fall_damage(6.0), 2);
        // 越高越痛但不會秒殺（20 格高 → 16 點，仍留 4 點）。
        assert_eq!(fall_damage(20.0), 16);
    }

    #[test]
    fn apply_damage_floors_at_zero() {
        assert_eq!(apply_damage(20, 5), 15);
        assert_eq!(apply_damage(3, 10), 0);
        assert_eq!(apply_damage(0, 5), 0);
    }

    #[test]
    fn drown_has_grace_then_ticks() {
        // 前 DROWN_GRACE_SECS 秒不扣血（給緩衝）。
        let (acc, tick, dmg) = tick_drown(true, 0.0, 0.0, 0.1);
        assert!(dmg == 0 && acc > 0.0 && tick == 0.0);

        // 累到剛好過緩衝：仍未到扣血間隔 → 0。
        let (acc2, _tick2, dmg2) = tick_drown(true, DROWN_GRACE_SECS, 0.0, 0.1);
        assert_eq!(dmg2, 0);
        assert!(acc2 > DROWN_GRACE_SECS);

        // 過緩衝且累到間隔 → 扣 1。
        let (_a, tick3, dmg3) = tick_drown(true, DROWN_GRACE_SECS + 1.0, DROWN_INTERVAL_SECS, 0.1);
        assert_eq!(dmg3, 1);
        assert!(tick3 < DROWN_INTERVAL_SECS, "扣血後 tick 累加器該回捲");
    }

    #[test]
    fn drown_resets_on_surfacing() {
        // 離水立即歸零（喘息）。
        let (acc, tick, dmg) = tick_drown(false, 5.0, 1.0, 0.1);
        assert_eq!((acc, tick, dmg), (0.0, 0.0, 0));
    }

    #[test]
    fn drown_full_cycle_takes_expected_time() {
        // 從入水到第一次扣血該花 GRACE 秒，之後每 INTERVAL 秒扣一次。
        let mut acc = 0.0;
        let mut tick = 0.0;
        let mut first_dmg_at = None;
        for i in 0..300 {
            let (a, tk, d) = tick_drown(true, acc, tick, 0.1);
            acc = a;
            tick = tk;
            if d > 0 && first_dmg_at.is_none() {
                first_dmg_at = Some((i + 1) as f32 * 0.1);
                break;
            }
        }
        let first = first_dmg_at.expect("該在合理時間內開始扣血");
        assert!(first >= DROWN_GRACE_SECS, "首次扣血不早於緩衝：{first}");
        assert!(first <= DROWN_GRACE_SECS + DROWN_INTERVAL_SECS + 0.2, "首次扣血不晚太多：{first}");
    }

    #[test]
    fn regen_when_full_fed_only() {
        // 飽食（>門檻）且未滿血 → 累到間隔回 1。
        let (acc, heal) = tick_regen(80.0, 10, REGEN_INTERVAL_SECS, 0.1);
        assert_eq!(heal, 1);
        assert!(acc < REGEN_INTERVAL_SECS);

        // 沒飽 → 不回。
        let (_a, heal2) = tick_regen(50.0, 10, REGEN_INTERVAL_SECS, 0.1);
        assert_eq!(heal2, 0);

        // 已滿血 → 不回。
        let (_a, heal3) = tick_regen(90.0, MAX_HEALTH, REGEN_INTERVAL_SECS, 0.1);
        assert_eq!(heal3, 0);
    }

    #[test]
    fn regen_takes_expected_time() {
        // 飽食下每 REGEN_INTERVAL_SECS 回 1，10 秒約回 2~3 點。
        let mut acc = 0.0;
        let mut healed = 0u32;
        for _ in 0..100 {
            let (a, h) = tick_regen(90.0, 10, acc, 0.1);
            acc = a;
            healed += h;
        }
        assert!(healed >= 2 && healed <= 3, "10 秒飽食回血約 2~3 點：{healed}");
    }

    #[test]
    fn respawn_prefers_bed_else_plaza() {
        let plaza = (0.0, 64.0, 0.0);
        // 有床 → 回床。
        assert_eq!(respawn_point(Some((10.0, 65.0, 20.0)), plaza), (10.0, 65.0, 20.0));
        // 沒床 → 回廣場。
        assert_eq!(respawn_point(None, plaza), plaza);
    }

    #[test]
    fn revived_is_full() {
        let s = revived_stats();
        assert_eq!(s.health, MAX_HEALTH);
        assert_eq!(s.hunger, MAX_HUNGER);
        assert_eq!(s.drown_acc, 0.0);
    }

    #[test]
    fn respawn_message_rotates_and_is_warm() {
        let m0 = respawn_message(0);
        let m1 = respawn_message(1);
        assert_ne!(m0, m1);
        // 輪替回頭。
        assert_eq!(respawn_message(0), respawn_message(4));
        assert!(!m0.is_empty());
    }

    #[test]
    fn persistence_round_trip() {
        // 存 → 載，血/飢完整保留。
        let s = PlayerStats { health: 13, hunger: 42.5, ..Default::default() };
        let row = StatsRow::from_stats("露娜", &s);
        let text = serialize_rows(&[row]);
        let parsed = parse_rows(&text);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].player, "露娜");
        let back = parsed[0].to_stats();
        assert_eq!(back.health, 13);
        assert_eq!(back.hunger, 42.5);
        // 累加器不持久化，載回為 0。
        assert_eq!(back.drown_acc, 0.0);
        assert_eq!(back.regen_acc, 0.0);
    }

    #[test]
    fn persistence_clamps_dirty_data() {
        // 髒資料（超上限）載入時夾回合法範圍，不 panic。
        let dirty = StatsRow { player: "x".into(), health: 999, hunger: 500.0 };
        let s = dirty.to_stats();
        assert_eq!(s.health, MAX_HEALTH);
        assert_eq!(s.hunger, MAX_HUNGER);

        // 壞行略過。
        let text = "not json\n{\"player\":\"a\",\"health\":10,\"hunger\":30.0}\n\n";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].player, "a");
    }

    #[test]
    fn multi_player_round_trip() {
        let rows = vec![
            StatsRow { player: "甲".into(), health: 20, hunger: 100.0 },
            StatsRow { player: "乙".into(), health: 5, hunger: 12.0 },
        ];
        let text = serialize_rows(&rows);
        let parsed = parse_rows(&text);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].player, "乙");
        assert_eq!(parsed[1].health, 5);
    }

    // ── 溫泉遺跡 v1（自主提案切片）───────────────────────────────────────────

    #[test]
    fn soaking_false_matches_plain_functions_exactly() {
        // soaking=false 時兩個新函式必須與原函式逐位元一致（零回歸保證）。
        assert_eq!(decay_hunger_soaking(50.0, 3.0, false), decay_hunger(50.0, 3.0));
        assert_eq!(
            tick_regen_soaking(80.0, 10, REGEN_INTERVAL_SECS, 0.1, false),
            tick_regen(80.0, 10, REGEN_INTERVAL_SECS, 0.1)
        );
    }

    #[test]
    fn soaking_slows_hunger_decay() {
        // 同樣 dt 下，泡溫泉的飢餓消耗應明顯少於平常（約 35%）。
        let normal = decay_hunger(100.0, 10.0);
        let soaking = decay_hunger_soaking(100.0, 10.0, true);
        assert!(soaking > normal, "泡溫泉時飢餓消耗應變慢：soaking={soaking} normal={normal}");
        let normal_drop = 100.0 - normal;
        let soaking_drop = 100.0 - soaking;
        assert!(
            (soaking_drop - normal_drop * HOT_SPRING_HUNGER_DECAY_MULT).abs() < 1e-4,
            "泡溫泉消耗應恰為平常的 {HOT_SPRING_HUNGER_DECAY_MULT} 倍"
        );
    }

    #[test]
    fn soaking_regens_below_normal_threshold_and_faster() {
        // 沒吃飽（低於一般 70 門檻，但高於溫泉寬鬆門檻 20）平常不會回血，泡溫泉會。
        let (_acc, heal_normal) = tick_regen(40.0, 10, REGEN_INTERVAL_SECS, 0.1);
        assert_eq!(heal_normal, 0, "沒吃飽時一般回血不該觸發");
        let (acc_soak, heal_soak) =
            tick_regen_soaking(40.0, 10, HOT_SPRING_REGEN_INTERVAL_SECS, 0.1, true);
        assert_eq!(heal_soak, 1, "泡溫泉門檻寬鬆，同樣飢餓值應能回血");
        assert!(acc_soak < HOT_SPRING_REGEN_INTERVAL_SECS);

        // 太餓（低於溫泉門檻 20）連泡溫泉也不回，飢餓仍是硬底線。
        let (_acc2, heal_too_hungry) =
            tick_regen_soaking(10.0, 10, HOT_SPRING_REGEN_INTERVAL_SECS, 0.1, true);
        assert_eq!(heal_too_hungry, 0, "太餓時泡溫泉也不該回血");

        // 已滿血：泡溫泉也不該多回（不是無上限外掛）。
        let (_acc3, heal_full) =
            tick_regen_soaking(80.0, MAX_HEALTH, HOT_SPRING_REGEN_INTERVAL_SECS, 0.1, true);
        assert_eq!(heal_full, 0);
    }
}
