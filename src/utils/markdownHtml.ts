import DOMPurify from 'dompurify'
import { marked } from 'marked'

/**
 * 轻量 Markdown → 安全 HTML（待办描述等**只读展示**用）。
 *
 * 与 `utils/markdown.ts`（纯正则的纯文本化，零依赖）分开两个模块：那边被笔记列表 /
 * 全局搜索等主路径静态引用，这里带 marked + DOMPurify 两个库，混在一起会让它们
 * 被拖进主包与浮窗包；调用方按需动态 import 本模块（见 TodoRow 的悬浮展示）。
 *
 * ⚠️ 必须过 DOMPurify：描述可能来自扩展桥（**不可信内容**），marked 默认不过滤内联 HTML，
 * 而主窗口没有 CSP —— 直接 v-html 时，内容里的 `<img onerror>` / `<svg onload>`
 * 就能执行任意脚本并调用应用的本地命令（同 ChatPanel 对模型输出的处理）。
 *
 * 结果按原文缓存：悬浮展示会反复渲染同一条描述，避免每次鼠标移动都重解析。
 */
const cache = new Map<string, string>()
const CACHE_MAX = 50

export function renderMarkdown(text: string): string {
  if (!text) return ''
  const hit = cache.get(text)
  if (hit != null) return hit
  // breaks: 单个换行即换行（描述多是手写多行文本，不按 Markdown 段落规则合并）
  const html = DOMPurify.sanitize(
    marked.parse(text, { async: false, breaks: true, gfm: true }) as string,
  )
  if (cache.size >= CACHE_MAX) cache.clear()
  cache.set(text, html)
  return html
}
