# x-hub Agent 底座实施方案（定稿）

> 状态：**方案定稿，尚未开发**（2026-08-29 定稿）。决策依据见 `docs/adr/0006-agent-runtime-selection.md`。
> 本文是**实施蓝图**：写明要改哪些文件、改成什么样、按什么顺序做。
> **全部决策已于 2026-08-29 拍板**（见 §10 决策记录），本文可直接作为开发任务书使用。

---

## 1. 目标与边界

### 目标
给 x-hub 一个**全软件共用的 AI 入口底座**：用户用自然语言说话，AI 能调用宿主自身的能力（笔记/待办/速记/速达/剪贴板/倒计时…）把事办完，而不只是聊天。

### 明确不做（边界）
| 不做 | 原因 |
|---|---|
| 分布/外挂任何外部 Agent 引擎（opencode/DSH/Codex…） | 体积 171–222 MB vs 本体 4.9 MB（实测，见 ADR 0006） |
| 引入 MCP 或任何跨进程工具协议 | 工具就是同进程 Rust 函数，直接调用；MCP 只在接第三方工具生态时需要 |
| 通用 coding 能力（bash/文件编辑/git/沙箱） | 语义错配：本产品用户是效率工作台用户，不是改代码的程序员 |
| 多 agent 编排、子 agent、Ralph/Workflow 类机制 | 首期不需要 |
| `set_setting`（让模型改配置） | 风险高（改坏配置用户难排查）、收益低；首期不做 |
| 跨会话记忆（RAG/向量检索/自动记忆） | 需要的事实全在本机库中，**现查比记忆更准**；记忆出错/过期反而让 AI 表现异常且用户难以纠正（见 §4 记忆分层） |

---

## 2. 总体架构

```
┌─ Vue 前端 ────────────────────────────────────────────────┐
│ ChatPanel.vue / ChatWindow.vue                            │
│   按「turn」分组渲染：用户消息 → [工具调用卡片…] → 最终回答    │
│   工具卡片内联「允许 / 拒绝 / 本会话允许」；顶部权限模式切换    │
└───────────────┬───────────────────────────────────────────┘
      invoke send_chat_message(turn 流)  │  on_event: Channel<ChatStreamEvent>
                ▼                        ▲
┌─ Rust 宿主 ───────────────────────────────────────────────┐
│ agent/mod.rs   loop 编排：请求 → 解析 tool_calls → 执行 → 回填 → 续跑 │
│ agent/tools.rs 工具注册表：schema + 级别 + 执行函数           │
│ agent/approval.rs 审批往返 + 会话级授权（内存）              │
│ agent/prompt.rs  system prompt 组装（含时间与用户档案注入）   │
│ agent/context.rs 上下文组装：按 turn 截断 + 工具消息成对剪枝   │
│ chat.rs        SSE 客户端（扩展：tools 参数 + tool_calls 解析）│
│ commands.rs    命令层；repo/chat.rs 会话与消息持久化           │
└───────────────┬───────────────────────────────────────────┘
                ▼ HTTPS（域名已备案，证书就绪）
   ┌────────────────────────┐      ┌──────────────────────┐
   │ 平台额度：x-hub-server  │  或  │ BYOK：用户自配供应商   │
   │ /v1/chat/completions   │      │ 直连，不经平台服务器   │
   └────────────────────────┘      └──────────────────────┘
```

**模块划分【已定】**：新增 `src-tauri/src/agent/`（`mod.rs` 循环编排、`tools.rs` 工具表、`approval.rs` 审批与会话级授权、`prompt.rs` 提示词组装、`context.rs` 上下文组装），`chat.rs` 做增量扩展而非重写。

---

## 3. 模型接入（双路并行）

### 服务端现状（`x-hub-server` M0 已交付）
- `POST /v1/chat/completions`：OpenAI 兼容 + **SSE 流式透传**（body 全量转发，故 `tools` 参数可过）
- 原子预扣额度、失败回补、`usage_logs` 落库
- 邀请码注册、每人 100 次额度、64 位 token（库存 sha256）、单设备在线、登录/注册限流
- 并发闸 `AI_PER_USER_ACTIVE=2` / `AI_TOTAL_ACTIVE=200`；`AI_ALLOWED_MODELS` 白名单

### 客户端接入【已定】
新增「平台额度」作为一种供应商写入 `chat_models`，与 BYOK 供应商同构：

| 字段 | 平台额度 | BYOK |
|---|---|---|
| `base_url` | `https://<domain>/v1`（已备案，HTTPS 就绪） | 用户填（内置主流厂商预设自动填） |
| 凭据 | 平台签发 token（存 keyring，复用现有机制） | 用户自己的 API Key（存 keyring） |
| 新增字段 | `auth_kind: "platform"` | `auth_kind: "byok"` |

**新增命令**：`platform_register`（邀请码 → 拿 token）、`platform_login`、`platform_quota`、`platform_logout`。
**新增 UI**：注册/登录页（设置「AI 助手」区 + 首启引导）、额度显示（对话框顶部）、429 分流提示。

**两条路都保留【已定】**：BYOK 保留 —— 用户可绕开平台直连供应商（省钱、隐私更好、平台零成本）。
**隐私口径【已定，必须讲清】**：平台额度 = 对话经平台服务器；BYOK = 直连。设置里并列展示并注明区别（x-hub 是 local-first 应用，这条不能含糊）。

**BYOK 降门槛【已定】**：内置主流供应商预设（DeepSeek / 智谱 / Kimi / 硅基流动 / OpenRouter…），选厂商 + 贴 Key 即可，不用懂 `base_url`。可与 `docs/adr/0001` 首启引导合并。

---

## 4. Agent Loop

### 一轮对话（turn）的时序

```
用户输入
  ↓ 生成 turn_id（UUID），落 user 消息
  ↓ ┌───────── 循环（上限 10 轮）─────────────┐
    │ 组装 messages（system + 历史 + 本轮工具结果）│
    │ POST /chat/completions {tools, stream:true}│
    │ 流式解析：content 分片 → Chunk 事件          │
    │           tool_calls 分片 → 按 index 累积    │
    │ 若本轮无 tool_calls → 结束循环               │
    │ 若有 → 按权限模式处理：                       │
    │    readonly：写工具不进 tools 列表，无从调用   │
    │    ask：命中会话级授权则直跑，否则            │
    │         emit ApprovalRequest → 等前端回传     │
    │    full：直接执行（不可逆类按配置可能仍提示）    │
    │    emit ToolCall / ToolResult 事件           │
    │    结果作为 role:"tool" 消息回填 → 继续循环   │
    └────────────────────────────────────────────┘
  ↓ 落 assistant 最终回复；累加 usage；emit TurnDone
```

### System Prompt【已定】
`agent/prompt.rs` 组装，每次请求带上。内容分四段：

1. **身份与边界**：你是 x-hub 个人效率工作台的助手；你能通过工具操作本机的笔记、待办、提示词、速达资源、便签、剪贴板与倒计时；**你没有的能力（联网搜索、写代码、操作文件系统）要如实说明，不要假装做过**。
2. **当前时间（动态注入）**：`现在是 2026-08-29 星期六 14:32（农历…）`。
   > **为什么不做成工具**：`get_date_info` 若做成工具，模型每轮都得先花一次调用去问"今天几号"，既费额度又慢。时间属于"每轮都需要的背景信息"，注入 prompt 是正解（DSH 的 `dsh-time-context` 即此思路）。**故首批工具表里没有日期工具。**
3. **行为准则**：需要事实（如"我有什么待办"）时**必须用工具查，不要凭记忆编造**；执行写操作前用一句话说明将要做什么；一次只做用户要求的事，不擅自扩大范围；跟随用户的语言回复。
4. **用户档案（可选，用户自填）**：来自设置「AI 助手 → 关于我」的一段自由文本（如"我是做后端的，常用项目是 x-hub；回复请简洁"）。**首期只读**——用户手写、注入 prompt，模型**不写回**。这本质上是 `AGENTS.md` 那个模式的个人版。若要给模型"自己记"的能力（`remember` 工具），须作为受权限管的写操作单独评估，且必须让用户看得见 AI 记了什么、能删。

### 上下文与记忆（三层，须分开处理）【已定】

> **会话内上下文、长会话压缩、跨会话记忆是三件事，方案不同，别混为一谈。**

**① 会话内上下文：按 turn 截断（不是按条数）**

现有 `chat::list_recent_messages(session_id, CHAT_CONTEXT_WINDOW)` 是**按「最近 N 条消息」**截断——在纯聊天里够用，但**在 agent 场景下不成立**：一个 turn 会产生多条消息（`user` + `assistant(tool_calls)` + 若干 `tool` + `assistant` 回复），按条数截断可能只覆盖一两个 turn，模型看不见前面的对话。

**改为按 turn 截断**：保留最近 N 个**完整** turn（同一 `turn_id` 的消息要么全留、要么全不留，**不可截断半个 turn**）。N 待实测，建议 6～8 个 turn 起步。

**② 历史工具消息剪枝：必须成对**

已完成 turn 里的工具调用与结果对上下文价值低（信息已体现在该 turn 的最终回复中），可剪枝省 token（DSH 专设 `compaction-tool-result-pruner` 做此事）。

**铁律：剪枝必须成对**——`role:"tool"` 消息必须紧跟在携带它的 `assistant.tool_calls` 之后，只删一边会被上游判 400（`tool_call_id` 找不到对应调用）。**按 turn 整体剪枝天然满足该约束**。

**③ 长会话压缩（compaction）：延后至 M5**

把老 turn 摘要成一段。首期不做——会话通常不长，有 ①② 打底足够；等实测出现"上下文吃紧"再上。

**④ 跨会话记忆：首期不做**

| | 靠记忆 | 靠工具查（采用） |
|---|---|---|
| "我有什么待办" | 记忆可能过期，答错的 | `list_todos` 查最新的，**必然正确** |
| "上次那个项目叫什么" | 可能记混 | `global_search` 搜一下 |

Agent 需要的事实**全在本机数据库中，现查比记忆更准**；而记忆出错/过期会让 AI 表现得"很奇怪"，且用户难以纠正（他不知道 AI 记了什么）。故：**不做 RAG/向量检索、不做自动记忆写入**；只用「关于我」这一条用户自填、可见、可编辑的显式记忆（§4 System Prompt 第 4 段）。

### 关键约束【已定】
- **循环上限 10 轮**（超出则强制收尾并提示「任务过于复杂，已停止」）。
- **流式 `tool_calls` 分片累积**：SSE 里 `delta.tool_calls[].function.arguments` 是**逐段拼接**的，必须按 `index` 累积后再 `serde_json::from_str`。现有 `chat.rs` 只解析 `delta.content`——**实现中最容易踩的一处**。
- **中断**：用户点中断 → 取消 HTTP 流 + 停止循环 + 丢弃未完成的工具调用；已执行的写操作**不回滚**，UI 如实显示"已执行"。
- **错误处理**：上游出错/额度耗尽 → 保留已生成内容并标注原因；工具执行失败 → 错误作为工具结果回填给模型（让它自己决定重试或换路），**不终止对话**。
- **工具结果截断**：见 §5。

### 事件协议扩展【已定】
`ChatStreamEvent`（现为 `Chunk` / `Done` / `Error`）新增：
| 事件 | 载荷 | 用途 |
|---|---|---|
| `ToolCall` | `{turn_id, call_id, name, args, level}` | 渲染工具卡片 |
| `ToolResult` | `{call_id, ok, summary, detail}` | 更新卡片状态 |
| `ApprovalRequest` | `{turn_id, call_id, name, args, level}` | 内联确认 |
| `TurnDone` | `{turn_id, message, session, usage, tool_count}` | 收尾、额度刷新 |

---

## 5. 工具系统

### 命名与 schema 规范【已定】
- 命名用**动作式**：`list_todos` / `create_todo` / `create_note` / `search_notes`（避免 `data.todos.list` 这类命名空间式——模型对动作式动词理解更稳）。
- 每个工具：`name` + `description`（写清"什么时候该用、什么时候不该用"）+ JSON Schema 参数（**参数尽量少**，必填项最小化）+ `level`。
- **数量有成本**：`tools` 定义会随**每一次**请求发送（LLM API 无状态，服务端渲染成 prompt 文本；13 个工具约占 2000–3000 tokens/次，agent loop 每轮重复一次）。仅靠 KV cache 摊薄，故：**定义顺序与内容必须稳定**（顺序固定，勿用 HashMap 随机序序列化）、description 精炼、参数最小化、首批严格控制在 13 个。

### 权限模式（三档）【已定】

| 模式 | 行为 | 适用 |
|---|---|---|
| **仅可查看** | 写/危险级工具**根本不进 `tools` 列表**（从根本上无法调用，且不浪费 token） | 只想让 AI 查资料/总结的用户 |
| **每次都问** | 读工具直跑；写/危险级每次弹**内联确认**；**支持「本会话内允许这类操作」** | 默认档，稳妥 |
| **完全权限** | 全部直接执行；**删除类与启动外部程序仍提示一次**（设置里提供可关闭的子选项，默认开） | 熟练用户 |

- 模式存 `AppConfig.agent_permission_mode`（`readonly` / `ask` / `full`，serde default 天然兼容老配置）。
- **对话框中提供快捷切换**（像现在的模型选择器一样），嫌"每次都问"烦时可即时切换。
- **会话级授权【已定】**：「每次都问」档下，确认卡片提供「本会话内允许这类操作」；授权**只存内存**（`agent/approval.rs` 里 `HashMap<session_id, HashSet<tool_name>>`），**不落库**——语义就是"本会话"，进程重启或换会话即失效，避免"临时允许"悄悄变成永久允许。
- **不可逆操作保护【已定】**：删除类（`delete_*`）与 `launch_resource` 在「完全权限」档下默认仍提示一次；用户可在设置里关掉该提示。

**权限口径【已定，易错】**：这是**用户对 AI 的授权**，与扩展系统 `CAPABILITIES` 的 manifest 权限是**两套独立体系**，不可混用、不可共用存储。

**审批交互【已定】**：确认卡片**内联在对话流里**（工具卡片上给「允许 / 拒绝 / 本会话允许」），不用弹窗——弹窗打断阅读，且避开约定 41 的窗口铁律（不新建 WebView2 窗口）。

### 首批工具（13 个）【已定】

**读（7）——直接执行**
| 工具 | 说明 |
|---|---|
| `list_todos` | 列待办（按逾期/今天/有日期/无日期分组，与 `todoSchedule` 口径一致） |
| `search_notes` | 按关键词搜笔记（复用 GlobalSearch 查询口径） |
| `get_note` | 读单篇笔记全文 |
| `list_resources` | 列速达资源（按分类） |
| `list_countdowns` | 列倒计时（含剩余时间） |
| `read_clipboard` | 读最近剪贴板条目 |
| `global_search` | 跨资源/笔记/待办统一搜索 |

**写（5）——受权限模式管**
| 工具 | 说明 |
|---|---|
| `create_todo` | 新建待办（可带截止日期/提醒/优先级） |
| `create_note` | 新建笔记（标题 + 正文） |
| `append_to_note` | 追加内容到已有笔记 |
| `add_snippet` | 存一条提示词/片段 |
| `write_sticky` | 写便签（slot 1/2） |

**危险（1）——影响外部世界**
| 工具 | 说明 |
|---|---|
| `launch_resource` | 启动应用 / 打开网页或文件 |

### 后续候选（不进首批，跑一阵后按实际使用增补）
| 领域 | 候选 |
|---|---|
| 速记 | `list_notes`、`update_note`、`delete_note`、`list_tags` |
| 待办 | `complete_todo`、`update_todo`、`schedule_todo`、`create_subtodo`、`delete_todo` |
| 提示词 | `list_snippets`、`update_snippet`、`toggle_snippet_pin`、`delete_snippet` |
| 速达 | `list_recent_resources`、`create_resource` |
| 便签 | `get_stickies` |
| 剪贴板 | `search_clipboard`、`set_clipboard`、`pin_clipboard` |
| 倒计时 | `create_countdown`、`toggle_countdown`、`delete_countdown` |
| 环境 | `get_system_status`、`get_weather` |
| 界面 | `open_view` |
| 设置 | `get_setting`（`set_setting` 首期不做，见 §1） |

### 工具结果长度控制【已定】
工具结果会进模型上下文并持久化。约定：**回填给模型的结果做截断**（列表类最多 20 条、正文类最多 2000 字），完整结果只存本地供 UI 展开——避免上下文爆掉与 token 浪费。

### 环境类工具的失败语义【已定】
`get_weather`（后续批次）受联网总开关控制：开关关闭时**工具自身要说明"联网已关闭，无法获取天气"**，而不是抛错误——否则模型会反复重试。同类工具（`global_search` 等在数据为空时）一律遵循"返回可用信息 + 原因"的口径，不返回异常。

---

## 6. 数据模型（必须迁移）

### 硬约束：现有表结构装不下工具调用
```sql
-- 现状（db.rs:164）
CREATE TABLE chat_messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id INTEGER NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),   -- ← 装不下 role:"tool"
  content TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (...)
);
```
`CHECK` 约束无法 `ALTER`，SQLite 需**重建表**（`db.rs` 已有先例：旧 `resources` 表含 `group_id` 时重建过，照抄该迁移模式）。

### 目标结构【已定】
```sql
CREATE TABLE chat_messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id INTEGER NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
  turn_id TEXT,                      -- 一次用户输入 = 一个 turn（UI 分组 + 额度计费 + 上下文截断共用）
  role TEXT NOT NULL CHECK (role IN ('user','assistant','tool')),
  content TEXT NOT NULL DEFAULT '',
  tool_calls TEXT,                   -- assistant 消息携带的调用请求（JSON 数组）
  tool_call_id TEXT,                 -- tool 消息对应的 call_id
  tool_name TEXT,                    -- tool 消息的工具名（UI 与统计用）
  tool_status TEXT,                  -- ok / error / denied
  created_at TEXT NOT NULL DEFAULT (...)
);
CREATE INDEX idx_chat_messages_turn ON chat_messages(session_id, turn_id, id);
```

**为什么引入 `turn_id`【已定】**：一个 turn 在 UI 上要渲染成一组（用户消息 + 若干工具卡片 + 最终回答），按 `turn_id` 分组最直接；**它还同时是上下文截断的单位**（§4 ①）**与服务端额度计费的键**（§8）——一个概念支撑三处，前后端口径统一。

`chat_sessions` 不动。额度信息**不入库**（实时向服务端查）。会话级工具授权**不入库**（只存内存，见 §5）。

---

## 7. UI 设计

### 时间线【已定】
- 现有 `ChatPanel.vue` 渲染 `messages` 数组（markdown 流），改为**按 `turn_id` 分组渲染**。
- 工具卡片：折叠态一行（图标 + 工具名 + 一句话结果摘要，如「查询待办 → 3 条」），点击展开看参数与完整结果。
- 状态可视：进行中 / 完成 / 失败 / 被拒绝，四态各有样式（复用现有 `--c-*` 语义色）。
- 审批：卡片内联「允许 / 拒绝 / 本会话允许」。
- 中断：发送中显示「停止」按钮（替换发送键）。
- 流式渲染沿用现有 `scheduleStreamRender` 的 80ms 节流机制，工具卡片不参与 markdown 重解析。

### 入口与设置【已定】
- **侧栏入口放开并改名「AI 助手」**（`index.vue` 的 `visibleNavigation` 过滤解除、导航项改名）。
- 会话沿用现有两形态：内嵌抽屉（`ChatPanel`）+ 独立窗（`ChatWindow`），不做第三套。
- 设置「AI 助手」区改版为四段：**模型接入**（平台额度 / BYOK 切换 + 注册登录 + 额度）｜**AI 能力授权**（三档权限模式 + 「不可逆操作仍提示」子开关）｜**关于我**（用户自填偏好文本，注入 system prompt，首期只读）｜**会话偏好**（循环上限、工具卡片默认折叠等）。

---

## 8. 服务端改造（额度口径）

### 问题【已定，必改】
服务端现为**按请求次数**扣额度（`routes/ai.ts`：`UPDATE quotas SET used = used + 1 WHERE user_id = ? AND used < total`，默认 total=100）。而 agent loop 一次用户输入会产生**多次** `/v1/chat/completions`：

| 场景 | 请求次数 |
|---|---|
| 简单问答（无工具） | 1 |
| 记一条待办 | 2～3 |
| 复杂多步任务 | 5～10 |

**即「100 次额度」实际只等于 10～30 次真实对话**，用户会觉得"聊几句就没了"。

### 方案【已定：A. turn 粒度计费】
客户端为每个 turn 生成 UUID，随请求带 `X-XHub-Turn-Id`；服务端 `usage_logs` 加 `turn_id` 列，**预扣前先查该 turn 是否已扣过**，已扣则跳过预扣（仅落日志）。

- 用户心智 = "一次对话扣一次"，与产品宣传口径一致
- 与 §6 的 `turn_id` 概念统一，前后端同键
- 依赖客户端诚实（可接受：客户端是自己的）
- **并发闸仍按请求计**（`AI_PER_USER_ACTIVE`），不随 turn 合并——每 turn 的请求是串行的，2 个在途额度够用

### 另外两处服务端待办【已定】
- **模型白名单要筛**：`AI_ALLOWED_MODELS` 只放**支持 function calling** 的模型，否则模型会忽略 `tools`，表现为"agent 光说不干"。
- **补成本熔断**：现有并发闸只防拥塞，不防账单失控；建议加日/月总预算上限，超限停止发放额度。

---

## 9. 分阶段落地

| 阶段 | 交付 | 验收标准 |
|---|---|---|
| **M1 模型接入** | 客户端接平台中转（注册/登录/额度/BYOK 并存）+ 供应商预设 + 首启引导 | 全新用户无需配置文件即可对话；能一键切到自己的 Key |
| **M2 Loop 骨架** | 数据模型迁移 + `chat.rs` 支持 tools + agent 循环 + system prompt + **按 turn 上下文截断** + 7 个只读工具 + 基础时间线 | "我的待办有什么？"能查出来并正确渲染卡片；长会话中前面 turn 的内容仍可见 |
| **M3 写操作与审批** | 5 个写工具 + `launch_resource` + 三档权限模式 + 内联审批 + 会话级授权 + 中断 + 历史工具消息剪枝 | "把这三件事记成待办"能完成，且写操作可确认、可回看 |
| **M4 体验打磨** | 工具按实际使用增补 + 「关于我」设置项 + 折叠/重试/错误恢复 + 结果截断策略 | 连续 20 轮复杂任务不崩、不失控、额度可预期 |
| **M5（可选）** | 长会话压缩（compaction）、主动能力（定时触发、事件驱动）、工具扩容；（若届时仍需要）跨会话记忆的显式方案 | 另行评估，不在本期承诺 |

**发版注意**：M2 起涉及 `chat_messages` 重建表迁移，按 `db.rs` 既有幂等迁移模式做，并在发版清单里加「老库升级后对话历史完整可读」的实机自查。

---

## 10. 决策记录（2026-08-29 拍板）

| # | 问题 | 决策 |
|---|---|---|
| 1 | 第一批工具范围 | **13 个**：读 7（`list_todos`/`search_notes`/`get_note`/`list_resources`/`list_countdowns`/`read_clipboard`/`global_search`）+ 写 5（`create_todo`/`create_note`/`append_to_note`/`add_snippet`/`write_sticky`）+ 危险 1（`launch_resource`） |
| 2 | 权限策略 | **三档模式**：仅可查看 / 每次都问 / 完全权限 |
| 3 | 额度口径 | **A. turn 粒度计费** |
| 4 | BYOK 是否保留 | **保留**（与平台额度并存，用户可切） |
| 5 | 入口形态 | **放开侧栏入口并改名「AI 助手」** |
| 6 | 单 turn 循环上限 | **10 轮** |
| 7 | 平台域名/证书 | **已备案，HTTPS 就绪** |
| 8 | 会话级授权 | **做**：「本会话内允许这类操作」，只存内存不落库 |
| 9 | 不可逆操作保护 | **做**：完全权限档下删除类与 `launch_resource` 仍提示一次，设置里可关（默认开） |
| 10 | 日期信息 | **不做成工具**，改为 system prompt 动态注入 |
| 11 | `set_setting` | **首期不做** |
| 12 | 会话内上下文 | **按 turn 截断**（非按条数）+ 历史工具消息**成对**剪枝；长会话压缩延后至 M5 |
| 13 | 跨会话记忆 | **首期不做**（不做 RAG/向量检索、不自动写入）；仅提供「关于我」用户自填文本，只读注入 system prompt |

---

## 11. 已知坑（实现时对照）

1. **`tool_calls` 分片累积**：`delta.tool_calls[].function.arguments` 逐段来，必须按 `index` 拼完再解析（`chat.rs` 目前只认 `delta.content`）。
2. **`role` 的 CHECK 约束**：SQLite 改不动，必须重建表；注意 `idx_chat_messages_session` 索引重建与老数据搬迁。
3. **工具消息剪枝必须成对**：只删 `role:"tool"` 不删对应的 `assistant.tool_calls`（或反之）会让上游报 400（`tool_call_id` 无对应调用）。按 turn 整体剪枝天然规避。
4. **按条数截断对 agent 失效**：一个 turn 有多条消息，"最近 N 条"可能只覆盖一两个 turn——必须按 turn 截断（§4 ①）。
5. **工具结果爆上下文**：列表/正文类结果必须截断后再回填（§5），否则上下文迅速膨胀、成本失控。
6. **审批与约定 41**：确认交互用内联卡片，不要为"批准弹窗"新建 WebView2 窗口（运行时建窗是卡死铁律）。
7. **权限口径混淆**：AI 工具授权 ≠ 扩展 manifest 权限，两套存储与 UI 都要分开，别顺手复用 `.permissions.json`。
8. **平台额度的 429 有两种**：`quota_exceeded`（额度用尽 → 引导续费/切 BYOK）与 `too_many_active_streams`/`server_busy`（拥塞 → 提示重试），UI 必须区分。
9. **中断后的状态一致性**：写操作已执行不可回滚，UI 要如实标注"已执行"，不能显示成"已取消"。
10. **权限模式影响 tools 列表**：「仅可查看」档下写工具不进列表——切换模式会使工具定义前缀变化、KV cache 失效一次（可接受，但别做成高频切换的动画开关）。
11. **会话级授权的作用域**：按 `session_id` 存内存即可；"新建会话/切换会话"后授权必须失效（这正是"本会话"的语义），不要图省事做成全局 HashMap。
12. **tools 定义顺序要稳定**：序列化顺序随机会让 KV cache 永远命不中（每次请求工具定义都按原价计费）。
