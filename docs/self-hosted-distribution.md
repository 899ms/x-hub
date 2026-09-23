# x-hub 分发端点迁移方案：腾讯云 COS 替代 Cloudflare R2

> ⚠️ **扩展发布入口已统一到服务端（2026-09）**：正式上架走 x-hub 客户端「扩展中心 → 发布」，
> 由 x-hub-server 审核台完成关卡 + 人工审核 + 服务端签名 + 推送 COS。
> 本文描述的本机脚本通道（`publish-extension.ps1` / `upload-market.ps1` 推 `dist-market`）与
> CI（`release-extension.yml`）**均已停用**，仅作分发端点拓扑与应急参考。
> 面向扩展作者的正式文档见 `x-hub-extensions/docs/guide/publishing.md`。

> 状态：**已切换完成 —— COS 是唯一「有效」的分发通道**。R2 桶本身还在、`r2.dckxx.com` 也仍在服务
> `releases/`（`-Target all` 今天还会往里写一份，校验也能 200 通过），但 **`extensions/` 已被清空**：
> 指向 `r2.dckxx.com/extensions/…` 的市场源一律 **404** —— 这正是客户端「市场源异常：拉取市场清单失败：
> HTTP 404 Not Found」的来源（`dist.x-hub.dev` 同样不可用）。§6 的三阶段切换停在阶段 1，
> 阶段 3 的 302 兜底**未采用**。
> 内置 endpoint 自 v0.5.1（commit `c9c203c`）起指向 COS；老 `app.json` 里残留的停用域名在
> **配置装载时一次性迁移**（`config.rs::migrate_retired_endpoints`，`market_endpoint` 与
> `update_endpoint` 都覆盖），所以升级到含该逻辑的版本后自动自愈，不需要用户手工改配置。
> **v0.6.1 又往前走了一步（2026-09-17）**：客户端**不再直连 COS**，也不再有 `market_endpoint` /
> `update_endpoint` 这两个配置项 —— 清单/包/截图/升级包一律走平台服务端接口
> （`https://x-hub.xfactor.top/api/v1/market/registry`、`/api/v1/app/update`，由 x-hub-server 的
> `src/modules/market` 代理 COS，见该仓 `docs/DEPLOY-market-serving.md`）。两个字段降级为**兼容占位**、
> 读到即被 `config.rs::migrate_legacy_endpoints` 归一为服务端地址并落盘；`DEFAULT_SERVER_URL` 同时切 https。
> 客户端（`market.rs` / `updater.rs`）依旧只认一个 URL + 内嵌 Ed25519 公钥，地址改为按服务端常量拼。
> 前置方案文档：`docs/r2-distribution-and-updater.md`（R2 时代的目录布局与签名约定，本文完全沿用；R2 相关部分已作废）。

## 1. 为什么 COS 优于自建 Nginx（当前处境下）

| 关注点 | 自建 Nginx（服务器） | 腾讯云 COS（采用） |
|---|---|---|
| 域名/HTTPS | 域名备案中，只能 `http://IP:8080` 明文 | **默认域名 `*.cos.<region>.myqcloud.com` 是腾讯已备案域名，自带 HTTPS，立即可用** |
| 运维 | Nginx + deploy 账号 + 防火墙/安全组 | 零运维 |
| 上传通道 | rclone sftp（CI 得走 rsync） | rclone s3（provider TencentCOS），与 R2 时代同构 |
| Range 断点续传 | 静态文件原生 206 | COS 支持 206 |
| 国内速度 | 取决于服务器带宽 | 腾讯云机房，国内快 |
| 费用 | 服务器带宽（已含在服务器里） | 无免费下行流量（约 0.5 元/GB 国内）+ 存储 ~0.1 元/GB·月；个人分发规模月成本通常在个位数元 |

安全性不依赖传输层：清单 Ed25519 分离签名 + 包体 sha256（由签名清单背书），没有私钥的中间人最多造成不可用，无法篡改。

## 2. COS 一次性准备

1. **开桶**：控制台 → 对象存储 COS → 创建桶。地域选离用户近的（如 `ap-guangzhou` / `ap-shanghai`）；桶名形如 `x-hub-dist-1251402600`（**含 APPID 后缀**，rclone/环境变量都要用完整名）。
2. **权限**：桶 → 权限管理 → **公有读私有写**（分发物本来就是公开的）。
3. **CAM 子账号密钥**（不要用主账号密钥）：
   - 访问管理 CAM → 用户 → 用户列表 → 新建用户 → 自定义创建 → 选「可访问资源并接收消息」；
   - 访问方式**只勾「编程访问」**；创建成功后**立即下载 CSV**（`SecretKey` 仅此时显示一次）；
   - 权限两档任选：
     - 速通：预置策略 `QcloudCOSDataFullControl`（官方预设，零配置；作用账号下全部 COS 桶）；
     - 最小化（推荐，锁死单桶）：新建自定义策略 → **按策略语法创建**。action 用服务级通配 `cos:*`（官方子账号授权示例的标准写法，避免逐个枚举 action 踩命名坑——注意 `cos:ListBucket` 是 AWS S3 命名，腾讯云不存在；列对象是 `cos:GetBucket`），安全边界靠 resource 锁桶：
       ```json
       {
         "version": "2.0",
         "statement": [
           {
             "effect": "allow",
             "action": [
               "cos:*"
             ],
             "resource": [
               "qcs::cos:ap-guangzhou:uid/1251402600:x-hub-dist-1251402600",
               "qcs::cos:ap-guangzhou:uid/1251402600:x-hub-dist-1251402600/*"
             ]
           }
         ]
       }
       ```
       （两条 resource 缺一不可：不带 `/*` 的授权桶级操作【列出对象，rclone lsd 依赖】，带 `/*` 的授权对象读写；策略生成器的分字段表单表达不了前者，请走策略语法模式。）
4. 目录布局与原 R2 桶一致（`extensions/…`、`releases/…`），由上传脚本自动维护，无需手动建。

## 3. 部署后验证（curl 三连）

```bash
BASE=https://x-hub-dist-1251402600.cos.ap-guangzhou.myqcloud.com

# ① 清单可访问且不缓存
curl -sI $BASE/extensions/registry.json     | grep -iE 'HTTP|cache-control'   # 200 + no-cache
# ② Range 断点续传（关键！updater 依赖 206）
curl -sI -H 'Range: bytes=0-99' $BASE/releases/update.json | grep -iE 'HTTP|content-range'  # 206 + content-range
# ③ 包体长缓存
curl -sI $BASE/extensions/registry.json.sig | grep -iE 'HTTP|cache-control'   # 200 + immutable
```

## 4. 本地上传通道（Windows 开发机）

环境变量（用户级持久化，`[Environment]::SetEnvironmentVariable(名,值,'User')`）：

| 变量 | 示例 | 用途 |
|---|---|---|
| `COS_SECRET_ID` / `COS_SECRET_KEY` | CAM 子账号密钥 | COS 通道（主） |
| `COS_BUCKET` / `COS_REGION` | `x-hub-dist-1251402600` / `ap-guangzhou` | COS 通道（主） |
| `XHUB_DIST_BASE_URL` | `https://x-hub-dist-1251402600.cos.ap-guangzhou.myqcloud.com` | 发布脚本拼清单内 URL（publish-*.ps1） |
| `XHUB_SIGNING_KEY` | 私钥文件路径（由使用者在本机设置，**不在此记录**） | Ed25519 签名私钥 |
| `R2_ACCOUNT_ID` / `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` | — | **已作废**（R2 的 `extensions/` 已清空 → 市场侧 404；`releases/` 侧虽仍写得进去但没人再读） |

用法（`-Target cos` 是默认且**现在唯一有效**的通道；`-Target sftp` 属备选 Nginx 方案（不启用）。R2 通道与其 `all` 双写模式已于 2026-09 **从脚本里摘除**——`-Target r2` / `-Target all` 现在会被参数校验直接拒绝，不再有「写了没人读、还静默不报错」的坑）：

```powershell
# 应用发版（仍然有效：客户端应用发布不经过扩展市场）
./scripts/publish-release.ps1 -ExePath src-tauri\target\release\x-hub.exe -Version 0.6.0 `
  -SignKey <私钥文件> -Notes "…"
./scripts/upload-release.ps1 -Target cos         # → 只写 COS（R2 通道已从脚本摘除）

# 扩展发布 —— ⛔ 已停用（2026-09）：改走客户端「扩展中心 → 发布」，由服务端审核台签名并推送
# ./scripts/publish-extension.ps1 -ExtDir …       # 需 XHUB_ALLOW_LOCAL_PUBLISH=1 才可绕过（应急）
# ./scripts/upload-market.ps1 -Target cos         # 需 XHUB_ALLOW_MANUAL_UPLOAD=1 才可绕过（应急）
```

脚本均以 rclone remote（环境变量临时配置，不落地）上传，末尾自动做 HTTP 200 + sha256 抽查校验；`upload-release.ps1` 保留 win-x64 只留最近 2 版的清理策略。对象存储通道的缓存头随上传设置（`--header-upload`，即写对象元数据 Cache-Control）。

> 网络提示：分发走腾讯云 COS 域名，国内通常直连即可；需要走代理的机器设置环境变量 `XHUB_UPLOAD_PROXY=http://127.0.0.1:7890`（或传 `-CheckProxy`）让脚本末尾的抽查下载走代理，只影响校验、不影响上传。

## 5. CI 通道（GitHub Actions，扩展发布）

`release-extension.yml` 曾从 rsync 改回 **rclone + TencentCOS**，并依赖一批 GitHub Secrets
（COS 凭据、签名私钥、CDN 基址，以及更早的 R2 凭据与 SSH/DEPLOY 部署密钥）。

> ⚠️ **该 CI 通道已停用，上述 secret 已全部删除**（2026-09）：扩展发布改由服务端审核台完成
> 关卡 + 审核 + 服务端签名 + 推送 COS，`release-extension.yml` 现为「触发即失败」的显式记录。
> 仓库 Actions 的 secrets / variables 当前均为 **0 条**，`.github/` 下无任何 `secrets.*` 引用；
> 签名私钥只在发布侧保管，不再进入任何 CI 环境。此处不再逐条列举名称。
> 本机上传通道（§4）与应用发版链路不受影响。

## 6. 过渡方案：三阶段切换，老用户升级不断链（**历史记录**）

> 阶段 0/1 已完成（分水岭版本 = v0.5.1，commit `c9c203c`）；阶段 3 的 302 兜底**未采用** ——
> R2 直接停用，客户端侧改由「配置装载时迁移停用域名」兜底（见文首状态说明）。
> 下面保留原文，便于回溯当时的判断依据。

**原理**：老客户端能否升级，取决于它二进制里烘焙的默认 endpoint（R2 时代 = R2，v0.5.1 起 = COS）是否可达；而「清单从哪拉来」与「清单里包 URL 指向哪」互相独立——从 R2 拉到的 update.json 完全可以把包 URL 指向 COS。所以：

- R2 存活期间，老用户永远能升级，且升级目标可以是 COS 上的包；
- 「默认 endpoint 切到 COS」的那个版本是**分水岭版本**：升级过它的客户端从此走新链路；
- R2 的退役时间由「活跃用户版本 ≥ 分水岭版本」决定，与 COS 上线时间解耦，**不存在「一换就升不了级」的窗口**。

### 阶段 0：双活（**已完成**）

1. COS 就位并按 §3 验证通过（重点 **206**）。
2. **存量拉平**（旧版本包必须留在 COS，各版本客户端都依赖旧路径）：
   一次性脚本 `scripts/sync-r2-to-cos.ps1` 当时用来把 R2 存量同步到 COS；**该脚本已于 2026-09 删除**
   （它用 `rclone sync` 会删除目标端多余对象，R2 侧清空后重跑等于把 COS 清空，留着是隐患）。
   存量核对改为只读手段：`upload-market.ps1 -Target cos` 末尾的 HTTP 200 + sha256 抽查，
   以及 x-hub-server 的 `npm run verify:registry`。
3. 本机先验证：当时是把数据根下 `app.json` 的 `market_endpoint` 改成
   `https://x-hub-dist-1251402600.cos.ap-guangzhou.myqcloud.com/extensions/registry.json`，
   刷新市场能拉清单、能安装扩展。
   ⚠️ 该字段自 v0.6.1 起**已废弃不再被读取**（地址按服务端常量拼），改它不会有任何效果；
   老 `app.json` 里的残留值会被 `config.rs::migrate_legacy_endpoints` 归一为服务端接口地址并落盘。
4. **每次发版双传**（历史做法，已废弃）：当时要求 §4 的 `-Target cos` + `-Target r2` 都跑；R2 通道现已从脚本摘除，正常发版只跑 `-Target cos`。

### 阶段 1：分水岭版本

- 改 `src-tauri/src/config.rs` 的 `DEFAULT_MARKET_ENDPOINT` / `DEFAULT_UPDATE_ENDPOINT` → COS 域名，发版；
  （v0.6.1 起这两个常量已被 `market_registry_url()` / `update_manifest_url()` 取代，地址改为按
  `DEFAULT_SERVER_URL` 拼平台服务端接口；自建分发要改的是 `DEFAULT_SERVER_URL` 并自行部署服务端代理。）- 该版本照常双传：老客户端从 R2 拿到这份 update.json → 包从 COS 下载 → 升级完成 → 从此走新链路；
- 分水岭版发布后仍**保持双传**，进入观察期。

### 阶段 2：观察期（建议 ≥ 4~8 周，覆盖 2~3 个发版周期）**（已跳过）**

- 继续双传；盯 Cloudflare R2 仪表盘的 Class B（读）操作数衰减；
- 读量降到接近零 = 活跃客户端基本都过了分水岭 → 进入阶段 3。

### 阶段 3：R2 优雅退役（302 兜底，客户端已验证跟随重定向）**（未采用）**

> 实际做法：**没有**配 302 —— R2 上的市场对象（`extensions/`，含 `registry.json`）已清空，
> 残留的 `releases/` 对象没有客户端会读；兜底改在客户端侧：
> `config.rs::migrate_retired_endpoints` 在装载配置时把指向这些域名的端点改写成内置 COS 地址。

- `market.rs` / `updater.rs` 的 reqwest 客户端均未关闭重定向（默认跟随最多 10 次），因此可在 Cloudflare 给 `r2.dckxx.com` 配 **Redirect Rule**：动态重定向 `concat("https://x-hub-dist-1251402600.cos.ap-guangzhou.myqcloud.com", http.request.uri.path)`，状态码 302——清单/`.sig`/包/图标按路径通配全部覆盖；
- DNS 侧：R2 桶的自定义域绑定可解除，但保留 `r2.dckxx.com` 的 DNS 记录并开启橙云代理（占位记录即可），Redirect Rule 才能接管该主机名的请求；
- 配好后旧客户端拉清单/包都会被 302 到 COS，**完全无感**；R2 桶内对象随后可清空（只留重定向规则），零流量成本；
- 最省事的替代：直接停止双传、R2 桶保留不删——旧客户端检查更新静默失败只是收不到更新提醒，不影响使用（市场验签失败回退本地缓存）。二选一，302 更体面。

## 7. 备案完成后的可选优化（非必需）

COS 默认域名长期可用，不必动。备案下来后如想用自己的域名：COS → 默认 CDN 加速域名 / 自定义源站域名绑定 + 上传证书 → `XHUB_DIST_BASE_URL` / `CDN_BASE_URL` 换新域名 → 发一版切默认 endpoint。步骤与三阶段切换同构。

## 8. 备选：自建 Nginx 静态托管（当前不启用，留作 PLAN B）

适用场景：想彻底省流量费、或 COS 不可用时。准备材料都在：

- `scripts/server/init-server.sh` + `scripts/server/x-hub-dist.conf`：服务器一键初始化（装 nginx / 建目录与 deploy 账号 / 写站点配置 / 放行防火墙），幂等可重复执行；缓存策略（清单 no-cache、包体 immutable）在 Nginx 配置里统一管理。
- 上传走 `-Target sftp`（环境变量 `XHUB_DEPLOY_HOST` / `XHUB_DEPLOY_PORT` / `XHUB_DEPLOY_USER` / `XHUB_DEPLOY_KEY`）；本机已生成部署密钥 `~\.ssh\xhub_deploy_ed25519`（本地用）与 `~\.ssh\xhub_deploy_ci_ed25519`（CI 用）。
- 备案期间只能 `http://<IP>:8080`（80/443 对未备案域名有拦截，非标端口直连 IP 不受限）；备案后切 `listen 443 ssl` + certbot 证书。

## 9. 与 R2 方案的差异备忘

| 关注点 | R2（已弃用） | COS（现役） |
|---|---|---|
| 域名 | 自定义域 `r2.dckxx.com`（`extensions/` 已清空 → **市场 404**；`releases/` 仍能 200） | 默认域 `<bucket>.cos.<region>.myqcloud.com`（腾讯签发证书） |
| 上传 | rclone s3 provider Cloudflare | rclone s3 provider TencentCOS |
| 缓存头 | 上传时 `--header-upload` | 同左（写对象元数据） |
| 断点续传 | 边缘支持 206 | 支持 206 |
| 出口流量 | 免费 | ~0.5 元/GB（国内），个人规模月成本个位数元 |
| 签名/验签 | 不变 | 不变 |
| 清单格式 | 不变 | 不变 |
| 退役兜底 | — | ~~R2 侧 Redirect Rule 302 → COS~~ **未采用**：直接停用 + 客户端装载时迁移停用域名 |
