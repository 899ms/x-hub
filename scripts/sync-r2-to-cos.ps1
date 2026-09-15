# sync-r2-to-cos.ps1 — 过渡期阶段 0（一次性）：把 R2 存量（extensions + releases）拉平到腾讯云 COS
#
# ⛔ R2 已于 2026-09-15 停用（对象已清空并备份到 E:\workspace\_x-hub-r2-backup，恢复步骤见该目录
#    的 README-恢复说明.md）。本脚本如今**极易误伤**：它用 `rclone sync`（会删除目标端多余对象），
#    而 R2 现在是空的 —— 直接跑会把 COS 上的扩展市场与升级包一起清空。因此已加硬闸：
#    源端为空时直接拒绝执行。需要重新启用 R2 时，先按备份说明书把它恢复出来。
#
# 详见 docs/self-hosted-distribution.md §6 阶段 0。旧版本包必须留在 COS 上，各版本客户端都依赖旧路径。
# 用法:
#   .\scripts\sync-r2-to-cos.ps1
# 环境变量:
#   R2_ACCOUNT_ID / R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY（源，从 GitHub Secrets 抄到本地）
#   COS_SECRET_ID / COS_SECRET_KEY / COS_BUCKET(含 APPID 后缀) / COS_REGION（目标）
# 依赖: rclone

param(
  # —— 源：R2 ——
  [string]$AccountId       = $env:R2_ACCOUNT_ID,
  [string]$AccessKeyId     = $env:R2_ACCESS_KEY_ID,
  [string]$SecretAccessKey = $env:R2_SECRET_ACCESS_KEY,
  [string]$Bucket          = "x-hub-dist",
  # —— 目标：COS ——
  [string]$CosSecretId  = $env:COS_SECRET_ID,
  [string]$CosSecretKey = $env:COS_SECRET_KEY,
  [string]$CosBucket    = $env:COS_BUCKET,
  [string]$CosRegion    = $env:COS_REGION
)

$ErrorActionPreference = "Stop"

if (-not $AccountId -or -not $AccessKeyId -or -not $SecretAccessKey) {
  Write-Error "缺少 R2 凭据（R2_ACCOUNT_ID / R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY）。"
}
if (-not $CosSecretId -or -not $CosSecretKey -or -not $CosBucket -or -not $CosRegion) {
  Write-Error "缺少 COS 参数（COS_SECRET_ID / COS_SECRET_KEY / COS_BUCKET / COS_REGION）。"
}

if (-not (Get-Command rclone -ErrorAction SilentlyContinue)) {
  Write-Error "未找到 rclone。请先安装: winget install --id Rclone.Rclone"
}

# --- 源：R2 ---
$env:RCLONE_CONFIG_R2_TYPE              = "s3"
$env:RCLONE_CONFIG_R2_PROVIDER          = "Cloudflare"
$env:RCLONE_CONFIG_R2_ACCESS_KEY_ID     = $AccessKeyId
$env:RCLONE_CONFIG_R2_SECRET_ACCESS_KEY = $SecretAccessKey
$env:RCLONE_CONFIG_R2_ENDPOINT          = "https://$AccountId.r2.cloudflarestorage.com"

# --- 目标：腾讯云 COS ---
$env:RCLONE_CONFIG_COS_TYPE              = "s3"
$env:RCLONE_CONFIG_COS_PROVIDER          = "TencentCOS"
$env:RCLONE_CONFIG_COS_ACCESS_KEY_ID     = $CosSecretId
$env:RCLONE_CONFIG_COS_SECRET_ACCESS_KEY = $CosSecretKey
$env:RCLONE_CONFIG_COS_ENDPOINT          = "cos.$CosRegion.myqcloud.com"

Write-Host "[1/3] 校验两端连接..."
rclone lsd "r2:$Bucket"
if ($LASTEXITCODE -ne 0) { Write-Error "R2 连接失败，请检查凭据。" }
rclone lsd "cos:$CosBucket"
if ($LASTEXITCODE -ne 0) { Write-Error "COS 连接失败，请检查桶名（含 APPID 后缀）/地域/密钥。" }

# --- 硬闸：源端为空时拒绝执行 ---
# rclone sync 会删除目标端多余对象；R2 若为空（已停用/未恢复），同步等于把 COS 清空。
$srcCount = @(rclone lsf "r2:$Bucket" --recursive 2>$null | Where-Object { $_ -notmatch '/$' }).Count
if ($srcCount -eq 0) {
  Write-Error @"
源端 R2 桶为空，已拒绝执行：这条同步会用空内容覆盖 COS（rclone sync 会删除目标端多余对象）。

  R2 已于 2026-09-15 停用，对象备份在 E:\workspace\_x-hub-r2-backup
  （恢复步骤见该目录的 README-恢复说明.md；先把备份推回 R2，再跑本脚本）
"@
}
Write-Host "      源端对象数 = $srcCount"

Write-Host "[2/3] 同步 extensions/ ..."
rclone sync "r2:$Bucket/extensions" "cos:$CosBucket/extensions" -P
if ($LASTEXITCODE -ne 0) { Write-Error "extensions 同步失败。" }
Write-Host "[3/3] 同步 releases/ ..."
rclone sync "r2:$Bucket/releases" "cos:$CosBucket/releases" -P
if ($LASTEXITCODE -ne 0) { Write-Error "releases 同步失败。" }

Write-Host ""
Write-Host "拉平完成 ✔ 建议抽查（清单/206/缓存头，见 docs/self-hosted-distribution.md §3）：" -ForegroundColor Green
Write-Host "  curl -sI https://$CosBucket.cos.$CosRegion.myqcloud.com/extensions/registry.json"
Write-Host "  curl -sI -H 'Range: bytes=0-99' https://$CosBucket.cos.$CosRegion.myqcloud.com/releases/update.json"
