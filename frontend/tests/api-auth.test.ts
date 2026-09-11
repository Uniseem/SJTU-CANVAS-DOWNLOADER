import { afterEach, expect, it, vi } from 'vitest'
import { advanceApiAuthEpoch, api, onApiUnauthorized } from '../src/services/api'
vi.mock('../src/services/demo', () => ({ demoApi: {} }))
afterEach(() => vi.unstubAllGlobals())

it.each([403, 409, 503])('keeps login on resource denial, stale response or verification outage (%s)', async (status) => {
  const notify = vi.fn()
  const unsubscribe = onApiUnauthorized(notify)
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(
    JSON.stringify({ error: { message: '当前资源暂不可用' } }), { status },
  )))
  try {
    await expect(api.files('94198')).rejects.toMatchObject({ status, message: '当前资源暂不可用' })
    expect(notify).not.toHaveBeenCalled()
  } finally { unsubscribe() }
})

it('coalesces parallel 401 notifications and ignores pre-login requests after a new login', async () => {
  const notify = vi.fn()
  const unsubscribe = onApiUnauthorized(notify)
  const responses: Array<(response: Response) => void> = []
  vi.stubGlobal('fetch', vi.fn(() => new Promise((resolve) => responses.push(resolve))))
  const first = api.courses().catch(() => {})
  const second = api.files('42').catch(() => {})
  const denial = () => new Response('{"message":"unauthorized"}', { status: 401 })
  responses[0](denial()); await first
  responses[1](denial()); await second
  expect(notify).toHaveBeenCalledTimes(1)
  const old = api.courses().catch(() => {})
  advanceApiAuthEpoch()
  responses[2](denial()); await old
  expect(notify).toHaveBeenCalledTimes(1)
  unsubscribe()
})
