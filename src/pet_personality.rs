//! 寵物性格（ROADMAP 358「寵物有了脾氣」的純邏輯層）。
//!
//! 寵物自 ROADMAP 46 上線、343～345 長出身體（跟隨／玩耍／接物）以來，整條維度都在處理「身體怎麼動」，
//! 卻一直**沒有任何內在屬性**——同種寵物對每個主人都長得一模一樣、行為一模一樣。本模組給寵物第一個
//! **內在屬性**：性格。讓「會跟隨的身體」長成「有脾氣的個體」——同樣是飄舞精靈，你的這隻可能黏人到
//! 寸步不離，別人的那隻卻慵懶愛在後頭晃。
//!
//! 性格是**確定性湧現**，不是隨機、也不持久化：由「主人帳號 ＋ 寵物種類」雜湊而來——同一個主人的
//! 同種寵物永遠是同一種脾氣（重連／重啟都一致），但不同主人、或同一主人換了別種寵物，脾氣就不同。
//! 不寫死、不需 migration、零持久化（每次要用時即時算）。
//!
//! 這層只管**會影響行為的純邏輯**（性格 → 跟隨歇腳距離），呈現面（中文標籤、心情泡泡 emoji）留給前端，
//! 面向玩家字串集中前端、保留 i18n 空間。延續 `pet_follow` / `pet_play` 的慣例：純資料 ＋ 純函式、
//! 無 IO、不碰 WebSocket／遊戲迴圈，由 `game.rs` 每 tick 餵呼叫、由 `state.rs` 快照時即時算出 wire key。

use crate::pet::PetKind;

/// 寵物的四種性格。刻意只給四種、彼此鮮明好辨（黏人／活潑／好奇／慵懶），
/// 由「歇腳時離主人多近」直接讀得出來，不堆細節。
///
/// 變體順序即 `ALL` 的索引順序，是 `personality_for` 雜湊取模的穩定契約——**只可尾端追加、不可重排**
/// （重排會讓既有玩家的寵物換脾氣）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PetPersonality {
    /// 活潑：精力旺盛，在主人腳邊保持一般距離蹦蹦跳跳。
    Playful,
    /// 慵懶：愛在後頭悠悠地晃，歇腳離主人最遠、最放鬆。
    Lazy,
    /// 好奇：總想離主人遠一點東張西望，歇腳距離偏遠。
    Curious,
    /// 黏人：寸步不離，歇腳貼得最近。
    Clingy,
}

impl PetPersonality {
    /// 全部性格，順序即雜湊取模索引（穩定契約，只可尾端追加）。
    pub const ALL: [PetPersonality; 4] = [
        PetPersonality::Playful,
        PetPersonality::Lazy,
        PetPersonality::Curious,
        PetPersonality::Clingy,
    ];

    /// Wire key（前端 ＋ 序列化用，snake_case）。穩定協議契約。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Playful => "playful",
            Self::Lazy => "lazy",
            Self::Curious => "curious",
            Self::Clingy => "clingy",
        }
    }

    /// 從 wire key 解析（前端送來／測試用）。未知字串回 None。
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "playful" => Some(Self::Playful),
            "lazy" => Some(Self::Lazy),
            "curious" => Some(Self::Curious),
            "clingy" => Some(Self::Clingy),
            _ => None,
        }
    }

    /// 跟隨時的歇腳距離（px）——性格的**唯一行為差異**：黏人貼最近、慵懶／好奇愛在後頭。
    /// 以 `pet_follow::FOLLOW_STOP`(30) 為中心、上下小幅偏移，讓差異看得出來又不誇張、不影響跟隨手感。
    pub fn follow_stop(self) -> f32 {
        match self {
            Self::Clingy => 20.0,  // 寸步不離
            Self::Playful => 30.0, // 一般距離（＝既有預設）
            Self::Curious => 40.0, // 愛東張西望、離遠一點
            Self::Lazy => 46.0,    // 最放鬆、愛在後頭晃
        }
    }
}

/// 由「主人帳號 ＋ 寵物種類」確定性算出性格（FNV-1a 雜湊取模）。
///
/// 純函式、零狀態、結果確定可重現：同一 `(owner_id, kind)` 永遠得同一性格（重連／重啟一致），
/// 不同主人或不同種類則可能不同。`owner_id` 取玩家 UUID 的 16 bytes（`uuid::as_bytes`）。
pub fn personality_for(owner_id: &[u8; 16], kind: PetKind) -> PetPersonality {
    // FNV-1a 32-bit：簡單、無外部相依、確定可重現。先吃 owner id 16 bytes、再吃種類 wire key，
    // 讓「同主人換寵物種類」也會換脾氣。
    const FNV_OFFSET: u32 = 0x811c_9dc5;
    const FNV_PRIME: u32 = 0x0100_0193;
    let mut hash = FNV_OFFSET;
    for &b in owner_id {
        hash ^= b as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    for &b in kind.as_str().as_bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    PetPersonality::ALL[(hash % PetPersonality::ALL.len() as u32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_key_round_trips() {
        for p in PetPersonality::ALL {
            let s = p.as_str();
            assert_eq!(PetPersonality::from_str(s), Some(p), "wire key 往返：{s}");
        }
    }

    #[test]
    fn unknown_key_returns_none() {
        assert!(PetPersonality::from_str("grumpy").is_none());
        assert!(PetPersonality::from_str("").is_none());
        assert!(PetPersonality::from_str("Playful").is_none()); // 大小寫敏感
    }

    #[test]
    fn wire_keys_unique() {
        let keys: Vec<&str> = PetPersonality::ALL.iter().map(|p| p.as_str()).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "wire key 不可重複");
    }

    #[test]
    fn follow_stop_ordering_and_bounds() {
        // 黏人貼最近 < 活潑（預設）< 好奇 < 慵懶最放鬆；皆為正值、且落在合理範圍內。
        let clingy = PetPersonality::Clingy.follow_stop();
        let playful = PetPersonality::Playful.follow_stop();
        let curious = PetPersonality::Curious.follow_stop();
        let lazy = PetPersonality::Lazy.follow_stop();
        assert!(clingy < playful, "黏人應貼得比活潑近");
        assert!(playful < curious, "好奇應比活潑離得遠");
        assert!(curious < lazy, "慵懶應最放鬆、離得最遠");
        for s in [clingy, playful, curious, lazy] {
            assert!(s > 0.0, "歇腳距離應為正");
            assert!(s <= 60.0, "歇腳距離不該誇張到離主人太遠");
        }
    }

    #[test]
    fn personality_is_deterministic() {
        let id = [7u8; 16];
        let a = personality_for(&id, PetKind::FlutterSprite);
        let b = personality_for(&id, PetKind::FlutterSprite);
        assert_eq!(a, b, "同 (owner, kind) 必得同性格");
    }

    #[test]
    fn different_kind_can_differ() {
        // 同主人換種類會重新吃種類 wire key——不保證每次都不同，但雜湊輸入確實變了。
        // 用一個已知會分到不同桶的例子守住「種類有參與雜湊」這個契約。
        let id = [0u8; 16];
        let kinds = [
            PetKind::FlutterSprite,
            PetKind::CrystalGolem,
            PetKind::CoralCrab,
            PetKind::JadeWraith,
            PetKind::OriginGuardian,
        ];
        let got: std::collections::HashSet<_> =
            kinds.iter().map(|&k| personality_for(&id, k)).collect();
        // 同一主人的五種寵物，性格不該全擠在同一種（雜湊確實依種類分流）。
        assert!(got.len() >= 2, "種類應參與雜湊、產生分流，實得 {} 種", got.len());
    }

    #[test]
    fn distribution_covers_all_four() {
        // 掃過許多 owner id，四種性格都該出現（雜湊分佈不偏到只剩幾種）。
        let mut seen = std::collections::HashSet::new();
        for i in 0..256u32 {
            let mut id = [0u8; 16];
            id[0] = (i & 0xff) as u8;
            id[1] = ((i >> 8) & 0xff) as u8;
            seen.insert(personality_for(&id, PetKind::FlutterSprite));
        }
        assert_eq!(seen.len(), 4, "256 個 id 應涵蓋全部四種性格，實得 {}", seen.len());
    }

    #[test]
    fn all_array_matches_variant_count() {
        // ALL 應恰好列出所有變體（防尾端追加變體卻忘了加進 ALL）。
        assert_eq!(PetPersonality::ALL.len(), 4);
        // 每個變體都能 round-trip、有非空 wire key。
        for p in PetPersonality::ALL {
            assert!(!p.as_str().is_empty());
        }
    }
}
