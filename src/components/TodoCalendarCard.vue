<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { ArrowRight, CalendarDays, ChevronLeft, ChevronRight } from 'lucide-vue-next'
import { useStore } from '../stores/workbench'
import type { Todo, TodoOccurrence } from '../api/tauri'
import { calendarGrid, dueBadge, isoKey } from '../utils/todoSchedule'

/**
 * 工作台「日历」模块：月历形态展示待办分布（含周期待办的**虚拟实例**）。
 * 只读呈现——点整卡进待办视图做新建与改期；周期规则的展开结果来自 Rust 侧命令。
 */
const props = defineProps<{ onOpenDetail?: () => void; title?: string; hideTitle?: boolean }>()

const store = useStore()

const cursor = ref(new Date())
const occurrences = ref<TodoOccurrence[]>([])
const today = new Date()

const cells = computed(() => calendarGrid(cursor.value, today))
const monthLabel = computed(() => `${cursor.value.getFullYear()} 年 ${cursor.value.getMonth() + 1} 月`)

const topTodos = computed(() => store.state.todos.filter((t) => t.parent_id == null))

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

async function load() {
  const first = cells.value[0]
  const from = new Date(`${first.key}T00:00:00`)
  const to = new Date(from)
  to.setDate(to.getDate() + 42)
  occurrences.value = await store.expandTodoOccurrences(from.getTime(), to.getTime() - 1)
}

onMounted(() => void load())
watch(cursor, () => void load())

function shift(delta: number, e: MouseEvent) {
  e.stopPropagation()
  const d = new Date(cursor.value)
  d.setMonth(d.getMonth() + delta, 1)
  cursor.value = d
}

function dayCount(key: string): number {
  return (realByDay.value.get(key)?.length ?? 0) + (virtualByDay.value.get(key)?.length ?? 0)
}
</script>

<template>
  <section class="card todo-cal" :aria-label="title ?? '日历'" @click="props.onOpenDetail?.()">
    <header class="tc-header" :class="{ 'hd-float': hideTitle }">
      <h3 v-if="!hideTitle" class="tc-title">
        <CalendarDays :size="14" :stroke-width="2" aria-hidden="true" />
        <span>{{ title ?? '日历' }}</span>
      </h3>
      <span class="tc-month">{{ monthLabel }}</span>
      <span class="tc-spacer"></span>
      <button class="tc-nav" type="button" aria-label="上个月" @click="shift(-1, $event)">
        <ChevronLeft :size="13" :stroke-width="2" />
      </button>
      <button class="tc-nav" type="button" aria-label="下个月" @click="shift(1, $event)">
        <ChevronRight :size="13" :stroke-width="2" />
      </button>
      <ArrowRight v-if="!hideTitle" class="tc-more" :size="14" :stroke-width="2" aria-hidden="true" />
    </header>

    <div class="tc-grid">
      <div v-for="d in ['一', '二', '三', '四', '五', '六', '日']" :key="d" class="tc-dow">{{ d }}</div>
      <div
        v-for="c in cells"
        :key="c.key"
        class="tc-cell"
        :class="{ out: c.out, today: c.today }"
      >
        <span class="tc-day">{{ c.day }}</span>
        <div class="tc-chips">
          <span
            v-for="t in (realByDay.get(c.key) ?? []).slice(0, 2)"
            :key="'r' + t.id"
            class="tc-chip real"
            :class="{ late: dueBadge(t, today)?.kind === 'over' }"
            :title="t.title"
          >{{ t.title }}</span>
          <span
            v-for="o in (virtualByDay.get(c.key) ?? []).slice(0, 2)"
            :key="'v' + o.todo_id + o.at_ms"
            class="tc-chip virtual"
            :title="`${titleOf.get(o.todo_id) ?? '周期待办'}（虚拟实例）`"
          >{{ titleOf.get(o.todo_id) ?? '周期待办' }}</span>
          <span v-if="dayCount(c.key) > 2" class="tc-more-cnt">+{{ dayCount(c.key) - 2 }}</span>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.todo-cal {
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 12px;
  min-height: 0;
  cursor: pointer;
}
.tc-header {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
}
.tc-title {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  font-size: 0.8125rem;
  font-weight: 600;
  color: var(--text-1);
}
.tc-title :deep(svg) {
  color: var(--brand-500);
}
.tc-month {
  font-size: 0.6875rem;
  color: var(--text-3);
}
.tc-spacer {
  flex: 1;
}
.tc-nav {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}
.tc-nav:hover {
  background: var(--bg-card-soft);
  color: var(--text-1);
}
.tc-more {
  color: var(--text-4);
}
.tc-grid {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: repeat(7, minmax(0, 1fr));
  grid-auto-rows: minmax(0, 1fr);
  gap: 2px;
}
.tc-dow {
  font-size: 0.5625rem;
  font-weight: 700;
  color: var(--text-4);
  text-align: center;
}
.tc-cell {
  min-height: 0;
  padding: 2px 3px;
  border: 1px solid var(--border-soft);
  border-radius: 5px;
  background: var(--bg-card-soft);
  overflow: hidden;
}
.tc-cell.out {
  opacity: 0.4;
}
.tc-cell.today {
  border-color: var(--brand-500);
}
.tc-day {
  font-size: 0.5625rem;
  color: var(--text-4);
  font-variant-numeric: tabular-nums;
}
.tc-chips {
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.tc-chip {
  display: block;
  padding: 0 3px;
  font-size: 0.5rem;
  line-height: 1.5;
  border-radius: 3px;
  border: 1px solid var(--brand-500);
  background: var(--brand-50);
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tc-chip.virtual {
  border-style: dashed;
  background: transparent;
  color: var(--text-3);
}
.tc-chip.late {
  border-color: var(--c-red-ink);
  background: var(--c-red-soft);
  color: var(--c-red-ink);
}
.tc-more-cnt {
  font-size: 0.5rem;
  color: var(--text-4);
}
</style>
