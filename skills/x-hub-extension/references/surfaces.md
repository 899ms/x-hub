# 形态（surface）与工作台模块多形态

> **何时读我**：决定扩展以什么形态呈现、或写 `module` 入口时。

## 四种形态怎么选

| 形态 | 场景 | 容器 |
|---|---|---|
| `module` | 工作台常驻摘要卡 | 宿主工作台网格的一个格子，**宿主已给它套玻璃卡**，扩展页面保持透明 |
| `view` | 完整工具页 | 主工作区（侧栏导航进入） |
| `window` | 并排参考 / 需要独立空间 | 独立浮窗 |
| `drawer` | 轻量速查 / 边看边用 | 右滑面板；宿主抽屉容器在壁纸态已是真实取景模糊 + 玻璃底 |

- **`window` / `drawer` 与 `view` 共用入口**：`entry.window` 直接指向 `./view/index.html` 即可，不必复制一份页面。
- `module` 卡片格不大，按紧凑卡设计（大数字 + 一两行说明 + 可选一个按钮）。
- 一个扩展可以声明多个形态，共用 `manifest.id` 与同一份 `storage`。

## 工作台模块多形态（variant）

声明了 `manifest.moduleVariants` 的扩展，**module 入口在同一份 HTML 里按「当前形态」渲染不同内容**。取形态有三个通道：

### 1. URL query（首帧）

```js
const variant = new URLSearchParams(location.search).get('xhub-variant')
```

入口加载时即可用（宿主动态拼接在 iframe src 上）。

### 2. CSS / DOM 属性（推荐，纯 CSS 分支）

宿主把当前形态写到 `<html>` 的 `data-xhub-variant="<id>"` 属性和 `--xhub-variant` CSS 变量，切形态实时更新：

```css
:root[data-xhub-variant="compact"] .month-grid { display: none; }
:root[data-xhub-variant="month"] .today { display: block; }
```

⚠️ 分支只能靠**属性选择器**。CSS 里没有 `var(--xhub-variant) === 'compact' ? … : …` 这种写法——那是在 JS 里才成立的条件表达式。

### 3. 事件（运行时切换，不重载）

用户在工作台编辑器点 ⇄ 切形态时，宿主广播 `xhub:variant-changed`，扩展订阅后自行重绘：

```js
window.xhub.events.on('xhub:variant-changed', (variant) => render(variant))
```

### 入口写法约定

module 入口默认按**多形态自适应**写：尺寸跟随当前格子（cq 单位或 `container-type`），形态决定内容密度/结构。

预览环境用 `?xhub-variant=<id>` 调试，`__xhubPreview.setVariant(id)` 模拟切换。
