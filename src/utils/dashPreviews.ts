/**
 * 布局编辑器内的模块「样式化预览」渲染器。
 *
 * - clock / weather 在编辑器画布中挂载真实组件（所见即所得），其余模块 + 所有形态切换浮层的
 *   缩略图用这里的静态 HTML（cq 单位，随格子/缩略框比例伸缩），判断「铺没铺满」足够诚实。
 * - 类名统一 `dp-*`，样式由 DashboardLayoutEditor 的 scoped CSS（:deep）提供。
 */

import { dashVariantDef } from '../composables/useDashboardLayout'

const WEEK = ['日', '一', '二', '三', '四', '五', '六']

/** HTML 转义：扩展 id / 形态名等来自 manifest 的任意字符串必须先转义再拼 HTML，
 *  否则经 v-html 渲染进宿主主窗即成 XSS（扩展 iframe 的权限模型被绕过） */
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function monthCells(): string {
  const now = new Date()
  const y = now.getFullYear()
  const m = now.getMonth()
  const first = new Date(y, m, 1).getDay()
  const days = new Date(y, m + 1, 0).getDate()
  const prevDays = new Date(y, m, 0).getDate()
  const cells: string[] = []
  for (const wd of WEEK) cells.push(`<i class="dp-m-wd">${wd}</i>`)
  for (let i = first - 1; i >= 0; i--) cells.push(`<i class="dp-m-other">${prevDays - i}</i>`)
  for (let d = 1; d <= days; d++) {
    cells.push(`<i${d === now.getDate() ? ' class="dp-m-today"' : ''}>${d}</i>`)
  }
  while (cells.length % 7 !== 0) cells.push('<i class="dp-m-other">·</i>')
  return cells.join('')
}

function clockPreview(variant: string): string {
  switch (variant) {
    case 'lunar':
      return `<div class="dp-clock-lunar">
        <div class="dp-l-time">14:08</div>
        <div class="dp-l-solar">2026年9月6日 周日</div>
        <div class="dp-l-lunar">农历 七月廿五</div>
        <hr />
        <div class="dp-l-extra">丙午年 · 属马</div>
      </div>`
    case 'month':
      return `<div class="dp-clock-month">
        <div class="dp-m-head">2026年9月</div>
        <div class="dp-m-grid">${monthCells()}</div>
      </div>`
    case 'minimal':
      return `<div class="dp-clock-mini">14:08<span>36</span></div>`
    default:
      return `<div class="dp-clock-big">
        <div class="dp-c-time">14:08:36</div>
        <div class="dp-c-date">2026年9月6日 · 周日</div>
        <div class="dp-c-wx">⛅ 28° 多云 · 上海</div>
        <div class="dp-c-quote">星光不问赶路人</div>
      </div>`
  }
}

function weatherPreview(variant: string): string {
  if (variant === 'detail') {
    return `<div class="dp-w-detail">
      <div class="dp-wd-head"><b>28°</b><span>多云</span><i>上海</i></div>
      <div class="dp-wd-grid">
        <i><b>29°</b><span>体感</span></i>
        <i><b>62%</b><span>湿度</span></i>
        <i><b>3级</b><span>东南风</span></i>
        <i><b>中等</b><span>紫外线</span></i>
      </div>
    </div>`
  }
  return `<div class="dp-w-now"><span class="dp-w-ic">⛅</span><b>28°</b><span>上海</span></div>`
}

function sysmonPreview(): string {
  return `<div class="dp-card">
    <div class="dp-title">系统资源</div>
    <div class="dp-bar"><span>CPU</span><i><b style="width:34%"></b></i><em>34%</em></div>
    <div class="dp-bar"><span>内存</span><i><b style="width:58%"></b></i><em>58%</em></div>
  </div>`
}

function stickyPreview(): string {
  return `<div class="dp-card">
    <div class="dp-title">便签</div>
    <div class="dp-lines"><i></i><i style="width:86%"></i><i style="width:64%"></i></div>
  </div>`
}

function notesPreview(): string {
  return `<div class="dp-card">
    <div class="dp-title">速记概览</div>
    <div class="dp-list">
      <div class="dp-row"><b>周会纪要</b><span>2分钟前</span></div>
      <div class="dp-row"><b>灵感草稿</b><span>昨天</span></div>
      <div class="dp-row"><b>读书笔记</b><span>3天前</span></div>
    </div>
  </div>`
}

function todoOverviewPreview(): string {
  return `<div class="dp-card">
    <div class="dp-ring">3/5</div>
    <div class="dp-txt"><b>今日待办</b><span>还剩 2 项</span></div>
  </div>`
}

function resourcesPreview(): string {
  return `<div class="dp-card">
    <div class="dp-title">速达数量</div>
    <div class="dp-chips"><i>应用 24</i><i>网页 8</i><i>文件 12</i></div>
  </div>`
}

function countdownPreview(): string {
  return `<div class="dp-card">
    <div class="dp-title">倒计时</div>
    <div class="dp-bar"><span>午休</span><i><b style="width:40%"></b></i><em>12:04</em></div>
    <div class="dp-bar"><span>番茄钟</span><i><b style="width:72%"></b></i><em>07:30</em></div>
    <div class="dp-add">＋ 新建倒计时</div>
  </div>`
}

function promptsPreview(): string {
  return `<div class="dp-card">
    <div class="dp-title">提示词</div>
    <div class="dp-list">
      <div class="dp-row"><i class="dp-dot"></i><b>周报模板</b></div>
      <div class="dp-row"><i class="dp-dot"></i><b>翻译助手</b></div>
      <div class="dp-row"><i class="dp-dot"></i><b>代码审查</b></div>
    </div>
  </div>`
}

function todoPreview(): string {
  return `<div class="dp-card">
    <div class="dp-title">待办</div>
    <div class="dp-list">
      <div class="dp-row"><i class="dp-dot red"></i><b>提交周报</b><span>今天</span></div>
      <div class="dp-row"><i class="dp-dot"></i><b>回复邮件</b><span>今天</span></div>
      <div class="dp-row"><i class="dp-dot blue"></i><b>健身 1 小时</b><span>明天</span></div>
      <div class="dp-row"><i class="dp-dot"></i><b>读书 30 分钟</b><span>稍后</span></div>
    </div>
  </div>`
}

function recentPreview(): string {
  return `<div class="dp-card">
    <div class="dp-title">最近使用</div>
    <div class="dp-chips">
      <i>🐦 微信</i><i>🎨 设计稿</i><i>📝 便签</i><i>📁 项目</i><i>🌐 浏览器</i>
    </div>
  </div>`
}

function extPreview(name: string, variant?: string): string {
  const n = escapeHtml(name)
  const v = variant ? ` · ${escapeHtml(variant)}` : ''
  return `<div class="dp-card">
    <div class="dp-title">${n}${v}</div>
    <div class="dp-lines"><i></i><i style="width:70%"></i></div>
    <div class="dp-ext-name">${n}</div>
  </div>`
}

/** 模块+形态 → 静态预览 HTML（编辑器中非 live 模块的画布预览 + 形态浮层缩略图共用） */
export function dashPreviewHtml(modId: string, variant?: string): string {
  const v = variant ?? ''
  if (modId.startsWith('ext:')) {
    const vd = dashVariantDef(modId, variant)
    return extPreview(dashModuleName(modId), vd?.name)
  }
  switch (modId) {
    case 'clock':
      return clockPreview(v)
    case 'weather':
      return weatherPreview(v)
    case 'sysmon':
      return sysmonPreview()
    case 'sticky1':
    case 'sticky2':
      return stickyPreview()
    case 'notes':
      return notesPreview()
    case 'todo_overview':
      return todoOverviewPreview()
    case 'resources':
      return resourcesPreview()
    case 'countdown':
      return countdownPreview()
    case 'prompts':
      return promptsPreview()
    case 'todo':
      return todoPreview()
    case 'recent':
      return recentPreview()
    default:
      return `<div class="dp-card"><div class="dp-title">${escapeHtml(dashModuleName(modId))}</div><div class="dp-lines"><i></i><i></i></div></div>`
  }
}

/** 编辑器内对 clock / weather 挂载真实组件（所见即所得）；其余模块用静态样式预览 */
export function isLivePreview(modId: string): boolean {
  return modId === 'clock' || modId === 'weather'
}

function dashModuleName(id: string): string {
  if (id.startsWith('ext:')) return id.slice('ext:'.length)
  const names: Record<string, string> = {
    clock: '时钟',
    weather: '天气',
    sysmon: '系统资源',
    sticky1: '便签 1',
    sticky2: '便签 2',
    notes: '速记概览',
    todo_overview: '待办概览',
    resources: '速达数量',
    countdown: '倒计时',
    prompts: '提示词',
    todo: '待办',
    recent: '最近使用',
  }
  return names[id] ?? id
}
