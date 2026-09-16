<script setup lang="ts">
import { computed, inject, ref, watch } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { ImagePlus, X } from 'lucide-vue-next'
import { useFocusTrap } from '../composables/useFocusTrap'
import {
  isTauri,
  tauriApi,
  type DevSubmissionRow,
  type ExtensionEntry,
  type PrecheckResult,
  type SubmitResult,
} from '../api/tauri'

// 发布扩展：打包(客户端) → 上传(平台) → 展示服务端返回的关卡逐项结论。
// 客户端不内置任何审核规则，只负责"把包送上去 + 把结论如实展示出来"（红线：客户端只问不判）。
const props = defineProps<{ extension: ExtensionEntry | null }>()
const emit = defineEmits<{ close: [] }>()

const visible = computed(() => !!props.extension)
const cardRef = ref<HTMLElement | null>(null)
const showToast = inject<(msg: string) => void>('showToast', () => {})
useFocusTrap(visible, cardRef)

const changelog = ref('')
const homepage = ref('')
const minAppVersion = ref('')
const submitting = ref(false)
const result = ref<SubmitResult | null>(null)
const errorText = ref('')

const submissions = ref<DevSubmissionRow[]>([])
const listLoading = ref(false)
const quota = ref<{ drafts_remaining?: number; published_remaining?: number; daily_submits_remaining?: number } | null>(null)

const STATUS_TEXT: Record<string, string> = {
  uploaded: '已上传',
  gate_failed: '关卡未过',
  pending_review: '待审核',
  approved: '已通过',
  rejected: '已驳回',
  published: '已上架',
  withdrawn: '已撤回',
}

function fmtTime(ms: number): string {
  const d = new Date(ms)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

async function loadSubmissions() {
  if (!isTauri()) return
  listLoading.value = true
  try {
    const r = await tauriApi.devListSubmissions(1, 20)
    submissions.value = r.submissions ?? []
    quota.value = r.quota ?? null
  } catch {
    // 未登录 / 未配置服务器时静默：表单区会给出提示
  } finally {
    listLoading.value = false
  }
}

// 本地预检：让作者在点发布之前就发现问题（口径与服务端关卡一致，但不作为放行依据）
const precheck = ref<PrecheckResult | null>(null)
const prechecking = ref(false)

async function runPrecheck() {
  const ext = props.extension
  if (!ext || !isTauri()) return
  prechecking.value = true
  try {
    precheck.value = await tauriApi.precheckExtension(ext.id)
  } catch {
    precheck.value = null
  } finally {
    prechecking.value = false
  }
}

// ---- 截图（展示物料）：让用户一眼看出扩展是干嘛的 ----
// 数量在这里限制（最多 5 张）；**大小与真实类型由服务端把关**（按文件头判，不信扩展名）。
const MAX_SHOTS = 5
const screenshots = ref<string[]>([])
/** path → data URL（作者选的图不在资产白名单目录，只能读成 base64 预览） */
const shotPreviews = ref<Record<string, string>>({})

async function pickScreenshots() {
  if (!isTauri()) return
  try {
    const picked = await open({
      multiple: true,
      directory: false,
      filters: [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'webp'] }],
    })
    const paths = Array.isArray(picked) ? picked : typeof picked === 'string' ? [picked] : []
    if (!paths.length) return
    const room = MAX_SHOTS - screenshots.value.length
    if (room <= 0) {
      showToast(`最多 ${MAX_SHOTS} 张截图`)
      return
    }
    if (paths.length > room) showToast(`最多 ${MAX_SHOTS} 张，只取前 ${room} 张`)
    for (const p of paths.slice(0, room)) {
      if (screenshots.value.includes(p)) continue
      screenshots.value.push(p)
      try {
        shotPreviews.value[p] = await tauriApi.readImageDataUrl(p)
      } catch (e) {
        showToast(`该图无法预览：${e}`)
      }
    }
  } catch (e) {
    showToast(String(e))
  }
}

function removeShot(path: string) {
  screenshots.value = screenshots.value.filter((p) => p !== path)
  delete shotPreviews.value[path]
}

async function submit() {
  const ext = props.extension
  if (!ext) return
  if (!isTauri()) return
  submitting.value = true
  result.value = null
  errorText.value = ''
  try {
    result.value = await tauriApi.devSubmit(
      ext.id,
      changelog.value.trim(),
      minAppVersion.value.trim(),
      homepage.value.trim(),
      screenshots.value,
    )
    quota.value = result.value.quota ?? quota.value
    await loadSubmissions()
  } catch (e) {
    errorText.value = String(e)
  } finally {
    submitting.value = false
  }
}

async function withdraw(row: DevSubmissionRow) {
  try {
    await tauriApi.devWithdrawSubmission(row.id)
    await loadSubmissions()
  } catch (e) {
    errorText.value = String(e)
  }
}

// 展开某条历史提交的关卡逐项结论（服务端已返回，之前界面没接）
const detailId = ref<number | null>(null)
const detailItems = ref<{ id: string; label: string; ok: boolean; detail?: string | null }[]>([])
const detailLoading = ref(false)

async function toggleDetail(row: DevSubmissionRow) {
  if (detailId.value === row.id) {
    detailId.value = null
    return
  }
  detailId.value = row.id
  detailItems.value = []
  detailLoading.value = true
  try {
    const r = await tauriApi.devGetSubmission(row.id)
    detailItems.value = r.submission.gate_report ?? []
  } catch (e) {
    errorText.value = String(e)
  } finally {
    detailLoading.value = false
  }
}

watch(visible, (v) => {
  if (v) {
    result.value = null
    errorText.value = ''
    changelog.value = ''
    homepage.value = ''
    minAppVersion.value = ''
    precheck.value = null
    void runPrecheck()
    void loadSubmissions()
  }
})
</script>

<template>
  <Teleport to="body">
    <div v-if="visible" class="modal-mask" @click.self="emit('close')">
      <div ref="cardRef" class="modal-card pub-card" role="dialog" aria-label="发布扩展" aria-modal="true">
        <div class="pub-head">
          <div>
            <h2 class="dialog-title">发布「{{ extension?.name }}」</h2>
            <p class="pub-sub">
              {{ extension?.id }} · v{{ extension?.version }} ·
              {{ extension?.source === 'dev' ? '开发中（源码直挂）' : '已安装' }}
            </p>
          </div>
          <button class="icon-btn" type="button" title="关闭" aria-label="关闭" @click="emit('close')">
            ✕
          </button>
        </div>

        <div class="pub-body">
          <p class="pub-hint">
            打包在本机完成（不含 <code>node_modules</code> 与隐藏文件），上传后由平台跑关卡与人工审核。
            版本号取自扩展的 manifest。
          </p>

          <!-- 本地预检：只列需要修的问题（ok 项不刷屏），服务端关卡才是最终结论 -->
          <div v-if="prechecking" class="pub-precheck-loading">正在本地预检…</div>
          <div v-else-if="precheck" class="pub-precheck" :class="{ clean: precheck.clean }">
            <div class="pub-precheck-head">
              {{ precheck.clean ? '本地预检通过（服务端关卡仍是最终结论）' : '本地预检发现需要修的问题' }}
            </div>
            <ul>
              <li
                v-for="i in precheck.items.filter((x) => x.level !== 'ok')"
                :key="i.label"
                :class="i.level"
              >
                <span>{{ i.label }}</span>
                <span v-if="i.detail" class="pub-precheck-detail">{{ i.detail }}</span>
              </li>
              <li v-if="precheck.clean" class="ok">没有发现问题</li>
            </ul>
          </div>

          <label class="pub-field">
            <span>更新说明</span>
            <textarea v-model="changelog" rows="3" placeholder="这一版改了什么（会展示给用户）"></textarea>
          </label>

          <div class="pub-row">
            <label class="pub-field pub-field-sm">
              <span>宿主最低版本</span>
              <input v-model="minAppVersion" placeholder="如 0.5.5" />
            </label>
            <label class="pub-field pub-field-sm">
              <span>项目主页</span>
              <input v-model="homepage" placeholder="https://…" />
            </label>
          </div>

          <div class="pub-field">
            <span>截图（可选，最多 {{ MAX_SHOTS }} 张 · 单张 ≤ 2MB）</span>
            <div class="pub-shots">
              <div v-for="p in screenshots" :key="p" class="pub-shot">
                <img v-if="shotPreviews[p]" :src="shotPreviews[p]" :alt="p" />
                <span v-else class="pub-shot-pending">无法预览</span>
                <button class="pub-shot-del" type="button" title="移除这张" @click="removeShot(p)">
                  <X :size="12" :stroke-width="2.2" />
                </button>
              </div>
              <button
                v-if="screenshots.length < MAX_SHOTS"
                class="pub-shot-add"
                type="button"
                @click="pickScreenshots"
              >
                <ImagePlus :size="18" :stroke-width="1.8" />
                <span>添加图片</span>
              </button>
            </div>
            <p class="pub-hint">
              截图会展示在扩展详情页 —— 建议 1~3 张：主界面 + 典型用法，用户一眼就知道这扩展能干什么
            </p>
          </div>

          <div v-if="quota" class="pub-quota">
            剩余配额：待处理 {{ quota.drafts_remaining ?? '—' }} · 今日可提交
            {{ quota.daily_submits_remaining ?? '—' }} · 在架可新增 {{ quota.published_remaining ?? '—' }}
          </div>

          <div v-if="errorText" class="pub-error">{{ errorText }}</div>

          <div v-if="result" class="pub-result" :class="{ ok: result.gatePassed }">
            <div class="pub-result-head">
              {{
                result.gatePassed
                  ? '已提交，等待人工审核'
                  : '机器关卡未通过，请按下面提示修改后重新发布'
              }}
            </div>
            <ul class="pub-gate">
              <li v-for="g in result.gateItems" :key="g.id" :class="{ bad: !g.ok }">
                <span class="pub-gate-label">{{ g.label }}</span>
                <span v-if="g.detail" class="pub-gate-detail">{{ g.detail }}</span>
              </li>
            </ul>
          </div>

          <div class="pub-list">
            <div class="pub-list-head">
              <span>我的提交</span>
              <button class="ghost-btn" type="button" :disabled="listLoading" @click="loadSubmissions">
                刷新
              </button>
            </div>
            <p v-if="!submissions.length" class="pub-empty">还没有提交记录</p>
            <div v-for="s in submissions" :key="s.id" class="pub-item">
              <div class="pub-item-main">
                <span class="pub-item-title">{{ s.ext_id }} <b>v{{ s.version }}</b></span>
                <span class="pub-item-meta">
                  <span class="pub-tag" :class="`st-${s.status}`">{{ STATUS_TEXT[s.status] ?? s.status }}</span>
                  {{ fmtTime(s.created_at) }}
                </span>
                <span v-if="s.review_note" class="pub-item-note">驳回原因：{{ s.review_note }}</span>
                <ul v-if="detailId === s.id" class="pub-item-gate">
                  <li v-if="detailLoading">加载中…</li>
                  <li
                    v-for="g in detailItems"
                    :key="g.id"
                    :class="{ bad: !g.ok }"
                  >
                    <span>{{ g.label }}</span>
                    <span v-if="g.detail" class="pub-gate-detail">{{ g.detail }}</span>
                  </li>
                </ul>
              </div>
              <div class="pub-item-actions">
                <button class="ghost-btn" type="button" @click="toggleDetail(s)">
                  {{ detailId === s.id ? '收起' : '详情' }}
                </button>
                <button
                  v-if="['uploaded', 'gate_failed', 'pending_review'].includes(s.status)"
                  class="ghost-btn"
                  type="button"
                  @click="withdraw(s)"
                >
                  撤回
                </button>
              </div>
            </div>
          </div>
        </div>

        <div class="pub-foot">
          <button class="ghost-btn" type="button" @click="emit('close')">关闭</button>
          <button class="pill-btn" type="button" :disabled="submitting" @click="submit">
            {{ submitting ? '正在打包上传…' : '打包并发布' }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.pub-card {
  width: min(620px, 92vw);
  max-height: 86vh;
  display: flex;
  flex-direction: column;
}
.pub-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 18px 20px 12px;
}
.pub-sub {
  margin: 3px 0 0;
  font-size: 0.75rem;
  color: var(--text-3);
}
.pub-body {
  padding: 0 20px;
  overflow: auto;
  flex: 1;
}
.pub-hint {
  margin: 0 0 12px;
  font-size: 0.75rem;
  line-height: 1.7;
  color: var(--text-3);
}
.pub-hint code {
  font-size: 0.72rem;
  padding: 1px 4px;
  border-radius: 4px;
  background: var(--bg-card-soft);
}
.pub-field {
  display: block;
  margin-bottom: 10px;
}
.pub-field > span {
  display: block;
  margin-bottom: 4px;
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-2);
}
.pub-field textarea,
.pub-field input {
  width: 100%;
  box-sizing: border-box;
  padding: 7px 9px;
  font-size: 0.78rem;
  line-height: 1.6;
  border: 1px solid var(--border-soft);
  border-radius: 8px;
  background: var(--bg-card-soft);
  color: var(--text-1);
  font-family: inherit;
  resize: vertical;
}
.pub-row {
  display: flex;
  gap: 10px;
}
.pub-field-sm {
  flex: 1;
  min-width: 0;
}
.pub-quota {
  margin: 4px 0 10px;
  font-size: 0.72rem;
  color: var(--text-3);
}
.pub-precheck-loading {
  margin: 4px 0 10px;
  font-size: 0.74rem;
  color: var(--text-3);
}
.pub-precheck {
  margin: 4px 0 12px;
  padding: 9px 11px;
  border-radius: 10px;
  background: var(--c-orange-soft);
  color: var(--c-orange-ink);
}
.pub-precheck.clean {
  background: var(--c-green-soft);
  color: var(--c-green-ink);
}
.pub-precheck-head {
  font-size: 0.76rem;
  font-weight: 600;
  margin-bottom: 4px;
}
.pub-precheck ul {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.pub-precheck li {
  font-size: 0.73rem;
  line-height: 1.5;
}
.pub-precheck li.error {
  color: var(--c-red-ink);
}
.pub-precheck-detail {
  display: block;
  font-size: 0.7rem;
  opacity: 0.85;
}
.pub-error {
  margin: 6px 0 10px;
  padding: 8px 10px;
  border-radius: 8px;
  font-size: 0.75rem;
  background: var(--c-red-soft);
  color: var(--c-red-ink);
}
.pub-result {
  margin: 6px 0 12px;
  padding: 10px 12px;
  border-radius: 10px;
  background: var(--bg-card-soft);
}
.pub-result.ok {
  background: var(--c-green-soft);
}
.pub-result-head {
  font-size: 0.78rem;
  font-weight: 600;
  margin-bottom: 6px;
}
.pub-gate {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.pub-gate li {
  position: relative;
  padding-left: 16px;
  font-size: 0.74rem;
  color: var(--text-2);
}
.pub-gate li::before {
  content: '✓';
  position: absolute;
  left: 0;
  color: var(--c-green-ink);
}
.pub-gate li.bad {
  color: var(--c-red-ink);
}
.pub-gate li.bad::before {
  content: '✕';
  color: var(--c-red-ink);
}
.pub-gate-detail {
  display: block;
  font-size: 0.7rem;
  color: var(--text-3);
}
.pub-list {
  margin: 8px 0 4px;
}
.pub-list-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 0.78rem;
  font-weight: 600;
  color: var(--text-2);
  margin-bottom: 6px;
}
.pub-empty {
  margin: 0;
  font-size: 0.75rem;
  color: var(--text-3);
}
.pub-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 0;
  border-top: 1px solid var(--border-soft);
}
.pub-item-main {
  flex: 1;
  min-width: 0;
}
.pub-item-title {
  display: block;
  font-size: 0.76rem;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pub-item-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 0.7rem;
  color: var(--text-3);
}
.pub-item-note {
  display: block;
  margin-top: 2px;
  font-size: 0.72rem;
  color: var(--c-red-ink);
}
.pub-item-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}
.pub-item-gate {
  margin: 6px 0 2px;
  padding: 8px 10px;
  list-style: none;
  border-radius: 8px;
  background: var(--bg-card-soft);
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.pub-item-gate li {
  position: relative;
  padding-left: 15px;
  font-size: 0.72rem;
  line-height: 1.5;
  color: var(--text-2);
}
.pub-item-gate li::before {
  content: '✓';
  position: absolute;
  left: 0;
  color: var(--c-green-ink);
}
.pub-item-gate li.bad {
  color: var(--c-red-ink);
}
.pub-item-gate li.bad::before {
  content: '✕';
  color: var(--c-red-ink);
}
.pub-tag {
  padding: 0 6px;
  border-radius: 999px;
  background: var(--bg-card-soft);
  font-weight: 600;
}
.pub-tag.st-published {
  background: var(--c-green-soft);
  color: var(--c-green-ink);
}
/* 截图：缩略图条 + 虚线「添加图片」块 */
.pub-shots {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.pub-shot {
  position: relative;
  width: 128px;
  height: 76px;
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-sm);
  overflow: hidden;
  background: var(--bg-card-soft);
}
.pub-shot img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.pub-shot-pending {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  font-size: 0.68rem;
  color: var(--text-3);
}
.pub-shot-del {
  position: absolute;
  top: 4px;
  right: 4px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: var(--scrim);
  color: #fff;
  cursor: pointer;
}
.pub-shot-del:hover {
  background: var(--c-red-ink);
}
.pub-shot-add {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  width: 128px;
  height: 76px;
  border: 1px dashed var(--border-soft);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  font-size: 0.7rem;
  cursor: pointer;
  transition: color 150ms ease-out, border-color 150ms ease-out;
}
.pub-shot-add:hover {
  color: var(--brand-500);
  border-color: var(--brand-500);
}
.pub-hint {
  margin: 6px 0 0;
  font-size: 0.68rem;
  line-height: 1.5;
  color: var(--text-3);
}
.pub-tag.st-pending_review {
  background: var(--c-orange-soft);
  color: var(--c-orange-ink);
}
.pub-tag.st-gate_failed,
.pub-tag.st-rejected {
  background: var(--c-red-soft);
  color: var(--c-red-ink);
}
.pub-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 14px 20px 18px;
}
</style>
