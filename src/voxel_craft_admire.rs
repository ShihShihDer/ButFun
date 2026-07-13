//! 乙太方界·手藝仰慕 v1——公認名匠的聲望，第一次能拉近兩個素不相識的居民（自主提案切片）。
//!
//! **缺口 / 為誰做**：名匠聲望（888/889）讓「發明並教過某門手藝夠多次」的居民被公認為村裡
//! 該手藝的權威，聲望目前唯一的社會後果是**卡關優先找名匠來教**（`maybe_proximity_teach`
//! 讀 `master_by_goal` 挑老師）——但那條路徑要「先卡關、且對方已到老朋友門檻」才會發生。
//! 名匠的名聲本身，從沒有單獨、獨立於教學之外的社會後果：一位還不會這門手藝、甚至跟名匠
//! 素不相識的居民，路過名匠身邊也毫無反應，好像那份公認的手藝完全不存在。這正對著
//! PLAN_ETHERVOX §4「居民↔居民關係 → 小社會湧現」——真的活著的小村，名聲會自己把人
//! 拉近，不必等到「卡關」這個前提。
//!
//! 本刀補上：**還不會某門手藝的居民，路過這門手藝的公認名匠時，會停下來多看兩眼、由衷
//! 佩服，情誼因此加深（`record_visit`，比照既有慣例升級才播里程碑）**——刻意**不要求任何
//! 交情門檻**（陌生人也會，這正是本刀的重點：讓**名聲**本身、而非既有交情，成為連結兩個
//! 居民的新理由；反過來說，`voxel_share_meal` 要求相識以上才分食，是「交情驅動幫忙」，
//! 本刀是「名聲驅動注目」，兩條完全相反的因果，互補而非重複）。
//!
//! **與既有系統 razor-sharp 區隔**：
//! - 不是 875 寵物讚賞（`voxel_pet_admire`）——那是居民對**玩家寵物**的反應；本刀是居民
//!   對**另一位居民手藝聲望**的反應，主體、觸發源皆不同。
//! - 不是 717/878/883 教學或 `voxel_proximity_teach` 就地指導——那些會把**技能本身**教給
//!   學生（`learn_from`，永久改變技能庫）；本刀**不傳授任何技能**，仰慕者事後仍然不會這門
//!   手藝，純粹是社交/情誼層面的反應，不動發明引擎半分。
//! - 不是 888 以物易物（`maybe_encounter_barter`）——那是餘料互補的**背包交換**；本刀不涉及
//!   任何物品轉移。
//!
//! **記憶驅動行為（北極星）**：這是本刀刻意的設計——聲望本身（`master_by_goal` 由既有
//! 發明/師承紀錄重算而來）就足以觸發新關係，不需要事先有交情。名聲把陌生人也連在一起，
//! 是小社會湧現很真實的一面：手藝好的人，本來就更容易讓周遭的人記住。
//!
//! **這裡只放確定性純邏輯**（觸發門檻、台詞/記憶/Feed 文案），零 LLM、零鎖、零 IO、
//! 零 async，可單元測試。配對掃描／鎖／廣播全留在 `voxel_ws.rs`（沿用 875/800 的
//! 「快照 → 純函式判定 → 鎖外落地」慣例，守 prod 死鎖鐵律）。
//!
//! **成本 / 濫用防護**：不觸發 LLM、不開對外端點、不收玩家自由文字——仰慕台詞/記憶/Feed
//! 全為確定性模板，只嵌居民系統顯示名與居民自取的手藝名（皆已出現在既有技能簿/技藝譜
//! API，非本刀新開的信任面）；觸發純伺服器 tick 內部狀態（距離 + 冷卻），玩家無法自報、
//! 無法從外部催發；每位仰慕者 [`CRAFT_ADMIRE_COOLDOWN_SECS`] 全域冷卻 + 每 tick 至多促成
//! 一組 = 天然節流，不洗版泡泡/動態牆。冷卻純記憶體重啟歸零（過場狀態、零資料風險、零
//! migration），記憶/情誼走既有 append-only。

/// 仰慕者要多靠近名匠，才會停下多看兩眼（世界方塊距離）。與 `voxel_pet_admire::PET_ADMIRE_RADIUS`
/// 同量級——都是「就站在附近」才會注意到，不是隔著大半個村子。
pub const CRAFT_ADMIRE_RADIUS: f32 = 6.0;

/// 每位仰慕者的全域冷卻（秒，不分仰慕的是哪門手藝／哪位名匠）：讓「注意到名匠」這件事
/// 稀少而有份量,不會同一位居民短時間內反覆對人流露仰慕、洗版泡泡與動態牆。
pub const CRAFT_ADMIRE_COOLDOWN_SECS: u64 = 240;

/// 條件都滿足後是否真的觸發：距離在半徑內 + 冷卻已過。純函式、確定性、可測。
pub fn admire_triggers(dist_sq: f32, cooldown_ok: bool) -> bool {
    dist_sq <= CRAFT_ADMIRE_RADIUS * CRAFT_ADMIRE_RADIUS && cooldown_ok
}

/// 仰慕者停下腳步時冒出的心聲泡泡（四句輪替，嵌手藝名）。`craft` 空（理論上不會）→ 退成
/// 不點名手藝的泛稱，仍不留空洞。
pub fn admire_say_line(craft: &str, pick: usize) -> String {
    const TEMPLATES: [&str; 4] = [
        "這手「{c}」的功夫，看得我入神",
        "「{c}」能做到這樣，真是一把好手",
        "村裡說起「{c}」，果然不是叫假的",
        "看著看著，我也想學一手「{c}」了",
    ];
    let c = if craft.is_empty() { "手藝" } else { craft };
    TEMPLATES[pick % TEMPLATES.len()].replace("{c}", c)
}

/// 仰慕者掛在名匠名下的暖記憶（episodic，累積情誼）。`master`/`craft` 空 → 泛稱，仍不留空洞。
pub fn admire_memory_for_admirer(master: &str, craft: &str) -> String {
    let c = if craft.is_empty() { "這門手藝" } else { craft };
    if master.is_empty() {
        format!("我看著村裡的名匠俐落地施展「{c}」，忍不住多看了幾眼，這份本事我記著了。")
    } else {
        format!("我看著{master}俐落地施展「{c}」，忍不住多看了幾眼，這份本事我記著了。")
    }
}

/// 名匠掛在仰慕者名下的記憶（episodic，語氣是謙遜帶一點驕傲，與 `admire_memory_for_admirer`
/// 刻意不同——一邊是仰慕，一邊是被看見的踏實）。`admirer`/`craft` 空 → 泛稱。
pub fn admire_memory_for_master(admirer: &str, craft: &str) -> String {
    let c = if craft.is_empty() { "這門手藝" } else { craft };
    if admirer.is_empty() {
        format!("有位鄰居多看了我{c}的手藝幾眼——這份本事，村裡也有人看在眼裡。")
    } else {
        format!("{admirer}多看了我{c}的手藝幾眼——這份本事，村裡也有人看在眼裡。")
    }
}

/// 城鎮動態牆一行：讓不在場/回來的玩家也讀到「名聲把兩個居民連在了一起」。
pub fn admire_feed_line(admirer: &str, master: &str, craft: &str) -> String {
    let a = if admirer.is_empty() { "有位居民" } else { admirer };
    let m = if master.is_empty() { "村裡的名匠" } else { master };
    let c = if craft.is_empty() { "這門手藝" } else { craft };
    format!("{a}路過，看著{m}施展「{c}」的手藝看得入神——一份名聲，把兩人又拉近了一分。")
}

/// Feed 分類標籤。
pub const FEED_KIND: &str = "手藝仰慕";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_needs_radius_and_cooldown() {
        assert!(admire_triggers(0.0, true));
        let on_edge = CRAFT_ADMIRE_RADIUS * CRAFT_ADMIRE_RADIUS;
        assert!(admire_triggers(on_edge, true), "邊界含括");
        let just_out = on_edge + 0.01;
        assert!(!admire_triggers(just_out, true), "超出半徑不觸發");
        assert!(!admire_triggers(0.0, false), "冷卻未到不觸發");
    }

    #[test]
    fn say_line_rotates_and_embeds_craft() {
        for pick in 0..8 {
            let s = admire_say_line("燒玻璃", pick);
            assert!(s.contains("燒玻璃"));
            assert!(!s.is_empty() && s.chars().count() <= 40);
        }
        assert_eq!(admire_say_line("燒玻璃", 0), admire_say_line("燒玻璃", 4), "溢出取模包回");
    }

    #[test]
    fn say_line_falls_back_on_empty_craft() {
        let s = admire_say_line("", 0);
        assert!(!s.is_empty() && !s.contains("{c}"));
    }

    #[test]
    fn admirer_memory_embeds_or_falls_back() {
        let m = admire_memory_for_admirer("露娜", "燒玻璃");
        assert!(m.contains("露娜") && m.contains("燒玻璃"));
        let g = admire_memory_for_admirer("", "");
        assert!(!g.is_empty() && !g.contains("，，"));
    }

    #[test]
    fn master_memory_embeds_or_falls_back() {
        let m = admire_memory_for_master("諾娃", "燒玻璃");
        assert!(m.contains("諾娃") && m.contains("燒玻璃"));
        // 名匠側語氣需與仰慕者側不同（不是同一句換人名）。
        assert_ne!(
            admire_memory_for_master("諾娃", "燒玻璃").replace("諾娃", "X"),
            admire_memory_for_admirer("諾娃", "燒玻璃").replace("諾娃", "X"),
        );
        let g = admire_memory_for_master("", "");
        assert!(!g.is_empty());
    }

    #[test]
    fn feed_embeds_all_three_or_falls_back() {
        let f = admire_feed_line("諾娃", "露娜", "燒玻璃");
        assert!(f.contains("諾娃") && f.contains("露娜") && f.contains("燒玻璃"));
        let g = admire_feed_line("", "", "");
        assert!(!g.is_empty() && g.contains("居民") && g.contains("名匠"));
    }

    #[test]
    fn long_names_do_not_break_or_panic() {
        let long = "超".repeat(200);
        let _ = admire_say_line(&long, 0);
        let _ = admire_memory_for_admirer(&long, &long);
        let _ = admire_memory_for_master(&long, &long);
        let _ = admire_feed_line(&long, &long, &long);
    }
}
