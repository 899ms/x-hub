# 待办功能升级方案（标签 / 日历 / 周期 / 附件）

> 状态：设计定稿（§11 全部边界已确认），v1 实施中。生成日期：2026-09-21。
> 术语以 `CONTEXT.md` 的「待办（规划中，尚未实施）」一节为准；两条难逆决策见 `docs/adr/0009-recurring-todo-roll-in-place.md` 与 `docs/adr/0010-todo-tags-independent-from-note-tags.md`。
> 布局原型（方案乙定版）见 `docs/prototypes/todo-view-layout-prototype.html`。

## 1. 背景与目标

待办现状（`src-tauri/src/db.rs:68`）只有 `title` / `done` / `priority` / `due_at` / `remind_at` / `parent_id` / `sort_order` / `version`，能力止于「一次性截止 + 一次性提醒 + 一层子待办」。本次要补上五件事：**标签**、**日历**、**周期待办**、**附件**、**置顶**，外加两处通用交互修正（勾选父待办前确认、周期待办允许带子待办）。

原始需求里「有月待办、周待办，本月、本周」把两个不同概念混在一句话里，本方案强制拆开：

- **周期待办（recurring todo）**：一条任务按规则反复发生；
- **时间范围视图（horizon）**：今天 / 本周 / 本月 / 全部，是「看哪些」的筛选视角。

## 2. 已拍板决策

| # | 决策点 | 结论 |
|---|---|---|
| 1 | 交付形态 | 工作台卡片保持轻量（快速录入 + 逾期/今天），新建独立「待办」视图，做进 core，不做扩展 |
| 2 | 视图布局 | **宽窗方案乙：左右同屏**（左列表 40% / 右日历 60%）；**窄窗切方案丙**（单栏 + 双维工具栏），见 §6.3 |
| 3 | 周期规则 | 预设 + 有限自定义 + 结束条件，不做完整 RRULE |
| 4 | 周期完成语义 | 就地滚动，不逐次留历史；存完成计数（ADR 0009） |
| 5 | 未来实例 | 前端虚拟展开，不落库（ADR 0009） |
| 6 | 标签归属 | **待办专属**，与笔记标签两套独立定义（ADR 0010） |
| 7 | 标签外观 | 10 个预设色板 + 自定义十六进制；一条待办 0..N 个 |
| 8 | 日历档位 | 月 + 周两档，不做日视图 |
| 9 | 日历交互 | 点空白新建、拖拽改期；周期待办禁止拖拽（v1） |
| 10 | 描述字段 | 加 `description`（轻量 Markdown），**不放 checklist**（勾选一律用子待办） |
| 11 | 工作台入口 | 待办卡片标题栏右侧加「打开待办视图 →」；卡片不显示日历 |
| 12 | 工作台日历模块 | 新增独立 module（id `calendar`，标题「日历」），**只读**，点击整卡跳待办视图；原 clock 模块的「整月日历」形态一并删除（§7） |
| 13 | 附件 | 图片 + 任意文件，复制进数据根，单文件 ≤ 50MB、单条 ≤ 10 个；支持拖拽到待办；条目右侧显示回形针图标 + 数量 |
| 14 | 扩展桥 | `list/get` 立即带新字段；`create/update` 签名不扩，新字段走新方法 |
| 15 | 分期 | v1 = 视图 + 标签 + 月/周 + 周期 + 描述 + 置顶；v2 = 附件 + 跨周期例外 |
| 16 | 置顶 | 新增 `pinned` 列；置顶条目**脱离日期分组**，固定在最顶部「置顶」区并带图钉图标；置顶区内的次序沿用 `sort_order` |
| 17 | 勾选父待办的联动 | 父待办存在未完成子项时，先弹确认（「该待办下还有子待办未完成，是否确认」），确认后一键勾选全部未完成子级；取代现有「静默连勾」（`TodoRow.vue:92`） |
| 18 | 周期待办与子待办 | **允许**（一层，沿用现有 `parent_id` / `useTodoChildren`）；滚动时子项如何处置见 §11 第 6 项 |

## 3. 数据模型

### 3.1 `todos` 新增列（周期 + 描述）

| 列 | 类型 | 说明 |
|---|---|---|
| `description` | TEXT NOT NULL DEFAULT '' | 轻量 Markdown 正文 |
| `repeat_mode` | TEXT NOT NULL DEFAULT 'once' | `once`/`daily`/`weekly`/`monthly`/`yearly`/`weekdays`/`custom` |
| `repeat_every` | INTEGER | `custom`：每 N |
| `repeat_unit` | TEXT | `custom`：`day`/`week`/`month`/`year` |
| `repeat_weekdays` | INTEGER | 位掩码 bit0=周一 … bit6=周日（`weekly` 多选、`custom`+`week` 用） |
| `repeat_month_day` | INTEGER | `monthly`：1..31，`-1` = 月末 |
| `repeat_month_nth` | INTEGER | `monthly`：第几个（1..5，`-1` = 最后一个），非空时与 `repeat_weekdays` 组合表达「第几个星期几」 |
| `repeat_end_mode` | TEXT | `never`/`until`/`count` |
| `repeat_end_at` | INTEGER | `until`：截止日期（毫秒时间戳） |
| `repeat_count` | INTEGER | `count`：共 N 次 |
| `repeat_done_count` | INTEGER NOT NULL DEFAULT 0 | 累计完成次数（统计用） |
| `repeat_last_done_at` | TEXT | 上次完成时间 |
| `pinned` | INTEGER NOT NULL DEFAULT 0 | 置顶：脱离日期分组，固定在最顶部「置顶」区 |

**为什么内联而不是单开 `todo_recurrences` 表**：`countdowns` 已经把 `repeat_mode`/`interval_minutes` 内联在同一张表（`db.rs:126`），沿用它读者不用学第二套；且**局域网同步以单行 `version` 为单位**，1:1 拆表会让「改规则」变成跨两行的原子写，冲突检测复杂化。代价是 `todos` 变宽，可接受。

`repeat_mode` 是总开关：只有 `custom`/`weekly`/`monthly` 才读后面对应的列，其余列保持 NULL。这与 `countdowns.interval_minutes` 仅 `interval` 模式有值的做法一致。

### 3.2 标签（两张新表）

```
todo_tags (            -- 定义表（与笔记的 tags 无关）
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL UNIQUE,
  color TEXT NOT NULL DEFAULT '',   -- '' = 用默认色；否则 #rrggbb
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (...)
)
todo_tag_links (       -- 关联表
  todo_id INTEGER NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
  tag_id  INTEGER NOT NULL REFERENCES todo_tags(id) ON DELETE CASCADE,
  PRIMARY KEY (todo_id, tag_id)
)
```

命名刻意与笔记那对（`tags` + `note_tags`）不对称，理由见 ADR 0010：`todo_tags` 是标签本身，`todo_tag_links` 是关联。

### 3.3 附件（v2，两张新表 + 文件落盘）

```
todo_attachments (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  todo_id INTEGER NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
  file_name TEXT NOT NULL,          -- 展示用原名
  rel_path TEXT NOT NULL,           -- 相对 <data_root>/todos/attachments/
  size INTEGER NOT NULL DEFAULT 0,
  mime TEXT,
  hash TEXT NOT NULL,               -- 内容 sha256，去重 + 定位
  created_at TEXT NOT NULL DEFAULT (...)
)
```

文件按内容哈希命名存 `<data_root>/todos/attachments/<hash>.<ext>`，沿用笔记内嵌图片（`import_note_image` → `<data_root>/notes/images/<hash>.<ext>`）的思路。**已知代价：附件是文件、不在 SQLite 里**，所以任何走数据库的通道（扩展桥的外部消费方、将来的数据导出）都带不走附件本体，只能带走 `rel_path` 这条记录。v2 需要把这一口径写清并在 UI 上说明，而不是让用户以为附件跟着数据走。

## 4. 迁移策略

沿用 v0.3.4 给 `todos` 补列的既有模式（`db.rs:320` 起）：逐列 `ALTER TABLE todos ADD COLUMN ...`，用 `PRAGMA table_info` 判断后幂等执行。

- 所有新列**可空或带 DEFAULT**，老库启动即补齐，零数据改造。
- 两张新表（v1 的标签、v2 的附件）放进 `db.rs::migrate()` 的 `CREATE TABLE IF NOT EXISTS` 建表块。
- **索引必须放在逐列补齐之后**：`db.rs:188` 有一条明确注释——老库的 `todos` 表结构与新装库不同，把 `CREATE INDEX` 放进建表批次会失败。新增索引（`idx_todo_tag_links_tag`、`idx_todos_repeat`）同样遵守。
- **新增字段必须进乐观锁写回路径**：`repo/todo.rs` 的 `update_with_version` / `toggle_with_version` 只写 `title`/`priority`/`done`，而 `xhub_api.rs` 的 `data.todos.update` 等桥方法会带 `expected_version` 调它们——**这是 `version` 列目前唯一的消费方**（应用内没有同步功能，`db.rs`/`models.rs` 里「局域网同步冲突检测」的注释是当初为同步预留的意图，代码里并不存在同步模块或同步 UI）。因此 `description` / `repeat_*` 需要各自带乐观锁的写回函数，否则**扩展桥调用方**写回时会静默丢字段；将来真做同步时，这些路径同样是唯一入口。

## 5. 周期待办的行为规格

### 5.1 完成（滚动）

勾选一条周期待办：

1. `due_at` 推到**下一个未来时刻**：从当前 `due_at` 起按规则迭代，若结果仍在过去则继续迭代，直到落在未来（**不补做逾期历史**）。迭代设上限（如 1000 次）防规则错误导致死循环。
2. `remind_at` 按与 `due_at` 的相对偏移同步平移；`remind_fired` 复位。
3. `repeat_done_count + 1`，`repeat_last_done_at = now`。
4. **不置 `done`、不写 `completed_at`** —— 它不进「已完成」列表。
5. 前端播放一次短动效 + toast（如「已滚到 明天」），让用户看到日期确实变了。

撤销勾选：`repeat_done_count - 1`，`due_at` 回滚到本轮（把刚推过去的那次减回来）。

### 5.2 逾期

`due_at < now` 且未完成 → 列表显示「逾期 N 天」。逾期是**显示态**，不是数据态：数据上仍是一个 `due_at` 在过去、`done = 0` 的普通行。

### 5.3 结束条件

- `until`：下一个实例的日期 > `repeat_end_at` 时结束；
- `count`：`repeat_done_count >= repeat_count` 时结束。

结束时把 `repeat_mode` 置回 `once` 并**保留该行**（转为普通待办），不静默消失。若此时 `due_at` 已过，则按逾期处理。

### 5.4 虚拟实例展开

新增 `src-tauri/src/todo_recurrence.rs`（**周期规则的唯一实现**）：

- `next_occurrence(todo, from_ms) -> i64` —— 从 `from_ms` 起算的下一个实例时刻；§5.1 的滚动与这里的展开共用它；
- `expand_occurrences(todo, from_ms, to_ms) -> Vec<i64>` —— 展开区间内的实例时刻，设数量上限；
- 对外命令 `expand_todo_occurrences(from_ms, to_ms)` → `Vec<{ todo_id, at_ms }>`，只返回区间内会被触发的周期待办；日历切月/切周时各调一次（本地 IPC，开销可忽略）。

**前端不重写规则**，只渲染命令返回的时刻（虚线样式）。规则只存在一份，从根上不存在「两端漂移」问题。

**列表视图不展开**（只显示当前实例），否则同一件事会在列表里出现多次；虚拟实例只出现在日历/周视图，用虚线样式与真实条目区分。

### 5.5 周期待办与子待办

周期待办**允许**带子待办（一层，沿用现有 `parent_id` 与 `useTodoChildren`），此前「不能有子待办」的结论作废。

滚动时子项**全部复位为未完成**（§11 第 6 项）——否则第二轮进来子项全是勾好的，看起来像这一轮已经做完了。

### 5.6 置顶

- 置顶与日期无关：`pinned = 1` 的条目**不参与日期分组**，统一进列表最顶部的「置顶」区（在「逾期」组之上），因此周期待办滚动换组时不会掉出置顶区。
- 置顶区内的上下次序沿用 `sort_order`（与组内排序同一套机制）；取消置顶后回到它当前 `due_at` 所属的日期组。
- 置顶只影响**列表**分组，不影响日历：置顶条目仍按 `due_at` 出现在日历上。
- 辨识度：行首图钉图标 + 置顶区标题（做法见 §11 第 7 项待确认）。

## 6. 视图与交互

### 6.1 独立视图（方案乙）

- `ViewId` 增加 `'todos'`（`src/index/index.vue:122`），`navigation` 增加 `{ id: 'todos', label: '待办', icon: ListTodo }`（同文件 `:111`），主区加一个 `v-else-if` 段。
- 新组件 `src/components/TodoView.vue`：
  - **左栏 40%**：顶部范围选择器（今天/本周/本月/全部）+ 分组列表（**置顶 → 逾期 → 今天 → 本周 → 本月 → 无日期**，沿用 `utils/todoSchedule.ts` 的 `groupOf`/`GROUP_META` 语义并扩展；「置顶」组见 §5.6）。
  - **右栏 60%**：月/周切换 + 日历网格。
  - 以上是**宽窗**形态；窄窗切方案丙，见 §6.3。
- 时间范围定义（Q17）：**自然周（周一 00:00 起）+ 自然月**，与现有 `nextMonday`/周一起排的日历一致。
- 逾期条目（Q18）：在列表**顶部单列一组「逾期」**，不计入任何范围。

### 6.2 日历交互

| 交互 | 行为 |
|---|---|
| 点空白日期 | 打开新建，预填该日期 |
| 拖拽真实条目 | 改 `due_at`（仅非周期待办） |
| 拖拽周期待办 | **禁用**（`cursor: not-allowed` + 提示「周期待办请进编辑弹层改规则」，ADR 0009） |
| 拖拽虚拟实例 | **禁用**（不落库，无物化目标） |
| 点条目 | 打开编辑弹层 |

### 6.3 窄窗降级：切方案丙

方案乙在窄窗下左右两栏会互相挤扁。**窄窗直接切方案丙的布局**：单栏 + 一行双维工具栏（范围选择器 + 列表/日历切换），日历模式下再分月/周。这样「范围」与「展示形态」两个维度在两种宽度下都保持可用，窄窗只是把同屏改成切换，不会出现「切到日历就丢了范围」的问题。切换阈值建议 **720px**（内容区宽度），实现时按真实窗口尺寸微调。

### 6.4 工作台整合

- `TodoCard.vue` 标题栏右侧加「打开待办视图 →」（参考 `NotesOverviewCard.vue:53` 的 `no-more` 按钮写法），点击 `activeView = 'todos'`；卡片职责固定为快速录入 + 逾期/今天。
- 新增工作台模块 `todo_calendar`（见 §7）。

### 6.5 勾选父待办的确认

现有行为是**静默连勾**：勾父待办时把未完成的子项一并勾上，用户无从察觉（`TodoRow.vue:92-100`，取消完成不连带走子项）。改为：

1. 勾选父待办且**存在未完成子项**时，先弹确认：「该待办下还有 N 条子待办未完成，是否确认」；
2. 确认 → 父待办与全部未完成子项一起置为完成（即现有连勾行为，但用户已知情）；
3. 取消 → 整条操作不生效，父待办保持未完成；
4. 无未完成子项时行为不变（直接勾选，不弹窗）；**取消完成**依旧不连带走子项。

## 7. 工作台新模块（日历）

- `src/composables/useDashboardLayout.ts` 的 `DASH_MODULES` 增加：

  ```
  { id: 'calendar', title: '日历', defaultVariant: 'month',
    variants: [v('month', '整月日历', 4, 4, 5, 5, '待办分布')] }
  ```

- `src/index/index.vue:266` 的 `dashCardComponents` 增加 `calendar: TodoCalendarCard`。
- 新组件 `TodoCalendarCard.vue`：**只读**月历，渲染待办分布（含虚拟实例）；整卡点击 → `activeView = 'todos'`。

**原 clock 模块的「整月日历」形态已删除**（本次一并做掉）：删掉 `useDashboardLayout.ts` 的 `month` 形态定义、`ClockCard.vue` 与 `DashModulePreview.vue` 的渲染分支与样式、`useDashPreviewData.ts` 里配套的 `monthHead` / `monthCells`。理由：工作台不需要两个日历，新的 `calendar` 模块取代它并带上待办数据。旧布局数据里 `variant: 'month'` 的项由 `dashVariantDef()` 自动回退到 `clock` 的 `defaultVariant`（`big`），**无需数据迁移**。

`DASH_MODULES` 里现有 `todo_overview`（待办概览）与 `todo`（待办）两个待办模块，加上 `calendar` 是第三个；是否合并 `todo_overview` 属独立议题，本方案不动它。

## 8. 扩展桥（`window.xhub.data.todos.*`）

- `list` / `get` **立即返回新字段**（`serde` 序列化 `Todo` 结构体即自动带上；旧扩展忽略未知字段，天然兼容）。
- `create` / `update` **签名不扩**，新字段走新方法，避免破坏既有扩展的调用：
  - `todos.setDescription({ id, description })`
  - `todos.setRepeat({ id, repeat })`
  - `todos.setTags({ id, tagIds })`
  - `todos.completeRecurring({ id })`
  - 标签定义管理：`data.todoTags.list/create/update/delete`
- 改动落点：`src-tauri/src/xhub_api.rs` 的 `CAPABILITIES` 表 + `src-tauri/src/extension.rs:880` 的 `todos:{}` 桥对象。
- **必须同步更新 `xhub_api.rs:1825` 起的能力键断言测试**，否则 CI 会漏掉新方法。

## 9. 测试与验收

- Rust：`repo/todo.rs` 的既有 `#[cfg(test)]` 模式补周期滚动用例（含「逾期 3 天后直接跳到未来」「月末 31 号在 2 月的处理」「until/count 用尽」）；迁移用例补「老库补列后字段默认值正确」。
- **周期规则只实现一份，放在 Rust**（§5.4）：`src-tauri/src/todo_recurrence.rs` 同时服务滚动与虚拟展开，前端只消费 `expand_todo_occurrences` 的结果、**不重写规则**。规则唯一 → 不需要维护两份实现，也不存在「两端漂移」。
- 前端：`TodoView.vue` 的日历渲染逻辑单测（给定展开结果 → 格子分布正确）。
- 手工验收（v1）：建一条「每天」待办 → 勾选 → 日期滚到明天且不进已完成；建一条「每月 1 日」→ 日历上能看到下月 1 日的虚线实例；打标签 → 视图按标签筛选；拖拽非周期待办改期生效、拖拽周期待办被拒并有提示；置顶一条 → 出现在最顶部「置顶」区并带图钉图标，勾选完成后仍在置顶区；勾选带未完成子项的父待办 → 弹确认，确认后子项全勾、取消后父待办仍为未完成。

## 10. 分期

### v1（本次）

1. 迁移：`todos` 补 `description` + `repeat_*` + `pinned` 列；建 `todo_tags` / `todo_tag_links`。
2. Rust：`todo_recurrence.rs` 规则引擎（`next_occurrence` / `expand_occurrences`）与 `expand_todo_occurrences` 命令；周期滚动（含撤销）、描述/标签/置顶的写回（带乐观锁）、标签 CRUD。
3. 前端：`TodoView.vue`（宽窗方案乙 / 窄窗方案丙）；标签筛选；置顶区与图钉图标；编辑弹层（描述/标签/周期）；勾选父待办的确认弹窗（§6.5，改掉 `TodoRow.vue` 的静默连勾）。
4. 工作台：卡片标题栏入口；`calendar` 只读模块；删除 clock 的「整月日历」形态。
5. 扩展桥新方法 + 能力键测试。

### v2

1. 附件：拖拽上传、哈希落盘、条目回形针标识、删除待办时的文件生命周期与孤儿回收。
2. 日历高级交互：跨周期「只改这一次」（需引入例外机制，见 ADR 0009 的后果一节）。
3. 「清单（list）」能力（单归属容器），届时需与标签做清晰的概念切分。

## 11. 边界确认结果

| # | 议题 | 结论 |
|---|---|---|
| 1 | 周期待办能否有子待办 | **可以**（一层，沿用 `parent_id`）；滚动时子项处置见第 6 项 |
| 2 | 周期待办能否设优先级 / 手动排序 | 优先级**可以**；排序**两者都要**：置顶（§5.6）+ 组内 `sort_order` |
| 3 | 周期滚动时是否发轻提示 | **发**（toast 级，不进通知中心） |
| 4 | 窄窗阈值与降级形态 | **窄窗切方案丙**（§6.3），阈值 720px 实现时微调 |
| 5 | 附件与同步的关系 | 应用内**没有同步功能**（§4）；附件不在库里是既有事实，不是本次引入的限制 |
| 6 | 周期滚动时子待办如何处置 | **复位**：滚动到下一轮时，子待办一并复位为未完成（与「不逐次留历史」口径一致） |
| 7 | 置顶的辨识度做法 | **采用原型定版**：行首图钉图标 + 顶部独立「置顶」分区（在「逾期」之上）+ 该区淡紫底色 |

**第 6 项（已定：复位）**：勾选周期父待办时，确认弹窗会把未完成子项一并勾上（§6.5）。滚动到下一轮时，**全部子待办复位为未完成**——否则第二轮进来子项全是勾好的，看起来像这一轮已经做完了。代价是上一轮子项的完成记录不留痕，与 ADR 0009「不逐次留历史」的口径一致。

**第 7 项（已定）**：置顶辨识度采用原型定版——行首图钉图标 + 列表顶部独立的「置顶」分区（在「逾期」组之上）+ 该分区淡紫底色。

## 12. v1 收尾迭代（2026-09-21 反馈）

实现 v1 后收到 5 条反馈，前 4 条已改，第 5 条待定。逐条对应 §6.2 / §6.5：

| # | 反馈 | 处置 | 落地位置 |
|---|---|---|---|
| 1 | 日历里拖拽是不是没实现 | **已实现**：实际条目可在月/周日历内拖到其他日期 → 改 `due_at`（保留原时分）；`remind_at` 若已设，按同一日偏移平移，保持「提前量」不变 | `TodoView.vue`（`onChipPointerDown` / `moveToDay`） |
| 2 | 待办要不要和速记一样有卡片 | **待确认**（见下） | — |
| 3 | 点置顶小图标要能立即取消置顶 | **已实现**：图钉由静态图标改为按钮，点击即 `setTodoPinned(false)` + toast；取消后回到按日期分组的位置 | `TodoView.vue`（`unpin`）、`TodoRow.vue`（`unpin`） |
| 4 | 新建弹窗里弹出的日历 UI 与客户端不符 | **已修**：原生 `<input type="datetime-local">` 换成应用内 reka-ui 日历（与 `CountdownCard` 同一套组件与样式口径），弹出层用 `:global()` 设 z-index 130（modal 是 100） | 新增 `TodoDateTimeField.vue`；`TodoEditDialog.vue` |
| 5 | 「截止」「提醒」分两行，每行加快捷时间 | **已实现**：截止 = 今天 / 明天 / 本周六 / 下周一 / 本月末 / 下月初（都落当天 23:59）；提醒 = 同截止 / 提前 1 小时 / 提前 1 天 / 今天 09:00 / 明天 09:00；两行各带「清除」 | `TodoDateTimeField.vue`、`TodoEditDialog.vue`（`buildDuePresets` / `remindPresets`） |

**拖拽语义的两处实现取舍**（与 §6.2 的表格一致，补充细节）：

- 用**指针事件**而非 HTML5 DnD：Tauri 主窗口拦截原生拖放，`dragstart` 后收不到 `dragover`/`drop`（与卡片组内拖拽同一处理）。
- **虚拟实例（周期规则算出的未来轮次）不可拖**：它不是落库对象，按 §6.2 保持 `cursor: not-allowed`，按下给一句「请到编辑弹层改周期规则」。

**第 2 项待确认**：「速记有一个卡片」这句话有两种读法，不确定是哪个，实现前问一句——(a) 指工作台的模块卡（速记有「速记统计」卡，待办其实已有 `todo_overview` + `todo` + `calendar` 三张）；(b) 指速记**视图内**是卡片式列表，希望待办视图也从紧凑列表改成卡片式。等确认后再动。

### 12.1 顺手改掉的一处既存 bug（已披露）

周期区的「结束日期」原来用原生 `<input type="date">`，同时**存值有错**：`new Date('YYYY-MM-DD')` 按 UTC 解析，在 +08:00 下等于所选日期的 **08:00**，而 `until` 的判定是 `at_ms <= end_at`（`todo_recurrence.rs:318`）——于是「到 9-30 结束」把 9-30 当天也排除了（Rust 侧测试用的正是 `23:59`，说明预期语义是「含当天末尾」）。

改动：换成同一个 `TodoDateTimeField`（`dateOnly` 模式），落库值改为**所选当天 23:59:59.999（本地）**，与 Rust 测试口径一致。`repeat_end_at` 只是前端写入的值，Rust 无改动。


