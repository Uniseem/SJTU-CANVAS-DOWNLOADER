<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { api, getErrorMessage } from '@/services/api'
import { useSessionStore } from '@/stores/session'
import { useDownloadsStore } from '@/stores/downloads'
import type { Course } from '@/types'
import PageHeading from '@/components/PageHeading.vue'
import EmptyState from '@/components/EmptyState.vue'

const session = useSessionStore()
const downloads = useDownloadsStore()
const courses = ref<Course[]>([])
const loading = ref(true)
const error = ref<string | null>(null)

const greeting = computed(() => {
  const hour = new Date().getHours()
  if (hour < 6) return '夜深了'
  if (hour < 11) return '早上好'
  if (hour < 14) return '中午好'
  if (hour < 18) return '下午好'
  return '晚上好'
})

const activeCourseCount = computed(() => courses.value.filter((course) =>
  (course.enrollmentState ?? '').toLowerCase() === 'active').length)
const recentCourses = computed(() => courses.value.slice(0, 6))

async function load() {
  loading.value = true
  error.value = null
  try {
    courses.value = await api.courses()
  } catch (reason) {
    error.value = getErrorMessage(reason, '概览加载失败')
  } finally {
    loading.value = false
  }
}

onMounted(load)
</script>

<template>
  <div>
    <PageHeading
      eyebrow="Dashboard"
      :title="`${greeting}，${session.profile?.shortName || session.profile?.name || '同学'}`"
      description="课程资料和下载进度，都在这里。"
    >
      <template #actions>
        <v-btn to="/courses" color="primary" prepend-icon="mdi-magnify">查找课程资料</v-btn>
      </template>
    </PageHeading>

    <v-alert v-if="error" type="error" variant="tonal" class="mb-5" closable>
      {{ error }}
      <template #append><v-btn variant="text" size="small" @click="load">重试</v-btn></template>
    </v-alert>

    <template v-if="loading">
      <div class="stats-grid mb-6">
        <v-skeleton-loader v-for="index in 3" :key="index" type="article" class="skeleton-card" />
      </div>
      <div>
        <v-skeleton-loader type="table-heading, list-item-three-line@3" class="skeleton-card" />
      </div>
    </template>

    <template v-else-if="!error">
      <section class="stats-grid mb-6" aria-label="概览统计">
        <v-card to="/courses?state=active" class="stat-card stat-card--blue stat-card--link pa-5" aria-label="查看正在修读的课程">
          <div class="stat-icon"><v-icon icon="mdi-bookshelf" /></div>
          <div class="stat-value">{{ activeCourseCount }}</div>
          <div class="stat-label">正在修读</div>
          <div class="stat-note">当前修读课程的资料入口</div>
        </v-card>
        <v-card to="/courses" class="stat-card stat-card--link pa-5" aria-label="查看全部课程">
          <div class="stat-icon stat-icon--green"><v-icon icon="mdi-book-multiple-outline" /></div>
          <div class="stat-value">{{ courses.length }}</div>
          <div class="stat-label">全部课程</div>
          <div class="stat-note">包含历史学期和待接受课程</div>
        </v-card>
        <v-card to="/downloads" class="stat-card stat-card--link pa-5" aria-label="打开下载中心">
          <div class="stat-icon stat-icon--ink"><v-icon icon="mdi-tray-arrow-down" /></div>
          <div class="stat-value">{{ downloads.unfinishedCount }}</div>
          <div class="stat-label">进行中的下载</div>
          <div class="stat-note">{{ downloads.unfinishedCount ? `整体 ${downloads.overallProgress}%` : '队列空闲' }}</div>
        </v-card>
      </section>

      <div>
        <section class="panel-card">
          <div class="panel-head">
            <div>
              <div class="eyebrow">Courses</div>
              <h2>最近课程</h2>
            </div>
            <v-btn to="/courses" variant="text" color="primary" append-icon="mdi-arrow-right">全部课程</v-btn>
          </div>
          <div v-if="recentCourses.length" class="dashboard-course-list">
            <router-link
              v-for="(course, index) in recentCourses"
              :key="course.id"
              :to="`/courses/${course.id}`"
              class="dashboard-course-row"
            >
              <div class="course-index">{{ String(index + 1).padStart(2, '0') }}</div>
              <div class="course-line" :style="{ backgroundColor: course.color || undefined }"></div>
              <div class="course-row-copy">
                <strong>{{ course.name }}</strong>
                <span>{{ course.courseCode || '课程编号未提供' }}</span>
              </div>
              <v-icon icon="mdi-chevron-right" />
            </router-link>
          </div>
          <EmptyState v-else title="还没有课程" description="Canvas 中可见的课程会出现在这里。" icon="mdi-bookshelf" />
        </section>

      </div>

      <section v-if="downloads.unfinishedCount" class="active-download-banner mt-6">
        <div class="d-flex align-center ga-4 min-width-0">
          <div class="active-download-icon"><v-icon icon="mdi-download" /></div>
          <div class="min-width-0">
            <strong>{{ downloads.activeCount || downloads.queuedCount }} 个任务正在处理</strong>
            <p class="mb-0 text-truncate">离开页面也不会中断；保持本标签页打开即可。</p>
          </div>
        </div>
        <div class="download-banner-progress">
          <div><span>整体进度</span><strong>{{ downloads.overallProgress }}%</strong></div>
          <v-progress-linear :model-value="downloads.overallProgress" color="accent" bg-color="rgba(255,255,255,.18)" rounded height="7" />
        </div>
        <v-btn to="/downloads" color="accent" class="text-secondary">查看下载</v-btn>
      </section>
    </template>
  </div>
</template>
