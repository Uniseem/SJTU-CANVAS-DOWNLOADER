<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useDisplay } from 'vuetify'
import { useSessionStore } from '@/stores/session'
import { useDownloadsStore } from '@/stores/downloads'
import { initials } from '@/utils/format'

const route = useRoute()
const router = useRouter()
const session = useSessionStore()
const downloads = useDownloadsStore()
const { mdAndUp } = useDisplay()
const drawer = ref(true)

const standalone = computed(() => Boolean(route.meta.public))
const pageTitle = computed(() => String(route.meta.title ?? 'Canvas Pocket'))

const navigation = [
  { label: '概览', icon: 'mdi-view-dashboard-outline', activeIcon: 'mdi-view-dashboard', to: '/' },
  { label: '课程', icon: 'mdi-bookshelf', activeIcon: 'mdi-bookshelf', to: '/courses' },
  { label: '下载', icon: 'mdi-tray-arrow-down', activeIcon: 'mdi-tray-arrow-down', to: '/downloads', badge: true },
  { label: '设置', icon: 'mdi-tune-variant', activeIcon: 'mdi-tune-variant', to: '/settings' },
]

function isActive(path: string) {
  if (path === '/') return route.path === '/'
  return route.path.startsWith(path)
}

watch(
  () => session.authenticated,
  (authenticated, wasAuthenticated) => {
    if (!authenticated && wasAuthenticated) {
      downloads.cancelPending()
      downloads.tasks.forEach((task) => downloads.cancel(task.taskId))
    }
    if (authenticated || !wasAuthenticated || standalone.value || !session.initialized) return
    void router.replace({ name: 'login', query: { redirect: route.fullPath } })
  },
)

async function signOut() {
  await session.logout()
  await router.replace('/login')
}
</script>

<template>
  <v-app>
    <template v-if="standalone">
      <router-view />
    </template>

    <template v-else>
      <v-navigation-drawer
        v-if="mdAndUp"
        v-model="drawer"
        permanent
        :width="260"
        class="app-drawer"
      >
        <div class="brand-lockup px-5 pt-6 pb-8">
          <div class="brand-mark" aria-hidden="true"><span>C</span><span>P</span></div>
          <div>
            <div class="brand-name">Canvas Pocket</div>
            <div class="brand-caption">课程资料工作台</div>
          </div>
        </div>

        <nav class="px-3" aria-label="主导航">
          <v-list bg-color="transparent" class="pa-0" nav>
            <v-list-item
              v-for="item in navigation"
              :key="item.to"
              :to="item.to"
              :active="isActive(item.to)"
              class="nav-item mb-1"
              rounded="lg"
            >
              <template #prepend>
                <v-icon :icon="isActive(item.to) ? item.activeIcon : item.icon" size="21" />
              </template>
              <v-list-item-title>{{ item.label }}</v-list-item-title>
              <template v-if="item.badge && downloads.unfinishedCount" #append>
                <span class="nav-count">{{ downloads.unfinishedCount }}</span>
              </template>
            </v-list-item>
          </v-list>
        </nav>

        <template #append>
          <div class="drawer-foot ma-4 pa-4">
            <div class="d-flex align-center ga-3">
              <v-avatar size="38" color="secondary" class="profile-avatar">
                <v-img v-if="session.profile?.avatarUrl" :src="session.profile.avatarUrl" cover />
                <span v-else>{{ initials(session.profile?.name) }}</span>
              </v-avatar>
              <div class="min-width-0">
                <div class="text-body-2 font-weight-bold text-truncate">{{ session.profile?.name }}</div>
                <div class="profile-id text-truncate">{{ session.profile?.id }}</div>
              </div>
              <v-menu location="top end">
                <template #activator="{ props }">
                  <v-btn v-bind="props" icon="mdi-dots-horizontal" variant="text" size="small" aria-label="账户菜单" />
                </template>
                <v-list density="compact" min-width="160">
                  <v-list-item prepend-icon="mdi-logout" title="退出登录" @click="signOut" />
                </v-list>
              </v-menu>
            </div>
          </div>
        </template>
      </v-navigation-drawer>

      <v-app-bar flat class="app-topbar" :height="mdAndUp ? 72 : 62">
        <template v-if="!mdAndUp" #prepend>
          <div class="mobile-brand ml-3" aria-label="Canvas Pocket">CP</div>
        </template>
        <v-app-bar-title class="topbar-title">{{ pageTitle }}</v-app-bar-title>
        <template #append>
          <v-btn to="/downloads" icon class="mr-1" aria-label="打开下载中心">
            <v-badge v-if="downloads.unfinishedCount" :content="downloads.unfinishedCount" color="primary">
              <v-icon icon="mdi-tray-arrow-down" />
            </v-badge>
            <v-icon v-else icon="mdi-tray-arrow-down" />
          </v-btn>
          <v-menu>
            <template #activator="{ props }">
              <v-btn v-bind="props" icon class="mr-3" aria-label="账户菜单">
                <v-avatar size="34" color="secondary">
                  <v-img v-if="session.profile?.avatarUrl" :src="session.profile.avatarUrl" cover />
                  <span class="avatar-initials">{{ initials(session.profile?.name) }}</span>
                </v-avatar>
              </v-btn>
            </template>
            <v-list density="compact" min-width="176">
              <v-list-item prepend-icon="mdi-account-outline" :title="session.profile?.name" :subtitle="session.profile?.id" />
              <v-divider class="my-1" />
              <v-list-item prepend-icon="mdi-logout" title="退出登录" @click="signOut" />
            </v-list>
          </v-menu>
        </template>
      </v-app-bar>

      <v-main class="app-main">
        <div class="page-wrap">
          <router-view v-slot="{ Component }">
            <transition name="page" mode="out-in">
              <component :is="Component" :key="route.path" />
            </transition>
          </router-view>
        </div>
      </v-main>

      <v-bottom-navigation v-if="!mdAndUp" grow class="mobile-nav" :model-value="route.path">
        <v-btn
          v-for="item in navigation"
          :key="item.to"
          :value="item.to"
          :to="item.to"
          :aria-label="item.label"
        >
          <v-badge v-if="item.badge && downloads.unfinishedCount" :content="downloads.unfinishedCount" color="primary" offset-x="-6">
            <v-icon :icon="item.icon" />
          </v-badge>
          <v-icon v-else :icon="item.icon" />
          <span>{{ item.label }}</span>
        </v-btn>
      </v-bottom-navigation>
    </template>
  </v-app>
</template>
