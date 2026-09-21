<script setup lang="ts">
import { computed, inject, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import {
  CalendarDays,
  ChevronLeft,
  ChevronRight,
  ListTodo,
  Pin,
  Plus,
  Repeat,
  X,
} from 'lucide-vue-next'
import { useStore } from '../stores/workbench'
import type { Todo, TodoOccurrence } from '../api/tauri'
import {
  VIEW_GROUP_META,
  addDays,
  calendarGrid,
  compareByOrder,
  dueBadge,
  fmtHM,
  inHorizon,
  isoKey,
  repeatLabel,
  startOfDay,
  viewGroupOf,
  type Horizon,
} from '../utils/todoSchedule'
import TodoEditDialog from './TodoEditDialog.vue'
import ConfirmDialog from './ConfirmDialog.vue'

/**
 * 待办视图：宽窗左右同屏（列表 40% + 日历 60%），窄窗切单栏 + 双维工具栏。
 * 「范围」（今天/本周/本月/全部）与「展示形态」（列表/日历）是**两个正交维度**，
 * 窄窗只是把同屏改成切换，不会出现「切到日历就丢了范围」。
 * 周期规则的权威解释在 Rust 侧，这里只渲染 `expand_todo_occurrences` 的返回结果。
 */
const store = useStore()
const showToast = inject<(msg: string, action?: { label: string; onClick: () => void }) => void>(
  'showToast',
  () => {},
)

const WIDE_MIN = 720

const horizon = ref<Horizon>('week')
const mode = ref<'list' | 'calendar'>('list')
const tagFilter = ref<number[]>([])
const editing = ref<Todo | null>(null)
const editingOpen = ref(false)
const presetDue = ref<number | null>(null)
const confirming = ref<Todo | null>(null)
const confirmKids = ref(0)
const rootRef = ref<HTMLElement | null>(null)
const narrow = ref(false)

// 日历游标（月视图用月初，周视图用周一）
const cursor = ref(new Date())
const calUnit = ref<'month' | 'week'>('month')
const occurrences = ref<TodoOccurrence[]>([])

const today = new Date()

const HORIZONS: Array<{ key: Horizon; label: string }> = [
  { key: 'today', label: '今天' },
  { key: 'week', label: '本周' },
  { key: 'month', label: '本月' },
  { key: 'all', label: '全部' },
]

const allTags = computed(() => store.state.todoTags)

/** 未完成 + 已完成分开：视图默认只看未完成（已完成走列表底部折叠区） */
const showDone = ref(false)

function hasTag(t: Todo): boolean {
  if (!tagFilter.value.length) return true
  const ids = store.todoTagIds(t.id)
  return tagFilter.value.some((id) => ids.includes(id))
}

const topTodos = computed(() =>
  store.state.todos.filter((t) => t.parent_id == null),
)

const visible = computed(() =>
  topTodos.value.filter((t) => (showDone.value ? t.done : !t.done) && hasTag(t) && (showDone.value || inHorizon(t, horizon.value, today))),
)

/** 分组：置顶 → 逾期 → 今天 → 本周 → 本月 → 以后 → 无日期（组内按手动排序/创建时间） */
const groups = computed(() => {
  const buckets: Todo[][] = VIEW_GROUP_META.map(() => [])
  for (const t of visible.value) buckets[viewGroupOf(t, today)].push(t)
  return VIEW_GROUP_META.map((meta, i) => ({
    label: meta.label,
    items: buckets[i].slice().sort(compareByOrder),
  })).filter((g) => g.items.length > 0)
})

const kidsOf = computed(() => {
  const map = new Map<number, Todo[]>()
  for (const t of store.state.todos) {
    if (t.parent_id == null) continue
    const list = map.get(t.parent_id)
    if (list) list.push(t)
    else map.set(t.parent_id, [t])
  }
  for (const list of map.values()) list.sort(compareByOrder)
  return map
})

/** 日历：真实条目按 due_at 落格；虚拟实例按命令返回的时刻落格（虚线） */
const realByDay = computed(() => {
  const map = new Map<string, Todo[]>()
  for (const t of topTodos.value) {
    if (t.done || t.due_at == null) continue
    const key = isoKey(new Date(t.due_at))
    const list = map.get(key)
    if (list) list.push(t)
    else map.set(key, [t])
  }
  return map
})

const virtualByDay = computed(() => {
  const map = new Map<string, TodoOccurrence[]>()
  for (const o of occurrences.value) {
    const key = isoKey(new Date(o.at_ms))
    const list = map.get(key)
    if (list) list.push(o)
    else map.set(key, [o])
  }
  return map
})

const titleOf = computed(() => {
  const map = new Map<number, string>()
  for (const t of store.state.todos) map.set(t.id, t.title)
  return map
})

const cells = computed(() => calendarGrid(cursor.value, today))

const monthLabel = computed(
  () => `${cursor.value.getFullYear()} 年 ${cursor.value.getMonth() + 1} 月`,
)

const weekLabel = computed(() => {
  const start = weekStart(cursor.value)
  const end = new Date(start)
  end.setDate(end.getDate() + 6)
  return `${start.getMonth() + 1}月${start.getDate()}日 – ${end.getMonth() + 1}月${end.getDate()}日`
})

function weekStart(d: Date): Date {
  const x = new Date(d)
  x.setHours(0, 0, 0, 0)
  x.setDate(x.getDate() - ((x.getDay() + 6) % 7))
  return x
}

/** 周视图显示的 7 天 */
const weekCells = computed(() => {
  const start = weekStart(cursor.value)
  return Array.from({ length: 7 }, (_, i) => {
    const d = new Date(start)
    d.setDate(d.getDate() + i)
    return { key: isoKey(d), date: d }
  })
})

async function loadOccurrences() {
  const from = calendarGrid(cursor.value, today)[0]
  const first = new Date(`${from.key}T00:00:00`)
  const to = new Date(first)
  to.setDate(to.getDate() + 42)
  occurrences.value = await store.expandTodoOccurrences(first.getTime(), to.getTime() - 1)
}

watch([cursor, calUnit], () => void loadOccurrences())

onMounted(() => {
  void loadOccurrences()
  updateNarrow()
  if (typeof window !== 'undefined') window.addEventListener('resize', updateNarrow)
})

onBeforeUnmount(() => {
  if (typeof window !== 'undefined') window.removeEventListener('resize', updateNarrow)
})

function updateNarrow() {
  const w = rootRef.value?.clientWidth ?? window.innerWidth
  narrow.value = w < WIDE_MIN
}

function shift(delta: number) {
  const d = new Date(cursor.value)
  if (calUnit.value === 'month') d.setMonth(d.getMonth() + delta, 1)
  else d.setDate(d.getDate() + delta * 7)
  cursor.value = d
}

function goToday() {
  cursor.value = new Date()
}

function openNew(dueMs: number | null = null) {
  editing.value = null
  presetDue.value = dueMs
  editingOpen.value = true
}

function openEdit(t: Todo) {
  editing.value = t
  presetDue.value = null
  editingOpen.value = true
}

/** 勾选：周期待办走「完成本轮」；有未完成子项先确认（与卡片同一套口径） */
async function toggle(t: Todo) {
  const kids = kidsOf.value.get(t.id) ?? []
  const undone = kids.filter((k) => !k.done).length
  if (!t.done && undone > 0) {
    confirming.value = t
    confirmKids.value = undone
    return
  }
  await applyToggle(t)
}

async function applyToggle(t: Todo) {
  if (!t.done && t.repeat_mode !== 'once') {
    const updated = await store.completeTodoRecurring(t.id)
    if (updated) await loadOccurrences()
    return
  }
  await store.toggleTodo(t.id)
  const kids = kidsOf.value.get(t.id) ?? []
  if (!t.done) for (const k of kids) if (!k.done) await store.toggleTodo(k.id)
}

async function onConfirm() {
  const t = confirming.value
  confirming.value = null
  if (t) await applyToggle(t)
}

async function addChild(t: Todo, title: string) {
  const name = title.trim()
  if (!name) return
  await store.createTodo(name, t.id)
}

const childInput = ref<number | null>(null)
const childText = ref('')

function submitChild(t: Todo) {
  void addChild(t, childText.value)
  childText.value = ''
  childInput.value = null
}

function toggleTagFilter(id: number) {
  const i = tagFilter.value.indexOf(id)
  if (i >= 0) tagFilter.value.splice(i, 1)
  else tagFilter.value.push(id)
}

async function removeTag(id: number) {
  await store.deleteTodoTag(id)
  tagFilter.value = tagFilter.value.filter((x) => x !== id)
}

function tagsOf(t: Todo) {
  const ids = store.todoTagIds(t.id)
  if (!ids.length) return []
  return allTags.value.filter((x) => ids.includes(x.id))
}

function badgeOf(t: Todo) {
  return t.done ? null : dueBadge(t, new Date())
}

/** 点击图钉立即取消置顶（置顶区与日期无关，取消后回到按日期分组的位置） */
function unpin(t: Todo) {
  void store.setTodoPinned(t.id, false).then(() => showToast(`「${t.title}」已取消置顶`))
}

// ---- 日历拖拽改期 ----
// 指针实现而非 HTML5 DnD：Tauri 主窗口的原生拖放拦截与 WebView 内 HTML5 拖拽互斥
// （dragstart 后收不到 dragover/drop），与卡片组内拖拽同一口径。
// 语义：把**实际条目**拖到某天 → 截止改到该天同一时分（提醒若已设，按同一偏移平移）；
// 虚拟实例（周期规则算出的未来轮次）不是落库对象，不参与拖拽。
const dragId = ref<number | null>(null)
const dropDay = ref<string | null>(null)
/** 拖拽刚结束的短暂窗口内吞掉尾随 click，避免「拖完顺带打开编辑弹层」 */
let suppressClickUntil = 0
let chipDrag: { t: Todo; x: number; y: number; active: boolean } | null = null

function onChipPointerDown(t: Todo, e: PointerEvent) {
  if (e.button !== 0) return
  chipDrag = { t, x: e.clientX, y: e.clientY, active: false }
  window.addEventListener('pointermove', onChipPointerMove)
  window.addEventListener('pointerup', onChipPointerUp)
  window.addEventListener('pointercancel', onChipPointerUp)
}

function onChipPointerMove(e: PointerEvent) {
  const d = chipDrag
  if (!d) return
  if (!d.active) {
    if (Math.hypot(e.clientX - d.x, e.clientY - d.y) < 5) return
    d.active = true
    dragId.value = d.t.id
    document.body.classList.add('todo-chip-dragging')
    document.getSelection()?.removeAllRanges()
  }
  const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null
  dropDay.value = el?.closest<HTMLElement>('[data-day]')?.dataset.day ?? null
}

function onChipPointerUp() {
  window.removeEventListener('pointermove', onChipPointerMove)
  window.removeEventListener('pointerup', onChipPointerUp)
  window.removeEventListener('pointercancel', onChipPointerUp)
  const d = chipDrag
  const day = dropDay.value
  chipDrag = null
  dragId.value = null
  dropDay.value = null
  document.body.classList.remove('todo-chip-dragging')
  if (!d || !d.active) return
  suppressClickUntil = Date.now() + 350
  if (day == null || d.t.due_at == null) return
  if (isoKey(new Date(d.t.due_at)) === day) return
  void moveToDay(d.t, day)
}

async function moveToDay(t: Todo, day: string) {
  const [y, m, d] = day.split('-').map(Number)
  const due = new Date(t.due_at as number)
  const deltaDays = Math.round(
    (new Date(y, m - 1, d).getTime() - startOfDay(due).getTime()) / 86_400_000,
  )
  due.setFullYear(y, m - 1, d)
  const remind = t.remind_at == null ? null : addDays(new Date(t.remind_at), deltaDays).getTime()
  await store.scheduleTodo(t.id, due.getTime(), remind)
  showToast(`「${t.title}」已改到 ${m} 月 ${d} 日`)
}

function onChipClick(t: Todo) {
  if (Date.now() < suppressClickUntil) return
  openEdit(t)
}

/** 虚拟实例不是落库对象：不能拖、不能点开，给一句可执行的去处 */
function onVirtualDown(e: PointerEvent) {
  if (e.button !== 0) return
  showToast('这是周期规则算出的未来实例，请到编辑弹层改周期规则')
}
</script>

<template>
  <section ref="rootRef" class="todo-view">
    <header class="tv-head">
      <div class="tv-title">
        <ListTodo :size="16" :stroke-width="2" />
        <h2>待办</h2>
      </div>
      <span class="tv-spacer"></span>
      <div class="tv-tags">
        <button
          v-for="tag in allTags"
          :key="tag.id"
          type="button"
          class="tv-tagchip"
          :class="{ on: tagFilter.includes(tag.id) }"
          @click="toggleTagFilter(tag.id)"
        >
          <i class="dot" :style="{ background: tag.color || 'var(--brand-500)' }"></i>{{ tag.name }}
          <span class="x" title="删除该标签" @click.stop="removeTag(tag.id)"><X :size="10" :stroke-width="2.5" /></span>
        </button>
      </div>
      <button type="button" class="tv-primary" @click="openNew()">
        <Plus :size="14" :stroke-width="2.2" />新建待办
      </button>
    </header>

    <!-- 双维工具栏：范围（筛选）× 展示形态（列表/日历） -->
    <div class="tv-toolbar">
      <div class="tv-seg">
        <button
          v-for="h in HORIZONS"
          :key="h.key"
          type="button"
          :class="{ on: horizon === h.key }"
          @click="horizon = h.key"
        >
          {{ h.label }}
        </button>
      </div>
      <span class="tv-spacer"></span>
      <button type="button" class="tv-ghost" :class="{ on: showDone }" @click="showDone = !showDone">
        {{ showDone ? '看未完成' : '看已完成' }}
      </button>
      <div v-if="narrow" class="tv-seg">
        <button type="button" :class="{ on: mode === 'list' }" @click="mode = 'list'">列表</button>
        <button type="button" :class="{ on: mode === 'calendar' }" @click="mode = 'calendar'">日历</button>
      </div>
    </div>

    <div class="tv-body" :class="{ narrow }">
      <!-- 列表 -->
      <div v-show="!narrow || mode === 'list'" class="tv-list">
        <p v-if="!groups.length" class="tv-empty">这个范围里没有待办</p>
        <div v-for="g in groups" :key="g.label" class="tv-group">
          <div class="tv-group-h" :class="{ pinned: g.label === '置顶', overdue: g.label === '逾期' }">
            <span>{{ g.label }}</span>
            <span class="cnt">{{ g.items.length }}</span>
            <span class="line"></span>
          </div>
          <div
            v-for="t in g.items"
            :key="t.id"
            class="tv-row"
            :class="{ pinned: t.pinned, done: t.done }"
          >
            <button
              type="button"
              class="tv-check"
              :class="{ on: t.done }"
              :aria-label="t.done ? '取消完成' : '完成'"
              @click="toggle(t)"
            ></button>
            <div class="tv-main">
              <div class="tv-line">
                <button
                  v-if="t.pinned"
                  type="button"
                  class="tv-pin"
                  title="已置顶，点击取消"
                  aria-label="取消置顶"
                  @click.stop="unpin(t)"
                >
                  <Pin :size="12" :stroke-width="2.2" />
                </button>
                <span class="tv-label" @dblclick="openEdit(t)">{{ t.title }}</span>
                <span v-if="badgeOf(t)" class="tv-badge" :class="badgeOf(t)!.kind">{{ badgeOf(t)!.text }}</span>
                <span v-if="t.repeat_mode !== 'once'" class="tv-badge repeat" :title="`已累计完成 ${t.repeat_done_count} 次`">
                  <Repeat :size="10" :stroke-width="2" />{{ repeatLabel(t) }}
                </span>
                <span v-for="tag in tagsOf(t)" :key="tag.id" class="tv-rowtag">
                  <i class="dot" :style="{ background: tag.color || 'var(--brand-500)' }"></i>{{ tag.name }}
                </span>
              </div>
              <div v-if="kidsOf.get(t.id)?.length" class="tv-kids">
                <div v-for="k in kidsOf.get(t.id)" :key="k.id" class="tv-kid">
                  <button
                    type="button"
                    class="tv-check small"
                    :class="{ on: k.done }"
                    :aria-label="k.done ? '取消完成' : '完成'"
                    @click="store.toggleTodo(k.id)"
                  ></button>
                  <span :class="{ done: k.done }">{{ k.title }}</span>
                </div>
              </div>
            </div>
            <div class="tv-actions">
              <button type="button" class="tv-icon" title="编辑" @click="openEdit(t)">
                <CalendarDays :size="13" :stroke-width="2" />
              </button>
              <button type="button" class="tv-icon" title="加子待办" @click="childInput = childInput === t.id ? null : t.id">
                <Plus :size="13" :stroke-width="2" />
              </button>
            </div>
            <div v-if="childInput === t.id" class="tv-child-input">
              <input
                v-model="childText"
                class="tv-input"
                placeholder="子待办标题，回车添加"
                @keydown.enter="submitChild(t)"
                @keydown.esc="childInput = null"
              />
            </div>
          </div>
        </div>
      </div>

      <!-- 日历 -->
      <div v-show="!narrow || mode === 'calendar'" class="tv-cal">
        <div class="tv-cal-h">
          <button type="button" class="tv-icon" aria-label="上一页" @click="shift(-1)">
            <ChevronLeft :size="14" :stroke-width="2" />
          </button>
          <span class="tv-cal-title">{{ calUnit === 'month' ? monthLabel : weekLabel }}</span>
          <button type="button" class="tv-icon" aria-label="下一页" @click="shift(1)">
            <ChevronRight :size="14" :stroke-width="2" />
          </button>
          <button type="button" class="tv-ghost" @click="goToday">今天</button>
          <span class="tv-spacer"></span>
          <div class="tv-seg">
            <button type="button" :class="{ on: calUnit === 'month' }" @click="calUnit = 'month'">月</button>
            <button type="button" :class="{ on: calUnit === 'week' }" @click="calUnit = 'week'">周</button>
          </div>
        </div>

        <div class="tv-legend">
          <span><i class="swatch solid"></i>库里的条目（拖到其他日期可改期）</span>
          <span><i class="swatch dashed"></i>周期规则算出的虚拟实例（不落库，不能拖）</span>
          <span><i class="swatch late"></i>逾期</span>
        </div>

        <div v-if="calUnit === 'month'" class="tv-grid">
          <div v-for="d in ['一', '二', '三', '四', '五', '六', '日']" :key="d" class="tv-dow">{{ d }}</div>
          <div
            v-for="c in cells"
            :key="c.key"
            class="tv-cell"
            :class="{ out: c.out, today: c.today, 'drop-on': dropDay === c.key }"
            :data-day="c.key"
            @dblclick="openNew(new Date(`${c.key}T23:59:00`).getTime())"
          >
            <span class="tv-day">{{ c.day }}</span>
            <div class="tv-chips">
              <button
                v-for="t in realByDay.get(c.key) ?? []"
                :key="'r' + t.id"
                type="button"
                class="tv-chip real"
                :class="{ late: badgeOf(t)?.kind === 'over', dragging: dragId === t.id }"
                :title="`${t.title}（拖动到其他日期可改期）`"
                @pointerdown="onChipPointerDown(t, $event)"
                @click="onChipClick(t)"
                @dblclick.stop
              >
                {{ t.title }}
              </button>
              <span
                v-for="o in virtualByDay.get(c.key) ?? []"
                :key="'v' + o.todo_id + o.at_ms"
                class="tv-chip virtual"
                :title="`${titleOf.get(o.todo_id) ?? '周期待办'}（周期规则算出的未来实例，不能拖动）`"
                @pointerdown="onVirtualDown"
              >
                {{ titleOf.get(o.todo_id) ?? '周期待办' }}
              </span>
            </div>
          </div>
        </div>

        <div v-else class="tv-week">
          <div
            v-for="c in weekCells"
            :key="c.key"
            class="tv-weekcol"
            :class="{ today: isoKey(c.date) === isoKey(today), 'drop-on': dropDay === c.key }"
            :data-day="c.key"
          >
            <div class="tv-weekhead">{{ c.date.getMonth() + 1 }}/{{ c.date.getDate() }} 周{{ ['一', '二', '三', '四', '五', '六', '日'][(c.date.getDay() + 6) % 7] }}</div>
            <div class="tv-chips">
              <button
                v-for="t in realByDay.get(c.key) ?? []"
                :key="'r' + t.id"
                type="button"
                class="tv-chip real"
                :class="{ late: badgeOf(t)?.kind === 'over', dragging: dragId === t.id }"
                :title="`${t.title}（拖动到其他日期可改期）`"
                @pointerdown="onChipPointerDown(t, $event)"
                @click="onChipClick(t)"
                @dblclick.stop
              >
                {{ t.title }}
                <i v-if="t.due_at" class="time">{{ fmtHM(t.due_at) }}</i>
              </button>
              <span
                v-for="o in virtualByDay.get(c.key) ?? []"
                :key="'v' + o.todo_id + o.at_ms"
                class="tv-chip virtual"
                :title="`${titleOf.get(o.todo_id) ?? '周期待办'}（周期规则算出的未来实例，不能拖动）`"
                @pointerdown="onVirtualDown"
              >
                {{ titleOf.get(o.todo_id) ?? '周期待办' }}
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <TodoEditDialog
      :visible="editingOpen"
      :todo="editing"
      :preset-due-ms="presetDue"
      @close="editingOpen = false"
      @saved="loadOccurrences"
    />

    <ConfirmDialog
      :visible="confirming != null"
      title="还有子待办未完成"
      :message="`「${confirming?.title ?? ''}」下还有 ${confirmKids} 条子待办未完成，是否确认完成？`"
      hint="确认后会一并勾选这些子待办；取消则本条待办保持未完成。"
      confirm-text="确认并勾选子项"
      @confirm="onConfirm"
      @cancel="confirming = null"
    />
  </section>
</template>

<style scoped>
.todo-view {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 18px 16px;
  overflow: hidden;
}
.tv-head {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.tv-title {
  display: flex;
  align-items: center;
  gap: 7px;
  color: var(--text-1);
}
.tv-title h2 {
  margin: 0;
  font-size: 0.95rem;
  font-weight: 700;
}
.tv-spacer {
  flex: 1;
}
.tv-tags {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.tv-tagchip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  font-size: 0.7rem;
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-pill);
  background: var(--bg-card-soft);
  color: var(--text-3);
  cursor: pointer;
}
.tv-tagchip.on {
  border-color: var(--brand-500);
  color: var(--text-1);
  font-weight: 600;
}
.tv-tagchip .x {
  display: inline-flex;
  opacity: 0;
  color: var(--text-4);
}
.tv-tagchip:hover .x {
  opacity: 1;
}
.dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
}
.tv-primary {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 6px 13px;
  font-size: 0.78rem;
  font-weight: 600;
  color: #fff;
  background: var(--brand-500);
  border: none;
  border-radius: var(--radius-md);
  cursor: pointer;
}
.tv-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.tv-seg {
  display: inline-flex;
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-md);
  overflow: hidden;
}
.tv-seg button {
  padding: 4px 11px;
  font-size: 0.72rem;
  border: none;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}
.tv-seg button.on {
  background: var(--brand-500);
  color: #fff;
  font-weight: 600;
}
.tv-ghost {
  padding: 4px 10px;
  font-size: 0.72rem;
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}
.tv-ghost.on {
  color: var(--text-1);
  border-color: var(--border-strong);
}
.tv-body {
  flex: 1;
  min-height: 0;
  display: flex;
  gap: 14px;
}
.tv-body.narrow {
  flex-direction: column;
}
.tv-list,
.tv-cal {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-lg);
  background: var(--bg-card-soft);
  padding: 10px 12px;
}
.tv-body:not(.narrow) .tv-list {
  flex: 0 0 40%;
}
.tv-empty {
  margin: 12px 0;
  font-size: 0.76rem;
  color: var(--text-4);
  text-align: center;
}
.tv-group {
  margin-bottom: 10px;
}
.tv-group-h {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 3px 2px 5px;
  font-size: 0.7rem;
  font-weight: 700;
  letter-spacing: 0.03em;
  color: var(--text-3);
}
.tv-group-h.pinned {
  color: var(--brand-500);
}
.tv-group-h.overdue {
  color: var(--c-red-ink);
}
.tv-group-h .cnt {
  color: var(--text-4);
}
.tv-group-h .line {
  flex: 1;
  height: 1px;
  background: var(--border-soft);
}
.tv-row {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  padding: 7px 9px;
  margin-bottom: 4px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: var(--bg-card-solid);
  flex-wrap: wrap;
}
.tv-row.pinned {
  border-color: var(--brand-500);
  background: var(--brand-50);
}
.tv-row.done .tv-label {
  color: var(--text-4);
  text-decoration: line-through;
}
.tv-check {
  flex-shrink: 0;
  width: 15px;
  height: 15px;
  margin-top: 2px;
  border-radius: 50%;
  border: 1.6px solid var(--border-strong);
  background: transparent;
  cursor: pointer;
}
.tv-check.small {
  width: 12px;
  height: 12px;
  margin-top: 1px;
}
.tv-check.on {
  background: var(--brand-500);
  border-color: var(--brand-500);
}
.tv-main {
  flex: 1;
  min-width: 0;
}
.tv-line {
  display: flex;
  align-items: center;
  gap: 7px;
  flex-wrap: wrap;
}
.tv-pin {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--brand-500);
  cursor: pointer;
  flex-shrink: 0;
}
.tv-pin:hover {
  color: var(--c-red-ink);
}
.tv-label {
  font-size: 0.8rem;
  color: var(--text-1);
  cursor: text;
}
.tv-badge {
  padding: 0 7px;
  border-radius: var(--radius-pill);
  font-size: 0.62rem;
  font-weight: 600;
  background: var(--bg-card-soft);
  color: var(--text-4);
}
.tv-badge.over {
  background: var(--c-red-soft);
  color: var(--c-red-ink);
}
.tv-badge.today {
  background: var(--c-orange-soft);
  color: var(--c-orange-ink);
}
.tv-badge.repeat {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  background: var(--c-blue-soft);
  color: var(--c-blue-ink);
}
.tv-rowtag {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 0 6px;
  border-radius: var(--radius-pill);
  border: 1px solid var(--border-soft);
  font-size: 0.62rem;
  font-weight: 600;
  color: var(--text-3);
}
.tv-kids {
  margin-top: 4px;
  padding-left: 2px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.tv-kid {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 0.72rem;
  color: var(--text-2);
}
.tv-kid .done {
  color: var(--text-4);
  text-decoration: line-through;
}
.tv-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}
.tv-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}
.tv-icon:hover {
  background: var(--bg-card-soft);
  color: var(--text-1);
}
.tv-child-input {
  flex: 1 0 100%;
  padding-left: 24px;
}
.tv-input {
  width: 100%;
  padding: 5px 8px;
  font-size: 0.75rem;
  font-family: inherit;
  color: var(--text-1);
  background: var(--bg-card-soft);
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-sm);
  outline: none;
}
.tv-cal {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.tv-cal-h {
  display: flex;
  align-items: center;
  gap: 7px;
}
.tv-cal-title {
  font-size: 0.8rem;
  font-weight: 700;
  color: var(--text-1);
}
.tv-legend {
  display: flex;
  align-items: center;
  gap: 14px;
  flex-wrap: wrap;
  font-size: 0.66rem;
  color: var(--text-4);
}
.tv-legend span {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}
.swatch {
  width: 14px;
  height: 8px;
  border-radius: 3px;
  border: 1px solid var(--brand-500);
  background: var(--brand-50);
}
.swatch.dashed {
  border-style: dashed;
  background: transparent;
}
.swatch.late {
  border-color: var(--c-red-ink);
  background: var(--c-red-soft);
}
.tv-grid {
  display: grid;
  grid-template-columns: repeat(7, minmax(0, 1fr));
  gap: 4px;
}
.tv-dow {
  font-size: 0.64rem;
  font-weight: 700;
  color: var(--text-4);
  text-align: center;
}
.tv-cell {
  min-height: 74px;
  padding: 4px 5px;
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-sm);
  background: var(--bg-card-solid);
  overflow: hidden;
}
.tv-cell.out {
  opacity: 0.45;
}
.tv-cell.today {
  border-color: var(--brand-500);
}
.tv-day {
  font-size: 0.66rem;
  color: var(--text-3);
}
.tv-chips {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-top: 2px;
}
.tv-chip {
  display: block;
  width: 100%;
  padding: 1px 5px;
  font-size: 0.6rem;
  text-align: left;
  border-radius: 4px;
  border: 1px solid var(--brand-500);
  background: var(--brand-50);
  color: var(--text-1);
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tv-chip.virtual {
  border-style: dashed;
  background: transparent;
  color: var(--text-3);
  cursor: not-allowed;
}
.tv-chip.real {
  cursor: grab;
}
.tv-chip.dragging {
  opacity: 0.45;
}
:global(body.todo-chip-dragging) {
  cursor: grabbing;
  user-select: none;
}
.tv-cell.drop-on,
.tv-weekcol.drop-on {
  border-color: var(--brand-500);
  background: var(--brand-50);
}
.tv-chip.late {
  border-color: var(--c-red-ink);
  background: var(--c-red-soft);
  color: var(--c-red-ink);
}
.tv-chip .time {
  margin-left: 4px;
  font-style: normal;
  color: var(--text-4);
}
.tv-week {
  display: grid;
  grid-template-columns: repeat(7, minmax(0, 1fr));
  gap: 5px;
}
.tv-weekcol {
  min-height: 180px;
  padding: 5px;
  border: 1px solid var(--border-soft);
  border-radius: var(--radius-sm);
  background: var(--bg-card-solid);
}
.tv-weekcol.today {
  border-color: var(--brand-500);
}
.tv-weekhead {
  font-size: 0.64rem;
  font-weight: 700;
  color: var(--text-3);
  margin-bottom: 4px;
}
</style>
