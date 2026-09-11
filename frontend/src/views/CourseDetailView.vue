<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import type { ObjectDirective } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useDisplay } from 'vuetify'
import { ApiError, api, getErrorMessage } from '@/services/api'
import { useDownloadsStore } from '@/stores/downloads'
import { formatBytes, formatDate } from '@/utils/format'
import type { Course, CourseFile, DownloadRequestItem, Lesson, VideoTrackKind } from '@/types'
import { DEFAULT_VIDEO_TRACKS, VIDEO_TRACK_LABELS } from '@/utils/downloadLayout'
import { createLessonSizeLoader } from '@/utils/lessonSizes'
import EmptyState from '@/components/EmptyState.vue'

type TabName = 'lessons' | 'files'
type SectionName = TabName | 'course'
const route = useRoute()
const router = useRouter()
const { smAndDown } = useDisplay()
const downloads = useDownloadsStore()
const course = ref<Course | null>(null)
const files = ref<CourseFile[]>([])
const lessons = ref<Lesson[]>([])
const tab = ref<TabName>(route.query.tab === 'files' ? 'files' : 'lessons')
const query = ref<string | null>('')
const selected = ref<string[]>([])
const loading = ref(true)
const actionError = ref<string | null>(null)
const selectedTracks = ref<VideoTrackKind[]>([...DEFAULT_VIDEO_TRACKS])
const lessonRetrySeconds = ref(0)
const lessonNotice = ref(false)
let lessonRetryTimeout: number | undefined
let lessonRetryCountdown: number | undefined
let lessonAutoRetryUsed = false
const sectionErrors = reactive<Record<SectionName, string | null>>({
  course: null,
  lessons: null,
  files: null,
})
const sectionLoading = reactive<Record<SectionName, boolean>>({
  course: false,
  lessons: false,
  files: false,
})

const trackOptions: Array<{ title: string; value: VideoTrackKind; subtitle: string }> = [
  { title: VIDEO_TRACK_LABELS.slides, value: 'slides', subtitle: '默认' },
  { title: VIDEO_TRACK_LABELS.teacher, value: 'teacher', subtitle: '默认' },
  { title: '合成画面', value: 'composite', subtitle: '若课程提供' },
]

const courseId = computed(() => String(route.params.id))
const sizeLoader = createLessonSizeLoader(courseId.value, api.lessonSizes)
const observedLessons = new WeakMap<Element, string>()
let sizeObserver: IntersectionObserver | undefined
const vSizeVisible: ObjectDirective<HTMLElement, Lesson> = {
  mounted(element, { value: lesson }) {
    if (!lesson.available) return
    if (typeof IntersectionObserver === 'undefined') { sizeLoader.setVisible(lesson.videoId, true); return }
    sizeObserver ??= new IntersectionObserver((entries) => {
      entries.forEach((entry) => {
        const id = observedLessons.get(entry.target)
        if (id) sizeLoader.setVisible(id, entry.isIntersecting)
      })
    }, { rootMargin: '160px' })
    observedLessons.set(element, lesson.videoId)
    sizeObserver.observe(element)
  },
  unmounted(element, { value: lesson }) {
    sizeObserver?.unobserve(element)
    observedLessons.delete(element)
    sizeLoader.setVisible(lesson.videoId, false)
  },
}
function sizeView(lesson: Lesson) { return sizeLoader.summary(lesson.videoId, selectedTracks.value) }
watch(selectedTracks, (tracks) => sizeLoader.setTracks(tracks), { deep: true, immediate: true })
watch(tab, (value) => sizeLoader.setEnabled(value === 'lessons'), { immediate: true })

const filteredLessons = computed(() => filterBy(lessons.value, (item) => `${item.title} ${item.classroom}`))
const historicalLessonCount = computed(() => lessons.value.filter((item) => item.source === 'historical').length)
const filteredFiles = computed(() => filterBy(files.value, (item) => `${item.displayName} ${item.filename} ${item.contentType ?? ''}`))
const downloadableItems = computed(() => loading.value || sectionLoading[tab.value] || sectionErrors[tab.value]
  ? [] : tab.value === 'lessons' ? filteredLessons.value.filter((item) => item.available) : filteredFiles.value)

const allSelected = computed({
  get: () => downloadableItems.value.length > 0 && downloadableItems.value.every((item) => selected.value.includes(itemId(item))),
  set: (checked: boolean) => {
    const visible = new Set(downloadableItems.value.map(itemId))
    selected.value = checked ? [...new Set([...selected.value, ...visible])] : selected.value.filter((id) => !visible.has(id))
  },
})

const selectedCount = computed(() => selected.value.length)
const pendingTaskCount = computed(() => tab.value === 'lessons' ? selectedCount.value * selectedTracks.value.length : selectedCount.value)

function filterBy<T>(items: T[], value: (item: T) => string) {
  const needle = (query.value ?? '').trim().toLocaleLowerCase()
  return needle ? items.filter((item) => value(item).toLocaleLowerCase().includes(needle)) : items
}

function itemId(item: CourseFile | Lesson) {
  return 'videoId' in item ? item.videoId : item.id
}

function isSelected(id: string) {
  return selected.value.includes(id)
}

function toggle(id: string, checked: boolean | null) {
  selected.value = checked
    ? Array.from(new Set([...selected.value, id]))
    : selected.value.filter((value) => value !== id)
}

function requestFor(item: CourseFile | Lesson, track: VideoTrackKind = 'teacher'): DownloadRequestItem {
  if ('videoId' in item) {
    return {
      id: item.videoId,
      type: 'video',
      track,
      title: item.title,
      courseId: courseId.value,
      courseName: course.value?.name,
      beginTime: item.beginTime,
    }
  }
  return {
    id: item.id,
    type: 'file',
    filename: item.filename || item.displayName,
    courseId: courseId.value,
    courseName: course.value?.name,
  }
}

async function downloadOne(item: CourseFile | Lesson) {
  actionError.value = null
  try {
    const items = 'videoId' in item
      ? selectedTracks.value.map((track) => requestFor(item, track))
      : [requestFor(item)]
    await downloads.enqueue(items)
  } catch (reason) {
    actionError.value = getErrorMessage(reason, '无法创建下载')
  }
}

async function downloadSelected() {
  const selectedSet = new Set(selected.value)
  const source = tab.value === 'lessons' ? lessons.value : files.value
  const items = source.filter((item) => selectedSet.has(itemId(item))) as Array<CourseFile | Lesson>
  if (!items.length) return
  actionError.value = null
  try {
    const requests = tab.value === 'lessons'
      ? (items as Lesson[]).flatMap((item) => selectedTracks.value.map((track) => requestFor(item, track)))
      : items.map((item) => requestFor(item))
    if (await downloads.enqueue(requests)) selected.value = []
  } catch (reason) {
    actionError.value = getErrorMessage(reason, '无法创建批量下载')
  }
}

async function load() {
  loading.value = true
  clearLessonRetry()
  lessonAutoRetryUsed = false
  lessonNotice.value = false
  for (const key of Object.keys(sectionErrors) as SectionName[]) sectionErrors[key] = null
  const [courseResult, fileResult, lessonResult] = await Promise.allSettled([
    api.courses(),
    api.files(courseId.value),
    api.lessons(courseId.value),
  ])

  if (courseResult.status === 'fulfilled') {
    course.value = courseResult.value.find((item) => String(item.id) === courseId.value) ?? {
      id: courseId.value,
      name: '课程资料',
      courseCode: courseId.value,
    }
  } else {
    course.value = { id: courseId.value, name: '课程资料', courseCode: courseId.value }
    sectionErrors.course = getErrorMessage(courseResult.reason, '课程信息加载失败')
  }
  if (fileResult.status === 'fulfilled') files.value = fileResult.value
  else sectionErrors.files = getErrorMessage(fileResult.reason, '课程文件加载失败')
  if (lessonResult.status === 'fulfilled') lessons.value = lessonResult.value
  else setSectionError('lessons', lessonResult.reason, '课堂录像加载失败')
  loading.value = false
}

async function retrySection(section: SectionName, automatic = false) {
  if (section === 'lessons') {
    clearLessonRetry()
    lessonNotice.value = false
    if (!automatic) lessonAutoRetryUsed = false
  }
  sectionLoading[section] = true
  sectionErrors[section] = null
  try {
    if (section === 'course') {
      const courseList = await api.courses()
      course.value = courseList.find((item) => String(item.id) === courseId.value) ?? course.value
    } else if (section === 'files') {
      files.value = await api.files(courseId.value)
    } else if (section === 'lessons') {
      lessons.value = await api.lessons(courseId.value)
    }
  } catch (reason) {
    const fallback = section === 'course'
      ? '课程信息加载失败'
      : section === 'files'
        ? '课程文件加载失败'
        : '课堂录像加载失败'
    setSectionError(section, reason, fallback)
  } finally {
    sectionLoading[section] = false
  }
}

function setSectionError(section: SectionName, reason: unknown, fallback: string) {
  sectionErrors[section] = getErrorMessage(reason, fallback)
  if (section === 'lessons') lessonNotice.value = reason instanceof ApiError && reason.status === 422
  if (section === 'lessons'
    && reason instanceof ApiError
    && reason.status === 503
    && reason.retryAfterMs
    && !lessonAutoRetryUsed) {
    scheduleLessonRetry(reason.retryAfterMs)
  }
}

function scheduleLessonRetry(delayMs: number) {
  lessonAutoRetryUsed = true
  const retryAt = Date.now() + delayMs
  const updateCountdown = () => {
    lessonRetrySeconds.value = Math.max(1, Math.ceil((retryAt - Date.now()) / 1000))
  }
  updateCountdown()
  lessonRetryCountdown = window.setInterval(updateCountdown, 250)
  lessonRetryTimeout = window.setTimeout(() => {
    clearLessonRetry()
    void retrySection('lessons', true)
  }, delayMs)
}

function clearLessonRetry() {
  if (lessonRetryTimeout !== undefined) window.clearTimeout(lessonRetryTimeout)
  if (lessonRetryCountdown !== undefined) window.clearInterval(lessonRetryCountdown)
  lessonRetryTimeout = undefined
  lessonRetryCountdown = undefined
  lessonRetrySeconds.value = 0
}

watch(tab, (value) => {
  selected.value = []
  query.value = ''
  void router.replace({ query: { ...route.query, tab: value } })
})

onMounted(load)
onBeforeUnmount(clearLessonRetry)
onBeforeUnmount(() => { sizeObserver?.disconnect(); sizeLoader.dispose() })
</script>

<template>
  <div>
    <div class="detail-back mb-4">
      <v-btn to="/courses" variant="text" prepend-icon="mdi-arrow-left" size="small">返回课程</v-btn>
    </div>

    <section class="course-hero">
      <div>
        <div class="eyebrow mb-3">{{ course?.courseCode || 'Course' }}</div>
        <h1>{{ course?.name || (loading ? '正在读取课程…' : '课程资料') }}</h1>
        <div class="hero-meta mt-4">
          <span><v-icon icon="mdi-calendar-blank-outline" />{{ course?.term || '当前学期' }}</span>
          <span><v-icon icon="mdi-account-outline" />{{ course?.teacher || 'Canvas 课程' }}</span>
          <span><v-icon icon="mdi-folder-outline" />{{ files.length }} 份文件</span>
        </div>
      </div>
      <div class="hero-number" aria-hidden="true">{{ String(course?.name?.length ?? 0).padStart(2, '0') }}</div>
    </section>

    <v-alert v-if="sectionErrors.course" type="warning" variant="tonal" class="mt-5">
      课程标题暂未同步：{{ sectionErrors.course }}。录像和文件仍会独立加载。
      <template #append><v-btn variant="text" size="small" :loading="sectionLoading.course" @click="retrySection('course')">重试</v-btn></template>
    </v-alert>

    <section class="course-library mt-5">
      <v-tabs v-model="tab" color="primary" class="course-tabs" show-arrows>
        <v-tab value="lessons">
          <v-icon icon="mdi-video-outline" class="mr-2" />课堂录像
          <span class="tab-count">{{ loading || sectionLoading.lessons || sectionErrors.lessons ? '—' : lessons.length }}</span>
        </v-tab>
        <v-tab value="files">
          <v-icon icon="mdi-file-multiple-outline" class="mr-2" />课程文件
          <span class="tab-count">{{ files.length }}</span>
        </v-tab>
      </v-tabs>

      <div class="library-toolbar">
        <div class="select-all">
          <v-checkbox v-model="allSelected" :disabled="!downloadableItems.length" aria-label="全选当前列表" />
          <span>{{ selectedCount ? `已选择 ${selectedCount} 项` : '全选可下载内容' }}</span>
        </div>
        <v-select
          v-if="tab === 'lessons'"
          v-model="selectedTracks"
          :items="trackOptions"
          item-title="title"
          item-value="value"
          multiple
          chips
          closable-chips
          hide-details
          density="compact"
          label="录像分轨"
          class="track-select"
          :disabled="loading || !!sectionErrors.lessons || !downloadableItems.length"
          :rules="[(value) => value.length > 0]"
          @update:model-value="(value) => { if (!value?.length) selectedTracks = [...DEFAULT_VIDEO_TRACKS] }"
        >
          <template #chip="{ item, props }">
            <v-chip v-bind="props" size="small">{{ item.title }}</v-chip>
          </template>
        </v-select>
        <v-text-field
          v-model="query"
          prepend-inner-icon="mdi-magnify"
          :placeholder="tab === 'lessons' ? '搜索录像' : '搜索文件'"
          density="compact"
          hide-details
          clearable
          class="library-search"
        />
        <v-btn
          v-if="selectedCount"
          color="primary"
          prepend-icon="mdi-download-multiple"
          :loading="downloads.selectingDestination"
          @click="downloadSelected"
        >
          选择位置并下载 {{ pendingTaskCount }} 项
        </v-btn>
      </div>

      <p class="text-caption text-medium-emphasis px-5 pt-3 mb-0">
        {{ tab === 'lessons' ? '默认同时下载电脑屏幕和教室摄像头，每讲保存为两个独立视频。' : '' }}
        每批下载先选择文件夹；自动按课程{{ tab === 'lessons' ? '和讲次' : '' }}归档，同名文件保留副本。
      </p>
      <v-alert v-if="!downloads.supportsDirectoryApi" type="warning" variant="tonal" density="compact" class="ma-4">
        当前浏览器无法选择保存文件夹，请用支持目录访问的桌面 Chrome / Edge 打开本页。部署到 VPS 时需使用 HTTPS；未选择位置不会开始下载。
      </v-alert>
      <v-alert v-if="actionError" type="error" variant="tonal" density="compact" class="mx-4 mt-3" closable @click:close="actionError = null">
        {{ actionError }}
      </v-alert>

      <div v-if="loading" class="pa-4">
        <v-skeleton-loader type="list-item-avatar-three-line@5" />
      </div>

      <v-window v-else v-model="tab" :touch="false">
        <v-window-item value="lessons">
          <v-alert v-if="historicalLessonCount && !sectionLoading.lessons && !sectionErrors.lessons" type="info" variant="tonal" density="compact" class="ma-4">
            已自动回退“课堂视频旧版”，显示 {{ historicalLessonCount }} 条历史录像。
          </v-alert>
          <div v-if="sectionLoading.lessons" class="pa-4"><v-skeleton-loader type="list-item-avatar-three-line@4" /></div>
          <v-alert v-else-if="sectionErrors.lessons" :type="lessonNotice ? 'info' : 'error'" variant="tonal" class="ma-4">
            <div>
              {{ sectionErrors.lessons }}。这不会影响课程文件。
              <div v-if="lessonRetrySeconds" class="text-caption mt-1">服务器建议稍候，约 {{ lessonRetrySeconds }} 秒后自动重试一次。</div>
            </div>
            <template #append>
              <v-btn size="small" variant="text" :disabled="lessonRetrySeconds > 0" @click="retrySection('lessons')">
                {{ lessonRetrySeconds ? `${lessonRetrySeconds} 秒` : '重试录像' }}
              </v-btn>
            </template>
          </v-alert>
          <div v-else-if="filteredLessons.length" class="resource-list">
            <article v-for="lesson in filteredLessons" :key="lesson.videoId" v-size-visible="lesson" class="resource-row" :class="{ 'resource-row--disabled': !lesson.available }">
              <v-checkbox
                :model-value="isSelected(lesson.videoId)"
                :disabled="!lesson.available"
                :aria-label="`选择 ${lesson.title}`"
                @update:model-value="toggle(lesson.videoId, $event)"
              />
              <div class="resource-icon resource-icon--video"><v-icon icon="mdi-play" /></div>
              <div class="resource-main">
                <strong>{{ lesson.title }}</strong>
                <span>{{ formatDate(lesson.beginTime, true) }} · {{ lesson.classroom || '教室未知' }}</span>
                <span v-if="smAndDown && lesson.available" :title="sizeView(lesson).title">
                  <button v-if="sizeView(lesson).status === 'unavailable'" type="button" @click="sizeLoader.retry(lesson.videoId)">重试大小</button>
                  <template v-else>{{ sizeView(lesson).text }}</template>
                </span>
              </div>
              <div v-if="!smAndDown" class="resource-size" :title="sizeView(lesson).title" aria-live="polite">
                <button v-if="lesson.available && sizeView(lesson).status === 'unavailable'" type="button" :aria-label="`重新查询 ${lesson.title} 的大小`" @click="sizeLoader.retry(lesson.videoId)">重试大小</button>
                <template v-else>{{ lesson.available ? sizeView(lesson).text : '—' }}</template>
              </div>
              <v-chip v-if="!lesson.available" size="small" color="warning" variant="tonal">未开放或处理中</v-chip>
              <v-btn
                v-else
                icon="mdi-download-outline"
                variant="text"
                color="primary"
                :loading="downloads.selectingDestination"
                :aria-label="`下载 ${lesson.title}`"
                @click="downloadOne(lesson)"
              />
            </article>
          </div>
          <EmptyState
            v-else
            icon="mdi-video-off-outline"
            :title="query?.trim() ? '没有匹配的录像' : '视频平台暂未返回录像'"
            :description="query?.trim() ? '换个关键词，或清空搜索查看完整列表。' : '接口已成功响应，但当前可见列表为空。历史录像请在 Canvas 官网的“课堂视频旧版”入口核对；空列表不代表历史录像已被删除。'"
          >
            <v-btn v-if="query?.trim()" variant="tonal" @click="query = ''">清空搜索</v-btn>
            <v-btn v-else variant="tonal" prepend-icon="mdi-refresh" @click="retrySection('lessons')">重新获取</v-btn>
          </EmptyState>
        </v-window-item>

        <v-window-item value="files">
          <div v-if="sectionLoading.files" class="pa-4"><v-skeleton-loader type="list-item-avatar-three-line@4" /></div>
          <v-alert v-else-if="sectionErrors.files" type="error" variant="tonal" class="ma-4">
            {{ sectionErrors.files }}。这不会影响课堂录像。
            <template #append><v-btn size="small" variant="text" @click="retrySection('files')">重试文件</v-btn></template>
          </v-alert>
          <div v-else-if="filteredFiles.length" class="resource-list">
            <article v-for="file in filteredFiles" :key="file.id" class="resource-row">
              <v-checkbox
                :model-value="isSelected(file.id)"
                :aria-label="`选择 ${file.displayName}`"
                @update:model-value="toggle(file.id, $event)"
              />
              <div class="resource-icon"><v-icon :icon="file.contentType?.includes('pdf') ? 'mdi-file-pdf-box' : file.contentType?.includes('zip') ? 'mdi-folder-zip-outline' : 'mdi-file-outline'" /></div>
              <div class="resource-main">
                <strong>{{ file.displayName || file.filename }}</strong>
                <span>{{ file.filename }} · 更新于 {{ formatDate(file.updatedAt) }}</span>
              </div>
              <div v-if="!smAndDown" class="resource-size">{{ formatBytes(file.size) }}</div>
              <v-btn
                icon="mdi-download-outline"
                variant="text"
                color="primary"
                :loading="downloads.selectingDestination"
                :aria-label="`下载 ${file.displayName}`"
                @click="downloadOne(file)"
              />
            </article>
          </div>
          <EmptyState v-else icon="mdi-file-search-outline" title="没有找到文件" description="清除搜索条件，或稍后等待教师发布资料。" />
        </v-window-item>

      </v-window>
    </section>

    <v-snackbar :model-value="selectedCount > 0 && smAndDown" location="bottom" :timeout="-1" color="secondary" class="selection-snackbar">
      已选择 {{ selectedCount }} 讲 · 将创建 {{ pendingTaskCount }} 个任务
      <template #actions>
        <v-btn color="accent" variant="flat" :loading="downloads.selectingDestination" @click="downloadSelected">选择保存位置</v-btn>
      </template>
    </v-snackbar>

    <v-snackbar
      :model-value="Boolean(downloads.prepareError)"
      location="top"
      color="warning"
      :timeout="6000"
      @update:model-value="(value) => { if (!value) downloads.prepareError = null }"
    >
      <v-icon icon="mdi-alert-outline" class="mr-2" />{{ downloads.prepareError }}
      <template #actions><v-btn variant="text" @click="downloads.prepareError = null">知道了</v-btn></template>
    </v-snackbar>
  </div>
</template>
