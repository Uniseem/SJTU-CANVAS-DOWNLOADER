<script setup lang="ts">
import { computed, ref } from 'vue'
import { useDownloadsStore } from '@/stores/downloads'
import { usePreferencesStore } from '@/stores/preferences'
import { formatBytes, formatDate } from '@/utils/format'
import type { DownloadStatus, DownloadTask } from '@/types'
import PageHeading from '@/components/PageHeading.vue'
import EmptyState from '@/components/EmptyState.vue'
import { VIDEO_TRACK_LABELS } from '@/utils/downloadLayout'

const downloads = useDownloadsStore()
const preferences = usePreferencesStore()
const filter = ref<'all' | 'active' | 'finished'>('all')

const visibleTasks = computed(() => downloads.tasks.filter((task) => {
  if (filter.value === 'active') return ['queued', 'preparing', 'downloading', 'paused'].includes(task.status)
  if (filter.value === 'finished') return ['completed', 'failed', 'cancelled'].includes(task.status)
  return true
}))

const statusLabels: Record<DownloadStatus, string> = {
  queued: '等待中',
  preparing: '准备中',
  downloading: '下载中',
  paused: '已暂停',
  completed: '已完成',
  failed: '失败',
  cancelled: '已取消',
}

const statusColors: Record<DownloadStatus, string> = {
  queued: 'info',
  preparing: 'info',
  downloading: 'primary',
  paused: 'warning',
  completed: 'success',
  failed: 'error',
  cancelled: 'default',
}

function progress(task: DownloadTask) {
  if (task.status === 'completed') return 100
  return task.total ? Math.min(100, (task.received / task.total) * 100) : 0
}

function detail(task: DownloadTask) {
  if (task.status === 'downloading') {
    return `${formatBytes(task.received)} / ${formatBytes(task.total)} · ${formatBytes(task.speed)}/s`
  }
  if (task.status === 'completed') {
    return `${formatBytes(task.total)} · 保存完成`
  }
  if (task.status === 'paused') return `停在 ${formatBytes(task.received)}，可继续`
  if (task.status === 'failed') return task.error ?? '下载失败'
  if (task.status === 'cancelled') return '任务已取消'
  return task.status === 'queued' ? '等待可用下载位' : '正在生成下载地址'
}
</script>

<template>
  <div>
    <PageHeading eyebrow="Transfers" title="下载中心" description="资源会在有空闲下载位时即时签票；关闭本标签页会停止尚未开始的任务。">
      <template #actions>
        <v-btn v-if="downloads.tasks.some((task) => ['completed', 'failed', 'cancelled'].includes(task.status))" variant="outlined" prepend-icon="mdi-broom" @click="downloads.clearFinished">
          清理记录
        </v-btn>
        <v-btn to="/courses" color="primary" prepend-icon="mdi-plus">添加任务</v-btn>
      </template>
    </PageHeading>

    <section class="download-summary mb-5">
      <div class="summary-progress">
        <div class="summary-ring" :style="{ '--progress': `${downloads.overallProgress * 3.6}deg` }">
          <div><strong>{{ downloads.overallProgress }}</strong><span>%</span></div>
        </div>
        <div>
          <div class="eyebrow">Overall progress</div>
          <h2>{{ downloads.unfinishedCount ? '下载进行中' : downloads.tasks.length ? '队列已完成' : '队列空闲' }}</h2>
          <p>{{ downloads.activeCount }} 个进行中 · {{ downloads.queuedCount }} 个等待</p>
        </div>
      </div>
      <div class="summary-settings">
        <div class="summary-setting">
          <v-icon icon="mdi-lan-connect" />
          <div><span>默认线路</span><strong>{{ preferences.downloadMode === 'direct' ? '浏览器直连' : '服务器代理' }}</strong></div>
        </div>
        <div class="summary-setting">
          <v-icon icon="mdi-transit-connection-variant" />
          <div><span>并发任务</span><strong>{{ preferences.concurrency }} 个</strong></div>
        </div>
        <div class="summary-setting">
          <v-icon icon="mdi-folder-arrow-down-outline" />
          <div><span>保存方式</span><strong>每批选择文件夹</strong></div>
        </div>
      </div>
    </section>

    <v-alert v-if="downloads.prepareError" type="warning" variant="tonal" closable class="mb-4" @click:close="downloads.prepareError = null">
      {{ downloads.prepareError }}
    </v-alert>

    <section class="download-list-card">
      <div class="download-list-head">
        <v-btn-toggle v-model="filter" mandatory density="compact" color="primary" variant="text">
          <v-btn value="all">全部 <span class="filter-count">{{ downloads.tasks.length + downloads.pendingPrepareCount }}</span></v-btn>
          <v-btn value="active">进行中 <span class="filter-count">{{ downloads.unfinishedCount }}</span></v-btn>
          <v-btn value="finished">已结束</v-btn>
        </v-btn-toggle>
        <div class="queue-live"><span :class="{ active: downloads.activeCount > 0 || downloads.pendingPrepareCount > 0 }"></span>{{ downloads.activeCount || downloads.pendingPrepareCount ? '队列运行中' : '队列待机' }}</div>
      </div>

      <div v-if="downloads.pendingPrepareCount && filter !== 'finished'" class="pending-prepare-row">
        <div class="pending-prepare-icon"><v-icon icon="mdi-ticket-confirmation-outline" /></div>
        <div>
          <strong>{{ downloads.pendingPrepareCount }} 项等待即时签票</strong>
          <p>仅在出现空闲下载位时准备下一小批，避免下载地址在长队列中失效。</p>
        </div>
        <v-progress-circular v-if="downloads.preparing" indeterminate color="primary" size="24" width="3" />
        <v-btn variant="text" color="error" size="small" @click="downloads.cancelPending">取消等待</v-btn>
      </div>

      <div v-if="visibleTasks.length" class="download-task-list">
        <article v-for="task in visibleTasks" :key="task.taskId" class="download-task">
          <div class="task-file-icon" :class="{ 'task-file-icon--video': task.source.startsWith('video') }">
            <v-icon :icon="task.source.startsWith('video') ? 'mdi-video-outline' : 'mdi-file-outline'" />
          </div>
          <div class="task-content">
            <div class="task-title-row">
              <div class="min-width-0">
                <strong class="task-filename">{{ task.filename }}</strong>
                <div class="task-meta">
                  <v-chip size="x-small" :color="statusColors[task.status]" variant="tonal">
                    {{ statusLabels[task.status] }}
                  </v-chip>
                  <span>{{ task.mode === 'direct' ? '浏览器直连' : '服务器代理' }}</span>
                  <span v-if="task.source.startsWith('video:')">{{ (VIDEO_TRACK_LABELS as Record<string, string>)[task.source.split(':')[1] ?? ''] || task.source }}</span>
                  <span v-if="task.fallbackUsed" class="fallback-label"><v-icon icon="mdi-swap-horizontal" size="14" /> 已自动切换</span>
                  <span v-if="task.completedAt">{{ formatDate(new Date(task.completedAt).toISOString(), true) }}</span>
                </div>
              </div>
              <div class="task-progress-number">{{ Math.round(progress(task)) }}%</div>
            </div>
            <v-progress-linear
              :model-value="progress(task)"
              :indeterminate="task.status === 'downloading' && !task.total"
              :color="task.status === 'failed' ? 'error' : task.status === 'completed' ? 'success' : 'primary'"
              bg-color="surface-variant"
              height="6"
              rounded
              class="my-3"
            />
            <div class="task-detail" :class="{ 'text-error': task.status === 'failed' }">{{ detail(task) }}</div>
            <div class="task-save-path"><v-icon icon="mdi-folder-outline" size="14" /> {{ task.destinationName }} / {{ task.relativePath }}</div>
          </div>
          <div class="task-actions">
            <v-tooltip v-if="task.status === 'downloading'" text="暂停">
              <template #activator="{ props }"><v-btn v-bind="props" icon="mdi-pause" variant="text" @click="downloads.pause(task.taskId)" /></template>
            </v-tooltip>
            <v-tooltip v-if="task.status === 'paused'" text="继续">
              <template #activator="{ props }"><v-btn v-bind="props" icon="mdi-play" color="primary" variant="text" @click="downloads.resume(task.taskId)" /></template>
            </v-tooltip>
            <v-tooltip v-if="['failed', 'cancelled'].includes(task.status)" text="选择位置并重新下载">
              <template #activator="{ props }"><v-btn v-bind="props" icon="mdi-refresh" color="primary" variant="text" @click="downloads.retry(task.taskId)" /></template>
            </v-tooltip>
            <v-tooltip v-if="task.status === 'failed' && task.mode === 'direct'" text="改用服务器代理重试">
              <template #activator="{ props }"><v-btn v-bind="props" icon="mdi-server-network" color="warning" variant="text" @click="downloads.retry(task.taskId, 'proxy')" /></template>
            </v-tooltip>
            <v-tooltip v-if="!['completed', 'failed', 'cancelled'].includes(task.status)" text="取消">
              <template #activator="{ props }"><v-btn v-bind="props" icon="mdi-close" variant="text" @click="downloads.cancel(task.taskId)" /></template>
            </v-tooltip>
            <v-icon v-if="task.status === 'completed'" icon="mdi-check-circle" color="success" size="25" />
          </div>
        </article>
      </div>

      <EmptyState v-if="!visibleTasks.length && !(downloads.pendingPrepareCount && filter !== 'finished')" icon="mdi-tray-arrow-down" :title="filter === 'all' ? '还没有下载任务' : '这个分组是空的'" description="进入课程选择录像或文件，任务会自动出现在这里。">
        <v-btn v-if="filter === 'all'" to="/courses" color="primary">浏览课程</v-btn>
        <v-btn v-else variant="outlined" @click="filter = 'all'">查看全部</v-btn>
      </EmptyState>
    </section>

    <v-alert v-if="!downloads.supportsDirectoryApi" type="info" variant="tonal" class="mt-5" icon="mdi-information-outline">
      当前浏览器不支持选择保存文件夹，不会自动保存到默认目录。请使用支持目录访问的桌面 Chrome / Edge；VPS 站点需通过 HTTPS 访问。
    </v-alert>
  </div>
</template>
