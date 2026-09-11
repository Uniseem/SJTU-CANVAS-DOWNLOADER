<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import QRCode from 'qrcode'
import { useSessionStore } from '@/stores/session'
import { isDemoMode } from '@/services/api'
import type { LoginStage } from '@/types'

const session = useSessionStore()
const router = useRouter()
const route = useRoute()
const qrImage = ref<string | null>(null)
const qrRenderError = ref<string | null>(null)
let renderVersion = 0
let redirectTimer: ReturnType<typeof setTimeout> | undefined

const isBusy = computed(() => ['starting', 'confirming'].includes(session.login.stage))
const statusIcon = computed(() => ({
  idle: 'mdi-shield-key-outline',
  starting: 'mdi-loading',
  qr: 'mdi-qrcode-scan',
  scanned: 'mdi-cellphone-check',
  confirming: 'mdi-sync',
  success: 'mdi-check-circle',
  expired: 'mdi-clock-alert-outline',
  error: 'mdi-alert-circle-outline',
}[session.login.stage]))

const statusTone = computed(() => ({
  idle: 'primary',
  starting: 'primary',
  qr: 'primary',
  success: 'success',
  error: 'error',
  expired: 'warning',
  scanned: 'info',
  confirming: 'info',
} satisfies Record<LoginStage, string>)[session.login.stage])

watch(
  () => session.login.qrContent,
  async (content) => {
    const version = ++renderVersion
    qrRenderError.value = null
    if (!content) {
      qrImage.value = null
      return
    }
    if (content.startsWith('data:image/') || /^https?:\/\/[^\s]+\.(png|jpe?g|webp)(\?|$)/i.test(content)) {
      qrImage.value = content
      return
    }
    try {
      const image = await QRCode.toDataURL(content, {
        errorCorrectionLevel: 'M', margin: 2, width: 292,
        color: { dark: '#152027', light: '#ffffff' },
      })
      if (version === renderVersion) qrImage.value = image
    } catch {
      if (version === renderVersion) {
        qrImage.value = null
        qrRenderError.value = '二维码显示失败，请重新获取'
      }
    }
  },
  { immediate: true },
)

watch(
  () => session.authenticated,
  async (authenticated) => {
    if (!authenticated) return
    const destination = typeof route.query.redirect === 'string'
      && route.query.redirect.startsWith('/')
      && !route.query.redirect.startsWith('//')
      && !route.query.redirect.includes('\\')
      && !route.query.redirect.startsWith('/login')
      ? route.query.redirect
      : '/'
    clearTimeout(redirectTimer)
    redirectTimer = setTimeout(() => {
      if (session.authenticated) void router.replace(destination)
    }, 250)
  },
  { immediate: true },
)

onMounted(session.enterLoginPage)

onBeforeUnmount(() => {
  renderVersion += 1
  clearTimeout(redirectTimer)
  session.leaveLoginPage()
})
</script>

<template>
  <main class="login-page">
    <section class="login-story">
      <div class="login-story-inner">
        <div class="brand-lockup login-brand">
          <div class="brand-mark brand-mark-light" aria-hidden="true"><span>C</span><span>P</span></div>
          <div>
            <div class="brand-name">Canvas Pocket</div>
            <div class="brand-caption">Self-hosted · Your data</div>
          </div>
        </div>

        <div class="story-copy">
          <div class="story-kicker"><span></span> SJTU Canvas，收进口袋</div>
          <h1>把课程资料带走，<br />无需把账号交出去。</h1>
          <p>扫码建立临时会话。资料默认由你的浏览器直连下载，服务器只在你需要时提供代理。</p>
        </div>

        <div class="privacy-points">
          <div><v-icon icon="mdi-shield-lock-outline" /><span>会话加密保存在你的服务器</span></div>
          <div><v-icon icon="mdi-laptop" /><span>大文件优先直达当前设备</span></div>
          <div><v-icon icon="mdi-open-source-initiative" /><span>开源、自托管、可审计</span></div>
        </div>
      </div>
      <div class="story-index" aria-hidden="true">01 — ACCESS</div>
    </section>

    <section class="login-action">
      <div class="login-card-wrap">
        <div class="mobile-login-brand brand-lockup mb-8">
          <div class="brand-mark" aria-hidden="true"><span>C</span><span>P</span></div>
          <div>
            <div class="brand-name">Canvas Pocket</div>
            <div class="brand-caption">课程资料工作台</div>
          </div>
        </div>

        <div class="login-heading">
          <div class="eyebrow">安全登录</div>
          <h2>扫描二维码</h2>
          <p>使用交我办或微信扫码，并在手机端确认授权。</p>
        </div>

        <div class="qr-stage" :class="`qr-stage--${session.login.stage}`">
          <div v-if="qrImage && !['starting', 'confirming', 'success'].includes(session.login.stage)" class="qr-paper">
            <img :src="qrImage" alt="jAccount 登录二维码" width="232" height="232" @error="qrRenderError = '二维码图片未能加载，请重新获取'; qrImage = null" />
            <div v-if="session.login.stage === 'expired'" class="qr-overlay">
              <v-icon icon="mdi-clock-alert-outline" size="34" />
              <strong>二维码已过期</strong>
              <v-btn color="primary" size="small" @click="session.refreshQr">立即刷新</v-btn>
            </div>
          </div>
          <div v-else class="qr-placeholder">
            <v-progress-circular v-if="isBusy" indeterminate color="primary" size="46" width="4" />
            <v-icon v-else :icon="statusIcon" :color="statusTone" size="54" />
            <strong v-if="session.login.stage === 'success'">授权完成</strong>
            <strong v-else-if="session.login.stage === 'error'">暂时无法登录</strong>
          </div>
        </div>

        <div class="login-status" :class="`text-${statusTone}`" role="status" aria-live="polite">
          <v-icon :icon="statusIcon" :class="{ 'spin-icon': isBusy }" size="18" />
          <span>{{ qrRenderError || session.login.message }}</span>
        </div>

        <v-btn
          v-if="isDemoMode && session.login.stage === 'qr'"
          block
          color="primary"
          size="large"
          prepend-icon="mdi-cellphone-check"
          class="mt-5"
          @click="session.completeDemoLogin"
        >
          模拟扫码成功
        </v-btn>
        <v-btn
          v-else-if="session.login.stage !== 'success'"
          block
          color="primary"
          size="large"
          prepend-icon="mdi-refresh"
          class="mt-5"
          :disabled="isBusy"
          :loading="isBusy"
          @click="session.refreshQr"
        >
          重新获取二维码
        </v-btn>

        <div class="login-help mt-7">
          <v-icon icon="mdi-information-outline" size="17" />
          <span>扫码后，本页会自动继续，无需手动刷新。</span>
        </div>
      </div>
    </section>
  </main>
</template>
