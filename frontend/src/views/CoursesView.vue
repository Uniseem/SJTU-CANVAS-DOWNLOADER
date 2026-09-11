<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { api, getErrorMessage } from '@/services/api'
import { usePreferencesStore } from '@/stores/preferences'
import type { Course } from '@/types'
import PageHeading from '@/components/PageHeading.vue'
import EmptyState from '@/components/EmptyState.vue'

const preferences = usePreferencesStore()
const route = useRoute()
const courses = ref<Course[]>([])
const loading = ref(true)
const error = ref<string | null>(null)
const query = ref<string | null>('')
const supportedStates = new Set(['all', 'active', 'pending', 'completed'])

function routeStateFilter() {
  const requested = typeof route.query.state === 'string' ? route.query.state : 'all'
  return supportedStates.has(requested) ? requested : 'all'
}

const stateFilter = ref(routeStateFilter())

const colors = ['#214ee5', '#168568', '#c56b16', '#7b50c7', '#3177a9', '#c94545']
const states = [
  { title: '全部课程', value: 'all' },
  { title: '正在修读', value: 'active' },
  { title: '待接受', value: 'pending' },
  { title: '历史课程', value: 'completed' },
]

const filtered = computed(() => {
  const needle = (query.value ?? '').trim().toLocaleLowerCase()
  return courses.value.filter((course) => {
    const matchesQuery = !needle || [course.name, course.courseCode, course.teacher]
      .filter(Boolean)
      .some((value) => value!.toLocaleLowerCase().includes(needle))
    const normalizedState = (course.enrollmentState ?? 'active').toLowerCase()
    const matchesState = stateFilter.value === 'all'
      || (stateFilter.value === 'active' && normalizedState === 'active')
      || (stateFilter.value === 'pending' && ['invited_or_pending', 'invited', 'pending'].includes(normalizedState))
      || (stateFilter.value === 'completed' && ['completed', 'inactive'].includes(normalizedState))
    return matchesQuery && matchesState
  })
})

async function load() {
  loading.value = true
  error.value = null
  try {
    courses.value = await api.courses()
  } catch (reason) {
    error.value = getErrorMessage(reason, '课程列表加载失败')
  } finally {
    loading.value = false
  }
}

onMounted(load)

watch(() => route.query.state, () => {
  stateFilter.value = routeStateFilter()
})
</script>

<template>
  <div>
    <PageHeading eyebrow="Library" title="全部课程" description="查找课堂录像和课程文件，按课程整理到本地。">
      <template #actions>
        <v-btn
          :icon="preferences.compactCourseCards ? 'mdi-view-grid-outline' : 'mdi-view-agenda-outline'"
          variant="outlined"
          aria-label="切换课程卡片密度"
          :aria-pressed="preferences.compactCourseCards"
          @click="preferences.compactCourseCards = !preferences.compactCourseCards"
        />
      </template>
    </PageHeading>

    <div class="course-toolbar mb-6">
      <v-text-field
        v-model="query"
        prepend-inner-icon="mdi-magnify"
        placeholder="搜索课程名称、代码或教师"
        hide-details
        clearable
        class="course-search"
        aria-label="搜索课程"
      />
      <v-select
        v-model="stateFilter"
        :items="states"
        hide-details
        class="course-filter"
        aria-label="筛选课程状态"
      />
      <div class="result-count">
        {{ filtered.length === courses.length ? `共 ${courses.length} 门课程` : `显示 ${filtered.length} / 共 ${courses.length} 门` }}
      </div>
    </div>

    <v-alert v-if="error" type="error" variant="tonal" class="mb-5">
      {{ error }}
      <template #append><v-btn variant="text" size="small" @click="load">重试</v-btn></template>
    </v-alert>

    <div v-if="loading" class="course-grid">
      <v-skeleton-loader v-for="index in 6" :key="index" type="image, article" class="skeleton-card" />
    </div>

    <div v-else-if="filtered.length" class="course-grid" :class="{ 'course-grid--compact': preferences.compactCourseCards }">
      <router-link
        v-for="(course, index) in filtered"
        :key="course.id"
        :to="`/courses/${course.id}`"
        class="course-card"
        :class="{ 'course-card--compact': preferences.compactCourseCards }"
      >
        <div class="course-card-top" :style="{ '--course-color': course.color || colors[index % colors.length] }">
          <span class="course-code">{{ course.courseCode || 'CANVAS' }}</span>
          <v-icon icon="mdi-arrow-top-right" size="21" />
          <div class="course-monogram" aria-hidden="true">{{ String(index + 1).padStart(2, '0') }}</div>
        </div>
        <div class="course-card-body">
          <div class="course-term">{{ course.term || '当前学期' }}</div>
          <h2>{{ course.name }}</h2>
          <div class="course-meta">
            <span><v-icon icon="mdi-account-outline" size="16" />{{ course.teacher || '教师信息未提供' }}</span>
            <span v-if="course.fileCount !== undefined"><v-icon icon="mdi-file-outline" size="16" />{{ course.fileCount }} 份资料</span>
          </div>
        </div>
      </router-link>
    </div>

    <div v-else class="panel-card">
      <EmptyState
        icon="mdi-book-search-outline"
        title="没有找到匹配课程"
        description="换一个课程名称或代码试试，也可以查看全部课程。"
      >
        <v-btn variant="outlined" @click="query = ''; stateFilter = 'all'">清除筛选</v-btn>
      </EmptyState>
    </div>
  </div>
</template>
