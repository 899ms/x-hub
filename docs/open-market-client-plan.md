# 客户端实施清单：开发者模式与扩展市场开放（P1a）

> 上游权威计划见 `x-hub-server/docs/PRD-extension-ecosystem.md` §11（v3，2026-09-12）。本文档只展开**客户端侧**的工作，服务端侧见 `x-hub-server/docs/PLAN-M1-submissions.md`。
> 相关决策：`docs/adr/0005-developer-mode-local-source-mount.md`、`docs/adr/0008-extension-content-origin-isolation.md`、`docs/adr/0007-third-party-extension-trust-model.md`。
> ⚠️ 本文档在开源仓，**不得写入**审核关卡阈值、配额参数、AI 预审规则、任何私钥或服务端判定逻辑。

## 0. 前置安全（接收任何第三方扩展之前必须完成）

| # | 任务 | 落点 | 验收 |
|---|---|---|---|
| T0.1 | **资产作用域收紧**：scope 由 `$APPDATA/**` 与整个数据根改为白名单子目录（extensions / icons / wallpapers / notes-images / clipboard-images / market），并排除数据库（含 WAL/SHM）、`app.json`、`logs/` | `src-tauri/tauri.conf.json` 的 `assetProtocol.scope`、`lib.rs` 的 `allow_directory` 调用 | 数据根下的数据库与配置文件不再可由资产协议读取 |
| T0.2 | **扩展内容独立协议**：新增 `xhub-ext://`（参照既有 `xhub-note` 协议）；扩展入口与其相对资源由该协议服务，扩展 origin 与用户数据不同源 | `lib.rs` 注册协议、`extension.rs::read_extension_entry`（不再依赖 `.xhpack` 写盘与 `convertFileSrc`）、`composables/useExtensionFrame.ts` 的 URL 构造 | 扩展入口、相对资源（JS/CSS/图片）、桥 API 全部照常；扩展 `fetch` 数据文件路径失败 |
| T0.3 | **安全探针扩展 + 人工回归清单**：探针尝试 `fetch` 数据根下的数据库与配置并显示结果（数据根绝对路径需手填——页面里拿不到 `%APPDATA%`）；回归清单逐项过：图标 / 壁纸 / 剪贴板图片 / 扩展入口 / 扩展 service / 笔记图片 | 探针在 `tests/extensions/security-probe/`（**不发布到市场**） | 探针必须报「读取失败」；回归清单全部正常 |

**为什么 T0.1 不能靠探针验证**：跨源 `fetch` 会被 CORS 挡下（asset 协议不返回 CORS 头），"作用域拒绝"与"跨源拒绝"在扩展侧表现相同、无法区分。所以 T0.1 的保证来自**代码级约束**（白名单子目录 + 配置里不再有 `$APPDATA/**`）+ 回归清单（确认白名单没漏掉该放行的目录）；T0.2 的保证来自探针（扩展 origin 已不是 `asset.localhost`，读不到任何宿主数据）。

**顺序约束**：T0.1 可立即做；T0.2 是开放第三方上架的硬前置。

### 0.1 实施进度（2026-09-12）

- ✅ **T0.1 已完成**：`tauri.conf.json` 的 `assetProtocol.scope` 由 `["$APPDATA/**"]` 改为 `[]`；`lib.rs` 启动时改为按白名单子目录动态放行（`extensions` / `icons` / `wallpapers` / `clipboard/images`），并在放行前 `create_dir_all`（`allow_directory` 会额外注册 canonicalize 后的模式变体，目录不存在则匹配不上）。数据库、`app.json`、`logs/` 从此不在作用域内。
- ✅ **T0.2 已完成**：新增 `src-tauri/src/ext_protocol.rs` 与 `xhub-ext` 自定义协议。URL 形态 `http://xhub-ext.localhost/<扩展 id>/<相对路径>`；入口 HTML 由协议处理器**动态注入桥脚本**（不再写 `<扩展目录>/.xhpack/<surface>.html`，开发目录因此不会被宿主写脏）；安全校验 = 扩展 id 形状白名单 + 相对路径逐段 percent 解码后禁止 `..`/`\`/`:`/NUL/控制字符 + `canonicalize` 后必须仍在扩展目录内（防符号链接逃逸）；响应一律 `Cache-Control: no-store`。`read_extension_entry` 改为返回该协议 URL，前端 `useExtensionFrame` 不再自拼 asset 地址。
- ✅ **T2.1–T2.5 账号登录已完成**：设置页新增「账号」分区（~~服务器地址~~、GitHub 设备码登录、邮箱验证码登录、额度展示、兑换邀请码、申请成为开发者、退出登录）；Rust 侧 `account.rs` 与 `chat.rs` 同口径存 token（钥匙串优先、文件回退），启动时异步校验会话（401 即静默清理）。**服务端的 GitHub Device Flow 与邮箱验证码端点已就绪**（不需要 client_secret，也不需要 deep link 回调）。**后续变更（v0.5.6）**：平台域名 `http://x-hub.xfactor.top` 正式启用，服务端地址改为内置常量 `config::DEFAULT_SERVER_URL`，设置里的「服务器地址」入口已移除（理由见 AGENTS.md 约定 52）。
- ✅ **T2.7 多设备管理已完成**：设置 → 账号 → 「我的设备」列出在线设备（名称 + 最近使用时间）并可单独撤销；同账号最多 5 台同时在线，**换机不会被顶下线**。
- ⏳ **待做**：COS 实现替换本地存储（可选，本地实现已能跑完整链路）。
- ✅ **T2.6 平台 AI 中转已完成**：设置「AI 助手」新增「使用平台免费额度」卡片 —— 登录后一键把平台模型加进本地配置（`base_url` 指向账号服务器、Key 用占位符 `__xhub_platform__`，调用时由 `chat.rs::resolve_api_key` 换成账号会话 token）。**自备 Key 的供应商完全不受影响，也不需要登录**（R18 的取舍）。
- ⏳ **T0.3 待人工执行**：探针扩展已就位（`tests/extensions/security-probe/`），需在实机（`npm run tauri:dev`）跑一次并记录结果。**注意**：启动调试实例前必须先退出托盘里正在运行的正式版 x-hub（单实例机制会拦掉第二个实例，日志表现为「启动后无任何新记录」）。
  - 操作完之后可以跑 **`pwsh -File scripts/verify-runtime.ps1`**：它只读日志，逐项核对「扩展是否走 xhub-ext 独立协议」「本机源码目录是否重放」「资产作用域有无放行失败」「有没有 ERROR」，把后端层面的事从"目测"变成"有据可查"（界面观感仍需人工看）。
- ✅ **T1.x 本机源码目录直挂已完成**（后端 + 前端 + 热重载 + 扩展中心「我的扩展」标签页）。**v0.6.x 修订：取消「开发者模式」开关，登记即加载**（原文见 §1 的实施状态说明与 ADR 0005「修订」段）。
- ✅ **T3.1 客户端打包已完成**：`pack_extension_archive` 命令（内部 `pack_dir_to_archive` 可单测），产 `.xhpack`、manifest 在包根、排除 `node_modules` 与 `.` 开头项，并返回 sha256/size 供上传比对。
- ✅ **T3.2–T3.5 发布链路已完成**：扩展中心给「开发中」的扩展加「发布」按钮 → `ExtensionPublishDialog`（更新说明 / 宿主最低版本 / 主页 + 剩余配额）→ `dev_submit`（**在临时目录打包，绝不写开发者的源码目录**；multipart 上传）→ 展示服务端返回的**关卡逐项结论**（未过项标红并给出人话原因）；弹窗下方是「我的提交」列表（状态、被拒理由、撤回）。客户端**不内置任何审核规则**，只展示服务端结论（红线：只问不判）。
- ✅ **T4.1 安装告知已完成**：市场详情弹窗新增「服务型扩展」风险提示与权限分级清单（高危项标红），权限来源为清单的 `permissions` 字段（服务端发布时从 manifest 写入）。
- ✅ **T4.2 撤销消费已完成**：`MarketStatus.revoked` 贯通到前端；已装版本命中撤销 → 扩展中心「已下架」红标 + 说明（**只警示，绝不静默卸载或禁用**）；被撤销的目标版本不提供更新入口。
- 🟡 **T4.3 部分完成**：市场卡片与详情弹窗已展示**发布者**（`MarketExtension.author`，服务端清单已有该字段；老清单缺字段时降级不显示）；**「验证状态」尚未实现** —— 清单结构与服务端下发里都没有 verified 类字段，做之前需先在 PRD/服务端清单里定字段语义。
- ⏳ **剩下没做的**：T0.3 人工复核（见上）、T4.3 的验证状态、COS 实现替换本地存储（可选）。

## 1. 开发者模式（ADR 0005）

> **实施状态（2026-09-17 按代码同步）**：T1.1–T1.10 均已落地，与初稿表格的差异有三处 ——
> ① **没有「开发者模式」开关**（v0.6.x 修订：`dev_mode_enabled` 与 `set_dev_mode_enabled` 已删除/废弃，登记即加载；理由见 ADR 0005「修订」段）；
> ② 目录增删 UI 不在「设置 → 扩展」，而在**扩展中心「我的扩展」标签页**（第三个标签），设置里只留一行指路文案；因此 T1.8 验收里的「开关关闭时入口全部隐藏」已不适用；
> ③ 配置字段由 `Vec<{path, enabled}>` 简化为 `Vec<String>`（`dev_extensions`），目录即条目、没有 enabled 位。

| # | 任务 | 落点 | 验收 |
|---|---|---|---|
| T1.1 | 配置项：`AppConfig.dev_extensions: Vec<{path, enabled}>`（serde default，老配置天然兼容） | `src-tauri/src/config.rs` | 老配置文件不受影响 |
| T1.2 | 命令：`register_dev_extension(path)` / `unregister_dev_extension(path)` / `list_dev_extensions()` | `src-tauri/src/extension.rs` + `lib.rs` 注册 | 注册前 `create_dir_all`（否则资产作用域匹配不到）；返回可读的错误而非静默失败 |
| T1.3 | 运行期放开作用域：`app.asset_protocol_scope().allow_directory(path, true)`；注销时 `forbid_directory` | 同上 | 注册后立即可加载；注销后不可加载 |
| T1.4 | 启动重放：setup 中按 `dev_extensions` 逐个重新放开 | `src-tauri/src/lib.rs`（现有 data_root allow 处） | 重启后开发扩展仍可用 |
| T1.5 | 扫描合并：`scan_extensions` 把开发目录并入结果，`ExtensionEntry` 增 `source: "installed" \| "dev"` 字段 | `extension.rs` | 前端能区分两类；开发扩展不参与市场更新与卸载 |
| T1.6 | 变更检测：开发目录按「全目录 FNV + mtime」计算戳（现有 `extensions_stamp` 只扫 manifest，需另口径），前端轮询 | `extension.rs` + `stores/workbench.ts` | 改 HTML/CSS/JS 即触发重载；改 manifest 触发列表刷新 |
| T1.7 | 热重载：戳变化 → 重新生成入口 → 重设 iframe `src` 并附加 `?t=<mtime>` 破缓存 | `composables/useExtensionFrame.ts` | 保存文件后 ≤1s 自动重载；重载不破坏宿主 UI |
| T1.8 | 设置 UI：「开发者模式」开关 + 目录管理（添加/移除/打开所在文件夹/手动重载） | `components/SettingsView.vue`（扩展分区） | 开关关闭时入口全部隐藏，不影响普通用户 |
| T1.9 | 扩展中心：开发扩展单独分区，标「开发中」，提供「打包 / 发布」「移除」动作 | `components/ExtensionCenter.vue` | 已装清单与开发清单视觉可分、动作不串 |
| T1.10 | service 支持：开发扩展的 service 后端按开发目录解析路径并启动 | `src-tauri/src/service.rs`、`runtime.rs` | 开发目录下的 service 扩展能起后端并走既有代理 |

**注意**：开发扩展的 id 可能与已装扩展冲突 → 冲突时以「已装」为准并给出明确提示，不允许静默覆盖。

## 2. 登录与账号

| # | 任务 | 落点 | 验收 |
|---|---|---|---|
| T2.1 | 登录 UI（「设置 → 账号」区）：登录方式入口、当前账号与额度展示、退出登录 | `SettingsView.vue` 新增分区 | 未登录时功能入口置灰并说明原因 |
| T2.2 | 回调通道：系统浏览器打开授权页 → 回调唤起客户端。优先用既有 `single-instance` 拿命令行参数；不足时再引入 `tauri-plugin-deep-linking` | `src-tauri/src/lib.rs` | 授权完成后客户端自动完成登录，无需手动复制粘贴 |
| T2.3 | token 存储：复用 `chat.rs` 的系统钥匙串机制（**不明文落盘、界面脱敏**） | `src-tauri/src/`（新模块，如 `account.rs`） | keyring 内可见、配置文件内不可见 |
| T2.4 | GitHub OAuth（PKCE，无 client secret） | 客户端 + 服务端配合 | 端到端可用 |
| T2.5 | 邮箱验证码登录（兜底） | 同上 | 端到端可用；错误提示可读 |
| T2.6 | 平台 AI 中转接入：作为 chat 的一个 provider 接入；**自备 Key 的路径完全不变、无需登录** | `src-tauri/src/chat.rs`、`components/AiProviders.vue`、`ChatPanel.vue` | 未登录 + 自备 Key 可用；已登录 + 无 Key 可用平台额度；额度耗尽有明确提示 |
| T2.7 | 多设备：客户端按「每设备一个 token」工作（服务端配合改造，见服务端清单） | `account.rs` | 台式机与笔记本可同时在线 |

## 3. 发布链路（客户端为唯一入口）

| # | 任务 | 落点 | 验收 |
|---|---|---|---|
| T3.1 | 客户端打包：对当前开发目录 zip + 校验 manifest + 排除 `node_modules`（与 `pack.mjs` 口径一致） | `src-tauri/src/extension.rs`（新命令） | 打包产物与服务端预期结构一致（manifest 在包根） |
| T3.2 | 发布表单：版本号（语义化校验）、更新说明、宿主最低版本、主页、权限确认、用途自述 | `components/`（新组件，如 `ExtensionPublishDialog.vue`） | 缺必填项不能提交；展示将要申请的权限 |
| T3.3 | 上传：multipart + 进度 + 失败重试；大包给出体积提示 | 同上 + `api/tauri.ts` | 网络中断可重试且不产生重复提交 |
| T3.4 | 「我的扩展」列表：状态（草稿/审核中/已上架/被拒）、**被拒理由**、撤回提交 | `ExtensionCenter.vue` 新分区 | 状态与后台一致；被拒理由可读 |
| T3.5 | 开发者申请入口：**仅登录后可见**；填理由提交；显示申请状态 | `SettingsView.vue` / `ExtensionCenter.vue` | 未登录不可见（不给未登录用户看到死入口） |
| T3.6 | **红线**：客户端不内置任何服务端判定逻辑；配额只显示「剩余次数」；驳回理由只展示服务端给的人话文案 | 全局约束 | 代码审查项 |

## 4. 撤销、告知与呈现

| # | 任务 | 落点 | 验收 |
|---|---|---|---|
| T4.1 | 安装确认弹窗显式告知：web 扩展按权限分级展示；**service 扩展必须明说「可读写你的文件并连接网络」** | `MarketDetailDialog.vue` / `ExtensionSettingsDialog.vue` | 文案直白，不用行话 |
| T4.2 | 消费 `revoked`：清单刷新时比对已装扩展 → 红色警示 + 停止自动更新；**不静默卸载、不静默禁用** | `stores/workbench.ts`、`ExtensionCenter.vue` | 撤销后用户看到明确解释与手动卸载入口 |
| T4.3 | 市场卡片展示发布者与验证状态（registry 追加字段） | `ExtensionCenter.vue` | 老清单（无新字段）正常降级显示 |

## 5. 发版顺序（硬约束）

1. **必须先于任何第三方上架发版**：T0.1 + T0.2（安全前置）、T4.2（消费 revoked）。
2. **可与之并行**：T1.x（开发者模式）、T2.x（登录）。
3. **P1b 服务端就绪后**：T3.x（发布链路）、T2.6（平台 AI 中转，取决于服务端额度链路是否对外开放）。

## 6. 明确不做（本轮）

- ❌ 网页上传入口（发布只在客户端）
- ❌ 设备指纹/设备码（防滥用靠邀请码与限流，不靠设备识别）
- ❌ 系统级沙箱与 Job Object 资源限制（见 ADR 0007；service 扩展的治理靠审核、撤销与显式告知）
- ❌ 客户端埋点（已从 PRD 移出）
- ❌ 客户端内置审核规则或阈值
- ❌ 微信登录（个人主体不可行；将来具备企业/个体工商户主体后再评估）
