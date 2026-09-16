# 易错点清单

> **何时读我**：写代码前扫一眼，交付前逐条对一遍。每条都是实机踩过的。

1. **命名**：是 extension 不是 plugin / 插件；`id` 必须是**你自己的**反向域名（`com.你的域名.<短名>`）。**`com.x-hub.*` 是平台保留命名空间**（只给官方自营），第三方用了会在服务端关卡被拒——而本地预检只提示不拦，容易一路拖到上传才发现。
2. **`entry` 是 HTML**，不是 `.js` 文件。
3. **npm script 用 `deploy`**，不要叫 `install`（npm 生命周期钩子会误触发）。
4. **service 端口**：读 `process.env.PORT`，只监听 `127.0.0.1`；`/healthz` 是健康检查路径。
5. **前端调后端**：走 `window.xhub.service.request`，不直接 fetch 端口。
6. **桥 API 现状**以 `runtime.info().capabilities` 为准。已实现：`runtime(info/open/callExtension)`、`storage.*`、`config.*`、`sharedStorage.*`、**`data.*` 读写全套**、`fs.saveText/saveFile/saveAs`、`service.request`、`theme.get`、`events.on/emit`、`expose`。未实现：`clipboard.*`、`net.*`、`system.*`、`ui.*`、`fs.readText/writeText/readDir/exists`。
7. **权限声明**：读宿主数据要 `data:read`、写数据要 `data:write`、存文件到下载目录要 `fs`、广播事件要 `events`、跨扩展共享存储要 `shared-storage`；没声明就调用会被拒（`PERMISSION_DENIED`）。
8. **页面底用 `var(--xhub-page-bg, transparent)`**（无壁纸=宿主页面背景，有壁纸=transparent）；内容表面用 `var(--xhub-surface)`；**切勿用 `--xhub-bg-page` 铺底**（壁纸态会盖住壁纸，透底态是白底白字）；更细的壁纸态适配用 `data-xhub-wallpaper` / `data-xhub-wallpaper-clear` / `data-xhub-immersive`。
9. **部署 service 扩展前**：若 x-hub 正在运行并锁定了该扩展的后端文件，`deploy` 会报 EPERM——先退出 x-hub 再部署。
10. **字段名大小写**：`openIn`、`minSize`、`dependsOn`、`backend.engine.minVersion` 是驼峰，写错会解析失败。
11. **`requires` 用 `namespace.method`**：写宿主桥 API 能力名（如 `data.notes.list`），不是权限名（`data:read`）；写错会在扩展中心标「缺能力」。
12. **不要用 CDN**：Tailwind / Font Awesome / 外链字体在宿主 webview 里可能被 CSP 或离线拦掉，入口资源全部本地化。
13. **深浅双主题要双声明 fallback**：`:root` 浅色兜底 + `:root[data-xhub-theme="dark"]` 深色兜底，否则无宿主预览时深色露馅；强调色家族的派生 token（primary / soft / text）最容易漏。
14. **`xhub.storage` 异步且跨形态共享**：先读回再首绘；同扩展 module / view / window / drawer 共用同一份，键名自行保证唯一（如 `'progress'`、`'notes'`）。
15. **`window` / `drawer` 与 view 共用入口**：`entry.window` 直接指向 `./view/index.html` 即可，不必复制一份页面。
16. **`--xhub-bg-page` 是渐变，不是颜色**：`color: var(--xhub-bg-page)` 属无效声明，会静默退化成本身继承色（「看着碰巧对」，换个主题就错）。强调色实底上的文字色按主题显式给（亮色主题的 accent 是深色 → 白字；暗色主题 accent 是浅色 → 深字）。
17. **`data.*` 的 `update` 是全量覆盖**：可省字段会被写成空值（`notes.update` 不传 `content` 清空正文）。改前先 `get` 合并再提交；`todos` / `stickies` 的并发写用 `expectedVersion` 乐观锁，冲突会 reject `VERSION_CONFLICT`。
18. **CSS 里没有条件表达式**：module 多形态分支只能用属性选择器 `:root[data-xhub-variant="compact"] { … }`，不能写 `var(--xhub-variant) === 'compact' ? … : …`。
