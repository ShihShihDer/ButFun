//! 乙太方界·居民以物易物 v1——手上有餘料的居民，路遇身邊缺該料的老朋友，
//! 自然地「拿我多的換你多的」互利交換（自主提案切片）。
//!
//! **真缺口 / 為誰做**：居民各有採集背包（`res_inv`），不同居民擅長不同手藝、囤不同材料；
//! 世界已經有幾種居民↔居民的物資互動——但每一種都**不是互利對換**：
//! - 723 `voxel_resident_trade`：象徵性的（基於「特長分類」的抽象物名、對稱交換，**不動任何
//!   實際背包**，純 Feed＋記憶）——演的是「有做過生意」的氛圍，背包一顆料都沒少。
//! - 748 `voxel_share`：**真實**轉移，但是**單向贈與**（主人把餘料勻一份給訪客，不拿回任何東西）。
//! - 800 `voxel_share_meal`：飢餓時分一口飯——也是**單向**施予，回應的是「餓」這個生理缺口。
//!
//! 唯獨「**我木頭多、你石頭多，各自缺對方那樣 → 走近時提議『拿我的木頭換你的石頭？』→ 成交則
//! 雙方背包真的對換**」這種**雙向互利**的以物易物，從沒在居民之間發生過。這正對著北極星
//! 「AI 居民湧現出一個小社會」——一個真的活著的小村，鄰里之間會拿自己多的去換自己缺的。
//! 本刀把「各自的餘料」與「各自的缺料」這兩張帳，第一次在**真實背包**上湊成一樁互利的交換。
//!
//! **記憶驅動行為（北極星）**：換不換，看的是**兩人的交情**——只有老朋友（`Friend`）之間才會
//! 開口提議（陌生人擦身而過不會）。你不會看到路人隨機亂換，只會看到**處出了交情的鄰居**在彼此
//! 恰好一方有餘、一方有缺時，自然地互通有無。關係網（記憶累積出的 bonds）真的改變了行為。
//!
//! **這裡只放確定性純邏輯**（誰餘什麼／誰缺什麼的配對、公平換算、冷卻、台詞／記憶／Feed 文案），
//! 零 LLM、零鎖、零 IO、零 async，可單元測試。配對掃描 / 鎖 / 走動 / 廣播 / 真實背包對換全留在
//! `voxel_ws.rs`（沿用 `maybe_encounter_teach` 的「位置＋背包快照 → i≠j 循序掃描 → 每輪最多一對 →
//! 記憶/Feed 鎖外落地」慣例，守 prod 死鎖鐵律：bonds 讀 → inv 讀 → inv 寫 → memory 寫，
//! 短取即釋、不巢狀、循序）。
//!
//! **成本 / 濫用防護**：不觸發 LLM、不開對外端點、不收玩家自由文字——台詞與記憶全為確定性模板、
//! 只嵌居民系統顯示名（本就出現在動態牆），永不回放記憶原文或玩家原話（無注入 / NSFW 面）；觸發
//! 純伺服器 tick 內部狀態（背包餘缺 ＋ 交情 ＋ 每對冷卻 ＋ 低機率），玩家無法自報、無法從外部催發。
//! 餘缺/冷卻純記憶體重啟歸零（過場狀態、零資料風險、零 migration），記憶/情誼走既有 append-only。

use std::collections::HashMap;

use crate::voxel_bonds::BondTier;

/// 提議者要多靠近老朋友才會開口提議對換（世界方塊距離）。刻意比社交半徑近一點——
/// 以物易物是「就在你旁邊」的舉動，不是隔著大半個村子喊話。與分食（748）的近旁半徑一致。
pub const BARTER_RADIUS: f32 = 4.5;

/// 手握這麼多某種材料才算「有餘裕拿去換」——低於此不掏，免得打亂自己正在湊料的
/// 發明／建造計畫（`voxel_invent` 也吃 `res_inv`）。與 748 分享的 `SHARE_MIN_STOCK` 同量級。
pub const SURPLUS_MIN: u32 = 6;

/// 手上某種材料少於這麼多才算「缺」——只有對方餘、我又真的缺，才湊得成一樁互利的交換。
pub const SCARCE_MAX: u32 = 2;

/// 一樁交易雙方各給出這麼多份（**對等對換＝公平換算**：我給你 N 份、你也給我 N 份）。
/// 刻意小份——只是互通有無、不是大宗批發，也不會把任一方的餘裕掏空。
pub const BARTER_QTY: u32 = 2;

/// 同一對居民成交後的靜默冷卻（秒）：讓「以物易物」這件事稀少而有份量，不會同一對鄰居
/// 短時間內反覆對換、洗版泡泡與動態牆。
pub const BARTER_COOLDOWN_SECS: u64 = 240;

/// 每輪掃描擲一次骰的觸發機率（不隨在場人數膨脹）。就算旁邊剛好有餘缺互補的老友，
/// 也不是每一瞬間都會開口，偶爾才觸發，像真的生活裡的不期而遇。
pub const BARTER_CHANCE: f32 = 0.5;

/// Feed 動態牆種類（分類顯示用，面向玩家、集中可 i18n）。
pub const FEED_KIND: &str = "居民易物";

/// 台詞／記憶字元上限（泡泡／日記可讀，與其他社交台詞一致）。
pub const BARTER_MAX_CHARS: usize = 40;

/// 一樁湊成的以物易物：提議者（a）給出 `a_gives`、換得對方（b）給出的 `b_gives`，
/// 雙方各 `qty` 份（對等對換）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarterDeal {
    /// 提議者給出的物品 id（提議者有餘、對方缺）。
    pub a_gives: u8,
    /// 對方給出的物品 id（對方有餘、提議者缺）。
    pub b_gives: u8,
    /// 雙方各給出的份數（對等）。
    pub qty: u32,
}

/// 是否該開口提議對換：只在老朋友之間、且機率骰過門檻。純函式、確定性、可測。
/// （交情門檻＝老朋友優先，別全村亂換；實際「餘缺是否湊得成」另由 [`pick_barter`] 判定。）
pub fn should_barter(tier: BondTier, roll: f32) -> bool {
    tier == BondTier::Friend && roll < BARTER_CHANCE
}

/// 冷卻是否已過：`last` 為這對居民上次成交的時間（秒），`None` 代表從沒換過。純函式、可測。
pub fn cooldown_ok(now_secs: u64, last: Option<u64>) -> bool {
    match last {
        Some(prev) => now_secs.saturating_sub(prev) >= BARTER_COOLDOWN_SECS,
        None => true,
    }
}

/// 兩個居民 id 的**無序**冷卻鍵（`a|b`，字典序小者在前），讓 (甲,乙) 與 (乙,甲) 命中同一格
/// 冷卻。純函式、可測。
pub fn pair_key(id_a: &str, id_b: &str) -> String {
    if id_a <= id_b {
        format!("{id_a}|{id_b}")
    } else {
        format!("{id_b}|{id_a}")
    }
}

/// 從「我的背包」裡挑一份**我有餘、對方缺**、最適合拿去換的材料（純函式、可測）。
///
/// 規則（確定性、可重現）：
/// - 忽略 Air（0）；
/// - 只看我手上 `>= SURPLUS_MIN`（有餘裕）**且**對方手上 `< SCARCE_MAX`（真的缺）的材料；
/// - 在合格者中挑我囤最多的那種（我最不缺、最捨得換）；同量時 `block_id` 小者優先（穩定排序）。
///
/// 無任何合格材料 → `None`。
fn best_surplus_they_lack(mine: &HashMap<u8, u32>, theirs: &HashMap<u8, u32>) -> Option<u8> {
    mine.iter()
        .filter(|(&id, &qty)| {
            id != 0 && qty >= SURPLUS_MIN && theirs.get(&id).copied().unwrap_or(0) < SCARCE_MAX
        })
        // 主排序：囤越多越優先（`qty` 升冪比較，最大者勝）；同量則 `block_id` 小者勝
        //（`id_b.cmp(id_a)`：a 的 id 較小時回 Greater，讓小 id 被視為「較大」而中選）。
        .max_by(|(id_a, q_a), (id_b, q_b)| q_a.cmp(q_b).then_with(|| id_b.cmp(id_a)))
        .map(|(&id, _)| id)
}

/// 湊一樁互利的以物易物：`a`（提議者）與 `b`（對方）各自的採集背包（純函式、可測）。
///
/// 要同時滿足**雙向**才成交：
/// - `a` 有一樣東西**有餘、且 `b` 缺** → `a_gives`；
/// - `b` 有一樣東西**有餘、且 `a` 缺** → `b_gives`；
/// - 兩樣**必須不同**（換同一種沒意義）；
/// - 各給 `qty = min(BARTER_QTY, 雙方各自的餘量)` 份（對等對換＝公平換算；實務上恆為
///   `BARTER_QTY`，因兩者皆 `>= SURPLUS_MIN >= BARTER_QTY`，`min` 僅作安全夾值）。
///
/// 任一方向湊不成、或兩樣相同 → `None`（呼叫端當作這次沒得換，安靜跳過）。
/// **絕不會挑到自己沒有的東西去給**：`a_gives` 恆滿足 `a[a_gives] >= SURPLUS_MIN`，
/// `b_gives` 同理，由 [`best_surplus_they_lack`] 的門檻保證。
pub fn pick_barter(a: &HashMap<u8, u32>, b: &HashMap<u8, u32>) -> Option<BarterDeal> {
    let a_gives = best_surplus_they_lack(a, b)?;
    let b_gives = best_surplus_they_lack(b, a)?;
    if a_gives == b_gives {
        return None;
    }
    let a_stock = a.get(&a_gives).copied().unwrap_or(0);
    let b_stock = b.get(&b_gives).copied().unwrap_or(0);
    let qty = BARTER_QTY.min(a_stock).min(b_stock);
    if qty == 0 {
        return None;
    }
    Some(BarterDeal { a_gives, b_gives, qty })
}

/// 在**同一份** `res_inv` 快照上執行真實對換（零和守恆）：`a` 扣 `a_gives`／加 `b_gives`，
/// `b` 扣 `b_gives`／加 `a_gives`，雙方各 `deal.qty` 份。純函式、可測。
///
/// **競態防護**：動手前先確認雙方此刻仍各自握有足量（防與同 tick 其他消耗撞車）——任一方
/// 不足則**原封不動**回 `false`，呼叫端當作這次沒換成，不記憶不 Feed。先扣後加、循序 `get_mut`
/// （同一外層 map 不同時持兩個可變借用），成功回 `true`。呼叫端須包在 `res_inv` 寫鎖內。
pub fn apply_barter(
    bags: &mut HashMap<String, HashMap<u8, u32>>,
    a_id: &str,
    b_id: &str,
    deal: &BarterDeal,
) -> bool {
    let a_has = bags.get(a_id).and_then(|m| m.get(&deal.a_gives)).copied().unwrap_or(0);
    let b_has = bags.get(b_id).and_then(|m| m.get(&deal.b_gives)).copied().unwrap_or(0);
    if a_has < deal.qty || b_has < deal.qty {
        return false;
    }
    if let Some(m) = bags.get_mut(a_id) {
        *m.entry(deal.a_gives).or_insert(0) -= deal.qty; // 安全：a_has >= qty
        *m.entry(deal.b_gives).or_insert(0) += deal.qty;
    }
    if let Some(m) = bags.get_mut(b_id) {
        *m.entry(deal.b_gives).or_insert(0) -= deal.qty; // 安全：b_has >= qty
        *m.entry(deal.a_gives).or_insert(0) += deal.qty;
    }
    true
}

/// 截斷到泡泡框可容納的字數（與其他社交台詞一致）。
fn truncate_chars(s: &str) -> String {
    s.chars().take(BARTER_MAX_CHARS).collect()
}

/// 提議者開口的台詞（確定性、零 LLM，帶對方名＋雙方物名，≤40 字剛好在泡泡框內）。
pub fn barter_say_line_proposer(
    other_name: &str,
    my_item: &str,
    their_item: &str,
    pick: usize,
) -> String {
    let pool: &[&str] = &[
        "{o}，拿我的{a}換你的{b}？",
        "{o}，我{a}多，你{b}多，換換？",
        "{o}，我這{a}分你，你那{b}勻我點？",
        "{o}，正缺{b}呢，用我的{a}跟你換！",
    ];
    truncate_chars(
        &pool[pick % pool.len()]
            .replace("{o}", other_name)
            .replace("{a}", my_item)
            .replace("{b}", their_item),
    )
}

/// 對方應和成交的台詞（確定性、零 LLM）。
pub fn barter_say_line_accepter(other_name: &str, got_item: &str, pick: usize) -> String {
    let pool: &[&str] = &[
        "好啊{o}，正好我{g}也不夠！",
        "成交{o}！這{g}我收下了～",
        "{o}真夠意思，換得好！",
        "來吧{o}，各取所需！",
    ];
    truncate_chars(&pool[pick % pool.len()].replace("{o}", other_name).replace("{g}", got_item))
}

/// 提議者的記憶：我用多的換到了缺的（供日記端昇華成一則「懂得互通有無」的記憶）。
pub fn barter_memory_line(other_name: &str, gave_item: &str, got_item: &str) -> String {
    truncate_chars(&format!("用我多的{gave_item}跟{other_name}換了{got_item}，各取所需"))
}

/// Feed 動態文案（第三人稱、附在 actor＝提議者名後面），讓離線玩家回來翻動態也知道
/// 居民彼此在互通有無。提議者名由呼叫端作為 actor 傳入 `append_feed`，這裡不重複帶。
pub fn barter_feed_line(other_name: &str, gave_item: &str, got_item: &str, qty: u32) -> String {
    format!("用 {qty} 份{gave_item}跟老朋友{other_name}換了 {qty} 份{got_item}")
        .chars()
        .take(BARTER_MAX_CHARS + 12)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bag(pairs: &[(u8, u32)]) -> HashMap<u8, u32> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn should_barter_only_friend_and_roll() {
        // 老朋友、機率過門檻 → 觸發。
        assert!(should_barter(BondTier::Friend, 0.0));
        // 非老朋友 → 不觸發（交情門檻）。
        assert!(!should_barter(BondTier::Acquaintance, 0.0));
        assert!(!should_barter(BondTier::Stranger, 0.0));
        // 機率沒過門檻 → 不觸發。
        assert!(!should_barter(BondTier::Friend, BARTER_CHANCE + 0.01));
    }

    #[test]
    fn cooldown_gates_repeat() {
        // 從沒換過 → 可換。
        assert!(cooldown_ok(1000, None));
        // 冷卻剛好到 → 可換。
        assert!(cooldown_ok(1000, Some(1000 - BARTER_COOLDOWN_SECS)));
        // 冷卻還沒到 → 不可換。
        assert!(!cooldown_ok(1000, Some(1000 - BARTER_COOLDOWN_SECS + 1)));
        // 時鐘倒退也不 panic（saturating）。
        assert!(!cooldown_ok(10, Some(1000)));
    }

    #[test]
    fn pair_key_is_order_independent() {
        assert_eq!(pair_key("vox_res_1", "vox_res_3"), pair_key("vox_res_3", "vox_res_1"));
        assert_eq!(pair_key("a", "b"), "a|b");
        assert_eq!(pair_key("b", "a"), "a|b");
    }

    #[test]
    fn pick_barter_mutual_surplus_deficit() {
        // a 木頭(5) 多且 b 缺木頭；b 石頭(3) 多且 a 缺石頭 → 湊成一樁對換。
        let a = bag(&[(5, 8), (3, 1)]);
        let b = bag(&[(3, 7), (5, 0)]);
        assert_eq!(
            pick_barter(&a, &b),
            Some(BarterDeal { a_gives: 5, b_gives: 3, qty: BARTER_QTY })
        );
    }

    #[test]
    fn pick_barter_none_when_no_mutual_need() {
        // a 有餘、b 也有木頭夠多（不缺）→ 這方向湊不成。
        let a = bag(&[(5, 8)]);
        let b = bag(&[(5, 8), (3, 8)]);
        // a 想給木頭但 b 不缺木頭；b 想給石頭且 a 缺石頭，但 a 沒有東西是 b 缺的 → None。
        assert_eq!(pick_barter(&a, &b), None);
    }

    #[test]
    fn pick_barter_never_gives_what_you_lack() {
        // a 木頭只有 2 份（< SURPLUS_MIN），不算餘裕 → 不會拿去換（免得掏空自己）。
        let a = bag(&[(5, 2), (3, 0)]);
        let b = bag(&[(3, 8), (5, 0)]);
        // a 沒有任何「有餘且 b 缺」的東西 → None。
        assert_eq!(pick_barter(&a, &b), None);
        // 反向確認：給的一定是自己 >= SURPLUS_MIN 的東西、且量足。
        let a2 = bag(&[(5, SURPLUS_MIN), (3, 0)]);
        let b2 = bag(&[(3, SURPLUS_MIN), (5, 0)]);
        let deal = pick_barter(&a2, &b2).unwrap();
        assert!(a2[&deal.a_gives] >= deal.qty, "絕不換出自己沒有的量");
        assert!(b2[&deal.b_gives] >= deal.qty, "對方也絕不換出沒有的量");
    }

    #[test]
    fn pick_barter_fair_equal_qty_both_ways() {
        // 對等對換：雙方給出的份數必須一致（公平換算）。
        let a = bag(&[(5, 20)]);
        let b = bag(&[(3, 20)]);
        let deal = pick_barter(&a, &b).unwrap();
        assert_eq!(deal.qty, BARTER_QTY, "各給 BARTER_QTY 份，對等");
    }

    #[test]
    fn pick_barter_qty_bounded() {
        // 餘量恰卡 SURPLUS_MIN 時，份數不超 BARTER_QTY、不超餘量、不為 0。
        let a = bag(&[(5, SURPLUS_MIN)]);
        let b = bag(&[(3, SURPLUS_MIN)]);
        let deal = pick_barter(&a, &b).unwrap();
        assert!(deal.qty >= 1 && deal.qty <= BARTER_QTY);
        assert!(deal.qty <= SURPLUS_MIN);
    }

    #[test]
    fn pick_barter_ignores_air() {
        // Air(0) 再多也不算可換的餘料。
        let a = bag(&[(0, 100), (5, 8)]);
        let b = bag(&[(0, 100), (3, 8)]);
        let deal = pick_barter(&a, &b).unwrap();
        assert_ne!(deal.a_gives, 0);
        assert_ne!(deal.b_gives, 0);
    }

    #[test]
    fn pick_barter_ties_break_low_id() {
        // a 對 5 與 8 皆有 6 份（同量），b 兩者皆缺 → 穩定取 block_id 小者（5）。
        let a = bag(&[(5, 6), (8, 6)]);
        let b = bag(&[(3, 8)]);
        let deal = pick_barter(&a, &b).unwrap();
        assert_eq!(deal.a_gives, 5, "同量餘料，穩定取小 id");
        assert_eq!(deal.b_gives, 3);
    }

    #[test]
    fn pick_barter_none_when_symmetric_bags() {
        // 兩人都木頭多、都缺石頭——沒有一方能提供對方缺的、也拿不出互補的另一樣 → None。
        let a = bag(&[(5, 8), (3, 0)]);
        let b = bag(&[(5, 8), (3, 0)]);
        assert_eq!(pick_barter(&a, &b), None);
    }

    #[test]
    fn apply_barter_swaps_both_bags_conserving_total() {
        let mut bags: HashMap<String, HashMap<u8, u32>> = HashMap::new();
        bags.insert("a".into(), bag(&[(5, 8), (3, 1)])); // a 木頭多、缺石頭
        bags.insert("b".into(), bag(&[(3, 7), (5, 0)])); // b 石頭多、缺木頭
        let deal = BarterDeal { a_gives: 5, b_gives: 3, qty: 2 };
        assert!(apply_barter(&mut bags, "a", "b", &deal));
        // a：木頭 8-2=6、石頭 1+2=3。
        assert_eq!(bags["a"][&5], 6);
        assert_eq!(bags["a"][&3], 3);
        // b：石頭 7-2=5、木頭 0+2=2。
        assert_eq!(bags["b"][&3], 5);
        assert_eq!(bags["b"][&5], 2);
        // 零和守恆：木頭總量 8→8、石頭總量 8→8。
        assert_eq!(bags["a"][&5] + bags["b"][&5], 8);
        assert_eq!(bags["a"][&3] + bags["b"][&3], 8);
    }

    #[test]
    fn apply_barter_refuses_when_stock_insufficient() {
        // a 此刻只剩 1 份木頭（< qty=2，模擬同 tick 被別處消耗）→ 拒絕、原封不動。
        let mut bags: HashMap<String, HashMap<u8, u32>> = HashMap::new();
        bags.insert("a".into(), bag(&[(5, 1)]));
        bags.insert("b".into(), bag(&[(3, 8)]));
        let deal = BarterDeal { a_gives: 5, b_gives: 3, qty: 2 };
        assert!(!apply_barter(&mut bags, "a", "b", &deal));
        assert_eq!(bags["a"][&5], 1, "拒絕時不得動任何一方");
        assert_eq!(bags["b"][&3], 8);
    }

    #[test]
    fn empty_bags_yield_none() {
        assert_eq!(pick_barter(&HashMap::new(), &HashMap::new()), None);
        assert_eq!(pick_barter(&bag(&[(5, 8)]), &HashMap::new()), None);
    }

    #[test]
    fn say_lines_non_empty_within_bubble_have_names() {
        for pick in 0..6 {
            let p = barter_say_line_proposer("諾娃", "木頭", "石頭", pick);
            assert!(!p.is_empty());
            assert!(p.chars().count() <= BARTER_MAX_CHARS, "台詞不得破泡泡框");
            assert!(p.contains("諾娃"), "應含對方名");
            assert!(p.contains("木頭") && p.contains("石頭"), "應含雙方物名");

            let acc = barter_say_line_accepter("露娜", "木頭", pick);
            assert!(!acc.is_empty());
            assert!(acc.chars().count() <= BARTER_MAX_CHARS);
            assert!(acc.contains("露娜"));
        }
    }

    #[test]
    fn say_lines_vary_by_pick() {
        assert_ne!(
            barter_say_line_proposer("諾娃", "木頭", "石頭", 0),
            barter_say_line_proposer("諾娃", "木頭", "石頭", 1),
        );
        assert_ne!(
            barter_say_line_accepter("露娜", "木頭", 0),
            barter_say_line_accepter("露娜", "木頭", 1),
        );
    }

    #[test]
    fn memory_and_feed_carry_names_and_items() {
        let m = barter_memory_line("諾娃", "木頭", "石頭");
        assert!(m.contains("諾娃") && m.contains("木頭") && m.contains("石頭"));
        assert!(m.chars().count() <= BARTER_MAX_CHARS);

        let f = barter_feed_line("諾娃", "木頭", "石頭", 2);
        assert!(f.contains("諾娃") && f.contains("木頭") && f.contains("石頭"));
        assert!(f.contains('2'));
    }
}
