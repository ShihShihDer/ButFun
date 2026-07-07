//! 乙太方界·自由市集成交後，賣家離線也會知道 v1（自主提案切片，接續 832 玩家自由市集）。
//!
//! **真缺口**：832 讓玩家能在世界裡擺一個攤——放上一份材料、標明想換的東西，任何路過的旅人
//! 都能接手成交，**哪怕擺攤者早已下線也能兌現**。但擺攤者的貨物換了新的物資之後，這件事對
//! 擺攤者本人而言徹底無聲：伺服器把材料默默記進他的背包，全域動態牆固然留了一行「XXX 用材料
//! 和 YYY 的攤位成交了一筆交易」，但那是給所有玩家看的村莊八卦（且不保證擺攤者下次登入時還在
//! `welcome_back` 摘要的取樣窗內），擺攤者感受不到「我的攤位真的賣掉了」這件跟自己財產切身
//! 相關的事——你的攤位動了，你卻毫無所覺。
//!
//! 本刀比照 763（居民登門撲空、主人回家才感應到）的精神，把「錯過也該讓你知道」這條線從
//! 居民↔居民延伸到玩家↔玩家的經濟互動：擺攤者下次登入時，會收到一則私訊「你不在時，某某
//! 接手了你的攤位，換給你 N 個某物」——就算完全錯過成交那一刻，也終究會知道。
//!
//! **與 763 的區隔**：763 是居民等級的「感應」（需回到自家附近才觸發，走 tick 判定，感應方
//! 是 AI 居民）；本刀是玩家等級的「連線時投遞」（比照既有 `voxel_welcome` 連線時機制——玩家
//! 沒有「回到家附近」這個 tick 概念，一登入就送達，收信方是真人玩家）。
//!
//! **與 `voxel_welcome`（久別重逢摘要）的區隔**：久別重逢摘要是「世界發生了什麼」的村莊八卦
//! 摘要（全員共用同一份白名單事件、離線太短不顯示）；本刀是「你自己的攤位」這件私事，只要
//! 有成交、不論離線多久、一登入就送達，兩者互補、各自獨立。
//!
//! **純記憶體佇列**（比照 763 `MAX_PENDING_CALLERS` 慣例）：per-owner 一份等待投遞的成交清單，
//! 上限防洗版；送達後清空；重啟後佇列歸零（與其餘世界暫態 store 風險等同，可接受，零 migration）。

use std::collections::HashMap;

/// 每位賣家最多堆積幾筆待送達的成交通知，避免長時間離線後累積成長串——
/// 超過就不再堆（最舊的留著，等玩家上線清空後再重新開始收）。
pub const MAX_PENDING_NOTICES: usize = 5;

/// 一筆「你的攤位被接手」的成交通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StallSaleNotice {
    /// 接手你攤位的旅人名字。
    pub buyer: String,
    /// 換給你的物品中文名（已解析好的顯示名，非 block id）。
    pub got_item_name: String,
    /// 換給你的物品數量。
    pub got_count: u32,
}

/// per-owner 的待送達成交通知佇列。
pub type StallNoticeQueue = HashMap<String, Vec<StallSaleNotice>>;

/// 把一筆成交通知塞進賣家佇列。賣家為空字串（未登入/匿名，理論上不會擁有攤位）不塞；
/// 佇列已達上限（[`MAX_PENDING_NOTICES`]）就不再堆，避免長時間離線後一次爆量。
pub fn enqueue_sale(queue: &mut StallNoticeQueue, owner: &str, notice: StallSaleNotice) {
    if owner.is_empty() {
        return;
    }
    let bucket = queue.entry(owner.to_string()).or_default();
    if bucket.len() >= MAX_PENDING_NOTICES {
        return;
    }
    bucket.push(notice);
}

/// 把一筆成交通知格式化成私訊行（面向玩家字串，留 i18n 空間）。
fn notice_line(n: &StallSaleNotice) -> String {
    format!(
        "🛒 你不在時，{}接手了你的攤位，換給你 {} 個「{}」。",
        n.buyer, n.got_count, n.got_item_name
    )
}

/// 把賣家佇列裡的多筆通知組成一則私訊；沒有值得說的事就回 `None`（不顯示空訊息）。
pub fn format_notice_message(notices: &[StallSaleNotice]) -> Option<String> {
    if notices.is_empty() {
        return None;
    }
    Some(notices.iter().map(notice_line).collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(buyer: &str, item: &str, count: u32) -> StallSaleNotice {
        StallSaleNotice {
            buyer: buyer.to_string(),
            got_item_name: item.to_string(),
            got_count: count,
        }
    }

    #[test]
    fn enqueue_adds_to_owner_bucket() {
        let mut q = StallNoticeQueue::new();
        enqueue_sale(&mut q, "露娜", n("旅人", "木頭", 3));
        assert_eq!(q.get("露娜").map(|b| b.len()), Some(1));
    }

    #[test]
    fn enqueue_rejects_empty_owner() {
        let mut q = StallNoticeQueue::new();
        enqueue_sale(&mut q, "", n("旅人", "木頭", 3));
        assert!(q.is_empty());
    }

    #[test]
    fn enqueue_keeps_buckets_independent_per_owner() {
        let mut q = StallNoticeQueue::new();
        enqueue_sale(&mut q, "露娜", n("旅人甲", "木頭", 1));
        enqueue_sale(&mut q, "諾娃", n("旅人乙", "石頭", 2));
        assert_eq!(q.get("露娜").map(|b| b.len()), Some(1));
        assert_eq!(q.get("諾娃").map(|b| b.len()), Some(1));
    }

    #[test]
    fn enqueue_caps_at_max_pending() {
        let mut q = StallNoticeQueue::new();
        for i in 0..(MAX_PENDING_NOTICES + 3) {
            enqueue_sale(&mut q, "露娜", n(&format!("旅人{i}"), "木頭", 1));
        }
        assert_eq!(q.get("露娜").map(|b| b.len()), Some(MAX_PENDING_NOTICES));
    }

    #[test]
    fn notice_line_names_buyer_item_and_count() {
        let msg = format_notice_message(&[n("賽勒", "護身符", 2)]).unwrap();
        assert!(msg.contains("賽勒"));
        assert!(msg.contains("護身符"));
        assert!(msg.contains('2'));
    }

    #[test]
    fn format_message_empty_yields_none() {
        assert_eq!(format_notice_message(&[]), None);
    }

    #[test]
    fn format_message_joins_multiple_notices() {
        let msg = format_notice_message(&[
            n("旅人甲", "木頭", 1),
            n("旅人乙", "石頭", 2),
        ]).unwrap();
        assert!(msg.contains("旅人甲"));
        assert!(msg.contains("旅人乙"));
        assert_eq!(msg.lines().count(), 2);
    }
}
