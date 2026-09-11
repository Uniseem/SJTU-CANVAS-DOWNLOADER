import { computed, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import type { DownloadMode } from '@/types'

interface SavedPreferences {
  downloadMode: DownloadMode
  concurrency: number
  fallbackToProxy: boolean
  compactCourseCards: boolean
}

const STORAGE_KEY = 'canvas-pocket-preferences-v2'

const defaults: SavedPreferences = {
  downloadMode: 'direct',
  concurrency: 3,
  fallbackToProxy: true,
  compactCourseCards: true,
}

function restore(): SavedPreferences {
  try {
    const current = localStorage.getItem(STORAGE_KEY)
    // Keep network preferences, but migrate the old roomy default to compact.
    const parsed = JSON.parse(current ?? localStorage.getItem('canvas-pocket-preferences-v1') ?? '{}') as Partial<SavedPreferences>
    return {
      downloadMode: parsed.downloadMode === 'proxy' ? 'proxy' : defaults.downloadMode,
      concurrency: Math.min(6, Math.max(1, Number(parsed.concurrency) || defaults.concurrency)),
      fallbackToProxy: typeof parsed.fallbackToProxy === 'boolean' ? parsed.fallbackToProxy : defaults.fallbackToProxy,
      compactCourseCards: current && typeof parsed.compactCourseCards === 'boolean' ? parsed.compactCourseCards : defaults.compactCourseCards,
    }
  } catch {
    return { ...defaults }
  }
}

export const usePreferencesStore = defineStore('preferences', () => {
  const restored = restore()
  const downloadMode = ref<DownloadMode>(restored.downloadMode)
  const concurrency = ref(restored.concurrency)
  const fallbackToProxy = ref(restored.fallbackToProxy)
  const compactCourseCards = ref(restored.compactCourseCards)

  const snapshot = computed<SavedPreferences>(() => ({
    downloadMode: downloadMode.value,
    concurrency: concurrency.value,
    fallbackToProxy: fallbackToProxy.value,
    compactCourseCards: compactCourseCards.value,
  }))

  watch(snapshot, (value) => localStorage.setItem(STORAGE_KEY, JSON.stringify(value)), { deep: true })

  function reset() {
    downloadMode.value = defaults.downloadMode
    concurrency.value = defaults.concurrency
    fallbackToProxy.value = defaults.fallbackToProxy
    compactCourseCards.value = defaults.compactCourseCards
  }

  return { downloadMode, concurrency, fallbackToProxy, compactCourseCards, snapshot, reset }
})
