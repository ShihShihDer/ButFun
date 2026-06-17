//! 放天燈（ROADMAP 372）——夜裡親手放一盞承載祝願的天燈，緩緩升上全服共享的夜空、隨真實時間飄移。
//!
//! 夜空這條維度至今玩家幾乎只能**被動觀賞或玩連線小遊戲**：天象台（132）每黎明自動報星象、
//! 觀星連星座（347）照圖把星點連起來、月相（187）／流星雨（133）／夜泉（…）都是世界自己上演。
//! 玩家從沒有一個「親手在夜空留下點什麼、而且別人也看得見」的玩法。本切片給夜空維度
//! **第一個玩家能動性玩法**：你挑一句祝願、放一盞天燈，它升上夜空、隨真實時間飄移約十分鐘後燃盡；
//! 全服共享——眾人各自放的燈匯成一片飄動的燈海，第一次有了「整片夜空因大家而暖起來」的 communal 之美。
//!
//! 設計鐵律（與漂流瓶 354 一脈相承）：
//! - **零自由文字**：祝願只能從一組預設句子裡挑（wire key 白名單），杜絕 XSS／審查，天生 i18n 友善
//!   （後端只存 wire key，面向玩家的句子由前端鏡像；此處中文僅作後端報讀／世界頻道備援）。
//! - **記憶體模式、有界、會過期**：不持久化（重啟清空）；每盞燈有 TTL、升空一陣即燃盡；
//!   全服總量、每人同時在空的量都設上限，記憶體永遠有界。
//! - **純社交／純呈現，零平衡風險**：不送任何物品／乙太／戰力，只是一盞會飄的燈與一句祝願。
//! - **純函式可測**：放／燃盡／容量淘汰全是 `LanternSky` 上的純邏輯，與 IO／鎖無關，結果確定可重現。
//! - **零 LLM**：升空與飄移由前端依每盞燈的 `seed` 確定推算（無需逐幀廣播），後端只握「有哪些燈」。

use uuid::Uuid;

/// 預設祝願白名單：(wire key, 後端備援中文句)。
/// wire key 是穩定協議契約、不面向玩家；面向玩家的顯示句以前端鏡像為準（i18n 集中在前端），
/// 這裡的中文僅作世界頻道／報讀備援。新增句子兩邊（此處 + 前端 `LANTERN_WISHES`）要同步。
/// 主題沿蒸汽太空歌劇的療癒基調，避真實 IP。
pub const PRESET_WISHES: &[(&str, &str)] = &[
    ("peace", "願這片星海永遠安寧"),
    ("good_health", "願你身體康健、平安順遂"),
    ("reunion", "願與惦念的人再相逢"),
    ("bountiful_harvest", "願田畝豐收、爐火常暖"),
    ("safe_voyage", "願每段旅途都一路順風"),
    ("bright_future", "願前路有光、所願皆成"),
    ("gratitude", "謝謝陪我走到這裡的每個人"),
    ("courage", "願我有再往前一步的勇氣"),
];

/// 全服同時在空的天燈上限（量小、列表整批廣播也無壓力）。
pub const MAX_ALOFT: usize = 60;
/// 每位玩家同時在空的天燈上限（超過就頂掉自己最舊的，防洗版）。
pub const MAX_PER_AUTHOR: usize = 3;
/// 一盞天燈的存在時長（秒）。約 10 分鐘升空飄移後燃盡消失。
pub const LANTERN_TTL_SECS: f32 = 600.0;

/// 判斷一個 wire key 是否為合法預設祝願。
pub fn is_valid_wish_key(key: &str) -> bool {
    PRESET_WISHES.iter().any(|(k, _)| *k == key)
}

/// 取某祝願 key 的後端備援中文句（世界頻道飄字用）。未知 key 回 `None`。
pub fn wish_text(key: &str) -> Option<&'static str> {
    PRESET_WISHES.iter().find(|(k, _)| *k == key).map(|(_, zh)| *zh)
}

/// 一盞升在夜空、還沒燃盡的天燈。
#[derive(Debug, Clone)]
pub struct SkyLantern {
    /// 天燈唯一識別碼（遞增；前端用來去重、追蹤同一盞的動畫）。
    pub id: u64,
    /// 放燈玩家 id（用來算每人在空上限）。
    pub author_id: Uuid,
    /// 放燈玩家顯示名（快照用，不再回查 players）。
    pub author_name: String,
    /// 祝願 wire key（白名單內）。
    pub wish_key: String,
    /// 已升空秒數，升到 `LANTERN_TTL_SECS` 即燃盡。
    pub age: f32,
    /// 由 id 確定推導的種子：散開水平起點與飄擺相位，讓前端不必逐幀廣播就能各自推算飄移軌跡。
    pub seed: u32,
}

/// 全服夜空。記憶體、有界、會燃盡。
#[derive(Debug)]
pub struct LanternSky {
    /// 在空的天燈（FIFO：前面較舊）。
    aloft: Vec<SkyLantern>,
    /// 遞增 id 來源。
    next_id: u64,
}

impl LanternSky {
    pub fn new() -> Self {
        Self { aloft: Vec::new(), next_id: 1 }
    }

    /// 目前在空的天燈數。
    pub fn count(&self) -> usize {
        self.aloft.len()
    }

    /// 在空的天燈列表（廣播給前端渲染）。
    pub fn lanterns(&self) -> &[SkyLantern] {
        &self.aloft
    }

    /// 放一盞天燈。回 `Some(新燈 id)` 成功；`None` 表示祝願 key 不合法。
    /// 容量規則：先頂掉自己最舊的（若已達 `MAX_PER_AUTHOR`），再頂掉全服最舊的（若已達 `MAX_ALOFT`）。
    pub fn release(
        &mut self,
        author_id: Uuid,
        author_name: impl Into<String>,
        wish_key: &str,
    ) -> Option<u64> {
        if !is_valid_wish_key(wish_key) {
            return None;
        }
        // 同一玩家在空的天燈已達上限 → 頂掉他自己最舊的那盞（Vec 前面的較舊）。
        while self.aloft.iter().filter(|l| l.author_id == author_id).count() >= MAX_PER_AUTHOR {
            if let Some(idx) = self.aloft.iter().position(|l| l.author_id == author_id) {
                self.aloft.remove(idx);
            } else {
                break;
            }
        }
        // 全服已達上限 → 頂掉全服最舊的那盞（位置 0）。
        while self.aloft.len() >= MAX_ALOFT {
            self.aloft.remove(0);
        }
        let id = self.next_id;
        self.next_id += 1;
        // seed 由 id 確定推導（無 RNG，確定可重現）：取一個大奇數雜湊散開，前端據此定水平起點與飄擺相位。
        let seed = (id.wrapping_mul(2654435761) & 0xffff) as u32;
        self.aloft.push(SkyLantern {
            id,
            author_id,
            author_name: author_name.into(),
            wish_key: wish_key.to_string(),
            age: 0.0,
            seed,
        });
        Some(id)
    }

    /// 推進一個 tick：每盞燈升空計時，燃盡（age ≥ TTL）的移除。
    /// 回傳「在空的天燈數量是否變動」（讓呼叫端決定要不要重新廣播列表）。
    pub fn tick(&mut self, dt: f32) -> bool {
        let before = self.aloft.len();
        for l in &mut self.aloft {
            l.age += dt;
        }
        self.aloft.retain(|l| l.age < LANTERN_TTL_SECS);
        self.aloft.len() != before
    }
}

impl Default for LanternSky {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn valid_keys_only() {
        assert!(is_valid_wish_key("peace"));
        assert!(is_valid_wish_key("courage"));
        assert!(!is_valid_wish_key("definitely_not_a_key"));
        assert!(!is_valid_wish_key(""));
    }

    #[test]
    fn wish_text_maps_known_keys() {
        assert!(wish_text("peace").is_some());
        assert!(wish_text("bogus").is_none());
    }

    #[test]
    fn release_rejects_unknown_key() {
        let mut sky = LanternSky::new();
        assert_eq!(sky.release(pid(1), "阿光", "bogus"), None);
        assert_eq!(sky.count(), 0);
    }

    #[test]
    fn release_accepts_and_assigns_increasing_ids() {
        let mut sky = LanternSky::new();
        let a = sky.release(pid(1), "阿光", "peace").unwrap();
        let b = sky.release(pid(2), "小美", "courage").unwrap();
        assert!(b > a, "id 應遞增");
        assert_eq!(sky.count(), 2);
    }

    #[test]
    fn release_assigns_distinct_seeds() {
        let mut sky = LanternSky::new();
        sky.release(pid(1), "阿光", "peace").unwrap();
        sky.release(pid(2), "小美", "courage").unwrap();
        let seeds: Vec<u32> = sky.lanterns().iter().map(|l| l.seed).collect();
        assert_ne!(seeds[0], seeds[1], "相鄰兩盞燈的 seed 應不同（飄移軌跡才會散開）");
    }

    #[test]
    fn per_author_cap_evicts_own_oldest() {
        let mut sky = LanternSky::new();
        let p = pid(1);
        let first = sky.release(p, "阿光", "peace").unwrap();
        sky.release(p, "阿光", "courage").unwrap();
        sky.release(p, "阿光", "reunion").unwrap();
        // 第 4 盞：頂掉自己最舊的（first），仍維持 MAX_PER_AUTHOR 盞。
        sky.release(p, "阿光", "gratitude").unwrap();
        assert_eq!(sky.aloft.iter().filter(|l| l.author_id == p).count(), MAX_PER_AUTHOR);
        assert!(sky.aloft.iter().all(|l| l.id != first), "自己最舊的應被頂掉");
    }

    #[test]
    fn per_author_cap_does_not_evict_others() {
        let mut sky = LanternSky::new();
        let me = pid(1);
        let other = sky.release(pid(2), "別人", "peace").unwrap();
        // 我放滿 MAX_PER_AUTHOR + 1 盞，只該頂掉「我自己」最舊的，別人的不受影響。
        for _ in 0..(MAX_PER_AUTHOR + 1) {
            sky.release(me, "阿光", "peace").unwrap();
        }
        assert!(sky.aloft.iter().any(|l| l.id == other), "別人的燈不該被我頂掉");
        assert_eq!(sky.aloft.iter().filter(|l| l.author_id == me).count(), MAX_PER_AUTHOR);
    }

    #[test]
    fn global_cap_evicts_oldest() {
        let mut sky = LanternSky::new();
        for i in 0..MAX_ALOFT {
            sky.release(pid(i as u8), "玩家", "peace").unwrap();
        }
        assert_eq!(sky.count(), MAX_ALOFT);
        let oldest = sky.aloft[0].id;
        sky.release(pid(200), "新人", "courage").unwrap();
        assert_eq!(sky.count(), MAX_ALOFT, "總量維持上限");
        assert!(sky.aloft.iter().all(|l| l.id != oldest), "全服最舊的應被頂掉");
    }

    #[test]
    fn tick_burns_out_and_reports_change() {
        let mut sky = LanternSky::new();
        sky.release(pid(1), "阿光", "peace").unwrap();
        // 還沒燃盡：tick 不回報變動。
        assert!(!sky.tick(1.0));
        assert_eq!(sky.count(), 1);
        // 一次推進超過 TTL：燃盡、回報變動。
        assert!(sky.tick(LANTERN_TTL_SECS));
        assert_eq!(sky.count(), 0);
        // 空夜空再 tick：無變動。
        assert!(!sky.tick(1.0));
    }

    #[test]
    fn tick_ages_lanterns() {
        let mut sky = LanternSky::new();
        sky.release(pid(1), "阿光", "peace").unwrap();
        sky.tick(5.0);
        assert!((sky.lanterns()[0].age - 5.0).abs() < 1e-6, "age 應隨 tick 累加");
    }
}
