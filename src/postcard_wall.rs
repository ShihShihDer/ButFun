//! ROADMAP 482：旅人明信片牆——廣場上一面全服共見的公共風景牆。
//!
//! 旅途明信片（417 留影／480 寄給旅人）至今都是「私人或一對一」的紀念物：自己留著、
//! 或塞進某一位旅人的信箱。這個模組把它升級成「眾人共見」——玩家可以把當下框下的
//! 「此刻風景」明信片**貼上廣場的明信片牆**，匯成一面會輪替的公共風景牆，讓散落各地的
//! 旅程第一次在城鎮中心交織成大家都看得見的風景。
//!
//! 設計哲學沿用既有自我表達物件（雪人 478／螢燈 477）：
//! - **純記憶體、零持久化、零 migration**——重啟即清空，世界重新攢起一面新牆，不留包袱。
//! - **純資料＋純函式**——牆的容量／去重／淘汰規則全是確定性邏輯，好單元測試。
//! - **成本紀律**——零 LLM、零外部呼叫，只是一個有界環形緩衝。

use std::collections::VecDeque;

/// 牆上同時展示的明信片上限：只留最近的幾張，舊的自然被新的擠下牆。
/// 12 張足夠讓廣場熱鬧、又不至於擠爆畫面或快照頻寬。
pub const WALL_CAP: usize = 12;

/// 暱稱（署名）最長保留字元數——超出截斷，避免長名洗版牆面。
const MAX_BY_CHARS: usize = 16;

/// 一張貼在牆上的明信片。`key` 是貼牆者的穩定身分（玩家 id），**只用於同人去重**、
/// 不外送前端；其餘欄位是前端框成風景小卡所需的純文字內容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallCard {
    /// 貼牆者穩定身分（玩家 id 字串）。僅伺服器端，用來「同一人只保留最新一張」，不序列化給前端。
    pub key: String,
    /// 署名（貼牆者暱稱，已安全化）。
    pub by: String,
    /// 明信片標頭，如「晨光・🌸 春」。
    pub title: String,
    /// 所在地名。
    pub place: String,
    /// 此刻風景印記（手寫感的一句話）。
    pub flavor: String,
    /// 旅人資歷稱號。
    pub rank: String,
    /// 旅人等級。
    pub level: u32,
    /// 當下時辰 wire key（`dawn`／`day`／`dusk`／`night`）——前端據此替小卡上一層暖／冷色調。
    pub phase: String,
}

/// 廣場的明信片牆：一個有界的環形緩衝，最新貼的在最前。
#[derive(Debug, Default)]
pub struct PostcardWall {
    /// 牆上的卡片，最新在前（index 0）。長度恆 `<= WALL_CAP`。
    cards: VecDeque<WallCard>,
}

impl PostcardWall {
    pub fn new() -> Self {
        Self { cards: VecDeque::new() }
    }

    /// 把一張明信片貼上牆。規則（皆確定性）：
    /// 1. **同人去重**——若這位旅人（`key`）牆上已有舊卡，先撤下，避免一人連貼洗版整面牆；
    ///    `key` 為空（理論上不會發生，保守處理）則不去重、視為各自獨立。
    /// 2. 推到最前（最新）。
    /// 3. 超過 `WALL_CAP` 時從尾端淘汰最舊的，維持牆面有界。
    ///
    /// 回傳貼上後牆上的卡片數。
    pub fn pin(&mut self, card: WallCard) -> usize {
        if !card.key.is_empty() {
            self.cards.retain(|c| c.key != card.key);
        }
        self.cards.push_front(card);
        while self.cards.len() > WALL_CAP {
            self.cards.pop_back();
        }
        self.cards.len()
    }

    /// 牆上卡片（最新在前）。供快照建構逐張轉成前端 view。
    pub fn cards(&self) -> impl Iterator<Item = &WallCard> {
        self.cards.iter()
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }
}

/// 署名安全化：把控制字元折成空白、修剪首尾空白、截到 `MAX_BY_CHARS` 字元（依字元而非位元組，
/// 不切壞多位元組中文）；清理後為空則回「無名旅人」。比照 `postcard_mail::sanitize_note` 的防呆脈絡。
pub fn sanitize_by(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    let truncated: String = trimmed.chars().take(MAX_BY_CHARS).collect();
    let truncated = truncated.trim().to_string();
    if truncated.is_empty() {
        "無名旅人".to_string()
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(key: &str, by: &str) -> WallCard {
        WallCard {
            key: key.to_string(),
            by: by.to_string(),
            title: "晨光・🌸 春".to_string(),
            place: "翠幽谷".to_string(),
            flavor: "陽光把花香曬得滿地都是。".to_string(),
            rank: "旅者".to_string(),
            level: 12,
            phase: "day".to_string(),
        }
    }

    /// 最新貼的排在最前。
    #[test]
    fn newest_first() {
        let mut w = PostcardWall::new();
        w.pin(card("a", "阿光"));
        w.pin(card("b", "小白"));
        let names: Vec<_> = w.cards().map(|c| c.by.clone()).collect();
        assert_eq!(names, vec!["小白", "阿光"]);
    }

    /// 同一位旅人重複貼，只保留最新一張、且移到最前，不洗版。
    #[test]
    fn same_person_replaces_and_floats_to_front() {
        let mut w = PostcardWall::new();
        w.pin(card("a", "阿光"));
        w.pin(card("b", "小白"));
        // 阿光再貼一張（換個地名以便辨識是新卡）。
        let mut again = card("a", "阿光");
        again.place = "赤焰原".to_string();
        w.pin(again);
        assert_eq!(w.len(), 2, "同人去重後總數不變");
        let first = w.cards().next().unwrap();
        assert_eq!(first.by, "阿光");
        assert_eq!(first.place, "赤焰原", "保留的是最新那張");
    }

    /// 超過容量時從最舊端淘汰，牆面恆有界。
    #[test]
    fn caps_at_wall_cap_evicting_oldest() {
        let mut w = PostcardWall::new();
        for i in 0..(WALL_CAP + 5) {
            w.pin(card(&format!("p{i}"), &format!("旅人{i}")));
        }
        assert_eq!(w.len(), WALL_CAP, "不超過上限");
        // 最舊的幾張（p0..p4）應已被擠下牆，最新的 p{cap+4} 在最前。
        let newest = w.cards().next().unwrap();
        assert_eq!(newest.by, format!("旅人{}", WALL_CAP + 4));
        assert!(
            w.cards().all(|c| c.key != "p0"),
            "最舊的卡片已被淘汰"
        );
    }

    /// 空 key 不去重（保守：視為各自獨立的匿名卡）。
    #[test]
    fn empty_key_not_deduped() {
        let mut w = PostcardWall::new();
        w.pin(card("", "訪客"));
        w.pin(card("", "訪客"));
        assert_eq!(w.len(), 2);
    }

    /// 署名安全化：控制字元折空白、截長、空字串退「無名旅人」。
    #[test]
    fn sanitize_by_rules() {
        assert_eq!(sanitize_by("  阿光  "), "阿光");
        assert_eq!(sanitize_by("阿\n光"), "阿 光");
        assert_eq!(sanitize_by(""), "無名旅人");
        assert_eq!(sanitize_by("   "), "無名旅人");
        // 超長截到 MAX_BY_CHARS 個字元（中文不切壞）。
        let long = "字".repeat(40);
        let out = sanitize_by(&long);
        assert_eq!(out.chars().count(), MAX_BY_CHARS);
    }
}
