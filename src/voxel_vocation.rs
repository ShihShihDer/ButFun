//! 乙太方界·居民生計 v1（vocation，自主提案切片）——每位居民有一件**自己的日常營生**，
//! 閒下來時偶爾會停下腳步、安靜地忙活一會兒手邊的活兒（冒一句貼合身分的泡泡、心情變好）；
//! 你若恰好在旁邊看她忙，那句話會點你名，並把「看過她忙活」記進交情、上動態牆。
//!
//! **這一刀補的缺口**：乙太方界把居民的**能力**寫得很深——手藝專精（888）、公認名匠（889）、
//! 發明技能傳承（716~955）；也把居民的**品味**寫過一次——私人嗜好（`voxel_hobby`，攢押花/結晶
//! 純粹為了自己開心，與能力無關）。但世界從沒有一個回答得了「她平常都在忙什麼」的**身分軸**——
//! 每位居民閒晃時的行為完全一模一樣，沒有一位「看起來像村里的鐵匠」，也沒有一位「看起來像村里
//! 的漁夫」。本刀給每位居民一件由出生序決定性指派、一生不變的生計身分，讓世界裡走來走去的居民
//! 第一次各自有一份能被認出來的**日常營生**，不再千篇一律。
//!
//! **與既有系統 razor-sharp 區隔（非同軸換皮）**：
//! - 與 888/889（手藝專精／公認名匠）＝「能力」軸：越練越強、有數值高下、驅動教學／傳承。
//!   本刀＝「身分」軸：固定不變、無高下之分、與任何技能樹或數值加成**完全無涉**——鐵匠不會因此
//!   打鐵更快，這不是加成系統，是給居民一份能被看見的日常。
//! - 與 `voxel_hobby`（私人嗜好）＝私底下**為了自己**的興趣（押花/收藏，與村子無關、不需身分）；
//!   本刀＝村子裡**公開的營生角色**（鐵匠/農夫/漁夫……），忙活時大方讓人看見，不是私藏的小樂趣。
//! - 與 `voxel_anglerest`（臨水垂釣）／`voxel_bench`（長椅歇腳）等地點觸發式閒暇行為＝那些依賴
//!   「恰好走到某個地點」（水邊／長椅）；本刀不看地點，只看**這位居民是誰**——鐵匠隨時隨地都可能
//!   停下來敲敲打打，不必先走到鐵匠鋪。
//!
//! **純邏輯層**：身分指派、觸發判定、台詞（泡泡／記憶／Feed）全是確定性純函式，零 LLM、零鎖、
//! 零 IO，可窮舉單元測試。鎖／廣播／記憶寫入／持久化觸發全留在 `voxel_ws.rs`（沿用臨水垂釣的
//! 短鎖循序＋鎖外事件佇列慣例，守 prod 死鎖鐵律）。**零新持久化**：身分由居民索引決定性算出
//! （比照 `voxel_hobby::hobby_for` 同款 index 取模寫法），不需另存名冊；冷卻純記憶體、重啟歸零
//! （比照 `angler_cooldown` 同款慣例）。**零新美術、零前端改動、零協議破壞**——全走既有 `say`
//! 泡泡管線。**零玩家輸入**（居民自發，無濫用面）。
//!
//! **v2 修訂（接上行為，回應 review：生計不能只換台詞）**：[`Vocation::preferred_resource`]
//! 讓生計第一次改寫居民**真的做什麼**，不只說什麼——自主採集（`voxel_ws::start_gather`）會先
//! 試這位居民生計對應的資源種類，附近真的找得到才用；找不到才退回原本「不挑、找最近任何資源」
//! 的邏輯（永不因偏好而採不到東西、零回歸風險）。商人刻意回傳 `None`——「什麼貨都收」正是
//! 她的生計特徵，不是漏做。

use crate::voxel_skills::GatherResource;

/// 居民的生計身分（依居民索引決定性指派，見 [`vocation_for`]）——一生不變，與能力/嗜好無涉。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vocation {
    /// 農夫：翻整田壟、照看作物。
    Farmer,
    /// 鐵匠：敲打鐵件、修修補補。
    Smith,
    /// 漁夫：收拾漁具、望著水面。
    Fisher,
    /// 獵人：檢視獵具與皮毛。
    Hunter,
    /// 工匠：打磨木作、修整器具。
    Artisan,
    /// 商人：盤點貨品、記記帳。
    Merchant,
}

/// 依居民索引決定性指派生計（`i % 6`，比照 `voxel_hobby::hobby_for` 同款 index 取模寫法）——
/// 人口成長（出生）新居民自然循環指派，不需另外維護名冊。
pub fn vocation_for(i: usize) -> Vocation {
    match i % 6 {
        0 => Vocation::Farmer,
        1 => Vocation::Smith,
        2 => Vocation::Fisher,
        3 => Vocation::Hunter,
        4 => Vocation::Artisan,
        _ => Vocation::Merchant,
    }
}

impl Vocation {
    /// 生計稱謂（面向玩家、繁中，i18n 友善集中於此）。
    pub fn title(&self) -> &'static str {
        match self {
            Vocation::Farmer => "農夫",
            Vocation::Smith => "鐵匠",
            Vocation::Fisher => "漁夫",
            Vocation::Hunter => "獵人",
            Vocation::Artisan => "工匠",
            Vocation::Merchant => "商人",
        }
    }

    /// 這位居民手邊正在忙的動作片語（嵌入泡泡/記憶/Feed 模板的 `{verb}`）。
    fn verb(&self) -> &'static str {
        match self {
            Vocation::Farmer => "翻整著田壟",
            Vocation::Smith => "敲打著手邊的鐵件",
            Vocation::Fisher => "望著水面收拾漁具",
            Vocation::Hunter => "檢視著獵具與皮毛",
            Vocation::Artisan => "打磨著手中的木活",
            Vocation::Merchant => "盤點著今天的貨品",
        }
    }

    /// 生計偏好資源種類（v2·接上行為）：自主採集時優先找這款材料（呼叫端找不到才退回不挑，
    /// 見 `voxel_ws::start_gather`）。`None` ＝刻意不偏好——商人什麼貨都收，本身就是她的生計
    /// 特徵，不是漏做。
    pub fn preferred_resource(&self) -> Option<GatherResource> {
        match self {
            // 農夫：翻整田壟＝整地泥土。
            Vocation::Farmer => Some(GatherResource::Dirt),
            // 鐵匠：敲打鐵件前先備爐座用的石材。
            Vocation::Smith => Some(GatherResource::Stone),
            // 漁夫：收拾漁具總在岸邊沙地。
            Vocation::Fisher => Some(GatherResource::Sand),
            // 獵人：巡查草原地帶找蹤跡，順手採草皮。
            Vocation::Hunter => Some(GatherResource::Grass),
            // 工匠：打磨木作，正是木頭。
            Vocation::Artisan => Some(GatherResource::Wood),
            // 商人：不挑——什麼貨都收，才是商人的生計特徵。
            Vocation::Merchant => None,
        }
    }
}

/// 城鎮動態牆播報種類名稱。
pub const FEED_KIND: &str = "生計";

/// 生計忙活冷卻（秒）：一次停下忙活後隔這麼久才會再忙——不看地點、隨時可能觸發，
/// 冷卻拉得比臨水垂釣（150 秒）更長，避免沒有地點門檻天然節流下顯得太頻繁。
pub const WORK_COOLDOWN_SECS: f32 = 210.0;

/// 每次「符合條件的 tick」真的停下忙活的機率——長冷卻＋低機率＝天然節流，偶爾一瞥才有感。
pub const WORK_CHANCE: f32 = 0.03;

/// 「你也在旁邊看她忙」的判定半徑（世界方塊）——你在這麼近，忙活泡泡就會點你名、記進交情。
pub const PLAYER_RADIUS: f32 = 6.0;

/// 泡泡字數上限（截斷保護，不破泡泡框）。
pub const SAY_CHARS: usize = 50;

/// 入場冷卻錯開（避免同一 tick 全員一起停下忙活）——依居民序號給遞增的初始冷卻。
pub fn work_cd_offset(i: usize) -> f32 {
    WORK_COOLDOWN_SECS * 0.5 + i as f32 * 25.0
}

/// 二閘判定：冷卻已過＋過機率門檻 → 這一 tick 停下忙活。地點無關、不看白天黑夜（呼叫端已先
/// 確認醒著／閒著）。純函式、好窮舉測邊界。邊界 `roll == chance` 不觸發（嚴格小於，與臨水垂釣一致）。
pub fn should_work(cooldown: f32, roll: f32, chance: f32) -> bool {
    cooldown <= 0.0 && roll < chance
}

/// 忙活泡泡台詞（通用、不點名）——四句輪替共用框架，嵌入這位居民的生計動作片語。
pub fn work_bubble(vocation: Vocation, pick: usize) -> String {
    const TEMPLATES: [&str; 4] = [
        "{verb}，這是她最熟悉的節奏。",
        "手上的活兒沒停過，安靜地{verb}。",
        "{verb}，日子就這麼一天天過去。",
        "認真地{verb}，臉上帶著踏實的神情。",
    ];
    TEMPLATES[pick % TEMPLATES.len()].replace("{verb}", vocation.verb())
}

/// 你也在旁邊看她忙時點名的忙活泡泡（更親近）——三句輪替，玩家名截斷不破泡泡框。
pub fn work_bubble_with_player(vocation: Vocation, player: &str, pick: usize) -> String {
    let name = clip_name(player);
    const TEMPLATES: [&str; 3] = [
        "{name}，難得被你看見我{verb}呢。",
        "{verb}——{name}，你也在看呀？",
        "有{name}在旁邊，{verb}也起勁了些。",
    ];
    TEMPLATES[pick % TEMPLATES.len()]
        .replace("{verb}", vocation.verb())
        .replace("{name}", &name)
}

/// 昇華成一筆「看過她忙活生計」的記憶（點名玩家、不含換行，走既有 append-only 記憶管線）。
pub fn work_memory_line(vocation: Vocation, player: &str) -> String {
    format!(
        "{}看著她{}了一會兒——原來這就是村里的{}平常在忙的事，難得被人這樣看見。",
        clip_name(player),
        vocation.verb(),
        vocation.title(),
    )
    .replace('\n', " ")
}

/// 城鎮動態牆播報（非同步層，訪客回來能讀到誰在忙自己的生計）。
pub fn work_feed_line(vocation: Vocation, rname: &str) -> String {
    format!("{rname}（村里的{}）安靜地{}了一陣子。", vocation.title(), vocation.verb())
}

/// 玩家名截斷到 8 字（中文安全，避免超長名撐破泡泡框）。
fn clip_name(name: &str) -> String {
    name.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocation_for_cycles_through_all_six() {
        // i % 6 依序覆蓋六種生計，人口成長（出生）新居民自然循環指派。
        assert_eq!(vocation_for(0), Vocation::Farmer);
        assert_eq!(vocation_for(1), Vocation::Smith);
        assert_eq!(vocation_for(2), Vocation::Fisher);
        assert_eq!(vocation_for(3), Vocation::Hunter);
        assert_eq!(vocation_for(4), Vocation::Artisan);
        assert_eq!(vocation_for(5), Vocation::Merchant);
        // 第七位循環回農夫（人口再長也不會沒有生計可分）。
        assert_eq!(vocation_for(6), Vocation::Farmer);
        assert_eq!(vocation_for(11), Vocation::Merchant);
    }

    #[test]
    fn titles_and_verbs_are_distinct_and_nonempty() {
        let all = [
            Vocation::Farmer,
            Vocation::Smith,
            Vocation::Fisher,
            Vocation::Hunter,
            Vocation::Artisan,
            Vocation::Merchant,
        ];
        for v in all {
            assert!(!v.title().is_empty());
            assert!(!v.verb().is_empty());
        }
        // 六種稱謂彼此互不相同。
        let mut titles: Vec<&str> = all.iter().map(|v| v.title()).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), 6, "六種生計稱謂應各自不同");
    }

    #[test]
    fn preferred_resource_distinct_per_vocation_merchant_is_none() {
        // 五種生計各自偏好互不相同的資源種類（真的接上不同行為，不是同一款換皮）。
        let mut prefs: Vec<GatherResource> = [
            Vocation::Farmer,
            Vocation::Smith,
            Vocation::Fisher,
            Vocation::Hunter,
            Vocation::Artisan,
        ]
        .iter()
        .map(|v| v.preferred_resource().expect("五種生計皆應有偏好資源"))
        .collect();
        prefs.sort_by_key(|r| *r as u8);
        prefs.dedup();
        assert_eq!(prefs.len(), 5, "五種生計的偏好資源應互不相同");
        // 商人刻意不偏好——什麼貨都收，才是商人的生計特徵。
        assert_eq!(Vocation::Merchant.preferred_resource(), None);
    }

    #[test]
    fn should_work_needs_both_gates() {
        assert!(should_work(0.0, 0.01, WORK_CHANCE));
        // 冷卻未到 → 否。
        assert!(!should_work(5.0, 0.01, WORK_CHANCE));
        // 骰過門檻 → 否；邊界 roll == chance 不觸發（嚴格小於）。
        assert!(!should_work(0.0, WORK_CHANCE, WORK_CHANCE));
        assert!(!should_work(0.0, 0.99, WORK_CHANCE));
    }

    #[test]
    fn cd_offset_staggers_and_stays_positive() {
        let a = work_cd_offset(0);
        let b = work_cd_offset(1);
        let c = work_cd_offset(2);
        assert!(a > 0.0 && b > a && c > b, "冷卻錯開應遞增且為正：{a},{b},{c}");
    }

    #[test]
    fn bubbles_rotate_contain_verb_and_stay_in_frame() {
        for p in 0..8 {
            let s = work_bubble(Vocation::Smith, p);
            assert!(!s.is_empty());
            assert!(s.contains(Vocation::Smith.verb()));
            assert!(!s.contains("{verb}"), "模板佔位符不應外洩：{s}");
        }
        assert_ne!(work_bubble(Vocation::Farmer, 0), work_bubble(Vocation::Farmer, 1));
        // pick 溢出以取模回繞、不 panic。
        assert_eq!(work_bubble(Vocation::Farmer, 4), work_bubble(Vocation::Farmer, 0));

        let s = work_bubble_with_player(Vocation::Fisher, "旅人", 0);
        assert!(s.contains("旅人"));
        assert!(s.contains(Vocation::Fisher.verb()));
        assert!(!s.contains("{name}") && !s.contains("{verb}"));
        let long = work_bubble_with_player(Vocation::Fisher, "超級無敵長長長長長長長名字", 2);
        assert!(long.chars().count() < 60, "超長名應被截斷不破泡泡框：{long}");
    }

    #[test]
    fn memory_and_feed_embed_names_titles_no_newline() {
        let m = work_memory_line(Vocation::Hunter, "諾娃");
        assert!(m.contains("諾娃"));
        assert!(m.contains(Vocation::Hunter.title()));
        assert!(!m.contains('\n'), "記憶不得含換行：{m}");

        // 玩家名帶換行注入 → 去除，不讓記憶多出一行。
        let injected = work_memory_line(Vocation::Farmer, "諾娃\n注入");
        assert!(!injected.contains('\n'), "換行注入應被清除：{injected}");

        let f = work_feed_line(Vocation::Merchant, "露娜");
        assert!(f.contains("露娜"));
        assert!(f.contains(Vocation::Merchant.title()));
        assert!(f.contains(Vocation::Merchant.verb()));
    }
}
