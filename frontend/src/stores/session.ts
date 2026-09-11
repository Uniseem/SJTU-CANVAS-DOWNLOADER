import { computed, reactive, ref } from 'vue'
import { defineStore } from 'pinia'
import { advanceApiAuthEpoch, ApiError, api, getErrorMessage, isDemoMode, onApiUnauthorized } from '@/services/api'
import { demoApi } from '@/services/demo'
import type { LoginStage, LoginState, Profile, SessionView } from '@/types'

type LoginPayload = Record<string, unknown>
const initialLoginState = (): LoginState => ({
  attemptId: null, stage: 'idle', qrContent: null, message: '准备安全登录', expiresAt: null,
})
const terminalStages = new Set<LoginStage>(['success', 'expired', 'error'])

export const useSessionStore = defineStore('session', () => {
  const session = ref<SessionView>({ authenticated: false, profile: null })
  const initialized = ref(false)
  const loading = ref(false)
  const login = reactive<LoginState>(initialLoginState())
  let initPromise: Promise<void> | null = null
  let initVersion = 0
  let authVersion = 0
  let loginVersion = 0
  let loginPageActive = false
  let eventSource: EventSource | null = null
  let loginController: AbortController | null = null
  let pollTimer: ReturnType<typeof setTimeout> | undefined
  let phaseTimer: ReturnType<typeof setTimeout> | undefined
  let expiryTimer: ReturnType<typeof setTimeout> | undefined
  let lastRevision = -1
  let lastGeneration = -1
  let confirming = false
  let starting = false

  const authenticated = computed(() => session.value.authenticated)
  const profile = computed<Profile | null>(() => session.value.profile ?? null)

  async function initialize(force = false) {
    if (!force && initialized.value) return
    if (!force && initPromise) return initPromise
    const version = ++initVersion
    const owner = authVersion
    loading.value = true
    const pending = api.session().then((value) => {
      if (version !== initVersion || owner !== authVersion) return
      if (value.authenticated !== session.value.authenticated) advanceApiAuthEpoch()
      session.value = value
      initialized.value = true
    }).finally(() => {
      if (version === initVersion) { loading.value = false; initPromise = null }
    })
    initPromise = pending
    return pending
  }

  function closeEvents() {
    loginVersion += 1
    eventSource?.close()
    eventSource = null
    loginController?.abort()
    loginController = null
    clearTimeout(pollTimer)
    clearTimeout(phaseTimer)
    clearTimeout(expiryTimer)
    pollTimer = phaseTimer = expiryTimer = undefined
    confirming = false
    starting = false
  }

  function current(version: number) { return version === loginVersion }
  function signal(timeout: number) {
    const timeoutSignal = AbortSignal.timeout(timeout)
    return loginController ? AbortSignal.any([loginController.signal, timeoutSignal]) : timeoutSignal
  }
  function setLoginStage(stage: LoginStage, message: string) { login.stage = stage; login.message = message }
  function fail(message: string, stage: 'error' | 'expired' = 'error') {
    closeEvents()
    setLoginStage(stage, message)
  }
  function armPhaseTimeout(version: number, milliseconds = 45000) {
    clearTimeout(phaseTimer)
    phaseTimer = setTimeout(() => {
      if (current(version)) fail('获取或确认二维码超时，请点击“重新获取二维码”重试')
    }, milliseconds)
  }
  function getText(payload: LoginPayload, keys: string[]): string | null {
    for (const key of keys) {
      const value = payload[key]
      if (typeof value === 'string' && value.trim()) return value
    }
    const data = payload.data
    return data && typeof data === 'object' ? getText(data as LoginPayload, keys) : null
  }

  async function acceptLoginEvent(payload: LoginPayload, version: number, eventName = '') {
    if (!current(version) || confirming) return
    if (payload.attemptId && payload.attemptId !== login.attemptId) return
    const generation = typeof payload.generation === 'number' ? payload.generation : -1
    const revision = typeof payload.revision === 'number' ? payload.revision : -1
    if (generation >= 0 && generation < lastGeneration) return
    if (revision >= 0 && revision <= lastRevision) return
    if (generation >= 0) lastGeneration = generation
    if (revision >= 0) lastRevision = revision
    const stage = String(payload.state ?? payload.stage ?? payload.type ?? payload.status ?? eventName).toLowerCase().replace(/-/g, '_')
    const message = getText(payload, ['message', 'detail', 'hint'])
    const content = getText(payload, ['qrContent', 'qrDataUrl', 'qrUrl', 'loginUrl', 'url', 'qrCode', 'qrcode'])
    const expiresAt = getText(payload, ['expiresAt', 'expireAt'])

    if (['authorized', 'success', 'complete', 'completed', 'logged_in'].includes(stage)) {
      confirming = true
      clearTimeout(expiryTimer)
      armPhaseTimeout(version, 20000)
      setLoginStage('confirming', '扫码已确认，正在同步本站登录状态…')
      try {
        await initialize(true)
        if (!current(version)) return
        if (!authenticated.value) { fail('扫码状态已更新，但本站会话尚未登录，请重新获取二维码'); return }
        closeEvents()
        setLoginStage('success', '登录成功，正在进入工作台')
      } catch (error) {
        if (current(version)) fail(getErrorMessage(error, '无法确认登录状态，请重试'))
      }
      return
    }
    if (['expired', 'timeout', 'cancelled'].includes(stage)) {
      fail(message ?? '二维码已过期，请重新获取', 'expired')
      return
    }
    if (['error', 'failed', 'failure'].includes(stage)) {
      fail(message ?? '登录服务暂不可用，请重试')
      return
    }
    if (['preparing', 'starting', 'reconnecting', 'created'].includes(stage) && !content) {
      if (login.stage !== 'starting') armPhaseTimeout(version)
      login.qrContent = null
      login.expiresAt = null
      clearTimeout(expiryTimer)
      setLoginStage('starting', message ?? '正在获取二维码…')
      return
    }
    if (['authorizing', 'confirming', 'scanned'].includes(stage)) {
      clearTimeout(expiryTimer)
      armPhaseTimeout(version)
      setLoginStage(stage === 'scanned' ? 'scanned' : 'confirming', message ?? '已扫码，正在确认授权…')
      return
    }
    if (content) {
      login.qrContent = content
      login.expiresAt = expiresAt
      clearTimeout(phaseTimer)
      clearTimeout(expiryTimer)
      const deadline = expiresAt ? Date.parse(expiresAt) : Date.now() + 60000
      expiryTimer = setTimeout(() => {
        if (current(version) && login.stage === 'qr') fail('二维码已过期，未收到更新，请重新获取', 'expired')
      }, Math.max(0, (Number.isFinite(deadline) ? deadline : Date.now() + 60000) - Date.now() + 5000))
      setLoginStage('qr', message ?? '请扫码并在手机上确认')
    } else if (message) login.message = message
  }

  function connectEvents(attemptId: string, version: number) {
    let polling = false
    let lastHealthy = Date.now()
    async function poll() {
      if (!current(version) || polling || terminalStages.has(login.stage)) return
      clearTimeout(pollTimer)
      polling = true
      try {
        const payload = await api.qrStatus(attemptId, signal(8000))
        if (!current(version)) return
        lastHealthy = Date.now()
        await acceptLoginEvent(payload, version)
      } catch (error) {
        if (!current(version)) return
        if (error instanceof ApiError && [401, 403, 404, 410].includes(error.status)) {
          fail('扫码会话已失效，请重新获取二维码', 'expired')
        } else if (Date.now() - lastHealthy > 30000) {
          fail('无法读取扫码状态，请检查网络后重新获取二维码')
        }
      } finally {
        polling = false
        if (current(version) && !terminalStages.has(login.stage)) pollTimer = setTimeout(() => void poll(), 3000)
      }
    }
    try {
      const source = new EventSource(`/api/auth/qr/${encodeURIComponent(attemptId)}/events`, { withCredentials: true })
      eventSource = source
      const consume = (event: Event) => {
        // Native EventSource error is an Event without data, not a login failure.
        if (!current(version) || !('data' in event) || typeof event.data !== 'string') return
        try {
          const payload = JSON.parse(event.data) as LoginPayload
          lastHealthy = Date.now()
          void acceptLoginEvent(payload, version, event.type).catch(() => {
            if (current(version)) fail('扫码状态处理失败，请重试')
          })
        } catch { /* Ignore malformed events; polling can recover. */ }
      }
      source.onmessage = consume
      for (const name of ['created', 'qr', 'qr_ready', 'scanned', 'confirming', 'success', 'expired', 'error', 'status']) source.addEventListener(name, consume)
      source.onerror = (event) => {
        if (!current(version) || ('data' in event && typeof event.data === 'string')) return
        login.message = login.qrContent ? '二维码可继续扫码，正在通过备用通道同步状态…' : '正在通过备用通道获取二维码…'
        void poll()
      }
    } catch { /* EventSource unavailable: polling alone can complete login. */ }
    void poll()
  }

  async function beginQrLogin(refresh = false) {
    if (starting) return
    const previousId = login.attemptId
    const previousRevision = lastRevision
    const previousGeneration = lastGeneration
    closeEvents()
    const version = loginVersion
    loginController = new AbortController()
    starting = true
    Object.assign(login, initialLoginState(), { stage: 'starting', message: '正在向 jAccount 请求二维码…' })
    lastRevision = lastGeneration = -1
    armPhaseTimeout(version)
    try {
      if (isDemoMode) {
        login.attemptId = 'demo-attempt'
        login.qrContent = `https://jaccount.sjtu.edu.cn/demo-login?attempt=${Date.now()}`
        clearTimeout(phaseTimer)
        setLoginStage('qr', '演示模式：扫描或点击下方按钮完成登录')
        return
      }
      let id: string | undefined
      if (refresh && previousId) {
        try {
          const response = await api.refreshQr(previousId, signal(12000))
          if (!current(version)) return
          id = response?.attemptId ?? previousId
          if (id === previousId) { lastRevision = previousRevision; lastGeneration = previousGeneration }
        } catch (error) {
          if (!current(version)) return
          if (!(error instanceof ApiError) || ![404, 409, 410].includes(error.status)) throw error
        }
      }
      if (!id) id = (await api.startQr(signal(12000))).attemptId
      if (!current(version)) return
      if (!id) throw new Error('服务器没有返回扫码会话，请重试')
      login.attemptId = id
      connectEvents(id, version)
    } catch (error) {
      if (current(version)) fail((error as { name?: string }).name === 'TimeoutError'
        ? '创建二维码请求超时，请重新获取' : getErrorMessage(error, '无法创建登录二维码'))
    } finally {
      if (current(version)) starting = false
    }
  }

  function refreshQr() { return beginQrLogin(true) }
  function enterLoginPage() {
    loginPageActive = true
    if (authenticated.value || starting) return
    if (login.attemptId && !terminalStages.has(login.stage) && !isDemoMode) {
      closeEvents()
      loginController = new AbortController()
      armPhaseTimeout(loginVersion)
      lastRevision = -1
      connectEvents(login.attemptId, loginVersion)
    } else void beginQrLogin()
  }
  function leaveLoginPage() { loginPageActive = false; closeEvents() }

  async function completeDemoLogin() {
    if (!isDemoMode) return
    setLoginStage('confirming', '正在同步演示账户…')
    await demoApi.login()
    await initialize(true)
    setLoginStage('success', '登录成功，正在进入工作台')
  }

  async function logout() {
    closeEvents()
    authVersion += 1
    initVersion += 1
    advanceApiAuthEpoch()
    initPromise = null
    loading.value = true
    try {
      await api.logout()
      session.value = { authenticated: false, demo: isDemoMode, profile: null }
      Object.assign(login, initialLoginState())
    } finally { loading.value = false }
  }

  function invalidate() {
    // Once anonymous, late 401s must not reset a newly-started QR flow.
    if (!authenticated.value) return
    closeEvents()
    authVersion += 1
    initVersion += 1
    initPromise = null
    session.value = { authenticated: false, demo: isDemoMode, profile: null }
    initialized.value = true
    loading.value = false
    Object.assign(login, initialLoginState())
    if (loginPageActive) void beginQrLogin()
  }
  onApiUnauthorized(invalidate)

  return {
    session, initialized, loading, login, authenticated, profile, initialize,
    beginQrLogin, refreshQr, completeDemoLogin, logout, invalidate, closeEvents,
    enterLoginPage, leaveLoginPage,
  }
})
