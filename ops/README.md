# ButFun 一人公司 — 接案營運系統

> **角色分工**
> - **你 (CEO + 業務 + PM)**: 註冊平台帳號、登入掃案、跟客戶對談、收款
> - **Claude (CTO + 全部工程師)**: 評估案件、寫 proposal、報價、實際開發、產出文件

---

## 目錄結構

```
ops/
├── README.md          ← 你正在看的檔案 (操作手冊)
├── profile.md         ← 你的接案者檔案 (對外文案的事實來源)
├── platforms.md       ← 接案平台清單 (按優先序)
├── playbook.md        ← SOP: 評估、估時、報價、收款、溝通規則
├── pipeline.md        ← 案件看板 (CRM)
├── inbox/             ← 你貼客戶 brief / 對話到這
├── leads/             ← Claude 評估後的案件報告 + proposal 草稿
├── projects/          ← 接到的案子 (一案一目錄)
└── templates/
    ├── proposal-en.md ← 英文 proposal 模板
    ├── proposal-zh.md ← 中文 proposal 模板
    ├── proposal-es.md ← 西文 proposal 模板
    └── quote.md       ← 三段式報價單模板
```

---

## 你每天的標準流程

### 早上 (15 分鐘)
1. 登入 Upwork / Workana / 104 / 你選的平台
2. 找符合條件的案件,**整段複製**(包含 brief、預算、客戶資訊、deadline)
3. 在 `inbox/` 建立檔案,例如 `20260502-upwork-claude-bot.md`,把內容貼進去
4. (如果客戶有跟你互動) 把對話也貼進對應檔案

### 晚上 (30 分鐘)
1. 開 Claude Code,在 ButFun 專案裡執行
2. 跟我說:**「評估 inbox 的所有新案件」**
3. 我會:
   - 對每個案件做紅旗檢查 + 評分
   - 在 `leads/` 產出評估報告
   - 對通過的案件寫好客製 proposal 草稿
   - 更新 `pipeline.md`
4. 你 review 草稿,有要改的告訴我,然後**你登入平台貼 proposal**
5. 客戶回覆後,把對話貼到對應 `leads/` 檔,跟我說「客戶 X 案有回覆,擬下一步」

---

## 常用指令範本 (你可以直接貼給我)

### 評估新案
```
評估 inbox 裡所有未處理的案件。對每個案件:
1. 紅旗檢查
2. 估時
3. 評分 (用 playbook.md 的標準)
4. 通過的案件寫 proposal 草稿到 leads/
5. 更新 pipeline.md
```

### 客戶有回覆
```
客戶在 leads/<代號>.md 回了訊息 (我貼在檔案最後)。
擬一份回覆,要做到:
- 處理客戶問題
- 推進到下一步 (報價 / 簽約 / 開工)
- 不要做出承諾我們做不到
```

### 接到案子,要開工
```
我接到 leads/<代號> 了。
1. 在 projects/ 建立目錄
2. 開新 git 分支 client/<代號>
3. 從 brief 整理 spec 到 brief.md
4. 列出第一個 sprint 的任務
5. 估算 milestone 1 的工時
```

### 進度報告
```
我要給客戶 leads/<代號> 一份進度更新。
從 projects/<代號>/timeline.md 整理:
- 已完成
- 進行中
- 阻塞
- 下一步
寫成客戶能看懂的版本 (不要技術細節太多)。
```

### 月底結算
```
看 pipeline.md 的 Recently Closed,整理上個月:
- 總收入
- 案件數
- 平均實際時薪
- 命中率
- 最賺/最不賺的案
- 下個月策略建議
```

---

## 策略決策紀錄

> CEO 已決定: **全平台 + 全語言 (中/英/西) 鋪滿,機會最大化**。
> 不採用單點突破策略 (傳統建議是先攻 Upwork)。
> 這個策略的優點: 風險分散、品牌曝光最大;缺點: 各平台都需要維護 profile,初期工作量較大。

---

## 第一週啟動 checklist

- [ ] 跟 Claude 一起把 `profile.md` 裡所有 `[TODO]` 填完
- [ ] 確認接案用名字 (真名 / 英文名 / ButFun 品牌名)
- [ ] 在 `platforms.md` Tier S + Tier A 全部註冊 (約 8-10 個平台)
- [ ] 在每個平台建立 profile (Claude 幫你寫對應語言的 bio,直接從 profile.md 產出)
- [ ] GitHub pinned repos 整理 3-5 個展示用 repo (Claude 可以快速幫你做幾個 demo project)
- [ ] LinkedIn 改成接案 mode (Claude 幫你寫 headline + about,中英雙語)
- [ ] 設定每日掃案的時段 (建議早 7:00 + 晚 22:00,各 15 min)
- [ ] 第一週目標: 投 10-15 個案 (跨平台、跨語言),驗證流程而非要求命中

---

## 成功指標 (前 3 個月)

| 月 | 投標數 | 命中率目標 | 月收入目標 |
|---|---|---|---|
| 1 | 30-50 | 5% (1-2 案) | NT$ 5,000-20,000 |
| 2 | 50-80 | 8% (4-6 案) | NT$ 30,000-80,000 |
| 3 | 60-100 | 10% (6-10 案) | NT$ 80,000-150,000 |

> 別跟全職薪水比 — 接案的價值是時間自由 + 收入上限沒天花板。前 3 個月是建立評價,第 4 個月後才看得出真實水準。

---

## 安全提醒

- **絕對不要給客戶**: profile.md 裡的 TODO 內幕、playbook.md (報價策略)、leads/ 內部評估、pipeline.md 的真實命中率
- **可以給客戶看**: proposal 草稿 (你發出去的版本)、quote.md 對應段落、projects/ 裡的 brief.md / timeline.md
- **API key / token / 帳密**: 永遠不要 commit 進 git,放 `~/.config/butfun/secrets.env`,`.gitignore` 已排除

---

## 下一步

完成第一週 checklist 後,跟我說「**啟動 day 1**」,我會給你當天的具體 todo list。
