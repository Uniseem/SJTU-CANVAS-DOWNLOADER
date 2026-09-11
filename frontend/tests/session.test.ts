import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useSessionStore } from '../src/stores/session'

const mocks = vi.hoisted(() => ({
  session: vi.fn(), startQr: vi.fn(), refreshQr: vi.fn(), qrStatus: vi.fn(), logout: vi.fn(),
  advanceApiAuthEpoch: vi.fn(), unauthorized: undefined as undefined | (() => void),
}))
vi.mock('../src/services/api', () => ({
  api: mocks, advanceApiAuthEpoch: mocks.advanceApiAuthEpoch, isDemoMode: false,
  onApiUnauthorized: (handler: () => void) => { mocks.unauthorized = handler },
  getErrorMessage: (error: Error, fallback: string) => error?.message || fallback,
  ApiError: class ApiError extends Error { constructor(message: string, public status: number) { super(message) } },
}))
import { ApiError } from '../src/services/api'

class FakeEventSource extends EventTarget {
  static instances: FakeEventSource[] = []
  closed = false
  onmessage: ((event: Event) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  constructor(public url: string) { super(); FakeEventSource.instances.push(this) }
  close() { this.closed = true }
  message(payload: unknown, type = 'status') {
    const event = new MessageEvent(type, { data: JSON.stringify(payload) })
    this.dispatchEvent(event)
    if (type === 'message') this.onmessage?.(event)
    if (type === 'error') this.onerror?.(event)
  }
  disconnect() { const event = new Event('error'); this.dispatchEvent(event); this.onerror?.(event) }
}
const anonymous = { authenticated: false, profile: null }
const signedIn = { authenticated: true, profile: { id: '42', name: '测试' } }
const waiting = (revision = 2, id = 'attempt-1') => ({
  attemptId: id, generation: 1, revision, state: 'waiting', qrDataUrl: 'data:image/png;base64,test',
  expiresAt: new Date(Date.now() + 58000).toISOString(),
})
let store: ReturnType<typeof useSessionStore>
const flush = () => vi.advanceTimersByTimeAsync(0)

beforeEach(() => {
  vi.useFakeTimers()
  setActivePinia(createPinia())
  vi.stubGlobal('EventSource', FakeEventSource)
  FakeEventSource.instances = []
  mocks.session.mockReset().mockResolvedValue(anonymous)
  mocks.startQr.mockReset().mockResolvedValue({ attemptId: 'attempt-1' })
  mocks.refreshQr.mockReset().mockResolvedValue(undefined)
  mocks.qrStatus.mockReset().mockImplementation(async (id: string) => ({ attemptId: id, state: 'preparing', revision: 1, generation: 1 }))
  mocks.logout.mockReset().mockResolvedValue(undefined)
  store = useSessionStore()
})
afterEach(() => { store.leaveLoginPage(); vi.useRealTimers(); vi.unstubAllGlobals() })

describe('QR login recovery', () => {
  it('does not reset the new login on multiple late unauthorized requests', async () => {
    store.session = signedIn
    mocks.unauthorized!()
    store.enterLoginPage()
    await flush()
    FakeEventSource.instances[0].message(waiting())
    await flush()
    mocks.unauthorized!()
    mocks.unauthorized!()
    expect(store.login.stage).toBe('qr')
    expect(store.login.qrContent).toContain('data:image')
    expect(FakeEventSource.instances[0].closed).toBe(false)
    expect(mocks.startQr).toHaveBeenCalledTimes(1)
  })

  it('obtains QR through the status endpoint even if SSE delivers no events', async () => {
    mocks.qrStatus.mockResolvedValue(waiting())
    await store.beginQrLogin()
    await flush()
    expect(store.login.stage).toBe('qr')
    expect(mocks.qrStatus).toHaveBeenCalledTimes(1)
  })

  it('treats native SSE error as transport failure and recovers by polling', async () => {
    await store.beginQrLogin(); await flush()
    mocks.qrStatus.mockResolvedValue(waiting())
    FakeEventSource.instances[0].disconnect()
    await flush()
    expect(store.login.stage).toBe('qr')
    expect(FakeEventSource.instances[0].closed).toBe(false)
  })

  it('still displays an actual server error event', async () => {
    await store.beginQrLogin(); await flush()
    FakeEventSource.instances[0].message({ attemptId: 'attempt-1', state: 'error', message: '学校服务不可用', revision: 2 }, 'error')
    await flush()
    expect(store.login.stage).toBe('error')
    expect(store.login.message).toBe('学校服务不可用')
    expect(FakeEventSource.instances[0].closed).toBe(true)
  })

  it('reconnects on page reentry rather than retaining a dead QR transport', async () => {
    mocks.qrStatus.mockResolvedValue(waiting())
    store.enterLoginPage(); await flush()
    store.leaveLoginPage()
    expect(FakeEventSource.instances[0].closed).toBe(true)
    store.enterLoginPage(); await flush()
    expect(FakeEventSource.instances).toHaveLength(2)
    expect(store.login.stage).toBe('qr')
    expect(mocks.startQr).toHaveBeenCalledTimes(1)
  })

  it('ignores a delayed start response after leaving the login page', async () => {
    let resolve!: (value: { attemptId: string }) => void
    mocks.startQr.mockReturnValueOnce(new Promise((done) => { resolve = done }))
    store.enterLoginPage()
    store.leaveLoginPage()
    resolve({ attemptId: 'old' }); await flush()
    expect(FakeEventSource.instances).toHaveLength(0)
    expect(store.login.attemptId).toBeNull()
  })

  it('does not let a stale poll or previous attempt overwrite a newer QR event', async () => {
    let resolve!: (value: unknown) => void
    mocks.qrStatus.mockReturnValueOnce(new Promise((done) => { resolve = done }))
    await store.beginQrLogin()
    FakeEventSource.instances[0].message(waiting(3))
    resolve({ state: 'preparing', attemptId: 'attempt-1', generation: 1, revision: 1 })
    await flush()
    FakeEventSource.instances[0].message({ state: 'error', attemptId: 'wrong', revision: 50 })
    await flush()
    expect(store.login.stage).toBe('qr')
  })

  it('replaces finished actors when refresh returns 409', async () => {
    await store.beginQrLogin(); await flush()
    mocks.refreshQr.mockRejectedValueOnce(new ApiError('ended', 409))
    mocks.startQr.mockResolvedValueOnce({ attemptId: 'attempt-2' })
    mocks.qrStatus.mockResolvedValue(waiting(2, 'attempt-2'))
    await store.refreshQr(); await flush()
    expect(store.login.attemptId).toBe('attempt-2')
    expect(store.login.stage).toBe('qr')
    expect(mocks.startQr).toHaveBeenCalledTimes(2)
  })

  it('times out a blank QR instead of waiting indefinitely', async () => {
    await store.beginQrLogin(); await flush()
    await vi.advanceTimersByTimeAsync(46000)
    expect(store.login.stage).toBe('error')
    expect(store.login.message).toContain('超时')
    expect(FakeEventSource.instances[0].closed).toBe(true)
  })

  it('handles timed-out start requests with a retryable error', async () => {
    mocks.startQr.mockRejectedValueOnce(new DOMException('timeout', 'TimeoutError'))
    await store.beginQrLogin()
    expect(store.login.stage).toBe('error')
    await store.refreshQr(); await flush()
    expect(store.login.attemptId).toBe('attempt-1')
  })

  it('expires an unrefreshed QR locally', async () => {
    mocks.qrStatus.mockResolvedValue(waiting())
    await store.beginQrLogin(); await flush()
    await vi.advanceTimersByTimeAsync(64000)
    expect(store.login.stage).toBe('expired')
  })

  it('polling can complete authorization and stops all transports', async () => {
    mocks.session.mockResolvedValue(signedIn)
    mocks.qrStatus.mockResolvedValue({ state: 'authorized', attemptId: 'attempt-1', revision: 3, generation: 1 })
    await store.beginQrLogin(); await flush()
    expect(store.authenticated).toBe(true)
    expect(store.login.stage).toBe('success')
    expect(FakeEventSource.instances[0].closed).toBe(true)
    await vi.advanceTimersByTimeAsync(10000)
    expect(mocks.qrStatus).toHaveBeenCalledTimes(1)
  })

  it('never calls an unconfirmed local session a successful login', async () => {
    mocks.qrStatus.mockResolvedValue({ state: 'authorized', attemptId: 'attempt-1', revision: 3 })
    await store.beginQrLogin(); await flush()
    expect(store.login.stage).toBe('error')
    expect(store.authenticated).toBe(false)
  })

  it('does not restore stale session data after invalidation', async () => {
    store.session = signedIn
    let resolve!: (value: unknown) => void
    mocks.session.mockReturnValueOnce(new Promise((done) => { resolve = done }))
    const initializing = store.initialize(true)
    store.invalidate()
    resolve(signedIn)
    await initializing
    expect(store.authenticated).toBe(false)
  })
})
