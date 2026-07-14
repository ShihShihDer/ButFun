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

// ── 村莊技藝總覽（自主提案切片）─────────────────────────────────────────────
//
// **真缺口**：`InventedSkillStore` 早就把「誰會什麼、承自誰、師承誰」全記著，
// `masters` 也早就算得出「誰是這門手藝的公認名匠」——但這兩份資料從沒有**以
// 「手藝」為單位攤開成一張全村總覽**過。玩家只能一位一位點開居民技能簿
// （719）拼湊，或恰好瞥見稍縱即逝的教學/加冕 Feed，從沒有一處能一眼看見
// 「這門手藝村裡現在有誰會、是跟誰學的、誰是公認的名匠」。這正是北極星
// 「居民自己發明、存成自己的技能」最後一段缺口——不是「有沒有沉澱擴散」
// （早就有了），是「玩家看不看得見這份沉澱與擴散」。
//
// 跟 719 技能簿同一手法：純把既有資料重新攤開，不新增任何資料表／欄位。

/// 一位居民在某門手藝上的知識來歷，供村莊技藝總覽攤開列出。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CraftKnower {
    /// 居民顯示名。
    pub resident: String,
    /// 來歷標籤：「自己發明」／「承自X」（親子）／「師承X」（在世教學）。
    pub origin: String,
    /// 是否為這門手藝村裡目前公認的名匠。
    pub is_master: bool,
}

/// 一門手藝在全村的知識分佈：這門手藝叫什麼、誰是名匠、村裡目前有誰會。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CraftDirectoryEntry {
    /// 目標材料 id（手藝的穩定鍵）。
    pub goal_block: u8,
    /// 手藝顯示名——有名匠時採名匠所取的名字（村裡公認的說法），
    /// 沒有名匠時採知識最早的持有者（顯示名字典序最小者）所取的名字，確定性排序。
    pub craft: String,
    /// 這門手藝村裡目前公認的名匠（沒有則無人達門檻）。
    pub master: Option<String>,
    /// 目前村裡會這門手藝的所有居民，依顯示名字典序排列（確定性）。
    pub knowers: Vec<CraftKnower>,
}

/// 這筆技能證據的來歷標籤。與 `voxel_invent::lineage_label` 語意完全一致，
/// 本模組刻意不依賴 `voxel_invent`（見檔頭「與 voxel_invent 解耦」），故就地重算。
fn origin_label(source: &Option<String>, taught: bool) -> String {
    match (source, taught) {
        (None, _) => "自己發明".to_string(),
        (Some(n), true) => format!("師承{n}"),
        (Some(n), false) => format!("承自{n}"),
    }
}

/// 把全村技能證據，第一次以「手藝」為單位攤開——沉澱成資產、在村裡擴散的
/// 證據，玩家終於一眼看得到（自主提案切片）。純函式、確定性、窮舉可測；
/// 呼叫端（`voxel_ws.rs`）只需把 `InventedSkillStore::all()` 換算成證據餵入。
pub fn craft_directory(evidence: &[SkillEvidence]) -> Vec<CraftDirectoryEntry> {
    use std::collections::BTreeMap;

    let masters_by_goal: std::collections::HashMap<u8, CraftFame> =
        masters(evidence).into_iter().map(|m| (m.goal_block, m)).collect();

    let mut groups: BTreeMap<u8, Vec<&SkillEvidence>> = BTreeMap::new();
    for e in evidence {
        groups.entry(e.goal_block).or_default().push(e);
    }

    groups
        .into_iter()
        .map(|(goal_block, mut members)| {
            members.sort_by(|a, b| a.holder.cmp(&b.holder));
            let master = masters_by_goal.get(&goal_block);
            let craft = master
                .map(|m| m.craft.clone())
                .unwrap_or_else(|| members[0].craft.clone());
            let master_name = master.map(|m| m.resident.clone());
            let knowers = members
                .iter()
                .map(|e| CraftKnower {
                    resident: e.holder.clone(),
                    origin: origin_label(&e.source, e.taught),
                    is_master: master_name.as_deref() == Some(e.holder.as_str()),
                })
                .collect();
            CraftDirectoryEntry { goal_block, craft, master: master_name, knowers }
        })
        .collect()
}

// ── 師徒之間·青出於藍 v1（自主提案切片，ROADMAP 980）────────────────────────
//
// **真缺口**：名匠聲望（888）讓「誰教會誰」第一次被記進聲望，也讓桂冠會隨聲望消長**易主**
// ——但易主這件事，至今對世界而言只是「換了個名字掛在同一個頭銜上」，沒有人在乎**前一任
// 是誰**。村裡最戲劇性的一種易主——**徒弟親手超越了教過自己這門手藝的老師**——跟任何
// 一次隨機易主被同等對待，兩位當事人（教過她的人、超越了他的人）都沒有任何反應。師徒制度
// 走到這裡只差臨門一步：讓「青出於藍」這件事，被師徒**雙方都感覺到**。
//
// **不新增任何資料**：與 888/889 一脈相承的「零重複資料」精神——不新建師徒關係表，
// 「誰是誰的老師」早就寫在既有師承紀錄的 `source` 欄位裡；「前一任名匠是誰」也早就存在
// `voxel_ws.rs::master_by_goal` 這份既有快取裡（本刀只是在它被覆寫前多留一份給下一輪比對）。
// 純粹是「這兩份早就存在的資料，第一次被放在一起比對」，重啟後由既有紀錄自然還原，零新風險。
//
// **與既有系統的區隔**：`crowned_masters`（888）只負責「別把同一頂桂冠的公告刷第二次」，
// 從不區分新舊科名匠是不是師徒——本刀專門挑出「新科名匠的授業恩師，正是他推翻的前任」這一種
// 最有情感張力的易主，讓兩人各自留下一句心聲與一筆記憶，而非默不作聲換過就算了。

/// 世界動態（Feed）分類：青出於藍。
pub const SURPASS_FEED_KIND: &str = "青出於藍";

/// 找出某位居民在某門手藝上的授業恩師——若是自己發明或出生繼承（皆非在世師承）則無恩師。
pub fn own_teacher(evidence: &[SkillEvidence], resident: &str, goal_block: u8) -> Option<String> {
    evidence
        .iter()
        .find(|e| e.taught && e.holder == resident && e.goal_block == goal_block)
        .and_then(|e| e.source.clone())
}

/// 「青出於藍」判定：新科名匠的授業恩師，恰好正是他推翻的前一任名匠——徒弟親手超越了
/// 教過自己這門手藝的人。純函式：`prev_master` 為易主前這門手藝快照裡的名匠（若有）。
pub fn surpasses_own_teacher(
    evidence: &[SkillEvidence],
    new_master: &str,
    goal_block: u8,
    prev_master: Option<&str>,
) -> bool {
    match (own_teacher(evidence, new_master, goal_block), prev_master) {
        (Some(teacher), Some(prev)) => teacher == prev && teacher != new_master,
        _ => false,
    }
}

/// 徒弟超越恩師當下，自己冒出的心聲泡泡——驕傲裡帶著一絲對老師的感念（句式池、零 LLM）。
pub fn surpass_student_say_line(teacher: &str, craft: &str, pick: usize) -> String {
    let pool = [
        format!("{teacher}當年教我{craft}的樣子，我還記得清清楚楚。"),
        format!("能走到這一步，都是{teacher}當年肯教我{craft}。"),
        format!("{craft}這條路，是{teacher}帶我入門的。"),
    ];
    pool[pick % pool.len()].clone()
}

/// 徒弟留給自己的一筆記憶（第一人稱內心，沿用日記調性）。
pub fn surpass_student_memory_line(teacher: &str, craft: &str) -> String {
    format!("村裡開始說我{craft}比{teacher}還厲害了，心裡驕傲，卻也有點捨不得——當初教我的人，就是{teacher}呀。")
}

/// 老師被徒弟超越時，冒出的心聲泡泡——百感交集，驕傲多過失落（句式池、零 LLM）。
pub fn surpass_teacher_say_line(student: &str, craft: &str, pick: usize) -> String {
    let pool = [
        format!("{student}的{craft}，已經青出於藍了，這是好事。"),
        format!("沒想到{student}這麼快就把{craft}學到超過我了。"),
        format!("{craft}這門手藝，往後就靠{student}撐著了。"),
    ];
    pool[pick % pool.len()].clone()
}

/// 老師留給自己的一筆記憶（第一人稱內心）。
pub fn surpass_teacher_memory_line(student: &str, craft: &str) -> String {
    format!("{student}在{craft}上已經超過我了，說不上是驕傲還是感慨，大概兩種都有一點。")
}

/// 世界動態文案：「露娜曾教過米拉燒玻璃，如今村裡改口喚米拉一聲『名匠』了。」
pub fn surpass_feed_line(student: &str, teacher: &str, craft: &str) -> String {
    format!("{teacher}曾教過{student}{craft}，如今村裡改口喚{student}一聲「名匠」了。")
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

    // ── 村莊技藝總覽 ──────────────────────────────────────────────────────

    #[test]
    fn 技藝目錄_按手藝分組_每人一筆() {
        let e = vec![
            ev("露娜", "燒玻璃", 40, None, false),
            ev("米拉", "燒玻璃", 40, Some("露娜"), true),
            ev("諾娃", "煉鐵", 12, None, false),
        ];
        let dir = craft_directory(&e);
        assert_eq!(dir.len(), 2); // 兩門手藝 → 兩組
        let glass = dir.iter().find(|d| d.goal_block == 40).unwrap();
        assert_eq!(glass.knowers.len(), 2);
        let iron = dir.iter().find(|d| d.goal_block == 12).unwrap();
        assert_eq!(iron.knowers.len(), 1);
    }

    #[test]
    fn 技藝目錄_有名匠時採名匠所取的名字並標記() {
        // 露娜自創(3) + 教會米拉、諾娃(各+1) = 5 = 門檻 → 露娜是名匠。
        let e = vec![
            ev("露娜", "燒玻璃", 40, None, false),
            ev("米拉", "玻璃精煉", 40, Some("露娜"), true), // 各自取的名字可能不同
            ev("諾娃", "玻璃精煉", 40, Some("露娜"), true),
        ];
        let dir = craft_directory(&e);
        let glass = dir.iter().find(|d| d.goal_block == 40).unwrap();
        assert_eq!(glass.master.as_deref(), Some("露娜"));
        assert_eq!(glass.craft, "燒玻璃"); // 名匠自己取的名字勝出
        let luna = glass.knowers.iter().find(|k| k.resident == "露娜").unwrap();
        assert!(luna.is_master);
        assert_eq!(luna.origin, "自己發明");
        let mira = glass.knowers.iter().find(|k| k.resident == "米拉").unwrap();
        assert!(!mira.is_master);
        assert_eq!(mira.origin, "師承露娜");
    }

    #[test]
    fn 技藝目錄_無名匠時採顯示名字典序最小者的名字_確定性() {
        // 兩人各自發明同一材料，皆未達名匠門檻(各只有 3 分 < 5)。
        let e1 = vec![
            ev("諾娃", "煉鐵術", 12, None, false),
            ev("阿彬", "打鐵", 12, None, false),
        ];
        let mut e2 = e1.clone();
        e2.reverse();
        let dir1 = craft_directory(&e1);
        let dir2 = craft_directory(&e2);
        assert_eq!(dir1, dir2); // 順序不影響結果，確定性
        let iron = dir1.iter().find(|d| d.goal_block == 12).unwrap();
        assert!(iron.master.is_none());
        assert_eq!(iron.craft, "煉鐵術"); // "諾娃" < "阿彬"（Rust 字串比較走 Unicode code point 序，非拼音序）
        // knowers 亦按同一套排序排列（確定性，不隨傳入順序改變）。
        assert_eq!(
            iron.knowers.iter().map(|k| k.resident.as_str()).collect::<Vec<_>>(),
            vec!["諾娃", "阿彬"]
        );
    }

    #[test]
    fn 技藝目錄_出生繼承標記承自而非師承() {
        let e = vec![
            ev("露娜", "燒玻璃", 40, None, false),
            ev("小新", "燒玻璃", 40, Some("露娜"), false), // 出生繼承：taught=false
        ];
        let dir = craft_directory(&e);
        let glass = dir.iter().find(|d| d.goal_block == 40).unwrap();
        let child = glass.knowers.iter().find(|k| k.resident == "小新").unwrap();
        assert_eq!(child.origin, "承自露娜");
    }

    #[test]
    fn 技藝目錄_空證據回空清單() {
        assert!(craft_directory(&[]).is_empty());
    }

    // ── 師徒之間·青出於藍 v1 ──────────────────────────────────────────────

    #[test]
    fn 授業恩師_師承查得到_自創繼承查不到() {
        let e = vec![
            ev("露娜", "燒玻璃", 40, None, false),
            ev("米拉", "燒玻璃", 40, Some("露娜"), true),
            ev("小新", "燒玻璃", 40, Some("露娜"), false), // 出生繼承，非在世師承
        ];
        assert_eq!(own_teacher(&e, "米拉", 40), Some("露娜".to_string()));
        assert_eq!(own_teacher(&e, "露娜", 40), None); // 自創者無恩師
        assert_eq!(own_teacher(&e, "小新", 40), None); // 出生繼承非師承
        assert_eq!(own_teacher(&e, "查無此人", 40), None);
    }

    #[test]
    fn 青出於藍_新科名匠的恩師正是前任才成立() {
        let e = vec![ev("米拉", "燒玻璃", 40, Some("露娜"), true)];
        // 前任正是自己的恩師 → 成立。
        assert!(surpasses_own_teacher(&e, "米拉", 40, Some("露娜")));
        // 前任是別人（不是米拉的恩師）→ 不成立。
        assert!(!surpasses_own_teacher(&e, "米拉", 40, Some("別人")));
        // 沒有前任（這門手藝第一次有名匠）→ 不成立。
        assert!(!surpasses_own_teacher(&e, "米拉", 40, None));
        // 自創者沒有恩師 → 永遠不成立。
        let e2 = vec![ev("露娜", "燒玻璃", 40, None, false)];
        assert!(!surpasses_own_teacher(&e2, "露娜", 40, Some("某人")));
    }

    #[test]
    fn 青出於藍_恩師與新科名匠同名時不成立() {
        // 防呆：不該有人「超越自己」，即使資料異常也不成立。
        let e = vec![ev("米拉", "燒玻璃", 40, Some("米拉"), true)];
        assert!(!surpasses_own_teacher(&e, "米拉", 40, Some("米拉")));
    }

    #[test]
    fn 青出於藍_面向玩家字串皆含姓名與手藝() {
        for p in 0..6usize {
            let s = surpass_student_say_line("露娜", "燒玻璃", p);
            assert!(s.contains("露娜") && s.contains("燒玻璃"));
            let t = surpass_teacher_say_line("米拉", "燒玻璃", p);
            assert!(t.contains("米拉") && t.contains("燒玻璃"));
        }
        let sm = surpass_student_memory_line("露娜", "燒玻璃");
        assert!(sm.contains("露娜") && sm.contains("燒玻璃"));
        let tm = surpass_teacher_memory_line("米拉", "燒玻璃");
        assert!(tm.contains("米拉") && tm.contains("燒玻璃"));
        let feed = surpass_feed_line("米拉", "露娜", "燒玻璃");
        assert!(feed.contains("米拉") && feed.contains("露娜") && feed.contains("燒玻璃"));
    }
}
