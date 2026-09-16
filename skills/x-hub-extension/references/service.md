# service 后端写法（仅 runtime: "service"）

> **何时读我**：扩展需要后端 / 调 AI / 调平台 API / 用原生能力时。

## 最小后端

零依赖 Node，监听宿主注入的 `PORT`：

```js
// service/index.js
const http = require('http')
const port = parseInt(process.env.PORT || '0', 10)
const server = http.createServer((req, res) => {
  res.setHeader('Content-Type', 'application/json; charset=utf-8')
  if (req.url === '/healthz') { res.end(JSON.stringify({ ok: true })); return }
  res.end(JSON.stringify({ message: 'hi', extId: process.env.XHUB_EXT_ID }))
})
server.listen(port, '127.0.0.1', () => console.log('listening', server.address().port))
```

## 铁律

1. **只监听 `127.0.0.1`**，端口读 `process.env.PORT`（宿主动态分配，`manifest.backend.port` 写 `0`）。
   - `manifest.backend.host` 缺省就是回环，**保持缺省**。
   - 真要对外开服务（`0.0.0.0` / 局域网地址）**必须**声明 `network` 权限，否则宿主直接拒绝启动后端；这类扩展用户会被要求授权，非必要别做。宿主还会把实际监听主机经 `XHUB_LISTEN_HOST` 环境变量传给后端。
2. **前端调后端走 `window.xhub.service.request('/api/x')`**，**不要**直接 fetch 端口——宿主持 `/svc/<extId>/*` 反向代理统一解决 CORS。
3. **后端只带代码、不依赖本地 `node_modules`**（运行时由宿主管理）。确需自带完整运行时的重型应用是例外，需额外说明。
4. 提供 `health` 路径（如 `/healthz`）便于宿主探活。

> 后端自己发外部 HTTP 请求（抓取、调第三方 API）**不需要任何权限**——那是 Node 进程自己的事，权限系统只管扩展 ↔ 宿主的桥调用与对外监听。

## 前端调用

```js
const res = await window.xhub.service.request('/api/x', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ q: 'hello' }),
})
const data = await res.json()   // 也可 res.status / res.headers / res.text()
```

`runtime.info()` 返回的 `serviceReady` / `proxyPrefix` 可用于探测后端就绪状态；后端未起来时 `service.request` 会失败，界面要给得出人话的提示而不是白屏。

## 部署注意

部署 service 扩展前，若 x-hub 正在运行并锁定了该扩展的后端文件，`deploy` 会报 **EPERM**——先退出 x-hub 再部署。
