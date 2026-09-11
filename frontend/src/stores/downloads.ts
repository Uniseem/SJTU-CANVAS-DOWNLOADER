import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { ApiError, api, getErrorMessage, isDemoMode, notifyApiUnauthorized } from '@/services/api'
import { allocateDownloadFile, downloadLayout } from '@/utils/downloadLayout'
import type { DownloadMode, DownloadRequestItem, DownloadTask } from '@/types'
import { usePreferencesStore } from '@/stores/preferences'

const terminalStates = new Set<DownloadTask['status']>(['completed', 'failed', 'cancelled'])

type PreparePreference = 'auto' | 'direct' | 'proxy'

interface SaveDestination {
  handle: FileSystemDirectoryHandle
  name: string
}

interface PendingPreparation {
  item: DownloadRequestItem
  requestedMode: DownloadMode
  preference: PreparePreference
  destination: SaveDestination
  generation: number
}

export const useDownloadsStore = defineStore('downloads', () => {
  const preferences = usePreferencesStore()
  const tasks = ref<DownloadTask[]>([])
  const preparing = ref(false)
  const prepareError = ref<string | null>(null)
  const pendingPrepareCount = ref(0)
  const selectingDestination = ref(false)
  const destinations = new Map<string, SaveDestination>()
  const controllers = new Map<string, AbortController>()
  const fileHandles = new Map<string, FileSystemFileHandle>()
  const pendingPreparations: PendingPreparation[] = []
  let scheduling = false
  let preparePumpRunning = false
  let preparationGeneration = 0
  let prepareFailureCount = 0
  let inFlightPreparationCount = 0

  const supportsDirectoryApi = computed(() => typeof window.showDirectoryPicker === 'function')
  const activeCount = computed(() => tasks.value.filter((task) => task.status === 'downloading').length)
  const queuedCount = computed(() => tasks.value.filter((task) => task.status === 'queued').length + pendingPrepareCount.value)
  const unfinishedCount = computed(() => tasks.value.filter((task) => !terminalStates.has(task.status)).length + pendingPrepareCount.value)
  const overallProgress = computed(() => {
    const relevant = tasks.value.filter((task) => !['cancelled', 'failed'].includes(task.status))
    const totalCount = relevant.length + pendingPrepareCount.value
    if (!totalCount) return 0
    const completed = relevant.reduce((sum, task) => sum + (task.status === 'completed' ? 1 : task.total ? Math.min(task.received / task.total, 1) : 0), 0)
    return Math.round((completed / totalCount) * 100)
  })

  async function chooseDestination(): Promise<SaveDestination | null> {
    if (selectingDestination.value) return null
    if (!window.showDirectoryPicker) {
      prepareError.value = '当前浏览器无法选择保存文件夹。请使用支持目录访问的桌面 Chrome / Edge；VPS 站点需通过 HTTPS 访问。未选择保存位置，不会开始下载。'
      return null
    }
    selectingDestination.value = true
    try {
      // Invoke directly within the click's user activation, before any API
      // call. Each invocation returns its own immutable destination snapshot.
      const handle = await window.showDirectoryPicker({ id: 'canvas-pocket', mode: 'readwrite' })
      return { handle, name: handle.name }
    } catch (error) {
      if ((error as { name?: string }).name === 'AbortError') return null
      prepareError.value = getErrorMessage(error, '无法访问所选文件夹')
      return null
    } finally {
      selectingDestination.value = false
    }
  }

  async function enqueue(items: DownloadRequestItem[], mode?: DownloadMode) {
    if (!items.length || selectingDestination.value) return false
    if (!pendingPreparations.length && !preparePumpRunning) {
      prepareError.value = null
      prepareFailureCount = 0
    }
    const generation = preparationGeneration
    const requests = items.map((item) => ({ ...item }))
    const destination = await chooseDestination()
    if (!destination || generation !== preparationGeneration) return false
    const requestedMode = mode ?? preferences.downloadMode
    const preference: PreparePreference = requestedMode === 'proxy'
      ? 'proxy'
      : preferences.fallbackToProxy
        ? 'auto'
        : 'direct'

    pendingPreparations.push(...requests.map((item) => ({
      item,
      requestedMode,
      preference,
      destination,
      generation,
    })))
    syncPendingPrepareCount()
    void pumpPreparations()
    return true
  }

  async function pumpPreparations() {
    if (preparePumpRunning) return
    preparePumpRunning = true
    try {
      while (pendingPreparations.length) {
        const first = pendingPreparations[0]
        if (!first) break
        const existingQueued = tasks.value.filter((task) => task.status === 'queued').length
        const openSlots = Math.max(0, preferences.concurrency - activeCount.value - existingQueued)
        const capacity = Math.min(openSlots, 10)
        if (capacity <= 0) {
          await delay(180)
          continue
        }

        const batch: PendingPreparation[] = []
        while (batch.length < Math.min(capacity, 10, 100)) {
          const candidate = pendingPreparations[0]
          if (!candidate
            || candidate.generation !== first.generation
            || candidate.requestedMode !== first.requestedMode
            || candidate.preference !== first.preference
            || candidate.destination !== first.destination) break
          batch.push(pendingPreparations.shift()!)
        }
        inFlightPreparationCount += batch.length
        syncPendingPrepareCount()
        if (!batch.length) continue

        const batchGeneration = batch[0]!.generation
        preparing.value = true
        try {
          const response = await api.prepare(batch.map((entry) => entry.item), first.preference)
          if (batchGeneration !== preparationGeneration) continue
          if (response.failures?.length) {
            recordPrepareFailures(response.failures.length, response.failures[0]?.message)
          }
          const failedIndices = new Set(response.failures?.map((failure) => failure.index) ?? [])
          const successful = batch.filter((_, index) => !failedIndices.has(index))
          if (successful.length !== response.items.length) throw new Error('服务器返回的任务与请求不匹配，已停止本批以免保存到错误目录')
          const created = createTasks(response.items, successful)
          schedule()
          // Do not sign the next URLs until these descriptors have actually
          // entered execution. The following loop also waits for a real free
          // slot, keeping signed URLs fresh even for very long batches.
          while (created.some((task) => task.status === 'queued')) await delay(50)
        } catch (error) {
          if (batchGeneration !== preparationGeneration) continue
          recordPrepareFailures(batch.length, getErrorMessage(error, '生成下载任务失败'))
          if (error instanceof ApiError && error.status === 401) {
            pendingPreparations.splice(0)
            syncPendingPrepareCount()
            break
          }
        } finally {
          inFlightPreparationCount = Math.max(0, inFlightPreparationCount - batch.length)
          syncPendingPrepareCount()
          preparing.value = false
        }
      }
    } finally {
      preparePumpRunning = false
      syncPendingPrepareCount()
      if (pendingPreparations.length) void pumpPreparations()
    }
  }

  function createTasks(
    descriptors: Awaited<ReturnType<typeof api.prepare>>['items'],
    preparations: PendingPreparation[],
  ) {
    const created: DownloadTask[] = []
    for (const [index, descriptor] of descriptors.entries()) {
      const preparation = preparations[index]!
      const layout = downloadLayout(preparation.item, descriptor.filename)
      const task: DownloadTask = {
        ...descriptor,
        taskId: `${descriptor.source}-${descriptor.id}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        filename: layout.filename,
        status: 'queued',
        mode: preparation.requestedMode === 'direct' && descriptor.directSupported && descriptor.directUrl ? 'direct' : 'proxy',
        received: 0,
        total: descriptor.size ?? null,
        speed: 0,
        createdAt: Date.now(),
        relativePath: [...layout.directories, layout.filename].join('/'),
        destinationName: preparation.destination.name,
        request: preparation.item,
      }
      destinations.set(task.taskId, preparation.destination)
      tasks.value.unshift(task)
      created.push(task)
    }
    return created
  }

  function recordPrepareFailures(count: number, message?: string) {
    prepareFailureCount += count
    prepareError.value = `${prepareFailureCount} 项未能加入：${message || '资源暂不可用'}。其余项目会继续处理。`
  }

  function cancelPending() {
    preparationGeneration += 1
    pendingPreparations.splice(0)
    syncPendingPrepareCount()
  }

  function syncPendingPrepareCount() {
    pendingPrepareCount.value = pendingPreparations.length + inFlightPreparationCount
  }

  function delay(ms: number) {
    return new Promise<void>((resolve) => window.setTimeout(resolve, ms))
  }

  function schedule() {
    if (scheduling) return
    scheduling = true
    queueMicrotask(() => {
      scheduling = false
      const capacity = preferences.concurrency - activeCount.value
      if (capacity <= 0) return
      const selected: DownloadTask[] = []
      for (const task of tasks.value.filter((item) => item.status === 'queued' && !controllers.has(item.taskId))) {
        if (selected.length >= capacity) break
        selected.push(task)
      }
      selected.forEach((task) => void run(task))
    })
  }

  async function run(task: DownloadTask) {
    if (task.status !== 'queued') return
    task.status = 'downloading'
    task.error = undefined
    const controller = new AbortController()
    controllers.set(task.taskId, controller)
    try {
      if (isDemoMode) {
        await runDemo(task, controller.signal)
      } else {
        await streamToDirectory(task, controller)
      }
      if (task.status === 'downloading') {
        task.status = 'completed'
        task.completedAt = Date.now()
        task.speed = 0
        if (task.total) task.received = task.total
      }
    } catch (error) {
      const latestStatus = task.status as DownloadTask['status']
      if (latestStatus === 'paused' || latestStatus === 'cancelled' || (controller.signal.aborted && latestStatus === 'queued')) return
      task.status = 'failed'
      task.speed = 0
      task.error = getErrorMessage(error, '下载中断')
    } finally {
      controllers.delete(task.taskId)
      if (terminalStates.has(task.status)) {
        fileHandles.delete(task.taskId)
        destinations.delete(task.taskId)
      }
      schedule()
    }
  }

  async function runDemo(task: DownloadTask, signal: AbortSignal) {
    const total = task.total ?? 24_000_000
    const start = task.received
    const steps = 18
    for (let index = 1; index <= steps; index += 1) {
      if (signal.aborted) throw new DOMException('Aborted', 'AbortError')
      await new Promise((resolve) => window.setTimeout(resolve, 90 + Math.random() * 80))
      task.received = Math.min(total, start + ((total - start) * index) / steps)
      task.speed = 4_000_000 + Math.random() * 5_000_000
    }
    const handle = await outputFile(task, signal)
    const writer = await handle.createWritable()
    await writer.write(new Blob([`Canvas Pocket 演示下载：${task.filename}\n`]))
    await writer.close()
  }

  async function outputFile(task: DownloadTask, signal: AbortSignal) {
    signal.throwIfAborted()
    const existing = fileHandles.get(task.taskId)
    if (existing) return existing
    const destination = destinations.get(task.taskId)
    if (!destination) throw new Error('保存位置已失效，请重新选择位置下载')
    const directories = task.relativePath.split('/').slice(0, -1)
    const allocation = await allocateDownloadFile(destination.handle, directories, task.filename)
    task.filename = allocation.filename
    task.relativePath = allocation.relativePath
    fileHandles.set(task.taskId, allocation.handle)
    signal.throwIfAborted()
    return allocation.handle
  }

  async function streamToDirectory(task: DownloadTask, controller: AbortController) {
    const handle = await outputFile(task, controller.signal)
    try {
      await fetchAndWrite(task, handle, controller, task.mode)
    } catch (error) {
      if (controller.signal.aborted) throw error
      if (task.mode === 'direct' && preferences.fallbackToProxy) {
        task.mode = 'proxy'
        task.fallbackUsed = true
        task.received = 0
        task.speed = 0
        await fetchAndWrite(task, handle, controller, 'proxy')
        return
      }
      throw error
    }
  }

  async function fetchAndWrite(task: DownloadTask, handle: FileSystemFileHandle, controller: AbortController, mode: DownloadMode) {
    const resumeAt = task.received
    const url = mode === 'direct' && task.directUrl ? task.directUrl : task.proxyUrl
    const requestHeaders: Record<string, string> = {}
    if (resumeAt > 0) {
      requestHeaders.Range = `bytes=${Math.floor(resumeAt)}-`
      if (task.etag) requestHeaders['If-Range'] = task.etag
    }
    const response = await fetch(url, {
      signal: controller.signal,
      credentials: mode === 'proxy' ? 'include' : 'omit',
      headers: Object.keys(requestHeaders).length ? requestHeaders : undefined,
    })
    if (mode === 'proxy' && response.status === 401) notifyApiUnauthorized()
    if (!response.ok || !response.body) throw new Error(`下载源响应异常（${response.status}）`)

    const resumed = resumeAt > 0 && response.status === 206
    if (!resumed && resumeAt > 0) task.received = 0
    const contentLength = Number(response.headers.get('Content-Length')) || 0
    const contentRange = response.headers.get('Content-Range')
    const rangeTotal = contentRange?.match(/\/(\d+)$/)?.[1]
    task.etag = response.headers.get('ETag') ?? task.etag
    task.total = rangeTotal
      ? Number(rangeTotal)
      : contentLength
        ? (resumed ? resumeAt + contentLength : contentLength)
        : task.total

    const writer = await handle.createWritable({ keepExistingData: resumed })
    if (resumed) await writer.write({ type: 'seek', position: Math.floor(resumeAt) })
    let lastBytes = task.received
    let lastTime = performance.now()
    const reader = response.body.getReader()
    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        await writer.write(value)
        task.received += value.byteLength
        const now = performance.now()
        if (now - lastTime >= 350) {
          task.speed = ((task.received - lastBytes) * 1000) / (now - lastTime)
          lastBytes = task.received
          lastTime = now
        }
      }
      if (task.total && task.received < task.total) {
        throw new Error(`下载连接提前结束（${Math.floor(task.received)} / ${Math.floor(task.total)} 字节）`)
      }
      await writer.close()
    } catch (error) {
      // A failed stream can be resumed only if the bytes counted in `received`
      // remain on disk. Cancellation deliberately discards the current write;
      // pause and ordinary network errors preserve the verified partial file.
      if (task.status !== 'cancelled') {
        await writer.close().catch(() => undefined)
      } else {
        await writer.abort(error).catch(() => undefined)
      }
      throw error
    } finally {
      reader.releaseLock()
    }
  }

  function pause(taskId: string) {
    const task = tasks.value.find((item) => item.taskId === taskId)
    if (!task || task.status !== 'downloading') return
    task.status = 'paused'
    task.speed = 0
    controllers.get(taskId)?.abort()
  }

  function resume(taskId: string) {
    const task = tasks.value.find((item) => item.taskId === taskId)
    if (!task || task.status !== 'paused') return
    task.status = 'queued'
    task.error = undefined
    schedule()
  }

  function cancel(taskId: string) {
    const task = tasks.value.find((item) => item.taskId === taskId)
    if (!task || terminalStates.has(task.status)) return
    task.status = 'cancelled'
    task.speed = 0
    controllers.get(taskId)?.abort()
    if (!controllers.has(taskId)) {
      fileHandles.delete(taskId)
      destinations.delete(taskId)
    }
  }

  async function retry(taskId: string, mode?: DownloadMode) {
    const task = tasks.value.find((item) => item.taskId === taskId)
    if (!task || !['failed', 'cancelled'].includes(task.status)) return
    // A restart is a new download with a fresh choice and fresh signed URL.
    // Pause/resume, in contrast, retains the original file and directory.
    if (await enqueue([task.request], mode ?? task.mode)) {
      tasks.value = tasks.value.filter((item) => item.taskId !== taskId)
    }
  }

  function clearFinished() {
    for (const task of tasks.value.filter((item) => terminalStates.has(item.status))) {
      if (controllers.has(task.taskId)) continue
      fileHandles.delete(task.taskId)
      destinations.delete(task.taskId)
    }
    tasks.value = tasks.value.filter((task) => !terminalStates.has(task.status))
  }

  return {
    tasks,
    preparing,
    prepareError,
    pendingPrepareCount,
    selectingDestination,
    supportsDirectoryApi,
    activeCount,
    queuedCount,
    unfinishedCount,
    overallProgress,
    enqueue,
    pause,
    resume,
    cancel,
    retry,
    cancelPending,
    clearFinished,
    schedule,
  }
})
