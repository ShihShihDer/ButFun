//! 乙太經濟：把收成的乙太「花出去」的純邏輯地基（Phase 0-G-O2「用乙太購買」）。
//!
//! 目前玩家收成只會讓乙太數字一直變大、沒有任何地方花——沒有目標、沒有「再玩一下」
//! 的理由（見 `docs/PLAN.md` 當前主攻：把乙太的去處做出來、收口種田經濟迴圈）。這層
//! 提供第一個**乙太消耗點**的權威規則：玩家用乙太「擴大自己那塊農地一格」，乙太數字
//! 隨之變少。規則只在這裡（伺服器權威），純資料 + 純函式、無 IO、不碰 WebSocket /
//! 遊戲迴圈，便於自動測試。延續 `crafting.rs` / `inventory.rs` / `field.rs` 的前置慣例：
//! 純邏輯先落地、標 `allow(dead_code)`，接線輪才有呼叫端。
//!
//! 之後接上（接線屬架構級、動 live 廣播 shape，留待 backend lane / 後續 PR）：
//!   - ws：玩家送「買一格擴張」意圖 → `PlotWallet::buy_expansion(player.ether)` 扣乙太、
//!     `Field` 多開一格可耕地；乙太不夠 / 達上限則回饋失敗、不扣款。
//!   - 持久化（接 0-E）：把每位玩家的 `PlotWallet`（已購擴張格數）存回 Postgres，
//!     跨重啟仍在（驗收標準「擴張跨重啟還在」）。
//!   - 前端：顯示「下一格要多少乙太」、乙太不足時反灰購買鈕。
//!
//! 設計重點：
//!   - **全有全無扣款**：乙太夠才扣、扣完絕不為負（`spend` 用 `checked_sub`），比照
//!     `crafting::craft` 的「材料不足不給合」語意——避免「扣了一半才發現不夠」。
//!   - **逐格漲價**：每多買一格，下一格更貴（`expansion_cost`）。這讓乙太成為**長期**
//!     消耗去處：愈擴愈貴，玩家得持續收成才買得起，種田迴圈才有可持續的目標。
//!   - **有上限**：一塊地最多擴 `MAX_EXPANSIONS` 格（世界有界、農地語意不該無限長）。
//!     上限也讓 `is_loadable` 有實質不變式可驗（接 0-E 載入時擋壞值）。

use serde::{Deserialize, Serialize};

/// 擴張「第一格」的基準價（乙太）。挑得比一次收成略高，讓第一格也要存幾輪才買得起、
/// 有「攢乙太換東西」的手感；接線後可依實際收成節奏再調。
pub const EXPANSION_BASE_COST: u32 = 10;

/// 一塊地最多能買幾格擴張。世界與農地語意都不該無限長大，故設上限；確切值是調校常數，
/// 接線（`Field` 實際多開格）時再依農地成長收斂。
pub const MAX_EXPANSIONS: u32 = 12;

/// 一筆消費的「全有全無」扣款核心：`balance` 夠付 `cost` 就回扣款後的新餘額，
/// 不夠（`cost > balance`）回 `None` 完全不扣——乙太餘額**永遠不為負**。
///
/// 用 `checked_sub` 一行表達這個不變式：所有乙太消耗點（擴地、日後買種子 / 商店）都
/// 該走這個單一真實來源，而不是各自手寫 `if balance >= cost` 容易漏掉的比較。
// 前置地基：接線輪才有呼叫端，比照本模組其他項標 `allow(dead_code)`。
#[allow(dead_code)]
pub fn spend(balance: u32, cost: u32) -> Option<u32> {
    balance.checked_sub(cost)
}

/// 已擁有 `owned` 格擴張時，買「下一格」要多少乙太。逐格線性漲價：
/// 第 1 格 = 基準價、第 2 格 = 2×、第 3 格 = 3×……愈擴愈貴，當長期乙太消耗去處。
///
/// 用 `saturating` 算術防溢位（`owned` 已被 `MAX_EXPANSIONS` 上限與型別夾住，
/// 正常範圍內不會接近溢位，但載入壞值時飽和到上限總比 panic 好）。
#[allow(dead_code)]
pub fn expansion_cost(owned: u32) -> u32 {
    EXPANSION_BASE_COST.saturating_mul(owned.saturating_add(1))
}

/// 一位玩家「用乙太買農地擴張」的累積狀態（買了幾格）。
///
/// 衍生 serde 作為持久化格式地基（接 0-E）：跨重啟存回已購格數，達成驗收標準
/// 「擴張跨重啟還在」。載入時以 `is_loadable` 驗證（不超上限），比照
/// `crops::is_loadable` / `inventory::is_loadable` 的載入防線。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlotWallet {
    /// 已購買的擴張格數（單一真實來源；下一格價格與是否達上限都由它推導）。
    expansions: u32,
}

// 整個型別是前置地基：接線輪（ws 購買意圖、Field 多開格、前端購買鈕）才有呼叫端。
#[allow(dead_code)]
impl PlotWallet {
    /// 全新玩家：一格都還沒買。
    pub fn new() -> Self {
        Self { expansions: 0 }
    }

    /// 目前已購買的擴張格數。
    pub fn expansions(&self) -> u32 {
        self.expansions
    }

    /// 是否還能再買（未達上限）。
    pub fn can_expand(&self) -> bool {
        self.expansions < MAX_EXPANSIONS
    }

    /// 買「下一格」的價格；已達上限則回 `None`（沒得買、前端據此反灰）。
    pub fn next_cost(&self) -> Option<u32> {
        if self.can_expand() {
            Some(expansion_cost(self.expansions))
        } else {
            None
        }
    }

    /// 用乙太買一格擴張（**全有全無**）：未達上限**且**乙太夠付下一格價，才扣款、
    /// 已購格數 +1，回扣款後的新乙太餘額；否則完全不動（不扣款、不加格）回 `None`。
    ///
    /// 兩道前提（`next_cost` 擋上限、`spend` 擋餘額不足）都在改動狀態**之前**用 `?`
    /// 早退，故失敗時 `expansions` 絕不會被加上——維持與 `crafting::craft` 一致的
    /// 全有全無語意。接線：`player.ether = wallet.buy_expansion(player.ether)?`。
    pub fn buy_expansion(&mut self, ether: u32) -> Option<u32> {
        let cost = self.next_cost()?; // 達上限：沒得買
        let new_balance = spend(ether, cost)?; // 乙太不夠：不扣款
        self.expansions += 1;
        Some(new_balance)
    }

    /// 載入防線（接 0-E 從 Postgres 讀回時驗證）：已購格數不得超過上限。
    /// `expansions` 是 `u32`、型別本身就擋掉 `NaN` / `Inf` / 負值，這裡只需驗上界。
    pub fn is_loadable(&self) -> bool {
        self.expansions <= MAX_EXPANSIONS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_deducts_when_affordable() {
        // 夠付：扣掉成本、回新餘額。
        assert_eq!(spend(100, 30), Some(70));
        // 剛好付完歸零。
        assert_eq!(spend(30, 30), Some(0));
    }

    #[test]
    fn spend_refuses_and_never_goes_negative() {
        // 不夠付：回 None，不會變負數。
        assert_eq!(spend(20, 30), None);
        assert_eq!(spend(0, 1), None);
    }

    #[test]
    fn expansion_cost_escalates_per_tile() {
        // 逐格漲價：第 1 格基準價、之後線性遞增。
        assert_eq!(expansion_cost(0), EXPANSION_BASE_COST);
        assert_eq!(expansion_cost(1), EXPANSION_BASE_COST * 2);
        assert_eq!(expansion_cost(2), EXPANSION_BASE_COST * 3);
        // 後一格一定比前一格貴（嚴格遞增），確保是「愈擴愈貴」的長期消耗。
        assert!(expansion_cost(3) > expansion_cost(2));
    }

    #[test]
    fn expansion_cost_saturates_instead_of_panicking_on_overflow() {
        // doc 承諾「載入壞值時飽和到上限總比 panic 好」（`expansion_cost` 用 `saturating_mul`/
        // `saturating_add`），但此前無測試把關這條 panic-safety 不變式。接 0-E 從 Postgres
        // 讀回的 `owned` 若被竄改 / 改版灌成超大值，`expansion_cost` 仍可能在某些路徑被呼叫
        // （`is_loadable` 是另一道防線、不保證每個呼叫端都先驗）。把「乘法飽和、絕不 overflow
        // panic」鎖成測試：日後有人把 `saturating_mul` 改回裸 `*`，debug build 的 cargo test
        // 會當場紅燈，而不是接線後在線上載入壞檔時炸開。
        // owned = u32::MAX 時：owned+1 先飽和到 u32::MAX、再 ×基準價仍飽和到 u32::MAX，不 panic。
        assert_eq!(expansion_cost(u32::MAX), u32::MAX);
        // 逼近溢位邊界同樣不 panic，且飽和後封頂、不會回繞變小（維持「不遞減」）。
        assert!(expansion_cost(u32::MAX) >= expansion_cost(u32::MAX - 1));
    }

    #[test]
    fn new_wallet_starts_empty_and_can_expand() {
        let w = PlotWallet::new();
        assert_eq!(w.expansions(), 0);
        assert!(w.can_expand());
        assert_eq!(w.next_cost(), Some(EXPANSION_BASE_COST));
    }

    #[test]
    fn buy_expansion_deducts_ether_and_grows_plot() {
        let mut w = PlotWallet::new();
        // 第一格價 = 基準價；付得起。
        let new_balance = w.buy_expansion(100);
        assert_eq!(new_balance, Some(100 - EXPANSION_BASE_COST));
        assert_eq!(w.expansions(), 1);
        // 下一格更貴。
        assert_eq!(w.next_cost(), Some(EXPANSION_BASE_COST * 2));
    }

    #[test]
    fn buy_expansion_is_all_or_nothing_when_short() {
        let mut w = PlotWallet::new();
        // 乙太不足以付第一格：完全不動。
        assert_eq!(w.buy_expansion(EXPANSION_BASE_COST - 1), None);
        assert_eq!(w.expansions(), 0);
        assert!(w.can_expand());
    }

    #[test]
    fn buy_expansion_drains_ether_correctly_over_several_buys() {
        let mut w = PlotWallet::new();
        // 連買三格：總價 = 1×+2×+3× 基準 = 6× 基準。
        let mut ether = EXPANSION_BASE_COST * 6;
        ether = w.buy_expansion(ether).expect("第一格買得起");
        ether = w.buy_expansion(ether).expect("第二格買得起");
        ether = w.buy_expansion(ether).expect("第三格買得起");
        assert_eq!(w.expansions(), 3);
        // 乙太被花光（種田迴圈閉環：收成 → 換到擴張 → 乙太變少）。
        assert_eq!(ether, 0);
    }

    #[test]
    fn buy_expansion_stops_at_max() {
        let mut w = PlotWallet::new();
        // 給足夠多的乙太，一路買到上限。
        let mut ether = u32::MAX;
        for _ in 0..MAX_EXPANSIONS {
            ether = w.buy_expansion(ether).expect("未達上限前都買得到");
        }
        assert_eq!(w.expansions(), MAX_EXPANSIONS);
        assert!(!w.can_expand());
        // 達上限後沒得買：不扣款、不加格。
        assert_eq!(w.next_cost(), None);
        let before = ether;
        assert_eq!(w.buy_expansion(ether), None);
        assert_eq!(w.expansions(), MAX_EXPANSIONS);
        assert_eq!(ether, before);
    }

    #[test]
    fn is_loadable_accepts_valid_and_rejects_over_cap() {
        // 上限內（含剛好等於上限）可載入。
        assert!(PlotWallet { expansions: 0 }.is_loadable());
        assert!(PlotWallet {
            expansions: MAX_EXPANSIONS
        }
        .is_loadable());
        // 超過上限的壞檔（被竄改 / 改版）拒收，讓呼叫端退回乾淨狀態。
        assert!(!PlotWallet {
            expansions: MAX_EXPANSIONS + 1
        }
        .is_loadable());
    }

    #[test]
    fn plot_wallet_serde_round_trips() {
        // 接 0-E 會序列化已購格數存回；round-trip 不失真。
        let mut w = PlotWallet::new();
        w.buy_expansion(100);
        w.buy_expansion(100);
        let json = serde_json::to_string(&w).expect("序列化");
        let back: PlotWallet = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(back, w);
        assert_eq!(back.expansions(), 2);
    }
}
