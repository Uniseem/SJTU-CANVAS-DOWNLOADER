<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { usePreferencesStore } from '@/stores/preferences'
import { useDownloadsStore } from '@/stores/downloads'
import { useSessionStore } from '@/stores/session'
import { isDemoMode } from '@/services/api'
import PageHeading from '@/components/PageHeading.vue'

const preferences = usePreferencesStore()
const downloads = useDownloadsStore()
const session = useSessionStore()
const router = useRouter()
const signingOut = ref(false)
const saved = ref(false)

function flashSaved() {
  saved.value = true
  window.setTimeout(() => { saved.value = false }, 1400)
}

async function signOut() {
  signingOut.value = true
  try {
    await session.logout()
    await router.replace('/login')
  } finally {
    signingOut.value = false
  }
}
</script>

<template>
  <div>
    <PageHeading eyebrow="Preferences" title="设置" description="这些偏好只保存在当前设备，不会上传到服务器。">
      <template #actions>
        <v-btn variant="outlined" prepend-icon="mdi-restore" @click="preferences.reset(); flashSaved()">恢复默认</v-btn>
      </template>
    </PageHeading>

    <div class="settings-layout">
      <div class="settings-main">
        <section class="settings-section">
          <div class="settings-section-head">
            <div class="settings-icon"><v-icon icon="mdi-source-branch" /></div>
            <div><h2>下载线路</h2><p>决定每个新任务先从哪里获取数据。</p></div>
          </div>
          <v-radio-group v-model="preferences.downloadMode" hide-details class="route-options" @update:model-value="flashSaved">
            <label class="route-option" :class="{ selected: preferences.downloadMode === 'direct' }">
              <v-radio value="direct" />
              <div class="route-copy">
                <div><strong>浏览器直连</strong><v-chip size="x-small" color="success" variant="tonal">推荐</v-chip></div>
                <p>由当前设备直接拉取学校资源，流式写入本次选择的文件夹，不占用 VPS 下载带宽。</p>
              </div>
              <v-icon icon="mdi-laptop" />
            </label>
            <label class="route-option" :class="{ selected: preferences.downloadMode === 'proxy' }">
              <v-radio value="proxy" />
              <div class="route-copy">
                <div><strong>服务器代理</strong></div>
                <p>资源先经过你的 VPS。适合直连受限或跨域请求失败的情况。</p>
              </div>
              <v-icon icon="mdi-server-network" />
            </label>
          </v-radio-group>
          <v-switch
            v-model="preferences.fallbackToProxy"
            color="primary"
            inset
            hide-details
            class="setting-switch"
            @update:model-value="flashSaved"
          >
            <template #label>
              <div><strong>直连失败时自动使用代理</strong><p>直连受限时切换到服务器代理，仍保存到本次选择的同一目录。</p></div>
            </template>
          </v-switch>
        </section>

        <section class="settings-section">
          <div class="settings-section-head">
            <div class="settings-icon settings-icon--green"><v-icon icon="mdi-transit-connection-variant" /></div>
            <div><h2>并发与保存</h2><p>根据设备性能和网络状况调整。</p></div>
          </div>
          <div class="concurrency-control">
            <div><strong>同时下载</strong><p>建议家庭网络使用 2–4 个并发任务。</p></div>
            <div class="concurrency-stepper">
              <v-btn icon="mdi-minus" size="small" variant="outlined" :disabled="preferences.concurrency <= 1" @click="preferences.concurrency--; flashSaved()" />
              <strong>{{ preferences.concurrency }}</strong>
              <v-btn icon="mdi-plus" size="small" variant="outlined" :disabled="preferences.concurrency >= 6" @click="preferences.concurrency++; flashSaved(); downloads.schedule()" />
            </div>
          </div>
          <v-divider />
          <div class="directory-picker mt-5">
            <div class="d-flex align-center ga-3 min-width-0">
              <v-icon icon="mdi-folder-outline" color="primary" />
              <div class="min-width-0"><strong>每次下载都选择保存位置</strong><span>单项、批量和重新下载都会询问；暂停后继续仍使用原目录。</span></div>
            </div>
          </div>
          <p class="text-body-2 text-medium-emphasis mt-4">录像默认同时保存电脑屏幕、教室摄像头两个文件；可在课程中调整视角。同名文件自动添加序号，不覆盖原文件。</p>
          <pre class="download-folder-example">所选文件夹/
  课程名称 [课程编号]/
    课堂录像/
      日期_时间 讲次 [标识]/
        电脑屏幕.mp4
        教室摄像头.mp4
    课程文件/
      讲义.pdf</pre>
          <v-alert v-if="!downloads.supportsDirectoryApi" type="warning" variant="tonal" density="compact" class="mt-4">
            当前浏览器不支持选择文件夹，请使用支持目录访问的桌面 Chrome / Edge。VPS 需通过 HTTPS 访问，本地 localhost / 127.0.0.1 可直接测试。不会退回默认下载目录。
          </v-alert>
        </section>

        <section class="settings-section">
          <div class="settings-section-head">
            <div class="settings-icon settings-icon--orange"><v-icon icon="mdi-view-grid-outline" /></div>
            <div><h2>界面</h2><p>调整课程库的显示密度。</p></div>
          </div>
          <v-switch v-model="preferences.compactCourseCards" color="primary" inset hide-details class="setting-switch" @update:model-value="flashSaved">
            <template #label><div><strong>紧凑课程卡片</strong><p>在同一屏幕中显示更多课程。</p></div></template>
          </v-switch>
        </section>
      </div>

      <aside class="settings-aside">
        <section class="account-card">
          <div class="eyebrow">Account</div>
          <div class="account-avatar">{{ session.profile?.name?.slice(-2) || 'CP' }}</div>
          <h2>{{ session.profile?.name }}</h2>
          <p>{{ session.profile?.id }}</p>
          <v-chip v-if="isDemoMode || session.session.demo" size="small" color="warning" variant="tonal">演示会话</v-chip>
          <v-divider class="my-5" />
          <div class="account-security"><v-icon icon="mdi-shield-check-outline" color="success" /><span>会话已加密保存在服务器</span></div>
          <v-btn block variant="outlined" color="error" prepend-icon="mdi-logout" :loading="signingOut" class="mt-5" @click="signOut">退出登录</v-btn>
        </section>

        <section class="about-card mt-4">
          <div class="brand-mark mb-4" aria-hidden="true"><span>C</span><span>P</span></div>
          <strong>Canvas Pocket</strong>
          <p>一个为自托管设计的 SJTU Canvas 资料工作台。</p>
          <span>Web 0.1.0</span>
        </section>
      </aside>
    </div>

    <v-snackbar v-model="saved" color="secondary" location="bottom" timeout="1200">
      <v-icon icon="mdi-check" class="mr-2" />设置已保存在当前设备
    </v-snackbar>
  </div>
</template>
