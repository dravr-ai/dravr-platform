// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the web privacy pane: the analytics-consent switch reaches the session's user, and "Who read your data" lists and revokes
// ABOUTME: A failed consent write reverts; Revoke revokes every grant of that client and rereads the shared keys, never for a key or support

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { AccessFeed, OAuthGrant } from '@pierre/shared-types'
import { i18n } from '@pierre/i18n'
import {
  ACCESS_FEED_DAY_READS_KEYS,
  ACCESS_FEED_DAY_VIEWS_KEYS,
  ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS,
  ACCESS_FEED_KEYS,
  ACCESS_FEED_PEER_DAY_READS_KEYS,
  ACCESS_FEED_PEER_DAY_VIEWS_KEYS,
} from '@pierre/shared-constants'
import PrivacySettingsTab from '../PrivacySettingsTab'

const {
  mockUpdateAnalyticsConsent,
  mockUpdateUser,
  mockUseAuth,
  mockGetAccessFeed,
  mockListConnectedApps,
  mockRevokeConnectedApp,
  mockListStandingPolicies,
} = vi.hoisted(() => ({
  mockUpdateAnalyticsConsent: vi.fn(),
  mockUpdateUser: vi.fn(),
  mockUseAuth: vi.fn(),
  mockGetAccessFeed: vi.fn(),
  mockListConnectedApps: vi.fn(),
  mockRevokeConnectedApp: vi.fn(),
  mockListStandingPolicies: vi.fn(),
}))

vi.mock('../../services/api', () => ({
  userApi: { updateAnalyticsConsent: mockUpdateAnalyticsConsent },
  accessFeedApi: { getAccessFeed: mockGetAccessFeed },
  oauthApi: {
    listConnectedApps: mockListConnectedApps,
    revokeConnectedApp: mockRevokeConnectedApp,
    listStandingPolicies: mockListStandingPolicies,
  },
}))
vi.mock('../../hooks/useAuth', () => ({ useAuth: () => mockUseAuth() }))
// The two neighbouring sections read their own endpoints; they are not under test.
vi.mock('../AccountDeletionSection', () => ({ default: () => null }))
vi.mock('../settings/AiConsentSettings', () => ({ default: () => null }))
vi.mock('../settings/HealthRecordsSettings', () => ({ default: () => null }))
vi.mock('../dataAccess/DataAccessRequestsSection', () => ({ default: () => null }))

const EMPTY_FEED: AccessFeed = { app_reads: [], coach_days: [], peer_days: [], api_key_days: [], days: 30 }

function renderTab() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  const invalidate = vi.spyOn(client, 'invalidateQueries')
  const view = render(
    <QueryClientProvider client={client}>
      <PrivacySettingsTab />
    </QueryClientProvider>
  )
  return { ...view, invalidate }
}

function signedInWith(consent: boolean) {
  mockUseAuth.mockReturnValue({
    user: { id: 'user-1', email: 'athlete@example.com', analytics_consent: consent },
    updateUser: mockUpdateUser,
  })
}

describe('PrivacySettingsTab — analytics consent', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockGetAccessFeed.mockResolvedValue(EMPTY_FEED)
    mockListConnectedApps.mockResolvedValue([])
    mockListStandingPolicies.mockResolvedValue([])
  })

  it('seeds the switch from the session user', () => {
    signedInWith(true)
    renderTab()

    expect(screen.getByRole('switch', { name: 'Usage Analytics' })).toHaveAttribute('aria-checked', 'true')
  })

  it('writes a grant and puts it on the session user, so analytics start now', async () => {
    signedInWith(false)
    mockUpdateAnalyticsConsent.mockResolvedValue({ message: 'Analytics consent updated', enabled: true })
    renderTab()

    fireEvent.click(screen.getByRole('switch', { name: 'Usage Analytics' }))

    await waitFor(() => expect(mockUpdateUser).toHaveBeenCalledWith({ analytics_consent: true, analytics_consent_answered: true }))
    expect(mockUpdateAnalyticsConsent).toHaveBeenCalledWith(true)
    expect(screen.getByRole('switch', { name: 'Usage Analytics' })).toHaveAttribute('aria-checked', 'true')
  })

  it('writes a revoke and puts it on the session user, so analytics stop now', async () => {
    signedInWith(true)
    mockUpdateAnalyticsConsent.mockResolvedValue({ message: 'Analytics consent updated', enabled: false })
    renderTab()

    fireEvent.click(screen.getByRole('switch', { name: 'Usage Analytics' }))

    await waitFor(() => expect(mockUpdateUser).toHaveBeenCalledWith({ analytics_consent: false, analytics_consent_answered: true }))
    expect(mockUpdateAnalyticsConsent).toHaveBeenCalledWith(false)
  })

  it('reverts the switch and leaves the session user alone when the write fails', async () => {
    signedInWith(false)
    mockUpdateAnalyticsConsent.mockRejectedValue(new Error('network down'))
    renderTab()

    fireEvent.click(screen.getByRole('switch', { name: 'Usage Analytics' }))

    await waitFor(() => expect(screen.getByRole('switch', { name: 'Usage Analytics' })).toHaveAttribute('aria-checked', 'false'))
    expect(mockUpdateAnalyticsConsent).toHaveBeenCalledWith(true)
    expect(mockUpdateUser).not.toHaveBeenCalled()
  })
})

function grant(id: string, clientId: string, scope: string): OAuthGrant {
  return { id, client_id: clientId, client_name: 'Claude', client_host: 'claude.ai', scope, granted_at: '2026-11-01T10:00:00Z', preset: null, max_tier: null, expires_at: null }
}

const FEED: AccessFeed = {
  days: 30,
  app_reads: [
    {
      read_at: '2026-11-20T10:00:00Z',
      kind: 'tool_call',
      tool: 'get_activities',
      tools: ['get_activities'],
      client_id: 'client-claude',
      client_name: 'Claude',
      client_host: 'claude.ai',
      credential: 'oauth_grant',
      transport: 'mcp_http',
      support_read: false,
    },
    {
      read_at: '2026-11-19T09:00:00Z',
      kind: 'tool_call',
      tool: 'get_sleep_sessions',
      tools: ['get_sleep_sessions'],
      client_id: 'client-script',
      client_name: 'Export script',
      client_host: null,
      credential: 'api_key',
      transport: 'mcp_http',
      support_read: false,
    },
    {
      read_at: '2026-11-18T08:00:00Z',
      kind: 'tool_call',
      tool: 'get_athlete',
      tools: ['get_athlete'],
      client_id: 'client-claude',
      client_name: 'Admin console',
      client_host: 'admin.dravr.ai',
      credential: 'oauth_grant',
      transport: 'mcp_http',
      support_read: true,
    },
    {
      read_at: '2026-11-17T07:00:00Z',
      kind: 'agent_turn',
      tool: null,
      tools: ['get_activities', 'get_athlete'],
      client_id: 'client-bridge',
      client_name: 'Training bridge',
      client_host: null,
      credential: 'a2a_client_credentials',
      transport: 'a2a',
      support_read: false,
    },
  ],
  coach_days: [
    {
      day: '2026-11-20',
      coach_name: 'Marie',
      group_id: 'group-1',
      group_name: 'Run Club',
      data_class: 'training',
      reads: 3,
      views: 1,
      served_to_assistant: true,
      support_read: false,
    },
  ],
  peer_days: [
    {
      day: '2026-11-21',
      member_name: 'Alice',
      group_id: 'group-1',
      group_name: 'Run Club',
      reads: 2,
      views: 1,
      served_to_assistant: true,
      support_read: false,
    },
    {
      day: '2026-11-20',
      member_name: null,
      group_id: 'group-2',
      group_name: null,
      reads: 1,
      views: 0,
      served_to_assistant: true,
      support_read: true,
    },
    {
      day: '2026-11-19',
      member_name: 'Bob',
      group_id: 'group-1',
      group_name: 'Run Club',
      reads: 0,
      views: 3,
      served_to_assistant: false,
      support_read: false,
    },
  ],
  api_key_days: [
    { day: '2026-11-21', key_id: 'key-1', key_name: 'Garage sync', key_prefix: 'pk_live_ab12', requests: 4 },
    { day: '2026-11-20', key_id: 'key-1', key_name: 'Garage sync', key_prefix: 'pk_live_ab12', requests: 1 },
  ],
}

describe('PrivacySettingsTab — who read your data', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    signedInWith(false)
    mockGetAccessFeed.mockResolvedValue(FEED)
    mockListConnectedApps.mockResolvedValue([
      grant('g1', 'client-claude', 'fitness:read'),
      grant('g2', 'client-claude', 'fitness:write'),
      grant('g3', 'client-other', 'fitness:read'),
    ])
    mockRevokeConnectedApp.mockResolvedValue(undefined)
    mockListStandingPolicies.mockResolvedValue([])
  })

  it('lists each app with its calls, and the coach\'s day with its counts', async () => {
    renderTab()

    const apps = await screen.findAllByTestId('access-feed-app')
    expect(apps.map((app) => within(app).getByTestId('access-feed-app-name').textContent)).toEqual([
      'Claude',
      'Export script',
      'Training bridge',
    ])
    expect(within(apps[0]).getAllByTestId('access-feed-read').map((row) => row.dataset.tool)).toEqual(['get_activities'])
    expect(within(apps[0]).getByTestId('access-feed-read-tool').textContent).toBe(
      i18n.t('accessFeed.appRow', { tool: 'get_activities' }),
    )

    const day = screen.getByTestId('access-feed-coach-day')
    expect(within(day).getByTestId('access-feed-coach-name').textContent).toBe('Marie')
    // Three reads read the plural, one view the singular, both naming the group.
    expect(within(day).getByTestId('access-feed-coach-counts').textContent).toBe(
      [
        i18n.t(ACCESS_FEED_DAY_READS_KEYS.many, { count: 3, group: 'Run Club' }),
        i18n.t(ACCESS_FEED_DAY_VIEWS_KEYS.one, { count: 1, group: 'Run Club' }),
      ].join(' · '),
    )
    expect(within(day).getByTestId('access-feed-coach-served')).toBeInTheDocument()
  })

  it('revokes every grant of that client, then rereads the shared Connected-apps key and the feed', async () => {
    const { invalidate } = renderTab()

    const [claude] = await screen.findAllByTestId('access-feed-app')
    await waitFor(() => expect(within(claude).getByTestId('access-feed-revoke')).toBeInTheDocument())
    fireEvent.click(within(claude).getByTestId('access-feed-revoke'))
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Revoke' }))

    await waitFor(() => expect(mockRevokeConnectedApp).toHaveBeenCalledTimes(2))
    expect(mockRevokeConnectedApp.mock.calls.map(([id]) => id)).toEqual(['g1', 'g2'])
    await waitFor(() =>
      expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual(
        expect.arrayContaining([['oauth', 'connected-apps'], ['access-feed']]),
      ),
    )
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
  })

  // carnet#1544: an always-allowed app's policy renews the grant on its next
  // connection, so its confirmation says so; another app's keeps the plain line.
  it('warns that revoking an always-allowed app is undone on its next connection, and only that app', async () => {
    const confirmOf = async (policyClient: string) => {
      mockListStandingPolicies.mockResolvedValue([
        { id: 'p1', client_id: policyClient, client_name: 'App', client_host: null, preset: 'training_coach', created_at: '2026-11-02T10:00:00Z' },
      ])
      const view = renderTab()
      const [claude] = await screen.findAllByTestId('access-feed-app')
      await waitFor(() => expect(mockListStandingPolicies).toHaveBeenCalled())
      await waitFor(() => expect(within(claude).getByTestId('access-feed-revoke')).toBeInTheDocument())
      fireEvent.click(within(claude).getByTestId('access-feed-revoke'))
      const text = screen.getByRole('dialog').textContent
      view.unmount()
      mockListStandingPolicies.mockClear()
      return text
    }

    expect(await confirmOf('client-claude')).toContain(i18n.t('app.confirmRevokeAlwaysAllowedApp', { app: 'Claude' }))
    expect(await confirmOf('client-other')).toContain(i18n.t('app.confirmRevokeAppAccess', { app: 'Claude' }))
    expect(mockRevokeConnectedApp).not.toHaveBeenCalled()
  })

  it('offers no Revoke to an API key, which it points to its settings, nor to support, which it does not name', async () => {
    renderTab()

    const apps = await screen.findAllByTestId('access-feed-app')
    await waitFor(() => expect(within(apps[0]).getByTestId('access-feed-revoke')).toBeInTheDocument())
    expect(within(apps[1]).queryByTestId('access-feed-revoke')).toBeNull()
    expect(within(apps[1]).getByTestId('access-feed-key-hint')).toBeInTheDocument()

    const support = screen.getByTestId('access-feed-support')
    expect(within(support).queryByTestId('access-feed-revoke')).toBeNull()
    expect(support.textContent).not.toContain('Admin console')
    expect(within(support).getByTestId('access-feed-read').dataset.tool).toBe('get_athlete')
  })

  it('reads an agent turn as one row that names no tool', async () => {
    renderTab()

    const apps = await screen.findAllByTestId('access-feed-app')
    const [turn] = within(apps[2]).getAllByTestId('access-feed-read')
    expect(turn.dataset.kind).toBe('agent_turn')
    expect(turn.dataset.tool).toBeUndefined()
    expect(within(turn).getByTestId('access-feed-read-tool').textContent).toBe(i18n.t(ACCESS_FEED_KEYS.agentTurnRow))
    expect(turn.textContent).not.toContain('null')
  })

  it('lists each member\'s day, a gone member and group by their stand-ins, and a views-only day by name', async () => {
    renderTab()

    const days = await screen.findAllByTestId('access-feed-peer-day')
    expect(days.map((day) => within(day).getByTestId('access-feed-peer-line').textContent)).toEqual([
      i18n.t(ACCESS_FEED_PEER_DAY_READS_KEYS.many, { member: 'Alice', count: 2, group: 'Run Club' }),
      i18n.t(ACCESS_FEED_PEER_DAY_READS_KEYS.one, {
        member: i18n.t(ACCESS_FEED_KEYS.formerMember),
        count: 1,
        group: i18n.t('humanCoach.readLog.unknownGroup'),
      }),
      'Bob',
    ])
    expect(within(days[0]).getByTestId('access-feed-peer-views').textContent).toBe(
      i18n.t(ACCESS_FEED_PEER_DAY_VIEWS_KEYS.one, { count: 1, group: 'Run Club' }),
    )
    expect(within(days[1]).queryByTestId('access-feed-peer-views')).toBeNull()
    expect(within(days[2]).getByTestId('access-feed-peer-views').textContent).toBe(
      i18n.t(ACCESS_FEED_PEER_DAY_VIEWS_KEYS.many, { count: 3, group: 'Run Club' }),
    )
  })

  it('marks a member\'s day served to their AI tool, and support\'s day as support\'s alone', async () => {
    renderTab()

    const days = await screen.findAllByTestId('access-feed-peer-day')
    expect(within(days[0]).getByTestId('access-feed-peer-served').textContent).toBe(
      i18n.t(ACCESS_FEED_KEYS.peerServedToAssistant),
    )
    expect(within(days[0]).queryByTestId('access-feed-peer-support')).toBeNull()
    expect(within(days[1]).getByTestId('access-feed-peer-support').textContent).toBe(i18n.t(ACCESS_FEED_KEYS.peerSupportRead))
    expect(within(days[1]).queryByTestId('access-feed-peer-served')).toBeNull()
    expect(within(days[2]).queryByTestId('access-feed-peer-served')).toBeNull()
    expect(within(days[2]).queryByTestId('access-feed-peer-support')).toBeNull()
  })

  it('lists an API key\'s days under its name and prefix, pointing to its settings with no Revoke', async () => {
    renderTab()

    const [key] = await screen.findAllByTestId('access-feed-api-key')
    expect(within(key).getByTestId('access-feed-api-key-name').textContent).toBe('Garage sync')
    expect(key.textContent).toContain('pk_live_ab12')
    expect(key.textContent).toContain(i18n.t('accessFeed.credential.apiKey'))
    expect(within(key).getByTestId('access-feed-key-hint')).toBeInTheDocument()
    expect(within(key).queryByTestId('access-feed-revoke')).toBeNull()
    expect(within(key).getAllByTestId('access-feed-api-key-requests').map((count) => count.textContent)).toEqual([
      i18n.t(ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS.many, { count: 4 }),
      i18n.t(ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS.one, { count: 1 }),
    ])
  })

  it('lists a feed that holds only a member\'s reads instead of calling it empty', async () => {
    mockGetAccessFeed.mockResolvedValue({ ...EMPTY_FEED, peer_days: FEED.peer_days.slice(0, 1) })
    renderTab()

    expect(await screen.findByTestId('access-feed-peer-day')).toBeInTheDocument()
    expect(screen.queryByTestId('access-feed-empty')).toBeNull()
  })

  it('lists a feed that holds only an API key\'s days instead of calling it empty', async () => {
    mockGetAccessFeed.mockResolvedValue({ ...EMPTY_FEED, api_key_days: FEED.api_key_days })
    renderTab()

    expect(await screen.findByTestId('access-feed-api-key')).toBeInTheDocument()
    expect(screen.queryByTestId('access-feed-empty')).toBeNull()
  })

  it('says nothing read the data when every list is empty', async () => {
    mockGetAccessFeed.mockResolvedValue(EMPTY_FEED)
    renderTab()

    expect(await screen.findByTestId('access-feed-empty')).toBeInTheDocument()
    expect(screen.queryByTestId('access-feed-app')).toBeNull()
    expect(screen.queryByTestId('access-feed-coach-day')).toBeNull()
  })
})
