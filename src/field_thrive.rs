//! 連片沃土（ROADMAP 367）——農地的空間湧現純邏輯層。
//!
//! 「照顧」不再只是一格一格各自為政：當玩家把作物**種成相鄰連成一片**時，這片土地
//! 會湧現成「生機田畝（沃土）」——連片照料的作物彼此牽動、長得更快。一格孤零零的作物
//! 只是一株苗；三格以上**四方相鄰**連成的一片，才是一塊有生機的田畝。
//!
//! 這層只做一件純事：給定「每格是否種了作物」的佔用遮罩，算出哪些格屬於
//! 「夠大的連通田畝」（size ≥ `THRIVE_MIN_PATCH`）。是純函式、確定性、無 IO，
//! 不依賴 `field.rs` 的 `Tile` 型別（吃 `&[bool]`），便於獨立測試。接線方式：
//!   - `field.rs::tick`：每 tick 先算遮罩，連片格的作物以 `grow_boosted` 加速成長。
//!   - `field.rs::view`：每格帶上 `thriving` 旗標，前端把連片田畝畫得更蒼翠。
//!
//! 演算法：以四方相鄰（上下左右，不含對角）為邊，對「有作物」的格求連通分量；
//! 落在 size ≥ 門檻的分量內者標 `true`。與 `town_blocs.rs` 同一個連通分量家族，
//! 但作用在**空間網格**、是非社交維度的另一種群體湧現。
//!
//! 效能：純整數掃描 + 迭代式洪水填（無遞迴、無 LLM、零配置），一塊地至多數十格，
//! 每幀成本近乎零。

/// 構成「一片沃土」所需的最小連通格數——三格以上四方相鄰才算（鏡像 `town_blocs`
/// 的 size ≥ 3 群體湧現門檻：一格是苗、兩格仍嫌孤，三格連片才見田畝生機）。
pub const THRIVE_MIN_PATCH: usize = 3;

/// 連片沃土的成長加速倍率——連片照料的作物成長快這麼多（濕度仍按真實時間消耗，
/// 不因加速而更快乾，維持公平）。刻意溫和（1.5×）：是「把田種得齊整」的療癒回饋，
/// 不送額外乙太、不碰經濟核心，近乎零平衡擾動。
pub const THRIVE_GROWTH_MULT: f32 = 1.5;

/// 給定 row-major 的「每格是否種了作物」遮罩與欄數，回傳等長的「是否屬於沃土」遮罩。
///
/// 四方相鄰（上下左右）連通；落在 size ≥ `min_patch` 連通分量內的格回 `true`。
/// `cols == 0`、或長度不是 `cols` 倍數時，視為無有效網格，整片回 `false`（防呆）。
/// 純函式、確定性：同輸入永遠同輸出。
pub fn thriving_mask(occupied: &[bool], cols: usize, min_patch: usize) -> Vec<bool> {
    let n = occupied.len();
    let mut out = vec![false; n];
    if cols == 0 || n % cols != 0 {
        return out;
    }
    let rows = n / cols;
    let mut visited = vec![false; n];
    // 迭代式洪水填（顯式堆疊，避免深格網遞迴爆棧）。
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..n {
        if visited[start] || !occupied[start] {
            continue;
        }
        // 蒐集這個連通分量的所有格。
        let mut component: Vec<usize> = Vec::new();
        visited[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            component.push(i);
            let row = i / cols;
            let col = i % cols;
            // 四方鄰居（邊界內、有作物、未訪）。
            let mut push_if = |ni: usize| {
                if !visited[ni] && occupied[ni] {
                    visited[ni] = true;
                    stack.push(ni);
                }
            };
            if col > 0 {
                push_if(i - 1);
            }
            if col + 1 < cols {
                push_if(i + 1);
            }
            if row > 0 {
                push_if(i - cols);
            }
            if row + 1 < rows {
                push_if(i + cols);
            }
        }
        // 夠大才算沃土，整個分量一起點亮。
        if component.len() >= min_patch {
            for i in component {
                out[i] = true;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // 用一個小工具：把 'X'（有作物）/ '.'（沒種）的字串列轉成佔用遮罩。
    fn mask_from(rows: &[&str], cols: usize) -> Vec<bool> {
        let mut v = Vec::new();
        for r in rows {
            let chars: Vec<char> = r.chars().collect();
            assert_eq!(chars.len(), cols, "每列字數須等於 cols");
            for c in chars {
                v.push(c == 'X');
            }
        }
        v
    }

    #[test]
    fn empty_grid_all_false() {
        let occ = mask_from(&["...", "...", "..."], 3);
        let m = thriving_mask(&occ, 3, THRIVE_MIN_PATCH);
        assert!(m.iter().all(|&t| !t));
        assert_eq!(m.len(), occ.len());
    }

    #[test]
    fn single_cell_not_thriving() {
        let occ = mask_from(&["X..", "...", "..."], 3);
        let m = thriving_mask(&occ, 3, THRIVE_MIN_PATCH);
        assert!(m.iter().all(|&t| !t));
    }

    #[test]
    fn pair_below_threshold_not_thriving() {
        // 兩格相鄰仍不足三格門檻。
        let occ = mask_from(&["XX.", "...", "..."], 3);
        let m = thriving_mask(&occ, 3, THRIVE_MIN_PATCH);
        assert!(m.iter().all(|&t| !t));
    }

    #[test]
    fn three_in_a_row_all_thriving() {
        let occ = mask_from(&["XXX", "...", "..."], 3);
        let m = thriving_mask(&occ, 3, THRIVE_MIN_PATCH);
        assert_eq!(&m[0..3], &[true, true, true]);
        assert!(m[3..].iter().all(|&t| !t));
    }

    #[test]
    fn three_in_a_column_all_thriving() {
        // 縱向連通（靠 ±cols 鄰居）。
        let occ = mask_from(&["X..", "X..", "X.."], 3);
        let m = thriving_mask(&occ, 3, THRIVE_MIN_PATCH);
        assert!(m[0] && m[3] && m[6]);
        assert_eq!(m.iter().filter(|&&t| t).count(), 3);
    }

    #[test]
    fn l_shape_three_thriving() {
        let occ = mask_from(&["X..", "XX.", "..."], 3);
        let m = thriving_mask(&occ, 3, THRIVE_MIN_PATCH);
        assert!(m[0] && m[3] && m[4]);
        assert_eq!(m.iter().filter(|&&t| t).count(), 3);
    }

    #[test]
    fn diagonal_only_not_connected() {
        // 對角相鄰不算連通（四方相鄰），各自孤立、皆不成片。
        let occ = mask_from(&["X..", ".X.", "..X"], 3);
        let m = thriving_mask(&occ, 3, THRIVE_MIN_PATCH);
        assert!(m.iter().all(|&t| !t));
    }

    #[test]
    fn two_separate_pairs_neither_thriving() {
        // 左上一對、右下一對，互不相連，皆 < 3。
        let occ = mask_from(&["XX..", "....", "..XX"], 4);
        let m = thriving_mask(&occ, 4, THRIVE_MIN_PATCH);
        assert!(m.iter().all(|&t| !t));
    }

    #[test]
    fn large_block_all_thriving() {
        let occ = mask_from(&["XXX", "XXX"], 3);
        let m = thriving_mask(&occ, 3, THRIVE_MIN_PATCH);
        assert!(m.iter().all(|&t| t));
    }

    #[test]
    fn separate_big_and_small_only_big_thriving() {
        // 上排三格連片（沃土），右下孤格不成片。
        let occ = mask_from(&["XXX.", "....", "...X"], 4);
        let m = thriving_mask(&occ, 4, THRIVE_MIN_PATCH);
        assert!(m[0] && m[1] && m[2]);
        assert!(!m[11]); // 右下孤格
        assert_eq!(m.iter().filter(|&&t| t).count(), 3);
    }

    #[test]
    fn min_patch_param_respected() {
        // 同一對相鄰格：門檻 2 時成片、門檻 3 時不成片。
        let occ = mask_from(&["XX.", "...", "..."], 3);
        assert!(thriving_mask(&occ, 3, 2)[0]);
        assert!(!thriving_mask(&occ, 3, 3)[0]);
    }

    #[test]
    fn deterministic_same_input_same_output() {
        let occ = mask_from(&["XXX", "X.X", "XXX"], 3);
        let a = thriving_mask(&occ, 3, THRIVE_MIN_PATCH);
        let b = thriving_mask(&occ, 3, THRIVE_MIN_PATCH);
        assert_eq!(a, b);
    }

    #[test]
    fn malformed_dims_return_all_false() {
        // cols=0 或長度非 cols 倍數一律防呆回全 false、長度與輸入一致。
        let occ = vec![true; 5];
        assert_eq!(thriving_mask(&occ, 0, THRIVE_MIN_PATCH), vec![false; 5]);
        assert_eq!(thriving_mask(&occ, 3, THRIVE_MIN_PATCH), vec![false; 5]);
    }
}
