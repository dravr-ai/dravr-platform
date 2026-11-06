// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Drives real queries through the app's own QueryClient and asserts what the athlete reads
// ABOUTME: A role 403 becomes the server's sentence; a refusal already being signed out stays silent;
// ABOUTME: an expired provider session is one translated reconnect toast however many reads it refused

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClientProvider, useQuery } from '@tanstack/react-query'
import { AxiosError, AxiosHeaders } from 'axios'
import type { AxiosInstance, AxiosResponse, InternalAxiosRequestConfig } from 'axios'
import { createAxiosClient } from '@pierre/api-client'
import { createWebAdapter } from '@pierre/api-client/adapters/web'
import { i18n } from '@pierre/i18n'
import { CONNECTIONS_ROUTE } from '../../constants/surfaceLayout'
import { AuthProvider } from '../../contexts/AuthContext'
import { ToastProvider } from '../../components/ui'
import { queryClient } from '../../services/queryClient'
import { useApiRefusalSurface } from '../useApiRefusalSurface'

const { mockAuthStorage } = vi.hoisted(() => ({
  mockAuthStorage: {
    setCsrfToken: vi.fn().mockResolvedValue(undefined),
    getCsrfToken: vi.fn().mockResolvedValue(null),
    setUser: vi.fn().mockResolvedValue(undefined),
    getUser: vi.fn().mockResolvedValue(null),
    clear: vi.fn().mockResolvedValue(undefined),
    getToken: vi.fn().mockResolvedValue(null),
    setToken: vi.fn().mockResolvedValue(undefined),
    removeToken: vi.fn().mockResolvedValue(undefined),
    getRefreshToken: vi.fn().mockResolvedValue(null),
    setRefreshToken: vi.fn().mockResolvedValue(undefined),
  },
}))

vi.mock('../../services/api', () => ({
  authApi: {
    beginSignIn: vi.fn(),
    completeSignIn: vi.fn(),
    logout: vi.fn().mockResolvedValue(undefined),
    getSession: vi.fn(),
  },
  adminApi: { endImpersonation: vi.fn() },
  userApi: { setTimezone: vi.fn().mockResolvedValue(undefined) },
  pierreApi: { adapter: { authStorage: mockAuthStorage } },
}))

const GROUP_REFUSAL = 'Group coaching requires a Professional or Enterprise plan'
const INSUFFICIENT_SCOPE_CHALLENGE =
  'Bearer resource_metadata="https://x/.well-known/oauth-protected-resource", ' +
  'error="insufficient_scope", scope="fitness:write"'

/**
 * A client that refuses everything, through the REAL shared interceptor.
 *
 * Not a hand-built `AxiosError`: whether this surface speaks depends on the mark
 * the interceptor leaves on a refusal it is already recovering, so a fabricated
 * rejection would test the assertion rather than the app. Only the wire is
 * stubbed.
 */
function refusingClient(
  status: number,
  data: unknown,
  headers: Record<string, string> = {},
): AxiosInstance {
  const client = createAxiosClient(createWebAdapter({ baseURL: 'http://api.test' }))
  client.defaults.adapter = (config: InternalAxiosRequestConfig) => {
    const response: AxiosResponse = {
      status,
      statusText: '',
      data,
      headers: AxiosHeaders.from(headers),
      config,
    }
    return Promise.reject(
      new AxiosError(
        `Request failed with status code ${status}`,
        AxiosError.ERR_BAD_REQUEST,
        config,
        null,
        response,
      ),
    )
  }
  return client
}

/** What a screen's `queryFn` is: one refused call through that client. */
function refusedCall(status: number, data: unknown, headers: Record<string, string> = {}) {
  const client = refusingClient(status, data, headers)
  return () => client.get('/api/anything')
}

function Surface() {
  useApiRefusalSurface()
  return null
}

function Probe({ id, queryFn }: { id: string; queryFn: () => Promise<unknown> }) {
  // retryDelay is the only default overridden: the retry PREDICATE under test
  // comes from the app's client, and waiting out exponential backoff would buy
  // the assertion nothing but seconds.
  const query = useQuery({ queryKey: [id], queryFn, retryDelay: 0 })
  return <div data-testid="query">{query.isError ? 'refused' : 'waiting'}</div>
}

function renderProbe(id: string, queryFn: () => Promise<unknown>) {
  return renderProbes([{ id, queryFn }])
}

/** Several screens' reads in flight at once, as a dashboard issues them. */
function renderProbes(probes: { id: string; queryFn: () => Promise<unknown> }[]) {
  return render(
    <QueryClientProvider client={queryClient}>
      <AuthProvider>
        <ToastProvider>
          <Surface />
          {probes.map(({ id, queryFn }) => (
            <Probe key={id} id={id} queryFn={queryFn} />
          ))}
        </ToastProvider>
      </AuthProvider>
    </QueryClientProvider>,
  )
}

/** The English sentence the server sends with an expired provider session. */
const PROVIDER_AUTH_ENGLISH = 'A connected fitness provider needs to be (re)authenticated'

/** The 403 every read of a provider answers once its session has expired. */
function providerAuthRefusal(provider: string) {
  return refusedCall(403, {
    code: 'ProviderAuthRequired',
    message: PROVIDER_AUTH_ENGLISH,
    details: { provider },
  })
}

describe('useApiRefusalSurface', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
    queryClient.clear()
  })

  afterEach(async () => {
    await i18n.changeLanguage('en')
    window.location.hash = ''
  })

  it('reads a role 403 back to the athlete in the server’s own words', async () => {
    const queryFn = vi.fn(refusedCall(403, { code: 'PermissionDenied', message: GROUP_REFUSAL }))

    renderProbe('role-403', queryFn)

    // The sentence, not "Request failed with status code 403", and under the
    // generic error title so the toast reads as one.
    expect(await screen.findByText(GROUP_REFUSAL)).toBeInTheDocument()
    expect(screen.getByText('Error')).toBeInTheDocument()

    // And asked exactly once: three more attempts at a refusal the server has
    // already answered only delay this sentence.
    await waitFor(() => {
      expect(screen.getByTestId('query')).toHaveTextContent('refused')
    })
    expect(queryFn).toHaveBeenCalledTimes(1)
  })

  it('says nothing for a 403 the transport is already recovering', async () => {
    // The real interceptor sees the challenge, clears the session and marks the
    // rejection on its way past. A permission toast on top of that would flash
    // at somebody already on their way to the login form.
    const queryFn = vi.fn(
      refusedCall(
        403,
        { code: 'InsufficientScope', message: 'Scope fitness:write required' },
        { 'www-authenticate': INSUFFICIENT_SCOPE_CHALLENGE },
      ),
    )

    renderProbe('scope-403', queryFn)

    await waitFor(() => {
      expect(screen.getByTestId('query')).toHaveTextContent('refused')
    })
    expect(screen.queryByText('Scope fitness:write required')).not.toBeInTheDocument()
    expect(screen.queryByText('Error')).not.toBeInTheDocument()
    expect(queryFn).toHaveBeenCalledTimes(1)
  })

  it('stays open for a role refusal after a scope refusal has been recovered', async () => {
    // The property that matters more than it looks: the surface decides per
    // refusal, from that refusal's own mark, and holds no latch. An earlier
    // implementation muted itself on the first sign-out and needed a successful
    // sign-in to speak again — so one expired session silenced every permission
    // message for the life of the page.
    renderProbe(
      'scope-first',
      vi.fn(
        refusedCall(
          403,
          { message: 'Scope fitness:write required' },
          { 'www-authenticate': INSUFFICIENT_SCOPE_CHALLENGE },
        ),
      ),
    )
    await waitFor(() => {
      expect(screen.getByTestId('query')).toHaveTextContent('refused')
    })
    expect(screen.queryByText('Error')).not.toBeInTheDocument()

    renderProbe(
      'role-after',
      vi.fn(refusedCall(403, { code: 'PermissionDenied', message: GROUP_REFUSAL })),
    )

    expect(await screen.findByText(GROUP_REFUSAL)).toBeInTheDocument()
  })

  it('leaves a server error to the screen that asked, and still retries it', async () => {
    const queryFn = vi.fn(() =>
      refusedCall(500, { message: 'Internal error: pool exhausted at db.rs:214' })(),
    )

    renderProbe('server-500', queryFn)

    await waitFor(() => {
      expect(screen.getByTestId('query')).toHaveTextContent('refused')
    })
    // React Query's own three retries, kept: a 5xx is the one failure a second
    // attempt genuinely fixes.
    expect(queryFn).toHaveBeenCalledTimes(4)
    // No global toast for anything but an authorization refusal — the ~15
    // screens that render their own error state would each be doubled.
    expect(screen.queryByText('Error')).not.toBeInTheDocument()
    expect(
      screen.queryByText('Internal error: pool exhausted at db.rs:214'),
    ).not.toBeInTheDocument()
  })

  it('shows five parallel reads refused by one expired session as one French reconnect toast', async () => {
    await i18n.changeLanguage('fr')
    const garminReads = Array.from({ length: 5 }, (_, i) => ({
      id: `garmin-route-${i}`,
      queryFn: vi.fn(providerAuthRefusal('sciotte_garmin')),
    }))

    renderProbes(garminReads)

    expect(await screen.findByText('Garmin doit être reconnecté.')).toBeInTheDocument()
    await waitFor(() => {
      expect(screen.getAllByTestId('query').every((probe) => probe.textContent === 'refused')).toBe(true)
    })
    // Every read was refused and reported, and the athlete reads it once.
    for (const { queryFn } of garminReads) {
      expect(queryFn).toHaveBeenCalledTimes(1)
    }
    expect(screen.getAllByText('Garmin doit être reconnecté.')).toHaveLength(1)
    expect(screen.getAllByText('Erreur')).toHaveLength(1)
    expect(screen.getAllByRole('button', { name: 'Reconnecter' })).toHaveLength(1)
    // Neither the server's English nor the backend's id reaches the screen.
    expect(screen.queryByText(PROVIDER_AUTH_ENGLISH)).not.toBeInTheDocument()
    expect(document.body.textContent).not.toContain('sciotte')
  })

  it('keeps the reconnect toast until the athlete acts on it, past the error toast timer', async () => {
    // Installed before the toast exists, so its timers run on this clock.
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      renderProbe('garmin-route', providerAuthRefusal('sciotte_garmin'))
      expect(await screen.findByText('Garmin needs reconnecting.')).toBeInTheDocument()

      // Well past the 5 s an error toast lasts by default.
      await act(async () => {
        vi.advanceTimersByTime(12_000)
      })

      expect(screen.getByText('Garmin needs reconnecting.')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Reconnect' })).toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })

  it('gives a second expired provider its own toast', async () => {
    renderProbes([
      { id: 'garmin-a', queryFn: providerAuthRefusal('sciotte_garmin') },
      { id: 'garmin-b', queryFn: providerAuthRefusal('garmin') },
      { id: 'strava-a', queryFn: providerAuthRefusal('strava') },
      { id: 'strava-b', queryFn: providerAuthRefusal('strava') },
    ])

    expect(await screen.findByText('Strava needs reconnecting.')).toBeInTheDocument()
    expect(await screen.findByText('Garmin needs reconnecting.')).toBeInTheDocument()
    await waitFor(() => {
      expect(screen.getAllByTestId('query').every((probe) => probe.textContent === 'refused')).toBe(true)
    })
    expect(screen.getAllByText('Strava needs reconnecting.')).toHaveLength(1)
    expect(screen.getAllByText('Garmin needs reconnecting.')).toHaveLength(1)
    expect(screen.getAllByRole('button', { name: 'Reconnect' })).toHaveLength(2)
  })

  it('says "a connected app" for a provider it cannot name, and never the raw id', async () => {
    await i18n.changeLanguage('fr')

    renderProbe('unknown-provider', providerAuthRefusal('sciotte_unheard_of'))

    expect(await screen.findByText('Une app connectée doit être reconnectée.')).toBeInTheDocument()
    expect(document.body.textContent).not.toContain('sciotte_unheard_of')
    expect(screen.queryByText(PROVIDER_AUTH_ENGLISH)).not.toBeInTheDocument()
  })

  it('takes the athlete to the connections pane from the reconnect action', async () => {
    renderProbe('reconnect-action', providerAuthRefusal('strava'))

    fireEvent.click(await screen.findByRole('button', { name: 'Reconnect' }))

    expect(window.location.hash).toBe(`#${CONNECTIONS_ROUTE}`)
    // The action answers the notice, so the notice goes with it.
    await waitFor(() => {
      expect(screen.queryByText('Strava needs reconnecting.')).not.toBeInTheDocument()
    })
  })
})
