//! ROADMAP 245：入夜聚會閒談。
//!
//! 入夜後村裡幾位主要 NPC 會從各自的白天崗位（沿街一字排開）收攏到夜間的聚會點
//! （`npc_schedule` 的 `night_pos`，彼此擠在一小塊空地上）。本模組讓圍聚在一起的
//! 兩位主要 NPC 第一次彼此攀談起來——而且語氣會隨他們在 `npc_relations` 裡的好感
//! 冷暖流動：交情好的熱絡寒暄、合不來的話裡帶刺、平平的就客套兩句。
//!
//! 與 ROADMAP 244（白天、主要 NPC ↔ 居民）刻意對成晝夜一對：244 是白天大人物對
//! 路過居民的招呼，本切片是入夜後大人物彼此之間的閒談。純後端啟發式模板、零 LLM、
//! 零協議改動、零持久化（守成本紀律「背景生活預設不燒 LLM」）。

use crate::npc_relations::NpcRelationsState;

/// 兩位主要 NPC 視為「圍在一起」的最大距離（像素）。
///
/// 對齊 `npc_schedule` 的夜間聚會點：入夜後幾位 NPC 會擠進這個半徑內，
/// 唯獨守在原地的里長（凱爾長老）離得遠、不入夥（他守著自己的家）。
pub const GATHER_DIST: f32 = 80.0;

/// 好感冷暖判定門檻（好感值域 0~100，中性 50）。
const WARM_THRESHOLD: i32 = 62;
const COOL_THRESHOLD: i32 = 46;

/// 攀談語氣——隨兩人好感而定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// 交情好——熱絡。
    Warm,
    /// 合不來——話裡帶刺。
    Cool,
    /// 平平——客套寒暄。
    Neutral,
}

/// 依好感值判定語氣。未知關係（`None`）一律當中性。
pub fn affinity_tone(score: Option<i32>) -> Tone {
    match score {
        Some(v) if v >= WARM_THRESHOLD => Tone::Warm,
        Some(v) if v <= COOL_THRESHOLD => Tone::Cool,
        _ => Tone::Neutral,
    }
}

// ── 攀談模板（{other} = 對方顯示名） ───────────────────────────────────────────

/// 開口者：熱絡（交情好）。
static GATHER_WARM: &[&str] = &[
    "{other}！忙了一天，來這歇會兒，正好跟你嘮嘮。",
    "嘿，{other}，有你在這兒，這夜裡都熱鬧不少！",
    "{other}，老交情了，今晚可得多聊兩句。",
    "看到你我就放心，{other}，這城裡有你撐著，踏實。",
];

/// 開口者：帶刺（合不來）。
static GATHER_COOL: &[&str] = &[
    "喲，{other}，你也在這兒啊……那我就不多待了。",
    "{other}，各做各的事，咱倆就別假客套了。",
    "哼，{other}，你那套做法，我到現在還是看不慣。",
    "{other}，話我撂這兒——別擋著我的道就行。",
];

/// 開口者：客套（平平）。
static GATHER_NEUTRAL: &[&str] = &[
    "{other}，今天也辛苦了，早點歇著吧。",
    "嘿，{other}，這天兒是越來越涼了。",
    "{other}，忙完啦？這一天過得真快。",
    "晚上好，{other}，明天又是一天呢。",
];

/// 回應者：熱絡。
static REPLY_WARM: &[&str] = &[
    "可不是嘛，{other}！我也正想找你說說話。",
    "哈哈，{other}，跟你聊著最痛快！",
    "有你這話我就暖了，{other}，回頭咱再細聊！",
];

/// 回應者：帶刺。
static REPLY_COOL: &[&str] = &[
    "彼此彼此，{other}，我也沒打算跟你多廢話。",
    "隨你，{other}。我可沒空陪你嘔氣。",
    "哼，{other}，等你做出點樣子，我再聽你說。",
];

/// 回應者：客套。
static REPLY_NEUTRAL: &[&str] = &[
    "是啊，{other}，您也早些休息。",
    "嗯，{other}，這日子過得是快。",
    "彼此彼此，{other}，明天見。",
];

fn pick<'a>(pool: &'a [&'a str], seed: usize) -> &'a str {
    pool[seed % pool.len()]
}

/// 取得開口者向同僚攀談的台詞（帶對方顯示名、隨語氣輪替）。
pub fn get_gather_line(listener_name: &str, tone: Tone, seed: usize) -> String {
    let pool = match tone {
        Tone::Warm => GATHER_WARM,
        Tone::Cool => GATHER_COOL,
        Tone::Neutral => GATHER_NEUTRAL,
    };
    pick(pool, seed).replace("{other}", listener_name)
}

/// 取得回應者的回話（帶開口者顯示名、隨語氣輪替）。
pub fn get_gather_reply(speaker_name: &str, tone: Tone, seed: usize) -> String {
    let pool = match tone {
        Tone::Warm => REPLY_WARM,
        Tone::Cool => REPLY_COOL,
        Tone::Neutral => REPLY_NEUTRAL,
    };
    pick(pool, seed).replace("{other}", speaker_name)
}

/// 一組入夜聚會攀談——開口者（a）說一句、聽者（b）回一句，雙方頭頂各冒一個泡泡。
#[derive(Debug, Clone, PartialEq)]
pub struct GatherChat {
    pub speaker_id: String,
    pub speaker_name: String,
    pub speaker_x: f32,
    pub speaker_y: f32,
    pub speaker_text: String,
    pub listener_id: String,
    pub listener_name: String,
    pub listener_x: f32,
    pub listener_y: f32,
    pub listener_text: String,
}

/// 從「圍在一起」的主要 NPC 裡挑一對攀談（純函式、確定性、可單元自驗）。
///
/// `npcs`：主要 NPC 的即時位置 `(id, 顯示名, x, y)`。掃描所有 `i<j` 配對，收齊
/// 彼此距離 ≤ `GATHER_DIST` 的候選，再以 `seed` 輪替挑一對；語氣取開口者對聽者的
/// 好感（`relations`）判定。無可攀談的配對時回 `None`。
pub fn pick_gather_pair(
    npcs: &[(String, String, f32, f32)],
    relations: &NpcRelationsState,
    seed: usize,
) -> Option<GatherChat> {
    if npcs.len() < 2 {
        return None;
    }

    // 收齊所有彼此圍在一起的配對。
    let mut candidates: Vec<(usize, usize)> = Vec::new();
    for i in 0..npcs.len() {
        for j in (i + 1)..npcs.len() {
            let dx = npcs[i].2 - npcs[j].2;
            let dy = npcs[i].3 - npcs[j].3;
            if dx * dx + dy * dy <= GATHER_DIST * GATHER_DIST {
                candidates.push((i, j));
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }

    // 確定性挑一對；再依 seed 決定誰先開口（讓同一對也能換邊起話）。
    let (mut a, mut b) = candidates[seed % candidates.len()];
    if seed % 2 == 1 {
        std::mem::swap(&mut a, &mut b);
    }

    let (a_id, a_name, ax, ay) = &npcs[a];
    let (b_id, b_name, bx, by) = &npcs[b];

    // 語氣取「開口者 → 聽者」的好感（關係不一定對稱）；回話取「聽者 → 開口者」。
    let speak_tone = affinity_tone(relations.get(a_id, b_id));
    let reply_tone = affinity_tone(relations.get(b_id, a_id));

    Some(GatherChat {
        speaker_id: a_id.clone(),
        speaker_name: a_name.clone(),
        speaker_x: *ax,
        speaker_y: *ay,
        speaker_text: get_gather_line(b_name, speak_tone, seed),
        listener_id: b_id.clone(),
        listener_name: b_name.clone(),
        listener_x: *bx,
        listener_y: *by,
        listener_text: get_gather_reply(a_name, reply_tone, seed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn npc(id: &str, x: f32, y: f32) -> (String, String, f32, f32) {
        (id.to_string(), format!("{}先生", id), x, y)
    }

    #[test]
    fn affinity_tone_thresholds() {
        assert_eq!(affinity_tone(Some(72)), Tone::Warm);
        assert_eq!(affinity_tone(Some(43)), Tone::Cool);
        assert_eq!(affinity_tone(Some(50)), Tone::Neutral);
        assert_eq!(affinity_tone(None), Tone::Neutral);
    }

    #[test]
    fn gather_line_contains_other_name_and_varies_by_tone() {
        let warm = get_gather_line("阿土", Tone::Warm, 0);
        let cool = get_gather_line("阿土", Tone::Cool, 0);
        assert!(warm.contains("阿土"));
        assert!(cool.contains("阿土"));
        assert_ne!(warm, cool, "不同語氣的台詞應有別");
    }

    #[test]
    fn reply_contains_speaker_name() {
        for tone in [Tone::Warm, Tone::Cool, Tone::Neutral] {
            let r = get_gather_reply("薇拉", tone, 1);
            assert!(r.contains("薇拉"));
            assert!(!r.is_empty());
        }
    }

    #[test]
    fn no_pair_when_too_far() {
        let npcs = vec![
            npc("merchant", 2400.0, 2200.0),
            npc("village_chief", 2720.0, 2080.0), // 離得遠，不入夥
        ];
        let rel = NpcRelationsState::new();
        assert!(pick_gather_pair(&npcs, &rel, 0).is_none());
    }

    #[test]
    fn picks_pair_when_clustered() {
        // 兩人擠在一起（< GATHER_DIST）。
        let npcs = vec![
            npc("merchant", 2400.0, 2200.0),
            npc("procurement_npc", 2380.0, 2180.0),
        ];
        let rel = NpcRelationsState::new();
        let chat = pick_gather_pair(&npcs, &rel, 0).expect("圍在一起時應挑出一對攀談");
        assert!(!chat.speaker_text.is_empty());
        assert!(!chat.listener_text.is_empty());
        // 開口者與聽者必為這兩位、且不同人。
        assert_ne!(chat.speaker_id, chat.listener_id);
    }

    #[test]
    fn warm_relation_yields_warm_tone() {
        // 商人 ↔ 採購代理人在 npc_relations 預設為 68（> 62）→ 熱絡。
        let npcs = vec![
            npc("merchant", 2400.0, 2200.0),
            npc("procurement_npc", 2410.0, 2200.0),
        ];
        let rel = NpcRelationsState::new();
        // seed 偶數 → a=merchant 先開口，對 procurement 好感 68 → Warm。
        let chat = pick_gather_pair(&npcs, &rel, 0).unwrap();
        assert_eq!(chat.speaker_id, "merchant");
        let warm_sample = get_gather_line(&chat.listener_name, Tone::Warm, 0);
        assert_eq!(chat.speaker_text, warm_sample, "好感高應走熱絡語氣");
    }

    #[test]
    fn single_npc_returns_none() {
        let npcs = vec![npc("merchant", 2400.0, 2200.0)];
        let rel = NpcRelationsState::new();
        assert!(pick_gather_pair(&npcs, &rel, 3).is_none());
    }
}
