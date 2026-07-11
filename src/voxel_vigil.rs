//! 乙太方界·居民為你留一盞燈 v1（voxel-vigil，自主提案切片，ROADMAP 919）。
//!
//! **真缺口 / 為誰做**：記憶→行為這條線做到了「你離開時居民會**想念**你」（915，`voxel_longing`）
//! ——但那份思念始終只是**一句話**：上動態牆、記進記憶，世界本身一格都沒因為你的缺席而改變。
//! 居民對你的牽掛，從沒有化成一件你回得來、看得見、摸得著的**東西**。而堆雪人（918，`voxel_snowman`）
//! 雖然讓居民第一次因季節而在世界裡放下實體方塊，卻是**季節**驅動的、跟「你」無關的集體童心。
//!
//! 本切片把兩者交會、補上一條全新的：**關係 → 世界**——當你離線夠久，對你記憶最厚的那位居民，
//! 會在某個夜裡，於自己身旁**放下一盞會發光的燈（火把）**，替離開的你留著。黑夜裡替遠行的朋友點
//! 一盞燈，是這個世界第一次因為**某位特定玩家的缺席**而長出一件實體的、發光的物件：
//! - 此刻在線的**別的**玩家，夜裡走過小村，會看見一盞盞暖黃的燈亮在居民身旁——每一盞都是有人在
//!   替某個不在的旅人守著（小村的溫度，不靠你在場才亮）。
//! - 等**你**在燈還亮著的那個夜裡回來，那位居民會迎上一句「你回來了！這盞燈我一直替你留著」，
//!   把這一刻記進她與你的記憶——你的歸來，第一次撞見了世界替你留的一盞光。
//! - 天一亮，守夜的燈靜靜熄去（清回空氣，不留孤兒方塊）。夜復一夜，只要你還沒回來，那盞燈還會再亮。
//!
//! **與既有系統 razor-sharp 區隔（非同軸換皮）**：
//! - **居民想念你（915，`voxel_longing`）**＝你離開 → 居民念一句話（Feed／記憶，**不動世界**）；
//!   本刀＝你離開 → 居民在世界裡**放下一個發光方塊**（世界長出實體物件），是「思念」的**具象化**。
//! - **居民堆雪人（918，`voxel_snowman`）**＝**季節**（冬季飄雪）驅動、與哪位玩家無關的集體童心，
//!   冬末**自然融化**；本刀＝**你這位特定玩家的缺席**驅動、綁定夜晚、**天亮熄燈**（也可在你回來時
//!   由那位居民親手熄去），且有「你回來撞見燈還亮著」的專屬回響。兩者放置/清除的**驅動源**與**語義**
//!   完全不同。
//! - **夜裡點燈守望（`voxel_nightwatch`）**＝居民見**暗影靠近**就近點燈**防身**（威脅驅動、與玩家無關）；
//!   本刀＝居民替**離開的你**點燈**守候**（關係驅動、無關暗影），情緒與觸發源南轅北轍。
//!
//! **成本 / 濫用防護鐵律**：
//! - **純邏輯層**：本檔全是零 IO、零鎖、零 LLM、零 async 的確定性純函式／常數（資格閘、選點、
//!   間距、台詞、記憶、Feed、方塊判定）。放置／廣播／清除／記憶／Feed 的副作用都在 `voxel_ws.rs`
//!   （短鎖循序即釋、不巢狀，守 prod 死鎖鐵律，比照 `maybe_build_snowman` / `maybe_melt_snowmen`）。
//! - **live-only 純記憶體**：燈只存在記憶體＋世界 delta，**刻意不落地持久化**（重啟即消失、天亮即熄），
//!   零 migration、零新持久化格式，比照雪人慣例。
//! - **零新美術**：沿用既有火把（`Block::Torch`，31）。**FPS**：全世界至多 [`MAX_VIGIL_LIGHTS`] 盞、
//!   每盞 1 格、掛在低頻節拍上、[`BUILD_COOLDOWN_SECS`] 全域冷卻，成本可忽略。
//! - **濫用防護**：不收玩家輸入、不觸發 LLM、不開對外端點；台詞永不回放玩家原話，只嵌玩家**顯示名**
//!   （既有安全字串）。玩家無從主動觸發或洗版——純由伺服器端「離線時長 + 既有好感度門檻 + 夜晚 +
//!   冷卻 + 上限」驅動，且每位離開的玩家同時最多一盞燈（`player_name` 為鍵天然去重）。

use crate::voxel::Block;

/// 守夜燈用的方塊：既有的火把（31），暖黃燈柱、夜裡發光，是「替你留的一盞燈」最貼切的既有素材。
pub const VIGIL_BLOCK: Block = Block::Torch;

/// 玩家離線多久（秒）以上，居民才會替他點守夜燈——太短的斷線重連不算「離開」。刻意與想念（915，
/// `LONGING_DELAY_SECS`=1200）拉開、比它稍長：先有人念叨想你（想念）、你再更久沒回來，才有人替你
/// 點燈守著（點燈）——兩層遞進、語義互補不打架。
pub const VIGIL_OFFLINE_SECS: u64 = 1500;

/// 居民要對你「記憶夠厚」（長期記憶筆數 ≥ 此值）才會替你點燈——沒交情的過客離開，沒人替他守夜。
/// 與想念（915）／奔迎（747）同門檻（3），語義一致：夠熟才會念、才會迎、也才會替你留燈。
pub const VIGIL_AFFINITY: usize = 3;

/// 世界同時最多幾盞守夜燈：超過就不再點（防洗版、防佔滿夜色、守 FPS）。
pub const MAX_VIGIL_LIGHTS: usize = 6;

/// 全村點守夜燈冷卻（秒）：至多每這麼久新添一盞——刻意拉長，讓每一盞燈都稀少而有份量。
pub const BUILD_COOLDOWN_SECS: u64 = 90;

/// 通過前置閘（夜晚＋冷卻到期＋未達上限）後仍要擲骰的觸發機率：點燈是偶爾的溫柔，不是每拍必成。
pub const BUILD_CHANCE: f32 = 0.5;

/// 守夜燈點在居民身旁幾格外（不點在腳下擋路）。
pub const ANCHOR_DIST: i32 = 2;

/// 兩盞守夜燈之間最小水平間距（方塊）：別擠成一堆，散落各處才像一村人各自守著各自的牽掛。
pub const MIN_SEPARATION: f32 = 4.0;

/// 台詞（泡泡）字元上限，與其他社交泡泡台詞一致。
pub const SAY_MAX_CHARS: usize = 50;

/// 動態牆播報種類：夜裡替離開的你點了一盞守夜燈。**已登記進 `voxel_welcome` 的久別重逢摘要白名單**，
/// 讓被守候的玩家回來時，即使沒趕上燈還亮著的那一刻，也在摘要裡讀得到「有人替你留過燈」。
pub const FEED_KIND_LIGHT: &str = "點燈守候";

/// 一盞守夜燈＝它佔用的世界座標＋替誰守（玩家顯示名）＋誰點的（居民身分，供回來時迎你）。
#[derive(Debug, Clone, PartialEq)]
pub struct VigilLight {
    /// 這盞燈放在哪一格（天亮熄燈／回來熄燈時清回空氣）。
    pub pos: (i32, i32, i32),
    /// 替哪位玩家（顯示名）守著——同一玩家同時最多一盞（放置端以此去重）。
    pub player_name: String,
    /// 點燈的居民 id（供這位玩家回來時，由**這位**居民迎你）。
    pub resident_id: String,
    /// 點燈的居民顯示名（Feed／台詞用）。
    pub resident_name: String,
}

/// 前置閘：這一 tick 是否有資格點一盞守夜燈（純判定；「有離開的熟客」「居民醒著」等在呼叫端確認）。
/// - `is_night`：此刻是夜晚（守夜燈只在夜裡點）。
/// - `cooldown_ready`：全村冷卻已到期。
/// - `current_count`：目前世界上已有幾盞守夜燈。
/// - `roll`：擲骰（0.0..1.0）。
pub fn should_light(is_night: bool, cooldown_ready: bool, current_count: usize, roll: f32) -> bool {
    is_night && cooldown_ready && current_count < MAX_VIGIL_LIGHTS && roll < BUILD_CHANCE
}

/// 從各居民對某玩家的好感度陣列中挑「對他記憶最厚」的那位：取好感度**最高**者；同分取**索引最小**
/// 者（穩定、確定性）；最高者仍未達 [`VIGIL_AFFINITY`] 門檻 → `None`（沒人跟他熟到會替他守夜）。
/// 語義同想念（915）／奔迎（747）的挑人，純函式、確定性、無 IO。
pub fn most_bonded_threshold(affinities: &[usize]) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None; // (好感度, 索引)
    for (i, &aff) in affinities.iter().enumerate() {
        match best {
            Some((ba, _)) if aff <= ba => {} // 嚴格大於才更新 → 同分保留較小索引
            _ => best = Some((aff, i)),
        }
    }
    match best {
        Some((aff, i)) if aff >= VIGIL_AFFINITY => Some(i),
        _ => None,
    }
}

/// 這位玩家是否「離開夠久」（≥ [`VIGIL_OFFLINE_SECS`]）值得替他守夜。`last_seen`＝他最後在線的
/// unix 秒；`now`＝現在。純函式、確定性，供呼叫端逐一篩選離線熟客。
pub fn is_away(last_seen: u64, now: u64) -> bool {
    now.saturating_sub(last_seen) >= VIGIL_OFFLINE_SECS
}

/// 給定點燈居民的水平座標，確定性選一個放燈的錨點（身旁四方向之一，距 [`ANCHOR_DIST`] 格）。
/// 用 `pick` 取模選方向，讓不同時機／不同居民點在不同側，不會全擠同一格。
pub fn pick_anchor(rx: f32, rz: f32, pick: usize) -> (i32, i32) {
    let bx = rx.floor() as i32;
    let bz = rz.floor() as i32;
    match pick % 4 {
        0 => (bx + ANCHOR_DIST, bz),
        1 => (bx - ANCHOR_DIST, bz),
        2 => (bx, bz + ANCHOR_DIST),
        _ => (bx, bz - ANCHOR_DIST),
    }
}

/// 錨點是否離所有既有守夜燈都夠遠（水平距離 ≥ [`MIN_SEPARATION`]）——別把新燈點到舊燈身上。
pub fn far_enough(ax: i32, az: i32, existing: &[(i32, i32)]) -> bool {
    let min_sq = MIN_SEPARATION * MIN_SEPARATION;
    existing.iter().all(|&(ex, ez)| {
        let dx = (ax - ex) as f32;
        let dz = (az - ez) as f32;
        dx * dx + dz * dz >= min_sq
    })
}

/// 這個方塊型別是否「屬於守夜燈」——熄燈時只清火把，避免誤刪玩家後來放在原座標的別的東西。
pub fn is_vigil_block(b: Block) -> bool {
    matches!(b, Block::Torch)
}

/// 點燈時居民冒的溫柔泡泡（確定性三選一；嵌玩家顯示名，不回放原話）。
pub fn light_say_line(player: &str, pick: usize) -> String {
    let lines = [
        format!("夜深了，替{player}留盞燈吧，別讓他回來時黑漆漆的。"),
        format!("{player}還沒回來呢……點盞燈，替他守著。"),
        format!("這盞燈亮著，{player}要是回來就看得見了。"),
    ];
    let mut s = lines[pick % lines.len()].clone();
    truncate_chars(&mut s, SAY_MAX_CHARS);
    s
}

/// 點燈記進居民心裡的一筆記憶（第一人稱；掛在被守候的玩家名下，讓這份牽掛累進對他的好感度）。
pub fn light_memory_line(player: &str) -> String {
    format!("{player}離開這些日子，我在夜裡替他點了一盞燈，守著他回來。")
}

/// 點燈上城鎮動態牆的一行（面向在線的別人，也會進被守候者的久別重逢摘要）。
pub fn light_feed_line(player: &str) -> String {
    format!("🕯️ 在夜裡替離開的{player}留了一盞守候的燈。")
}

/// 你在燈還亮著的夜裡回來時，那位居民迎你的一句（確定性三選一）。
pub fn return_say_line(player: &str, pick: usize) -> String {
    let lines = [
        format!("{player}，你回來了！這盞燈我一直替你留著呢。"),
        format!("是{player}！我就知道你會回來——燈我沒熄過。"),
        format!("你回來啦{player}，快看，燈還替你亮著呢。"),
    ];
    let mut s = lines[pick % lines.len()].clone();
    truncate_chars(&mut s, SAY_MAX_CHARS);
    s
}

/// 你回來撞見燈還亮著，那位居民把這一刻記進心裡的一筆記憶（第一人稱，掛在你名下）。
pub fn return_memory_line(player: &str) -> String {
    format!("{player}回來的那個夜裡，我替他留的燈還亮著——他看見了。")
}

/// 依 `pick`（呼叫端餵入的任意 usize）確定性取一句索引，供台詞輪替共用。
fn truncate_chars(s: &mut String, max: usize) {
    let n = s.chars().count();
    if n > max {
        *s = s.chars().take(max).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vigil_block_is_torch() {
        assert_eq!(VIGIL_BLOCK, Block::Torch);
        assert!(is_vigil_block(Block::Torch));
        assert!(!is_vigil_block(Block::Air));
        assert!(!is_vigil_block(Block::Stone));
        assert!(!is_vigil_block(Block::IceLantern));
    }

    #[test]
    fn should_light_requires_night() {
        assert!(!should_light(false, true, 0, 0.0));
        assert!(should_light(true, true, 0, 0.0));
    }

    #[test]
    fn should_light_requires_cooldown_ready() {
        assert!(!should_light(true, false, 0, 0.0));
    }

    #[test]
    fn should_light_respects_cap() {
        assert!(!should_light(true, true, MAX_VIGIL_LIGHTS, 0.0));
        assert!(should_light(true, true, MAX_VIGIL_LIGHTS - 1, 0.0));
    }

    #[test]
    fn should_light_respects_chance() {
        assert!(should_light(true, true, 0, BUILD_CHANCE - 0.01));
        assert!(!should_light(true, true, 0, BUILD_CHANCE + 0.01));
        // 邊界：roll == BUILD_CHANCE 不觸發（嚴格小於）。
        assert!(!should_light(true, true, 0, BUILD_CHANCE));
    }

    #[test]
    fn most_bonded_threshold_picks_highest_above_gate() {
        // 全未達門檻 → None。
        assert_eq!(most_bonded_threshold(&[0, 1, 2]), None);
        // 有人達門檻 → 取最高。
        assert_eq!(most_bonded_threshold(&[1, VIGIL_AFFINITY, 5]), Some(2));
        // 同分取最小索引。
        assert_eq!(most_bonded_threshold(&[VIGIL_AFFINITY, VIGIL_AFFINITY]), Some(0));
        // 恰好門檻 → 入選（≥）。
        assert_eq!(most_bonded_threshold(&[VIGIL_AFFINITY]), Some(0));
        // 空陣列 → None，不 panic。
        assert_eq!(most_bonded_threshold(&[]), None);
    }

    #[test]
    fn is_away_respects_threshold() {
        let now = 100_000u64;
        // 恰好門檻 → 算離開（≥）。
        assert!(is_away(now - VIGIL_OFFLINE_SECS, now));
        // 差一秒 → 還不算。
        assert!(!is_away(now - VIGIL_OFFLINE_SECS + 1, now));
        // 遠久 → 算。
        assert!(is_away(0, now));
        // last_seen 在未來（時鐘怪）→ saturating_sub 為 0，不算離開，不 panic。
        assert!(!is_away(now + 10, now));
    }

    #[test]
    fn pick_anchor_offsets_by_direction_and_is_deterministic() {
        assert_eq!(pick_anchor(5.5, 5.5, 0), (5 + ANCHOR_DIST, 5));
        assert_eq!(pick_anchor(5.5, 5.5, 1), (5 - ANCHOR_DIST, 5));
        assert_eq!(pick_anchor(5.5, 5.5, 2), (5, 5 + ANCHOR_DIST));
        assert_eq!(pick_anchor(5.5, 5.5, 3), (5, 5 - ANCHOR_DIST));
        assert_eq!(pick_anchor(5.5, 5.5, 4), pick_anchor(5.5, 5.5, 0));
    }

    #[test]
    fn pick_anchor_floors_negative_coords() {
        assert_eq!(pick_anchor(-0.5, -0.5, 0), (-1 + ANCHOR_DIST, -1));
    }

    #[test]
    fn far_enough_true_when_no_existing() {
        assert!(far_enough(0, 0, &[]));
    }

    #[test]
    fn far_enough_false_when_too_close() {
        // 距 3 格 < MIN_SEPARATION(4)。
        assert!(!far_enough(0, 0, &[(3, 0)]));
    }

    #[test]
    fn far_enough_boundary_exactly_min_separation() {
        // 恰好 4 格（≥）視為夠遠。
        assert!(far_enough(0, 0, &[(4, 0)]));
    }

    #[test]
    fn far_enough_true_when_all_far() {
        assert!(far_enough(0, 0, &[(10, 0), (0, 10)]));
    }

    #[test]
    fn light_say_line_varies_names_and_bounded() {
        let a = light_say_line("小明", 0);
        let b = light_say_line("小明", 1);
        assert_ne!(a, b);
        assert!(a.contains("小明"));
        assert!(!light_say_line("小明", 2).is_empty());
        // pick 取模循環。
        assert_eq!(light_say_line("小明", 0), light_say_line("小明", 3));
        // 長名截斷不超上限。
        let long = "超".repeat(80);
        assert!(light_say_line(&long, 0).chars().count() <= SAY_MAX_CHARS);
    }

    #[test]
    fn return_say_line_varies_names_and_bounded() {
        let a = return_say_line("露露", 0);
        let b = return_say_line("露露", 1);
        assert_ne!(a, b);
        assert!(a.contains("露露"));
        assert_eq!(return_say_line("露露", 0), return_say_line("露露", 3));
        let long = "光".repeat(80);
        assert!(return_say_line(&long, 0).chars().count() <= SAY_MAX_CHARS);
    }

    #[test]
    fn memory_and_feed_lines_mention_player() {
        assert!(light_memory_line("小明").contains("小明"));
        assert!(light_memory_line("小明").contains("燈"));
        assert!(light_feed_line("小明").contains("小明"));
        assert!(light_feed_line("小明").contains("燈"));
        assert!(return_memory_line("小明").contains("小明"));
        assert!(return_memory_line("小明").contains("燈"));
    }
}
