<script setup lang="ts">
// 扩展大类（左栏两个子项，对应下面两个 section）：
//   ① 扩展：service 运行时策略 + 开发者模式开关 —— 只管「是否加载」
//   ② 我的扩展：本机添加的本地扩展源码目录清单 —— 管「加了哪些」
//
// 从 SettingsView.vue 拆出（见该文件顶部说明）：设置页按大类按需加载，
// 首次打开只需外壳 + 当前大类的代码，切大类时才加载对应面板。
import { inject, onMounted, ref } from 'vue';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderCog, Lock, Trash2 } from 'lucide-vue-next';
import { isTauri, tauriApi } from '../../api/tauri';
import type { DevModeStatus } from '../../api/tauri';
import AppSelect from '../AppSelect.vue';
import { useStore } from '../../stores/workbench';

const showToast = inject<(msg: string) => void>('showToast', () => {})
const store = useStore()

const RUNTIME_STRATEGY_OPTIONS = [
  { value: 'auto', label: '自动检测（系统优先，缺失自动下载内置）' },
  { value: 'builtin', label: '始终内置（统一用下载的内置运行时）' },
  { value: 'system', label: '始终系统（只用系统 Node，不下载）' },
] as const

function onRuntimeStrategyChange(value: string) {
  void store.setRuntimeStrategy(value as 'auto' | 'builtin' | 'system').then(() => {
    showToast('运行时策略已更新，下次启动 service 扩展生效')
  })
}

// ---- 开发者模式 + 我的扩展 ----
// 开关与列表放在同一个面板里共用一个状态：这俩本来就是一件事（开关只决定下面这些目录加不加载），
// 分成两块各拉各的状态，很容易出现「这边显示开着、那边显示没开」。
const devMode = ref<DevModeStatus>({ enabled: false, extensions: [] })
const devModeBusy = ref(false)

async function loadDevMode() {
  if (!isTauri()) return
  try {
    devMode.value = await tauriApi.getDevModeStatus()
  } catch {
    // 后端不可用（浏览器预览）时静默：该分区只在桌面端有意义
  }
}

async function toggleDevMode() {
  if (devModeBusy.value) return
  devModeBusy.value = true
  try {
    devMode.value = await tauriApi.setDevModeEnabled(!devMode.value.enabled)
    showToast(devMode.value.enabled ? '开发者模式已开启' : '开发者模式已关闭')
  } catch (e) {
    showToast(`切换失败：${e}`)
  } finally {
    devModeBusy.value = false
  }
}

async function pickDir() {
  try {
    const picked = await open({
      directory: true,
      multiple: false,
      title: '选择扩展源码目录（须含 manifest.json）',
    })
    const path = typeof picked === 'string' ? picked : null
    if (!path) return
    devModeBusy.value = true
    devMode.value = await tauriApi.addDevExtension(path)
    showToast(
      devMode.value.enabled
        ? '已添加到「我的扩展」，改代码即自动重载'
        : '已添加到「我的扩展」；开启开发者模式后才会加载'
    )
  } catch (e) {
    showToast(String(e))
  } finally {
    devModeBusy.value = false
  }
}

async function removeDir(path: string) {
  try {
    devMode.value = await tauriApi.removeDevExtension(path)
    showToast('已从「我的扩展」移除')
  } catch (e) {
    showToast(String(e))
  }
}

onMounted(() => {
  void loadDevMode()
})
</script>

<template>
        <section id="sv-sec-extensions" class="sv-sec" aria-label="扩展">
          <h3 class="sv-sec-title">扩展</h3>
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-name">service 运行时策略</span>
              <span class="setting-desc">service 扩展后端的 Node 运行时来源：自动检测 / 始终内置 / 始终系统</span>
            </div>
            <AppSelect
              :model-value="store.state.config.runtime_strategy || 'auto'"
              :options="RUNTIME_STRATEGY_OPTIONS"
              aria-label="service 运行时策略"
              @update:model-value="onRuntimeStrategyChange"
            />
          </div>

          <!-- 开发者模式：只负责「是否加载」——加了哪些本地扩展见下面「我的扩展」（源码目录直挂见 ADR 0005） -->
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-name">开发者模式</span>
              <span class="setting-desc">
                开启后才加载「我的扩展」里添加的源码目录：目录即真源（不复制进已装列表），改代码自动重载，可用真实数据与 service 后端
              </span>
            </div>
            <button
              class="toggle"
              role="switch"
              type="button"
              :aria-checked="devMode.enabled"
              :class="{ on: devMode.enabled }"
              :disabled="devModeBusy"
              @click="toggleDevMode"
            >
              <span class="toggle-knob"></span>
            </button>
          </div>
        </section>

        <section id="sv-sec-myext" class="sv-sec" aria-label="我的扩展">
          <h3 class="sv-sec-title">我的扩展</h3>
          <p class="dev-hint">
            添加本机扩展源码目录（须含 manifest.json）。路径即真源，改完保存立刻生效。
          </p>

          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-name">添加本地扩展</span>
              <span class="setting-desc">选择含 manifest.json 的扩展源码目录，添加后出现在下面的列表里</span>
            </div>
            <button
              class="ghost-btn data-btn"
              type="button"
              :disabled="devModeBusy"
              @click="pickDir"
            >
              <FolderCog :size="14" :stroke-width="2" />
              选择目录
            </button>
          </div>

          <h4 class="sv-subtitle">已添加的本地扩展</h4>
          <p v-if="!devMode.extensions.length" class="dev-empty">还没有添加本地扩展</p>
          <div v-for="d in devMode.extensions" :key="d.path" class="setting-row">
            <div class="setting-info">
              <span class="setting-name">
                {{ d.name || d.id || '（manifest 无法解析）' }}
                <b v-if="d.version" class="dev-ver">v{{ d.version }}</b>
              </span>
              <span class="dev-dir-path" :title="d.id">{{ d.path }}</span>
              <span v-if="!d.exists" class="dev-dir-warn">目录不存在</span>
              <span v-else-if="!d.valid" class="dev-dir-warn">{{ d.error }}</span>
              <span v-else-if="d.conflict" class="dev-dir-warn">
                与已装扩展同 id：已装优先，直挂不生效（请先卸载已装版本）
              </span>
              <span v-else-if="!devMode.enabled" class="dev-dir-warn">
                开发者模式已关闭：当前不会被加载
              </span>
            </div>
            <button class="ghost-btn data-btn" type="button" @click="removeDir(d.path)">
              <Trash2 :size="14" :stroke-width="2" />
              移除
            </button>
          </div>

          <p class="settings-foot">
            <Lock :size="12" :stroke-width="2" class="settings-lock" aria-hidden="true" />
            本地扩展不进已装列表，也不参与市场更新与卸载
          </p>
        </section>
</template>

<style scoped>
/* 本面板专属样式（通用设置样式在 settings/shared.css，见约定 50） */
.dev-hint {
  margin: 0 0 10px;
  font-size: 0.75rem;
  line-height: 1.6;
  color: var(--text-2);
}
.dev-empty {
  margin: 0 0 10px;
  font-size: 0.72rem;
  color: var(--text-3);
}
/* 版本徽标：跟在扩展名后面，不占单独一行（好让目录紧贴名称下方） */
.dev-ver {
  margin-left: 6px;
  padding: 0 5px;
  border-radius: 999px;
  background: var(--bg-card-soft);
  font-family: ui-monospace, Consolas, monospace;
  font-size: 0.66rem;
  font-weight: 600;
  color: var(--text-2);
}
</style>
