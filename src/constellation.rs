//! 觀星連星座（ROADMAP 347）——天文維度的第一個玩家技巧玩法。
//!
//! 在此之前，天文這條維度全是「被動」的：`observatory`（132）每黎明自動廣播星象預報＋
//! 全服加成，`sky_codex`（337）走「在對的時刻在線」被動點亮天象圖鑑——玩家從沒有任何
//! **動手做**的觀星玩法。本模組補上它的第一個真玩法：夜裡抬頭，天上散著一片星點，玩家把它們
//! 依「今夜星座」連成線；連對了就把這個星座記進**星座錄**，給一小筆乙太＋探索熟練度。
//!
//! 機制刻意與既有切片**換骨架**（reviewer 自 #511 起一再要求別在同一維度連發同骨架）：
//!   - 不是釣魚（346）那種「等待→反應計時」狀態機，而是**空間連線**（把對的星點兩兩連起來）。
//!   - 不是表情共鳴／擊掌（338~342）那種「偵測配對→特效」，而是玩家**主動構圖**並由伺服器驗證。
//!
//! 設計取捨（最低風險）：
//!   - **純查表 / 純集合運算**，零 LLM、零額度。星座目錄是靜態原創資料（蒸汽太空歌劇主題、避 IP）。
//!   - **記憶體模式**：玩家已連過的星座壓成單一 `u64` bitmask，掛在 `Player` 上、不入快照、不持久化、
//!     零 migration（鏡像 `pet`／`ranching`／`fishing` 等記憶體切片）。重啟後星座錄歸零、可重新連、
//!     重新領那一小筆獎勵——獎勵刻意壓到近乎零經濟擾動（同 `sky_codex` 取向），farming 不破壞平衡。
//!   - **今夜星座**由一個「夜數」決定（`game.rs` 每進入一次夜晚就 +1）：星座逐夜輪替，
//!     人人同夜看到同一座；伺服器握有權威目錄，驗證一律以伺服器重算的今夜星座為準，
//!     前端送什麼星點都不算數（防作弊）。
//!   - **連線判定與方向／順序無關**：玩家點星點連邊，伺服器把兩端的邊正規化成 `(min,max)` 後
//!     比對「邊的集合」是否與星座完全一致（多一邊、少一邊都不算數），對玩家最寬容。
//!
//! 面向玩家字串（星座名）集中在本檔 `CATALOG`，為 i18n 集中替換點。

/// 連對一個「今夜星座」首次記入星座錄的乙太獎勵（刻意壓小，近乎零經濟擾動）。
pub const ETHER_REWARD: u32 = 6;

/// 連對一個「今夜星座」首次記入星座錄給的探索熟練度 XP（觀星＝探索的一種）。
pub const EXPLORER_XP: u32 = 12;

/// 一顆星在星圖面板裡的位置，正規化到 `[0,1] × [0,1]`（前端再映射到實際畫布大小）。
#[derive(Debug, Clone, Copy)]
pub struct Star {
    pub x: f32,
    pub y: f32,
}

/// 一個星座：一串星點 + 一串「該連起來的邊」（邊以星點索引對表示，方向無意義）。
#[derive(Debug, Clone, Copy)]
pub struct Constellation {
    /// 穩定 wire key（snake_case）：前端據此對應圖示與在地化字串；亦為 bitmask 對應的穩定契約。
    pub key: &'static str,
    /// 顯示名（繁中；i18n 集中替換點）。
    pub name: &'static str,
    /// 面板與播報用 emoji。
    pub emoji: &'static str,
    /// 星點座標（正規化）。索引即「星的編號」，前端與驗證共用同一套編號。
    pub stars: &'static [Star],
    /// 構成此星座的邊（星點索引對）。**目錄內每對務必已是 `a < b` 的正規化形式**，
    /// 且同一座內不重複——測試會守住這個不變式。
    pub edges: &'static [(u8, u8)],
}

/// 內部小工具：宣告一顆星。
const fn s(x: f32, y: f32) -> Star {
    Star { x, y }
}

/// 全部星座目錄（蒸汽太空歌劇主題、原創、避 IP）。
///
/// **順序為穩定契約**：bitmask 的第 i 位對應 `CATALOG[i]`，日後**只可往末尾新增、絕不重排／插隊**
/// （否則記憶體內已連過的位元語意會錯位；雖記憶體模式重啟即清，仍照圖鑑慣例釘死，最省心）。
/// 「今夜星座」以夜數對 `CATALOG.len()` 取模逐夜輪替，故目錄長度即一輪觀星的長度。
pub const CATALOG: &[Constellation] = &[
    // 0 ── 飛船座：船身一條斜桁 + 兩翼，像一艘斜飛的蒸汽飛船。
    Constellation {
        key: "airship",
        name: "飛船座",
        emoji: "🚀",
        stars: &[
            s(0.20, 0.30), // 0 船首
            s(0.50, 0.45), // 1 船身中
            s(0.80, 0.62), // 2 船尾
            s(0.42, 0.18), // 3 上翼
            s(0.60, 0.72), // 4 下翼
        ],
        edges: &[(0, 1), (1, 2), (1, 3), (1, 4)],
    },
    // 1 ── 齒輪座：四顆星圍成一圈、中心一顆，像一枚轉動的齒輪。
    Constellation {
        key: "gear",
        name: "齒輪座",
        emoji: "⚙️",
        stars: &[
            s(0.50, 0.20), // 0 上
            s(0.80, 0.50), // 1 右
            s(0.50, 0.80), // 2 下
            s(0.20, 0.50), // 3 左
            s(0.50, 0.50), // 4 軸心
        ],
        edges: &[(0, 4), (1, 4), (2, 4), (3, 4)],
    },
    // 2 ── 燈塔座：一條直立塔身 + 塔頂兩道斜光。
    Constellation {
        key: "lighthouse",
        name: "燈塔座",
        emoji: "🗼",
        stars: &[
            s(0.50, 0.85), // 0 塔基
            s(0.50, 0.45), // 1 塔身
            s(0.50, 0.18), // 2 塔頂燈
            s(0.25, 0.30), // 3 左光
            s(0.75, 0.30), // 4 右光
        ],
        edges: &[(0, 1), (1, 2), (2, 3), (2, 4)],
    },
    // 3 ── 茶壺座：壺身四角 + 壺嘴，療癒世界的一壺熱茶。
    Constellation {
        key: "teapot",
        name: "茶壺座",
        emoji: "🫖",
        stars: &[
            s(0.30, 0.40), // 0 壺身左上
            s(0.65, 0.40), // 1 壺身右上
            s(0.65, 0.70), // 2 壺身右下
            s(0.30, 0.70), // 3 壺身左下
            s(0.85, 0.52), // 4 壺嘴
        ],
        edges: &[(0, 1), (1, 2), (2, 3), (0, 3), (1, 4)],
    },
    // 4 ── 風箏座：菱形四角 + 一條尾巴。
    Constellation {
        key: "kite",
        name: "風箏座",
        emoji: "🪁",
        stars: &[
            s(0.50, 0.15), // 0 頂
            s(0.72, 0.42), // 1 右
            s(0.50, 0.62), // 2 底
            s(0.28, 0.42), // 3 左
            s(0.50, 0.88), // 4 尾
        ],
        edges: &[(0, 1), (1, 2), (2, 3), (0, 3), (2, 4)],
    },
    // 5 ── 王冠座：底座一橫 + 三只尖峰，星港之王的冠冕。
    Constellation {
        key: "crown",
        name: "王冠座",
        emoji: "👑",
        stars: &[
            s(0.20, 0.65), // 0 底左
            s(0.80, 0.65), // 1 底右
            s(0.30, 0.30), // 2 左峰
            s(0.50, 0.20), // 3 中峰
            s(0.70, 0.30), // 4 右峰
        ],
        edges: &[(0, 1), (0, 2), (2, 3), (3, 4), (1, 4)],
    },
];

/// 星座總數（一輪觀星的長度）。
pub const TOTAL: usize = CATALOG.len();

/// 依「夜數」取今夜星座（逐夜輪替；`game.rs` 每進入一次夜晚就把夜數 +1）。
pub fn tonight(night_index: u64) -> &'static Constellation {
    &CATALOG[(night_index % TOTAL as u64) as usize]
}

/// wire key → 目錄索引（即 bitmask 位元）。找不到回 `None`。
pub fn index_of(key: &str) -> Option<u8> {
    CATALOG.iter().position(|c| c.key == key).map(|i| i as u8)
}

/// 把一條邊正規化成 `(min, max)`：連線方向無意義，`(a,b)` 與 `(b,a)` 視為同一條。
fn normalize_edge(a: u8, b: u8) -> (u8, u8) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// 把一串（可能含重複 / 反向 / 自環）的邊正規化成「乾淨的邊集合」：
/// 去自環（兩端同星）、方向歸一、去重，回傳已排序的邊向量（穩定可比較）。
fn clean_edges(edges: &[(u8, u8)]) -> Vec<(u8, u8)> {
    let mut out: Vec<(u8, u8)> = edges
        .iter()
        .filter(|(a, b)| a != b)
        .map(|&(a, b)| normalize_edge(a, b))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// 驗證玩家連出的邊是否「恰好」構成這個星座：
/// 把玩家連的邊與星座的邊各自正規化成集合，完全相等才算連對（多一邊、少一邊都不算）。
///
/// 玩家送來的索引可能界外（亂送 / 舊客戶端 / 被竄改）；任何引用不存在星點的邊一律剔除，
/// 不會 panic（延續載入時驗證脈絡：壞輸入安全降級，不讓它影響判定）。
pub fn check_trace(c: &Constellation, drawn: &[(u8, u8)]) -> bool {
    let n = c.stars.len() as u8;
    // 先剔掉引用界外星點的邊，再走乾淨化。
    let in_range: Vec<(u8, u8)> = drawn
        .iter()
        .filter(|(a, b)| *a < n && *b < n)
        .copied()
        .collect();
    let player = clean_edges(&in_range);
    let target = clean_edges(c.edges);
    player == target
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tonight_rotates_by_night_and_wraps() {
        // 逐夜輪替，且跨過目錄長度會繞回第 0 座。
        assert_eq!(tonight(0).key, CATALOG[0].key);
        assert_eq!(tonight(1).key, CATALOG[1].key);
        assert_eq!(tonight(TOTAL as u64 - 1).key, CATALOG[TOTAL - 1].key);
        assert_eq!(tonight(TOTAL as u64).key, CATALOG[0].key);
        assert_eq!(tonight(TOTAL as u64 + 2).key, CATALOG[2].key);
    }

    #[test]
    fn index_of_is_stable_and_inverse_of_catalog_order() {
        for (i, c) in CATALOG.iter().enumerate() {
            assert_eq!(index_of(c.key), Some(i as u8));
        }
        assert_eq!(index_of("no_such_constellation"), None);
    }

    #[test]
    fn catalog_edges_are_normalized_unique_and_in_range() {
        // 目錄不變式：每對邊已是 a<b、同座不重複、且索引都在星點範圍內。
        for c in CATALOG {
            let n = c.stars.len() as u8;
            let mut seen = std::collections::HashSet::new();
            for &(a, b) in c.edges {
                assert!(a < b, "{} 的邊 ({a},{b}) 未正規化（需 a<b）", c.key);
                assert!(b < n, "{} 的邊 ({a},{b}) 索引界外（星數 {n}）", c.key);
                assert!(seen.insert((a, b)), "{} 的邊 ({a},{b}) 重複", c.key);
            }
            // 每個星座至少要有 3 條邊才像個「圖形」。
            assert!(c.edges.len() >= 3, "{} 邊太少", c.key);
        }
    }

    #[test]
    fn exact_match_traces_correctly() {
        let c = &CATALOG[0];
        // 照目錄原樣連 → 對。
        assert!(check_trace(c, c.edges));
    }

    #[test]
    fn order_and_direction_do_not_matter() {
        let c = &CATALOG[0]; // edges: (0,1),(1,2),(1,3),(1,4)
        // 打亂順序 + 反向 → 仍對。
        let drawn = [(2u8, 1u8), (4, 1), (1, 0), (3, 1)];
        assert!(check_trace(c, &drawn));
    }

    #[test]
    fn duplicate_and_self_loop_edges_are_tolerated() {
        let c = &CATALOG[0];
        // 重複連同一邊 + 不小心點到同一顆星（自環）→ 去重去自環後仍對。
        let drawn = [(0u8, 1u8), (1, 0), (1, 2), (1, 3), (1, 4), (2, 2)];
        assert!(check_trace(c, &drawn));
    }

    #[test]
    fn missing_edge_fails() {
        let c = &CATALOG[0];
        // 少連一邊 → 不算。
        let drawn = [(0u8, 1u8), (1, 2), (1, 3)];
        assert!(!check_trace(c, &drawn));
    }

    #[test]
    fn extra_edge_fails() {
        let c = &CATALOG[0];
        // 多連一條不屬於星座的邊 → 不算。
        let mut drawn: Vec<(u8, u8)> = c.edges.to_vec();
        drawn.push((0, 2));
        assert!(!check_trace(c, &drawn));
    }

    #[test]
    fn out_of_range_edges_are_dropped_not_panicking() {
        let c = &CATALOG[0]; // 5 顆星，合法索引 0..4
        // 正確的邊 + 一條引用界外星點(99)的邊：界外邊被剔除後，剩下的恰好＝星座 → 仍對。
        let mut drawn: Vec<(u8, u8)> = c.edges.to_vec();
        drawn.push((0, 99));
        assert!(check_trace(c, &drawn));
        // 只送界外邊 → 清空後與非空星座不等 → 不算（且不 panic）。
        assert!(!check_trace(c, &[(50, 99), (10, 20)]));
    }

    #[test]
    fn empty_trace_fails() {
        let c = &CATALOG[0];
        assert!(!check_trace(c, &[]));
    }

    #[test]
    fn all_constellations_validate_against_their_own_edges() {
        // 每一座都能被它自己的邊連對（守住目錄整體自洽）。
        for c in CATALOG {
            assert!(check_trace(c, c.edges), "{} 無法被自己的邊連對", c.key);
        }
    }
}
