//! 古代啟靈——沙漠遺跡的符文石板記藏古代秘文（ROADMAP 384）。
//!
//! 在此之前，沙漠遺跡這條維度對玩家而言只有一種互動：**敲碎石板、得到古代碎片**。
//! 石板只是「會掉材料的牆壁」，遺跡本身沒有任何故事、沒有任何玩家技巧介入。
//!
//! 本模組給它的**第一個真玩法**：石板背後藏著「古代秘文」。
//! 玩家消耗 3 塊古代碎片，向石板「啟靈」；石板依序閃現 4 個符文（記憶視窗 ~4 秒），
//! 玩家在時限內點按正確順序即可解鎖一篇秘文——得豐厚乙太獎勵＋探索熟練度＋全服廣播。
//!
//! 骨架刻意**與既有切片不同**（reviewer 自 #511 起的硬閘）：
//!   - 釣魚 346：反應計時（等咬鉤再收竿）。
//!   - 觀星 347：空間連線（把對的星點兩兩連起來）。
//!   - 礦脈 348：press-your-luck（越挖越深、見好就收）。
//!   - **本切片 384：序列記憶（記住閃現順序、再按出來）**——4 個符文中有重複的可能，
//!     靠的是短期記憶而非空間感或反應速度，是第四種不同骨架。
//!
//! 設計取捨：
//!   - **純靜態目錄**：6 篇秘文，序列固定，不隨機生成。讓老手可以「讀懂」所有銘文、
//!     仍可重複解鎖（重啟後記憶體清空、重新解也給一小筆獎勵），不懲罰新手。
//!   - **記憶體前置、不持久化、零 migration**（鏡像 `constellation` / `fishing`）。
//!   - **成本鐵律**：零 LLM、純整數比對，近乎零運算成本。
//!   - **獎勵刻意小**：不破壞現有乙太平衡；首次解碼乙太較多、重複解較少（由 ws.rs 判斷）。
//!
//! 面向玩家字串（銘文名稱、內容提示）集中在本檔 `CATALOG`，為 i18n 集中替換點。

use world_core::{biome_at, Biome};

/// 消耗的古代碎片數量（每次啟靈）。
pub const FRAGMENT_COST: u32 = 3;

/// 首次解碼秘文的乙太獎勵。
pub const ETHER_FIRST: u32 = 20;

/// 重複解碼（記憶體重啟後）的乙太獎勵。
pub const ETHER_REPEAT: u32 = 6;

/// 解碼成功給的探索熟練度 XP。
pub const EXPLORER_XP: u32 = 20;

/// 玩家在哪個半徑內（像素）視為「在沙漠遺跡區」可觸發啟靈。
const SAND_REACH: f32 = 80.0;

/// 6 個可用符文的 wire key（前端按鈕 key＋圖示對應表均以此為穩定契約）。
/// **順序為穩定契約，日後只可往末尾新增、絕不重排**。
pub const SYMBOLS: &[&str] = &["moon", "star", "flame", "gem", "thunder", "wave"];

/// 符文對應的 emoji（與 SYMBOLS 一一對應，供前端顯示）。
pub const SYMBOL_EMOJI: &[&str] = &["🌙", "⭐", "🔥", "💎", "⚡", "🌊"];

/// 一篇古代秘文：有固定名稱、emoji、以及一個4步符文序列（索引進 SYMBOLS）。
#[derive(Debug, Clone, Copy)]
pub struct Inscription {
    /// 穩定 wire key（snake_case），bitmask 位元以此索引為準。
    pub key: &'static str,
    /// 顯示名（繁中；i18n 集中替換點）。
    pub name: &'static str,
    /// 面板顯示 emoji。
    pub emoji: &'static str,
    /// 4 步符文序列（每個值為 SYMBOLS 的索引，0=moon...5=wave，可有重複）。
    pub sequence: &'static [u8],
}

/// 全部秘文目錄（蒸汽太空歌劇主題，原創不 IP）。
///
/// **順序為穩定契約**：bitmask 的第 i 位對應 `CATALOG[i]`。
/// 日後只可往末尾新增，絕不重排／插隊（否則記憶體前置 mask 語意會錯位）。
pub const CATALOG: &[Inscription] = &[
    // 0 ── 航行誌：星際航行者留下的星圖筆記。
    Inscription {
        key: "voyage_log",
        name: "航行誌",
        emoji: "🚀",
        sequence: &[1, 0, 2, 5], // ⭐ 🌙 🔥 🌊
    },
    // 1 ── 工匠心法：蒸汽機師留下的鍛造口訣。
    Inscription {
        key: "forge_creed",
        name: "工匠心法",
        emoji: "⚙️",
        sequence: &[4, 3, 4, 2], // ⚡ 💎 ⚡ 🔥
    },
    // 2 ── 燈塔誓言：守港人點燈不滅的承諾。
    Inscription {
        key: "lighthouse_vow",
        name: "燈塔誓言",
        emoji: "🗼",
        sequence: &[0, 5, 1, 0], // 🌙 🌊 ⭐ 🌙
    },
    // 3 ── 茶道哲思：星際旅人的修身之道。
    Inscription {
        key: "tea_wisdom",
        name: "茶道哲思",
        emoji: "🫖",
        sequence: &[5, 2, 3, 1], // 🌊 🔥 💎 ⭐
    },
    // 4 ── 風箏者歌：逐風而行的自由吟遊詩。
    Inscription {
        key: "kite_song",
        name: "風箏者歌",
        emoji: "🪁",
        sequence: &[1, 4, 0, 3], // ⭐ ⚡ 🌙 💎
    },
    // 5 ── 王冠詔書：星港古王的最後遺言。
    Inscription {
        key: "crown_decree",
        name: "王冠詔書",
        emoji: "👑",
        sequence: &[3, 0, 4, 5], // 💎 🌙 ⚡ 🌊
    },
];

/// 秘文總數。
pub const TOTAL: usize = CATALOG.len();

/// 依 wire key 取秘文（找不到回 None）。
pub fn by_key(key: &str) -> Option<&'static Inscription> {
    CATALOG.iter().find(|c| c.key == key)
}

/// wire key → 目錄索引（即 bitmask 位元）。找不到回 None。
pub fn index_of(key: &str) -> Option<u8> {
    CATALOG.iter().position(|c| c.key == key).map(|i| i as u8)
}

/// 驗證玩家送來的符文序列是否與秘文吻合。
///
/// `answer` 是玩家點按的 wire key 陣列（如 `["star", "moon", "flame", "wave"]`）；
/// 與 `inscription.sequence` 裡的 SYMBOLS 索引逐一比對。
/// 長度不符或含不合法符文 key 都算錯（安全降級、不 panic）。
pub fn check_sequence(inscription: &Inscription, answer: &[String]) -> bool {
    if answer.len() != inscription.sequence.len() {
        return false;
    }
    answer.iter().enumerate().all(|(i, key)| {
        // 找 key 在 SYMBOLS 中的索引，找不到即為非法符文 → 直接視為錯誤。
        if let Some(pos) = SYMBOLS.iter().position(|&s| s == key.as_str()) {
            inscription.sequence[i] == pos as u8
        } else {
            false
        }
    })
}

/// 判定玩家是否在沙漠生態域（Sand biome）附近——代表有遺跡可供啟靈。
///
/// 鏡像 `fishing::is_near_water`：取玩家周圍採樣點，任一落在 Sand 即算。
pub fn is_near_ruin(px: f32, py: f32) -> bool {
    let r = SAND_REACH;
    let samples: &[(f32, f32)] = &[
        (0.0, 0.0),
        (r, 0.0),
        (-r, 0.0),
        (0.0, r),
        (0.0, -r),
    ];
    samples.iter().any(|(dx, dy)| {
        biome_at((px + dx) as f64, (py + dy) as f64) == Biome::Sand
    })
}

/// 把秘文的 sequence 轉成 wire key 陣列，供協議傳輸。
pub fn sequence_keys(inscription: &Inscription) -> Vec<&'static str> {
    inscription.sequence.iter().map(|&i| SYMBOLS[i as usize]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_correct_total() {
        assert_eq!(CATALOG.len(), TOTAL);
        assert_eq!(TOTAL, 6);
    }

    #[test]
    fn every_inscription_has_unique_key() {
        let mut keys = std::collections::HashSet::new();
        for ins in CATALOG {
            assert!(keys.insert(ins.key), "重複 key：{}", ins.key);
        }
    }

    #[test]
    fn every_sequence_has_length_4() {
        for ins in CATALOG {
            assert_eq!(ins.sequence.len(), 4, "{} 的序列長度不是 4", ins.key);
        }
    }

    #[test]
    fn every_sequence_index_in_range() {
        for ins in CATALOG {
            for &idx in ins.sequence {
                assert!(
                    (idx as usize) < SYMBOLS.len(),
                    "{} 的序列索引 {} 超出 SYMBOLS 範圍",
                    ins.key, idx
                );
            }
        }
    }

    #[test]
    fn symbols_and_emoji_arrays_same_length() {
        assert_eq!(SYMBOLS.len(), SYMBOL_EMOJI.len());
    }

    #[test]
    fn index_of_is_stable_and_inverse_of_catalog_order() {
        for (i, ins) in CATALOG.iter().enumerate() {
            assert_eq!(index_of(ins.key), Some(i as u8));
        }
        assert_eq!(index_of("no_such_key"), None);
    }

    #[test]
    fn by_key_returns_correct_inscription() {
        for ins in CATALOG {
            let found = by_key(ins.key).expect("找不到秘文");
            assert_eq!(found.key, ins.key);
        }
        assert!(by_key("nonexistent").is_none());
    }

    #[test]
    fn check_sequence_correct_answer_passes() {
        for ins in CATALOG {
            let answer: Vec<String> = sequence_keys(ins).iter().map(|&s| s.to_string()).collect();
            assert!(check_sequence(ins, &answer), "{} 的正確序列驗證失敗", ins.key);
        }
    }

    #[test]
    fn check_sequence_wrong_order_fails() {
        let ins = &CATALOG[0]; // voyage_log: [star, moon, flame, wave]
        // 把第一個和第二個對調
        let mut answer: Vec<String> = sequence_keys(ins).iter().map(|&s| s.to_string()).collect();
        answer.swap(0, 1);
        assert!(!check_sequence(ins, &answer));
    }

    #[test]
    fn check_sequence_wrong_length_fails() {
        let ins = &CATALOG[0];
        let short: Vec<String> = sequence_keys(ins).iter().take(3).map(|&s| s.to_string()).collect();
        assert!(!check_sequence(ins, &short));
        let long: Vec<String> = {
            let mut v = sequence_keys(ins).iter().map(|&s| s.to_string()).collect::<Vec<_>>();
            v.push("moon".to_string());
            v
        };
        assert!(!check_sequence(ins, &long));
    }

    #[test]
    fn check_sequence_unknown_symbol_fails() {
        let ins = &CATALOG[0];
        let answer: Vec<String> = vec![
            "unknown_rune".to_string(),
            "moon".to_string(),
            "flame".to_string(),
            "wave".to_string(),
        ];
        assert!(!check_sequence(ins, &answer));
    }

    #[test]
    fn sequence_keys_maps_indexes_correctly() {
        // CATALOG[0]: sequence [1,0,2,5] -> [star, moon, flame, wave]
        let keys = sequence_keys(&CATALOG[0]);
        assert_eq!(keys, vec!["star", "moon", "flame", "wave"]);
    }

    #[test]
    fn fragment_cost_is_positive() {
        assert!(FRAGMENT_COST > 0);
    }

    #[test]
    fn first_reward_greater_than_repeat_reward() {
        assert!(ETHER_FIRST > ETHER_REPEAT);
    }

    #[test]
    fn is_near_ruin_callable() {
        // 只確認函式可正常呼叫且回傳 bool（與 is_near_water 同模式）。
        let _ = is_near_ruin(3000.0, 3000.0);
    }
}
