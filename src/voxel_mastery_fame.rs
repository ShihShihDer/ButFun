//! 名匠聲望 v1（ROADMAP 888）——居民靠「發明 + 教學」在村裡長出某門手藝的公認名匠。
//!
//! 北極星是「AI 居民自己發明技能、知識在村裡傳承、湧現小社會」。既有系統已經讓居民
//! 自己發明技能（`voxel_invent`）、親子繼承（承自X）、在世師承（師承X，`voxel_teach`／
//! `voxel_proximity_teach`）。**真缺口**：技能只是「會不會」，村裡沒有「誰是這門手藝的
//! 權威」。本模組補上這層——把**發明並教過某門手藝夠多次**的居民，自然認作村裡該手藝的
//! 『名匠』（如「露娜·燒玻璃名匠」），並讓這份聲望有社會後果：別人卡在該手藝時優先找名匠、
//! 名匠教學時稱謂現身、世界動態記下「村裡公認露娜是燒玻璃的一把好手」。
//!
//! **純湧現、療癒調性**：這是村民的驕傲與互敬，不是排行榜競爭——一門手藝只認一位公認名匠，
//! 沒有分數榜、沒有名次比較，只有「大家都說她是這方面的一把好手」。
//!
//! **零重複資料**：聲望**只以既有技能紀錄為輸入即時重算**（發明＝`source` 為 `None`；
//! 師承＝`taught` 為 `true` 且 `source` 為老師名）。不新增任何資料表／欄位／jsonl，
//! 重啟後由技能庫還原即自然重算，與既有資料同一風險等級（其實是零新風險）。純函式窮舉可測。

/// 發明一門手藝的聲望份量——無中生有最難，權重最高。
pub const INVENT_WEIGHT: u32 = 3;
/// 每教會一位村人的聲望份量——知識傳承一次算一分。
pub const TEACH_WEIGHT: u32 = 1;
/// 成為某門手藝「名匠」的聲望門檻。發明(3) + 教會兩人(2) = 5，或純教會五人也成。
/// 刻意設得「要真的投入夠久夠深」才掛名號，維持稱謂的稀有與份量（療癒而非灌水）。
pub const MASTER_THRESHOLD: u32 = 5;

/// 世界動態（Feed）分類：名匠加冕。
pub const FEED_KIND: &str = "名匠";

/// 從既有技能紀錄擷取的最小輸入（與 `voxel_invent` 解耦、方便窮舉單測）。
///
/// **一律以顯示名為鍵**：既有師承鏈的 `source` 存的就是老師顯示名（「師承露娜」），
/// 名匠稱謂也是名字（「露娜·燒玻璃名匠」），故發明者也換算成顯示名再聚合，全程口徑一致。
#[derive(Clone, Debug)]
pub struct SkillEvidence {
    /// 持有此技能的居民**顯示名**。
    pub holder: String,
    /// 手藝名（＝技能名，如「燒玻璃」）。
    pub craft: String,
    /// 目標材料 id——同一門手藝的穩定鍵；教學偏好用它對上學生卡關的目標。
    pub goal_block: u8,
    /// 來源顯示名：`None`＝自創；`Some(名)`＝承自親代或師承老師。
    pub source: Option<String>,
    /// `true`＝在世師承（`source` 為老師）；`false`＝自創或親代繼承。
    pub taught: bool,
}

/// 一位居民在某門手藝上的聲望構成。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CraftFame {
    /// 居民顯示名。
    pub resident: String,
    /// 手藝名。
    pub craft: String,
    /// 手藝的穩定鍵（目標材料 id）。
    pub goal_block: u8,
    /// 是否為這門手藝的原創發明者。
    pub invented: bool,
    /// 教會了幾位村人這門手藝。
    pub taught_count: u32,
    /// 聲望分＝發明權重 + 教學次數 × 教學權重。
    pub score: u32,
}

impl CraftFame {
    /// 是否已達「名匠」門檻。
    pub fn is_master(&self) -> bool {
        self.score >= MASTER_THRESHOLD
    }
}

/// 聚合每位居民在每門手藝上的聲望。以（手藝穩定鍵, 居民顯示名）為聚合鍵。
///
/// - **發明**：`source` 為 `None` 的持有者，即這門手藝的原創者（得 [`INVENT_WEIGHT`]）。
/// - **教學**：`taught` 且 `source` 為某老師名的紀錄，替那位老師在該手藝上 +1 教學分。
///   （紀錄反映「村裡現有多少人把這門手藝的功勞記在這位老師身上」，正是我們要的聲望義涵。）
pub fn compute_fame(evidence: &[SkillEvidence]) -> Vec<CraftFame> {
    use std::collections::BTreeMap;
    // key = (goal_block, resident_name) → (craft_name, invented, taught_count)
    let mut agg: BTreeMap<(u8, String), (String, bool, u32)> = BTreeMap::new();

    // ① 發明：source None 的持有者記為原創者，手藝名以其所取為準。
    for e in evidence {
        if e.source.is_none() {
            let ent = agg
                .entry((e.goal_block, e.holder.clone()))
                .or_insert_with(|| (e.craft.clone(), false, 0));
            ent.0 = e.craft.clone();
            ent.1 = true;
        }
    }
    // ② 教學：taught 且 source=Some(老師) → 老師在該手藝 +1 分。
    for e in evidence {
        if e.taught {
            if let Some(teacher) = &e.source {
                let ent = agg
                    .entry((e.goal_block, teacher.clone()))
                    .or_insert_with(|| (e.craft.clone(), false, 0));
                if ent.0.is_empty() {
                    ent.0 = e.craft.clone();
                }
                ent.2 = ent.2.saturating_add(1);
            }
        }
    }

    agg.into_iter()
        .map(|((goal_block, resident), (craft, invented, taught_count))| {
            let base = if invented { INVENT_WEIGHT } else { 0 };
            let score = base.saturating_add(taught_count.saturating_mul(TEACH_WEIGHT));
            CraftFame {
                resident,
                craft,
                goal_block,
                invented,
                taught_count,
                score,
            }
        })
        .collect()
}

/// `a` 是否比 `b` 更該當這門手藝的公認名匠（決定性排序，同一輸入永遠同一結果）：
/// 分數高者勝 → 同分優先原創者 → 再同以顯示名字典序小者定案。
fn outshines(a: &CraftFame, b: &CraftFame) -> bool {
    if a.score != b.score {
        return a.score > b.score;
    }
    if a.invented != b.invented {
        return a.invented;
    }
    a.resident < b.resident
}

/// 村裡每門手藝**至多一位公認名匠**（達門檻者中最出眾的那位）。純湧現、不排名次：
/// 同一門手藝只回一位，回傳集合即「村裡目前公認的名匠們」（各據一門手藝）。
pub fn masters(evidence: &[SkillEvidence]) -> Vec<CraftFame> {
    use std::collections::BTreeMap;
    let mut best: BTreeMap<u8, CraftFame> = BTreeMap::new();
    for cf in compute_fame(evidence).into_iter().filter(CraftFame::is_master) {
        match best.get(&cf.goal_block) {
            Some(cur) if !outshines(&cf, cur) => {}
            _ => {
                best.insert(cf.goal_block, cf);
            }
        }
    }
    best.into_values().collect()
}

/// 名匠稱謂（面向玩家字串，集中此處便於 i18n）：「露娜·燒玻璃名匠」。
pub fn master_epithet(name: &str, craft: &str) -> String {
    format!("{name}·{craft}名匠")
}

/// 加冕當下的世界動態文案：「村裡公認露娜是燒玻璃的一把好手。」
pub fn crown_feed_line(name: &str, craft: &str) -> String {
    format!("村裡公認{name}是{craft}的一把好手。")
}

/// 加冕當下名匠自己冒出的心聲泡泡（謙遜帶著驕傲的句式池，零 LLM）。
pub fn crown_say_line(craft: &str, pick: usize) -> String {
    let pool = [
        format!("沒想到大家都說我是{craft}的一把好手…真是不好意思。"),
        format!("能把{craft}的手藝傳給大家，是我的驕傲。"),
        format!("{craft}這門手藝，我會一直好好教下去的。"),
        format!("原來這些年的{craft}，大家都看在眼裡呀。"),
    ];
    pool[pick % pool.len()].clone()
}

/// 名匠親自授課時冒出的泡泡（讓「名匠」稱謂在世界裡反覆現身，句式池、零 LLM）。
pub fn master_teach_say_line(craft: &str, pick: usize) -> String {
    let pool = [
        format!("身為{craft}名匠，這一手我來教你。"),
        format!("來，{craft}的訣竅，我慢慢說給你聽。"),
        format!("這{craft}的手藝，就傳給你了。"),
    ];
    pool[pick % pool.len()].clone()
}

/// 加冕時替名匠自己留下的一筆記憶（第一人稱內心，沿用日記調性）。
pub fn crown_memory_line(craft: &str) -> String {
    format!("村裡開始叫我{craft}名匠了，我會把這門手藝好好傳下去。")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(holder: &str, craft: &str, goal: u8, source: Option<&str>, taught: bool) -> SkillEvidence {
        SkillEvidence {
            holder: holder.to_string(),
            craft: craft.to_string(),
            goal_block: goal,
            source: source.map(|s| s.to_string()),
            taught,
        }
    }

    #[test]
    fn 只發明未教學_不足以成名匠() {
        // 發明一門手藝 = 3 分 < 門檻 5。
        let e = vec![ev("露娜", "燒玻璃", 40, None, false)];
        let f = compute_fame(&e);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].resident, "露娜");
        assert!(f[0].invented);
        assert_eq!(f[0].taught_count, 0);
        assert_eq!(f[0].score, INVENT_WEIGHT);
        assert!(!f[0].is_master());
        assert!(masters(&e).is_empty());
    }

    #[test]
    fn 發明加教會兩人_恰達門檻成名匠() {
        // 露娜自創燒玻璃(3) + 教會米拉、諾娃(各 +1) = 5 = 門檻。
        let e = vec![
            ev("露娜", "燒玻璃", 40, None, false),
            ev("米拉", "燒玻璃", 40, Some("露娜"), true),
            ev("諾娃", "燒玻璃", 40, Some("露娜"), true),
        ];
        let f = compute_fame(&e);
        let luna = f.iter().find(|c| c.resident == "露娜").unwrap();
        assert!(luna.invented);
        assert_eq!(luna.taught_count, 2);
        assert_eq!(luna.score, INVENT_WEIGHT + 2 * TEACH_WEIGHT);
        assert!(luna.is_master());

        let m = masters(&e);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].resident, "露娜");
        assert_eq!(m[0].craft, "燒玻璃");
        assert_eq!(m[0].goal_block, 40);
    }

    #[test]
    fn 純教學五人也能成名匠() {
        // 沒發明、純靠教會五個人（承自親代的手藝也能教）→ 5 分達標。
        let mut e = vec![ev("米拉", "織布", 50, Some("外婆"), false)]; // 米拉承自親代
        for stu in ["甲", "乙", "丙", "丁", "戊"] {
            e.push(ev(stu, "織布", 50, Some("米拉"), true));
        }
        let f = compute_fame(&e);
        let mira = f.iter().find(|c| c.resident == "米拉").unwrap();
        assert!(!mira.invented);
        assert_eq!(mira.taught_count, 5);
        assert!(mira.is_master());
        assert_eq!(masters(&e)[0].resident, "米拉");
    }

    #[test]
    fn 門檻邊界_四分未達五分達() {
        // 發明(3)+教一人(1)=4 < 5 不成名匠。
        let e4 = vec![
            ev("露娜", "燒玻璃", 40, None, false),
            ev("米拉", "燒玻璃", 40, Some("露娜"), true),
        ];
        assert!(!compute_fame(&e4).iter().any(|c| c.is_master()));
        // 再教一人到 5 就成。
        let mut e5 = e4.clone();
        e5.push(ev("諾娃", "燒玻璃", 40, Some("露娜"), true));
        assert!(compute_fame(&e5).iter().any(|c| c.is_master()));
    }

    #[test]
    fn 一門手藝只認一位名匠_取最出眾者() {
        // 露娜(發明+教2=5) 對上 米拉(教5=5) 同分：門檻同分時優先原創者。
        let mut e = vec![
            ev("露娜", "燒玻璃", 40, None, false),
            ev("學A", "燒玻璃", 40, Some("露娜"), true),
            ev("學B", "燒玻璃", 40, Some("露娜"), true),
        ];
        for stu in ["a", "b", "c", "d", "e"] {
            e.push(ev(stu, "燒玻璃", 40, Some("米拉"), true));
        }
        let m = masters(&e);
        assert_eq!(m.len(), 1, "同一手藝(goal 40)只該有一位公認名匠");
        assert_eq!(m[0].resident, "露娜", "同分優先原創者");
    }

    #[test]
    fn 不同手藝各有各的名匠() {
        let e = vec![
            ev("露娜", "燒玻璃", 40, None, false),
            ev("s1", "燒玻璃", 40, Some("露娜"), true),
            ev("s2", "燒玻璃", 40, Some("露娜"), true),
            ev("米拉", "織布", 50, None, false),
            ev("t1", "織布", 50, Some("米拉"), true),
            ev("t2", "織布", 50, Some("米拉"), true),
        ];
        let mut m = masters(&e);
        m.sort_by_key(|c| c.goal_block);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].resident, "露娜");
        assert_eq!(m[1].resident, "米拉");
    }

    #[test]
    fn 決定性_同輸入重排順序結果一致() {
        let e1 = vec![
            ev("露娜", "燒玻璃", 40, None, false),
            ev("s1", "燒玻璃", 40, Some("露娜"), true),
            ev("s2", "燒玻璃", 40, Some("露娜"), true),
        ];
        let mut e2 = e1.clone();
        e2.reverse();
        assert_eq!(masters(&e1), masters(&e2));
    }

    #[test]
    fn 面向玩家字串符合預期() {
        assert_eq!(master_epithet("露娜", "燒玻璃"), "露娜·燒玻璃名匠");
        assert!(crown_feed_line("露娜", "燒玻璃").contains("公認"));
        assert!(crown_feed_line("露娜", "燒玻璃").contains("露娜"));
        assert!(crown_feed_line("露娜", "燒玻璃").contains("燒玻璃"));
        // 句式池有界、隨 pick 輪替、皆含手藝名。
        for p in 0..8usize {
            assert!(crown_say_line("燒玻璃", p).contains("燒玻璃"));
            assert!(master_teach_say_line("燒玻璃", p).contains("燒玻璃"));
        }
        assert!(master_teach_say_line("燒玻璃", 0).contains("名匠"));
        assert!(crown_memory_line("燒玻璃").contains("名匠"));
    }
}
