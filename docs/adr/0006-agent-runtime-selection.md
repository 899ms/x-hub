# 0006-AI 底座选型：宿主内自研 agent loop，不分发外部 Agent 引擎

> **状态：已决策，待实施**（决策于「给 x-hub 集成 Agent」讨论，2026-08-29。经三轮修订，记录如下以免后人误读：① 初稿按「service 扩展形态 + DSH 首选」；② 二稿按「宿主原生 + ACP client + 引擎可配置（opencode 首选）」；③ **终稿按「宿主内自研 loop + 不分发外部引擎」**——前两稿的根本错误是把目标用户当成「本机已装 agent CLI 的开发者」，而 x-hub 的用户是**普通用户**，必须开箱即用、零外部安装。该约束一排他，外部引擎方案全部出局。对照文档：`x-hub-extensions/docs/extension-evolution.md`。）

## 问题

x-hub 需要**整个软件共用的 AI 入口底座**：全软件的 AI 能力（对话、摘要、改写、语义查找、自动化动作）都走这一层，而现有 `chat.rs` 只有 OpenAI 兼容 SSE 单轮流式对话——**没有工具调用、没有 agent loop**，模型不能触碰宿主的笔记/待办/速记/资源。

**关键约束（本稿的决定性前提）**：目标用户是普通用户，不会安装 Node、Bun、opencode、DSH 等任何外部引擎。能力必须随安装包交付，开箱即用。

## 决策

1. **AI 底座 = 宿主内自研 agent loop（Rust）**，与 `chat.rs` 同级，复用现有供应商配置（`chat_models`）与 keyring 凭据，**不引入任何外部 agent 进程**。
2. **工具 = 宿主能力的 Rust 函数**，沿用 `xhub_api.rs` 的 `CAPABILITIES` 静态注册模式扩展一张「模型可见工具」表；**不引入 MCP 等跨进程工具通道**（同进程直接调用，零序列化开销、类型安全、可审计）。
3. **不随包分发任何外部 agent 引擎**（opencode/DSH/Codex 等），理由见下。
4. **外部引擎（ACP）降级为高级用户的可选挂载**，不作为默认路径，也不参与首期；将来若要接，仍走 `agentclientprotocol/rust-sdk` 做 ACP client（该设计已在二稿论证，可复用）。
5. **模型接入走双路并行**：默认用**平台免费额度**（经 `x-hub-server` 中转，其 M0 已交付），高级用户可切 **BYOK 直连**（自备 Key 直连供应商，不经平台服务器）；两者在设置中并存可切换，隐私口径须向用户讲清（见「实施前提」）。

## 依据

### 一、体积：外部引擎会让安装包膨胀 35–45 倍（本机实测）

| 对象 | 体积 | 说明 |
|---|---|---|
| **x-hub 现有安装包** | **4.9 MB** | `x-hub-0.5.0-win-x64.zip` |
| opencode 单文件 | **171.3 MB** | `opencode-ai/bin/opencode.exe` |
| DSH 依赖树 | **221.8 MB** | `@deepseek-ai/dsh/node_modules` 合计 |
| Bun 运行时 | 281.1 MB | DSH 的备选运行时 |
| 内置 Node（按需下载） | 解压后约 80–100 MB | `runtime.rs` 现有机制 |

这些都是**为程序员改代码设计的 CLI**，把它们打包进一个 4.9 MB 的效率工作台，不叫集成，叫套壳发货——且首启需下载、解压、常驻内存。

（Rust 系引擎如 Codex 为单二进制，体积远小于 Node 系、但仍属「几十 MB」量级，即安装包的十倍上下；未实测，不作精确引用。）

### 二、语义错配：它们的工具集与「工作台助手」几乎不重叠

外部引擎的工具是 **bash / 文件编辑 / git / 代码搜索**——因为它们的用户是程序员，任务载体是代码仓库。而 x-hub 的 AI 底座需要的是 **新建笔记 / 查待办 / 记速记 / 搜资源 / 设提醒 / 读剪贴板**。

**这些工具它们一个都没有**，无论选哪个引擎，都得由宿主另写一层（MCP server 或插件）喂进去。既然如此，就没有理由为用不到的 shell/沙箱/git 能力付出 200 MB 与一个常驻进程。

### 三、已有资产：底座其实已经完成了一半

- `chat.rs`：SSE 流式客户端 + usage 解析（含 DeepSeek 缓存字段别名）
- `chat_sessions`/`chat_messages` 表 + `repo/chat.rs`
- `ChatPanel.vue` / `ChatWindow.vue` 两形态 UI（含独立窗与主题跟随）
- `AiProviders.vue` + keyring：供应商/模型管理与凭据存取
- `xhub_api.rs` 的 `CAPABILITIES`：一张现成的「能力注册表」范式，可直接平移为「模型可见工具表」

**要新增的其实只有四件**：① tool calling 循环（解析 `tool_calls` → 执行 → 回填 → 续跑）；② 模型可见工具表（包装宿主函数 + JSON Schema）；③ 时间线 UI（工具调用卡片/审批）；④ `chat_messages` 扩列以存工具调用与结果。

**不需要**：bash/文件沙箱/git 集成/多 agent 编排/上下文压缩——这些是 coding agent 的刚需，不是效率工作台的。多步规划与上下文管理可在工具规模增长后再逐步补。

### 四、若确需现成框架

Rust 生态的 crate 形态是唯一能「真正编译进安装包」的现成方案，如 [rig](https://rig.rs/)（Rust agent 框架）——体积增量约数 MB，无运行时依赖。但对当前工具集规模（十几个宿主函数），手写循环更可控；**以 crate 引入框架属可选加速项，非必需**。

## 模型接入现状（对照）

**服务端 `x-hub-server` 的 M0 已交付**（Hono + better-sqlite3，Docker 部署）：

- `POST /v1/chat/completions`：**OpenAI 兼容 + SSE 流式透传**，body 全量转发；请求前原子预扣额度、失败自动回补、落 `usage_logs`（含 error 记录）
- 账号体系：邀请码注册（仅 admin 可签发）、每人 **100 次一次性额度**、64 位随机 token（库只存 sha256）、单设备在线、登录/注册失败限流
- 并发闸：`AI_PER_USER_ACTIVE=2`、`AI_TOTAL_ACTIVE=200`，超限 429；`AI_ALLOWED_MODELS` 模型白名单
- 管理后台：用户管理（调额度/停用/重置 token）、邀请码签发、用量日志、操作审计

**客户端 x-hub 现状：零接入**。仓库内无 quota/token/注册相关代码，`chat.rs` 的 `base_url` 只来自用户自行配置的 `chat_models`（`config.rs` 默认预置 DeepSeek 官方）。**即：缺的是客户端接入，不是服务端能力。**

## 实施前提

- **模型接入：服务端已就绪，客户端待接**（详见下方「模型接入现状」）。
- **额度口径必须先改，否则 agent 上线即烧光用户额度**：服务端现为**按请求次数**扣（`routes/ai.ts` 的 `UPDATE quotas SET used = used + 1`，默认 total=100）。而 agent loop 一次用户输入会产生**多次** `/v1/chat/completions`（每轮工具调用一次）——简单任务 2–3 次、复杂任务 5–10 次，即「100 次额度」实际只等于 **10–30 次真实对话**。建议改为 **turn 粒度计费**：客户端为一次用户输入生成 turn id 并随请求带上，服务端对同一 turn 只扣一次；或改为按 token 计量（需解析上游 usage）。
- **上游模型须支持 tool calling**：中转是 body 全量透传（`JSON.stringify(body)`），`tools` 参数能过；但 `AI_ALLOWED_MODELS` 白名单里必须只放**支持 function calling** 的模型，否则模型会忽略 tools 或直接报错，表现为「agent 不干活」或死循环。
- **流式 `tool_calls` 需分片累积**：SSE 里 `delta.tool_calls[].function.arguments` 是**逐段拼接**的，现有 `chat.rs` 只解析 `delta.content`，必须新增按 index 累积参数分片的逻辑（这是实现中最易踩的一处）。
- **成本熔断缺失**：服务端有并发闸（`AI_PER_USER_ACTIVE`/`AI_TOTAL_ACTIVE`）但无「总额度/总 token 预算上限」，上游账单存在失控风险，建议补日/月预算熔断。
- **数据模型要扩**：`chat_messages` 仅 `role`/`content` 两列，存不下工具调用、结果与审批记录，否则会话重开后时间线残缺。
- **UI 是宿主自己的活**：工具调用卡片、折叠、diff、审批按钮，现有 `ChatPanel` 只渲染 markdown 流。
- **审批策略**：宿主工具多数是写操作（建笔记、改待办），需定策略（每次确认 / 可授权白名单 / 只读模式），并保证「AI 改了什么是可回看的」。
- **工具表的权限口径**：`CAPABILITIES` 是**扩展**的权限模型（manifest 声明 + 用户授权）；模型可见工具需要**另一套**口径（用户对 AI 的授权，而非对某个扩展的授权），两者不可混用，需明确区分。

## 落点（顺序）

1. **定模型接入体验**（前置，阻塞性）——决定普通用户能否用上这个底座。
2. **Rust 侧实现 tool calling 循环**：扩 `chat.rs` 或新增 `agent/` 模块，解析 OpenAI 兼容 `tool_calls`、串行执行宿主工具、回填 `role: tool` 消息后续跑；流式事件复用现有 Channel 机制。
3. **建「模型可见工具表」**：把 5–8 个高频宿主能力（新建笔记 / 查待办 / 加待办 / 记速记 / 搜资源 / 读剪贴板…）包装成带 JSON Schema 的工具，逐条挂在注册表上。
4. **时间线 UI + 扩表**：前端渲染工具调用卡片与审批交互，`chat_messages` 增列（或 JSON 列）持久化。
5. **（可选，长期）外部引擎挂载**：给高级用户在设置里留「接入外部 ACP 引擎」的口子，走二稿已论证的 ACP client 设计，**不随包分发**。
