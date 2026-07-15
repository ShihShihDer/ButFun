//! 乙太方界·居民也會生病、鄰居與你的照顧讓她好轉快 v1（自主提案切片）。
//!
//! **缺口 / 為誰做**：乙太方界的居民已經有情緒（心情 677）、關係（情誼網 708）、生理需求
//! （餓 799）、甚至餓時彼此分食（800/801）——但牠們從沒有過一種**脆弱狀態**：一種需要
//! 「被照顧」才會好轉的狀態。餓可以自己回家吃飽解決，但世界裡完全沒有「這件事我一個人
//! 扛不完、需要別人陪一下才會好」這種情境——這是全庫唯一還空白的一種親密：**被照顧**。
//! 本刀給居民第一個「生病」狀態：偶爾會有點不舒服、動作慢下來，靠自己休息也會漸漸好轉，
//! 但若有交情夠的鄰居恰好經過陪她一會兒、或你送她一碗暖湯，她會好得更快——世界第一次
//! 出現「你我的陪伴，讓對方的難受縮短了」這種軟性依賴。
//!
//! **記憶驅動行為（北極星）**：陪不陪這位不舒服的鄰居，看的仍是**交情**（相識以上才會
//! 停下腳步陪伴）——跟 800 分食的閘一致，關係網真的決定了「誰會為誰停下腳步」。
//!
//! **與既有系統的分界（換維度·非同軸重複）**：
//! - 不是 799 餓（生理需求、**自理**可解、走回家吃就好）——生病是**脆弱**、不是需求，
//!   自己扛得住但**別人陪伴能加速好轉**，這是全新的「被照顧」情感深度。
//! - 不是 800/801 分食（給的是**食物**，解的是**餓**）——本刀給的是**陪伴／照顧**，解的是
//!   **不舒服**，玩家送湯只是其中一條路徑，鄰居陪伴（不需要任何物品）才是核心新意。
//! - 不是 678 打氣（治的是**孤獨**這種情緒缺口、且是打氣者**主動巡找** Lonely 目標）——
//!   本刀治的是**生病**這種身體缺口、且是**恰好路過**才觸發（零新巡路，鏡像 800 的路過慣例）。
//!
//! **這裡只放確定性純邏輯**（發病門檻、康復曲線、陪伴門檻、台詞/記憶/Feed 文案），零 LLM、
//! 零鎖、零 IO、零 async，可單元測試。配對掃描 / 鎖 / 走動 / 廣播全留在 `voxel_ws.rs`（沿用
//! 800 飢餓時的守望相助的「位置快照 → i≠j 循序掃描 → 每 tick 最多一對 → 記憶/Feed 鎖外
//! 落地」慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：不觸發 LLM、不開對外端點、不收玩家自由文字——發病/康復/陪伴台詞與
//! 記憶全為確定性模板，只嵌居民系統顯示名（本就出現在動態牆），永不回放記憶原文或玩家原話
//! （無注入 / NSFW 面）；觸發純伺服器 tick 內部狀態（低機率 + 長冷卻），玩家無法自報、無法
//! 從外部催發；陪伴冷卻（每位照顧者 [`CARE_COOLDOWN_SECS`]）+ 每 tick 低機率 + 需相識以上
//! ＝天然節流，不洗版泡泡/動態牆。病況/冷卻純記憶體重啟歸零（過場狀態、零資料風險、零
//! migration），記憶/情誼走既有 append-only。
//!
//! **v2（自主提案）：淋雨易著涼**——接上 700 雨天／815 雨天葉傘避雨兩套既有系統，讓「這場
//! 雨躲得好不好」第一次影響發病機率（見 [`RAIN_ONSET_MULTIPLIER`]），病倒台詞/動態牆也依
//! 雨因區分（[`onset_bubble_rain`]／[`onset_feed_line_rain`]），把兩套至今互不相知的既有
//! 系統接成一條看得見的因果，而非再開一條新軸線。
//!
//! **v3（自主提案）：生病真的「動作慢下來」**——本模組文件開頭第一段自己寫的承諾，從 v1
//! 上線以來從沒真的接上：`voxel_ws.rs` 唯一控制居民移動速度的 `speed_mult`（日夜作息 739）
//! 從沒讀過 `illness_severity` 一次，一位剛病倒、正硬撐著的居民走路/採集速度跟健康時一模
//! 一樣。本刀補上 [`illness_speed_mult`]——病況越重走得越慢（線性遞減、留速度下限，不會
//! 卡死不動），與既有日夜降速相乘套用（夜間又生病＝更慢，兩套降速正交疊加）。零協議破壞、
//! 零新狀態（沿用既有 `illness_severity`）、零新持久化。

use crate::voxel_bonds::BondTier;

/// 病況嚴重度上限：0.0 = 健康、[`ILLNESS_MAX`] = 剛病倒、最不舒服。
pub const ILLNESS_MAX: f32 = 1.0;

/// 發病機率（每次「有機會發病」的 tick 骰一次）：刻意很小——發病本身零場地限制，
/// 靠這個小機率 + 長冷卻讓「生病」稀少而有份量，不會變成擾人的日常噪音。
pub const ONSET_CHANCE: f32 = 0.02;

/// 淋雨易著涼倍率（自主提案切片，接上 700 雨天 / 815 雨天葉傘避雨）：正下雨、且此刻**沒在
/// 躲雨/歇著**（仍在雨裡走動、採集、蓋造）的居民，發病機率乘上這個倍率。雨天與生病兩套
/// 既有系統至今從未互相知道對方存在——躲雨躲得好不好，發病機率完全一樣；本刀讓「沒躲好雨」
/// 第一次有看得見的後果。倍率不高（避免生病從「稀少而有份量」淪為雨天日常噪音），乘完仍要
/// 過 [`ONSET_CHANCE`] 這道原本就很小的機率門檻，天然節流不變。
pub const RAIN_ONSET_MULTIPLIER: f32 = 3.0;

/// 這一刻的發病機率：淋雨（正下雨＋沒在躲雨/歇著）→ 乘 [`RAIN_ONSET_MULTIPLIER`]（夾在
/// `[0,1]`）；否則維持原機率。純函式、確定性、可測——呼叫端只需備好「此刻是否正淋著雨」
/// 這個布林（比照 815 用 `wait_timer <= 0.0` 判斷「不在躲雨/歇著中」）。
pub fn onset_chance_now(raining_unsheltered: bool, base_chance: f32) -> f32 {
    if raining_unsheltered {
        (base_chance * RAIN_ONSET_MULTIPLIER).min(1.0)
    } else {
        base_chance
    }
}

/// 康復（或剛出生）後的重新發病冷卻（秒）：約 50 分鐘內不會再病倒一次，
/// 「生病」才會是偶爾發生、讓人印象深刻的事，不是三天兩頭的小毛病。
pub const ONSET_COOLDOWN_SECS: f32 = 3000.0;

/// 病況自然消退速率（每秒）：靠自己休息，約 8 分鐘就能從剛病倒（`ILLNESS_MAX`）自然痊癒——
/// 不靠人陪也會好，只是慢；有人陪伴/送湯會好得更快（見 [`CARE_BOOST`] / [`SOUP_CARE_BOOST`]）。
pub const NATURAL_RECOVERY_PER_SEC: f32 = 1.0 / 480.0;

/// 病倒那一刻的原地歇息秒數（設 `wait_timer`）：身子不舒服，先停下腳步歇一會兒。
pub const ONSET_REST_SECS: f32 = 8.0;

/// 鄰居陪伴照顧要多靠近，才會停下腳步陪一會兒（世界方塊距離）。鏡像 800 分食的近距慣例——
/// 陪伴是「就在你旁邊坐下來」的舉動，不是隔著大半個村子喊。
pub const CARE_RADIUS: f32 = 4.5;

/// 陪伴者一次照顧後的靜默冷卻（秒）：讓「停下來陪一會兒」稀少而有份量，
/// 不會同一位居民短時間內反覆對人陪伴、洗版泡泡與動態牆。
pub const CARE_COOLDOWN_SECS: f32 = 220.0;

/// 條件都滿足後，這一 tick 真的停下來陪伴的機率。刻意不設 1.0——不是每次路過都會停下，
/// 偶爾才觸發，像真的生活裡的不期而遇（鏡像 800 的 `SHARE_CHANCE`）。
pub const CARE_CHANCE: f32 = 0.45;

/// 鄰居陪伴一次能減輕多少病況（直接扣除，夾在 `[0, ILLNESS_MAX]`）——陪伴讓病況大幅緩解，
/// 但通常還不到當場痊癒，仍留一點「這份陪伴幫了大忙，但完全好還要一會兒」的餘韻。
pub const CARE_BOOST: f32 = 0.55;

/// 玩家送一碗暖湯（既有 `voxel_craft::STEW_ID`，野菜暖湯）給正生病的居民能減輕多少病況——
/// 比鄰居陪伴更大方（玩家親手煮的一鍋料理，值得更明顯的療效），但仍留一絲餘韻、不強制當場全好。
pub const SOUP_CARE_BOOST: f32 = 0.85;

/// 病況隨時間自然消退 `dt` 秒（clamp 到 `[0, ILLNESS_MAX]`）。純函式、確定性、可測。
pub fn tick_recover(cur: f32, dt: f32) -> f32 {
    (cur - NATURAL_RECOVERY_PER_SEC * dt).clamp(0.0, ILLNESS_MAX)
}

/// 是否正處於生病狀態（病況 > 0）。
pub fn is_sick(severity: f32) -> bool {
    severity > 0.0
}

/// 生病時移動變慢的速度下限：病得越重、走得越慢，剛病倒（`ILLNESS_MAX`）時最慢，但仍留
/// 這個下限（不降到 0）——脆弱、拖著腳步，但不會被生病這件事徹底卡死不動。
pub const SICK_SPEED_FLOOR: f32 = 0.55;

/// 依此刻病況嚴重度，算出移動速度倍率（v3：生病真的「動作慢下來」）——健康（0）全速 1.0，
/// 隨病況線性遞減到 [`SICK_SPEED_FLOOR`]。純函式、確定性；壞值（負數／超過 `ILLNESS_MAX`）
/// 安全夾限，不會產生負值或超速。呼叫端只需乘上既有的日夜 `speed_mult` 即可，兩套降速
/// 正交疊加（夜間又生病＝更慢）。
pub fn illness_speed_mult(severity: f32) -> f32 {
    let s = severity.clamp(0.0, ILLNESS_MAX);
    1.0 - s * (1.0 - SICK_SPEED_FLOOR)
}

/// 發病門檻：目前健康（未生病）+ 冷卻已過 + 過了機率骰。純函式、確定性、可測。
pub fn should_fall_ill(currently_sick: bool, cooldown: f32, roll: f32, chance: f32) -> bool {
    !currently_sick && cooldown <= 0.0 && roll < chance
}

/// 這對居民的交情夠不夠深、值不值得停下來陪伴——相識（`Acquaintance`）以上才會陪，
/// 陌生人擦身而過不會。鏡像 800 分食的門檻，記憶（情誼）驅動行為。
pub fn tier_allows_care(tier: BondTier) -> bool {
    tier >= BondTier::Acquaintance
}

/// 陪伴門檻：陪伴者冷卻已過 + 過了機率骰。純函式、確定性、可測。
pub fn should_care(cooldown: f32, roll: f32, chance: f32) -> bool {
    cooldown <= 0.0 && roll < chance
}

/// 一次陪伴／送湯照顧後，病況減輕 `boost` 點（夾在 `[0, ILLNESS_MAX]`，不會變負值）。
pub fn apply_care(severity: f32, boost: f32) -> f32 {
    (severity - boost).clamp(0.0, ILLNESS_MAX)
}

/// 入場錯開初始發病冷卻（秒），避免啟動後短時間內全員扎堆病倒。
pub fn onset_cd_offset(i: usize) -> f32 {
    200.0 + i as f32 * 130.0
}

/// 病倒那一刻冒出的不舒服泡泡（四句輪替）。
pub fn onset_bubble(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "唔…頭有點暈，先歇一下…",
        "身子怎麼有點沉，不太舒服…",
        "咳…好像著涼了，得緩一緩",
        "有點提不起勁，先坐一會兒吧",
    ];
    LINES[pick % LINES.len()]
}

/// 淋雨引發的病倒泡泡（四句輪替，與 [`onset_bubble`] 刻意區隔——明確點出雨/濕，
/// 讓玩家看得出「這次是被雨淋的」這條因果，而非泛用著涼）。
pub fn onset_bubble_rain(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "都怪剛才那陣雨，身子發起冷來了…",
        "淋濕的衣服還沒乾，這下真的著涼了",
        "那陣雨終究沒躲乾淨，咳…頭有點暈",
        "雨裡淋久了，這會兒渾身發冷，先歇一下…",
    ];
    LINES[pick % LINES.len()]
}

/// 病況自然消退到 0（靠自己扛過去）時的痊癒泡泡（四句輪替）。
pub fn recovered_bubble(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "呼…總算好些了，撐過來了",
        "嗯，身子輕鬆多了，沒事了",
        "這下舒坦了，謝天謝地",
        "總算不難受了，可以起來動動了",
    ];
    LINES[pick % LINES.len()]
}

/// 陪伴者停下腳步、陪生病的鄰居坐一會兒時的暖泡泡（四句輪替）。
pub fn carer_line(pick: usize) -> &'static str {
    const LINES: [&str; 4] = [
        "別逞強，我陪你坐一會兒",
        "身子不舒服就好好歇著，我在這陪你",
        "來，靠著我這邊，慢慢會好的",
        "別擔心，有我陪著呢",
    ];
    LINES[pick % LINES.len()]
}

/// 被陪伴的居民延遲後冒出的道謝泡泡（嵌陪伴者名，四句輪替，截字前先組好）。
/// `carer` 空（理論上不會發生）→ 退成不點名的泛稱，仍不回放任何原話。
pub fn cared_thanks_line(carer: &str, pick: usize) -> String {
    const LINES: [&str; 4] = [
        "多虧你陪著，感覺好多了，謝謝",
        "有你在旁邊，真的安心不少",
        "這份陪伴，我記在心裡了",
        "謝謝你特地留下來陪我",
    ];
    let base = LINES[pick % LINES.len()];
    if carer.is_empty() {
        base.to_string()
    } else {
        format!("{carer}，{base}")
    }
}

/// 被陪伴者掛在陪伴者名下的暖記憶（episodic，累積情誼）。`carer` 空 → 泛稱。
pub fn cared_memory_for_patient(carer: &str) -> String {
    if carer.is_empty() {
        "我正不舒服的時候，有位鄰居留下來陪了我一會兒，這份情我記著。".to_string()
    } else {
        format!("我正不舒服的時候，{carer}留下來陪了我一會兒，這份情我記著。")
    }
}

/// 陪伴者掛在被陪伴者名下的記憶（episodic，累積情誼）。`patient` 空 → 泛稱。
pub fn cared_memory_for_carer(patient: &str) -> String {
    if patient.is_empty() {
        "有位鄰居身子不舒服，我留下來陪了她一會兒——這點小忙，樂意幫。".to_string()
    } else {
        format!("{patient}身子不舒服，我留下來陪了她一會兒——這點小忙，樂意幫。")
    }
}

/// 城鎮動態牆一行（鄰居陪伴）：讓不在場 / 回來的玩家也讀到「鄰里之間互相照應」。
pub fn care_feed_line(carer: &str, patient: &str) -> String {
    let c = if carer.is_empty() { "有位鄰居" } else { carer };
    let p = if patient.is_empty() { "一位不舒服的鄰居" } else { patient };
    format!("{p}身子不太舒服，{c}留下來陪了她一會兒——這份照應，暖了整座村子。")
}

/// 城鎮動態牆一行（病倒／痊癒，無鄰居/玩家在場也會留痕）。
pub fn onset_feed_line(name: &str) -> String {
    format!("{name}身子有點不舒服，停下腳步歇了一會兒。")
}

/// 城鎮動態牆一行（淋雨引發的病倒，與 [`onset_feed_line`] 刻意區隔，點出雨因）。
pub fn onset_feed_line_rain(name: &str) -> String {
    format!("{name}沒躲過那陣雨，淋出了病，停下腳步歇了一會兒。")
}

/// 城鎮動態牆一行（自然痊癒）。
pub fn recovered_feed_line(name: &str) -> String {
    format!("{name}總算緩過來了，又是活蹦亂跳的模樣。")
}

/// 玩家在居民正生病時送上一碗暖湯的專屬道謝泡泡（四句輪替，比一般贈禮道謝更觸動）。
/// `player` 空（訪客無顯示名）→ 退成不點名的泛稱，仍不回放任何原話。
pub fn soup_care_thanks_line(player: &str, pick: usize) -> String {
    const LINES: [&str; 4] = [
        "這時候端碗熱湯來，真是雪中送炭，謝謝你",
        "喝了這碗湯，身子暖多了，你來得正好",
        "難受的時候有你在，這碗湯我會記一輩子",
        "謝謝你特地送湯來看我，心裡暖暖的",
    ];
    let base = LINES[pick % LINES.len()];
    if player.is_empty() {
        base.to_string()
    } else {
        format!("{player}，{base}")
    }
}

/// 玩家送一碗暖湯給正生病的居民時，居民掛在玩家名下的深記憶——「你在我最難受的時候端了碗湯來」。
/// `player` 空（訪客無顯示名）→ 退成不點名的泛稱，仍不回放任何原話。
pub fn soup_care_memory(player: &str) -> String {
    if player.is_empty() {
        "我正不舒服的時候，有人端了碗熱湯來，這份暖，我記得特別牢。".to_string()
    } else {
        format!("我正不舒服的時候，{player}端了碗熱湯來，這份暖，我記得特別牢。")
    }
}

/// 玩家送湯照顧生病居民的城鎮動態牆一行。`player` 空 → 泛稱「有人」。
pub fn soup_care_feed_line(rname: &str, player: &str) -> String {
    let who = if player.is_empty() { "有人" } else { player };
    format!("{rname}正不舒服著，{who}端了碗熱湯來——這份暖，記得格外深。")
}

/// Feed 分類標籤（鄰居陪伴／自身病況）。
pub const FEED_KIND: &str = "鄰里照應";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_decays_and_clamps() {
        // 生病中：靠自己休息，病況隨時間下降。
        let s1 = tick_recover(ILLNESS_MAX, 60.0);
        assert!(s1 < ILLNESS_MAX && s1 > 0.0);
        // 大 dt 也不會降到負值。
        assert_eq!(tick_recover(0.1, 100_000.0), 0.0);
        // 已經健康：維持 0，不會變負值。
        assert_eq!(tick_recover(0.0, 10.0), 0.0);
    }

    #[test]
    fn natural_recovery_takes_about_eight_minutes() {
        // 從剛病倒到自然痊癒約 480 秒（8 分鐘）。
        let s = tick_recover(ILLNESS_MAX, 480.0);
        assert!((s - 0.0).abs() < 1e-4);
        // 略早於此則尚未痊癒。
        assert!(tick_recover(ILLNESS_MAX, 240.0) > 0.0);
    }

    #[test]
    fn is_sick_boundary() {
        assert!(!is_sick(0.0));
        assert!(is_sick(0.01));
        assert!(is_sick(ILLNESS_MAX));
    }

    #[test]
    fn illness_speed_mult_healthy_is_full_speed() {
        assert_eq!(illness_speed_mult(0.0), 1.0);
    }

    #[test]
    fn illness_speed_mult_max_severity_hits_floor() {
        let m = illness_speed_mult(ILLNESS_MAX);
        assert!((m - SICK_SPEED_FLOOR).abs() < 1e-6, "剛病倒應落在速度下限");
    }

    #[test]
    fn illness_speed_mult_monotonic_non_increasing() {
        let a = illness_speed_mult(0.2);
        let b = illness_speed_mult(0.6);
        let c = illness_speed_mult(ILLNESS_MAX);
        assert!(a >= b && b >= c, "病況越重速度應越慢（或持平），不會反過來變快");
    }

    #[test]
    fn illness_speed_mult_stays_in_bounds() {
        for tenths in 0..=10 {
            let severity = tenths as f32 / 10.0 * ILLNESS_MAX;
            let m = illness_speed_mult(severity);
            assert!(m >= SICK_SPEED_FLOOR - 1e-6 && m <= 1.0 + 1e-6, "速度倍率應落在 [下限, 1.0] 之間");
        }
    }

    #[test]
    fn illness_speed_mult_clamps_bad_values() {
        // 壞值：負數視同健康、超過上限視同最重病況——不會產生負值或超過 1.0 的怪速度。
        assert_eq!(illness_speed_mult(-1.0), illness_speed_mult(0.0));
        assert_eq!(illness_speed_mult(ILLNESS_MAX + 5.0), illness_speed_mult(ILLNESS_MAX));
    }

    #[test]
    fn should_fall_ill_needs_healthy_cooldown_and_roll() {
        // 已經生病：不會再次發病判定觸發（避免重複疊加）。
        assert!(!should_fall_ill(true, 0.0, 0.0, ONSET_CHANCE));
        // 冷卻未到：不發病（就算骰贏）。
        assert!(!should_fall_ill(false, 1.0, 0.0, ONSET_CHANCE));
        // 健康、冷卻到、骰贏：發病。
        assert!(should_fall_ill(false, 0.0, 0.001, ONSET_CHANCE));
        // 冷卻到、骰輸（roll == chance 不觸發，嚴格小於）：不發病。
        assert!(!should_fall_ill(false, 0.0, ONSET_CHANCE, ONSET_CHANCE));
        assert!(!should_fall_ill(false, 0.0, 0.9, ONSET_CHANCE));
    }

    #[test]
    fn tier_gate_requires_acquaintance() {
        assert!(!tier_allows_care(BondTier::Stranger));
        assert!(tier_allows_care(BondTier::Acquaintance));
        assert!(tier_allows_care(BondTier::Friend));
    }

    #[test]
    fn should_care_needs_cooldown_and_roll() {
        assert!(!should_care(1.0, 0.0, CARE_CHANCE));
        assert!(should_care(0.0, 0.1, CARE_CHANCE));
        assert!(!should_care(0.0, CARE_CHANCE, CARE_CHANCE));
        assert!(should_care(0.0, 0.0, 1.0));
    }

    #[test]
    fn apply_care_reduces_and_clamps() {
        // 一次陪伴／送湯減輕病況，但不會降到負值。
        let s = apply_care(0.6, CARE_BOOST);
        assert!((s - 0.05).abs() < 1e-4);
        assert_eq!(apply_care(0.2, CARE_BOOST), 0.0);
        // 對已經健康的居民套用照顧也不會變負值。
        assert_eq!(apply_care(0.0, SOUP_CARE_BOOST), 0.0);
    }

    #[test]
    fn rain_boosts_onset_chance_but_dry_stays_unchanged() {
        // 沒淋雨（沒下雨，或下雨但正躲雨/歇著）：機率不變。
        assert_eq!(onset_chance_now(false, ONSET_CHANCE), ONSET_CHANCE);
        // 淋雨（下雨中且沒在躲/歇）：機率乘上倍率。
        assert!((onset_chance_now(true, ONSET_CHANCE) - ONSET_CHANCE * RAIN_ONSET_MULTIPLIER).abs() < 1e-6);
        // 倍率再高也不會超過 1.0（機率上限，不 panic、不產生無意義值）。
        assert_eq!(onset_chance_now(true, 0.9), 1.0);
    }

    #[test]
    fn rain_onset_lines_rotate_bounded_and_distinct_from_generic() {
        for pick in 0..8 {
            let l = onset_bubble_rain(pick);
            assert!(!l.is_empty() && l.chars().count() <= 40);
        }
        assert_eq!(onset_bubble_rain(0), onset_bubble_rain(4));
        // 雨因台詞與泛用著涼台詞完全不重疊（玩家看得出這次是被雨淋的）。
        for pick in 0..4 {
            assert_ne!(onset_bubble_rain(pick), onset_bubble(pick));
        }
    }

    #[test]
    fn rain_onset_feed_line_embeds_name_and_mentions_rain() {
        let f = onset_feed_line_rain("賽勒");
        assert!(f.contains("賽勒") && f.contains("雨"));
        // 與泛用版本刻意不同文案。
        assert_ne!(onset_feed_line_rain("賽勒"), onset_feed_line("賽勒"));
    }

    #[test]
    fn cd_offsets_are_staggered() {
        let offs: Vec<f32> = (0..4).map(onset_cd_offset).collect();
        for w in offs.windows(2) {
            assert!(w[1] > w[0], "初始冷卻應遞增錯開");
        }
        assert!(offs[0] > 0.0);
    }

    #[test]
    fn lines_rotate_and_bounded() {
        for pick in 0..8 {
            assert!(!onset_bubble(pick).is_empty() && onset_bubble(pick).chars().count() <= 40);
            assert!(
                !recovered_bubble(pick).is_empty() && recovered_bubble(pick).chars().count() <= 40
            );
            assert!(!carer_line(pick).is_empty() && carer_line(pick).chars().count() <= 40);
            let tl = cared_thanks_line("諾娃", pick);
            assert!(!tl.is_empty() && tl.chars().count() <= 40);
        }
        // pick 溢出取模包回、不 panic。
        assert_eq!(onset_bubble(0), onset_bubble(4));
        assert_eq!(cared_thanks_line("露娜", 1), cared_thanks_line("露娜", 5));
    }

    #[test]
    fn thanks_embeds_or_falls_back() {
        let t = cared_thanks_line("賽勒", 0);
        assert!(t.contains("賽勒"));
        let g = cared_thanks_line("", 0);
        assert!(!g.is_empty() && !g.contains("，，"));
    }

    #[test]
    fn memories_embed_names_or_fall_back() {
        let mp = cared_memory_for_patient("露娜");
        assert!(mp.contains("露娜") && mp.contains("陪"));
        let mc = cared_memory_for_carer("奧瑞");
        assert!(mc.contains("奧瑞"));
        assert!(cared_memory_for_patient("").contains("陪"));
        assert!(!cared_memory_for_carer("").is_empty());
    }

    #[test]
    fn feed_lines_embed_both_or_fall_back() {
        let f = care_feed_line("露娜", "諾娃");
        assert!(f.contains("露娜") && f.contains("諾娃"));
        let g = care_feed_line("", "");
        assert!(g.contains("鄰居") && !g.is_empty());
        assert!(onset_feed_line("賽勒").contains("賽勒"));
        assert!(recovered_feed_line("奧瑞").contains("奧瑞"));
    }

    #[test]
    fn soup_care_thanks_rotates_and_embeds_or_falls_back() {
        for pick in 0..8 {
            let t = soup_care_thanks_line("露娜", pick);
            assert!(!t.is_empty() && t.chars().count() <= 40);
        }
        assert_eq!(soup_care_thanks_line("露娜", 0), soup_care_thanks_line("露娜", 4));
        let g = soup_care_thanks_line("", 0);
        assert!(!g.is_empty() && !g.contains("，，"));
    }

    #[test]
    fn soup_care_embeds_player_or_falls_back() {
        let m = soup_care_memory("諾娃");
        assert!(m.contains("諾娃") && m.contains("湯"));
        let g = soup_care_memory("");
        assert!(!g.contains("諾娃") && g.contains("湯"));
        let f = soup_care_feed_line("露娜", "旅人阿爾");
        assert!(f.contains("露娜") && f.contains("旅人阿爾"));
        let f2 = soup_care_feed_line("露娜", "");
        assert!(f2.contains("露娜") && f2.contains("有人"));
    }

    #[test]
    fn long_names_do_not_break_or_panic() {
        let long = "超".repeat(200);
        let _ = cared_thanks_line(&long, 0);
        let _ = cared_memory_for_patient(&long);
        let _ = cared_memory_for_carer(&long);
        let _ = care_feed_line(&long, &long);
        let _ = soup_care_memory(&long);
        let _ = soup_care_feed_line(&long, &long);
    }
}
