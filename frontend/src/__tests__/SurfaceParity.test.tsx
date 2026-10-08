// ABOUTME: Asserts web implements every surface the shared registry declares for it
// ABOUTME: Mounts the real Dashboard at each declared route and the real chat renderer on each declared block

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, act, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReplyBlock } from '@pierre/shared-types';
import { SURFACE_CAPABILITIES, USER_SURFACES, surfacesFor } from '@pierre/shared-constants';
import Dashboard from '../components/Dashboard';
import MessageItem from '../components/chat/MessageItem';
import type { Message } from '../components/chat/types';

/**
 * Mobile has had this test since the registry existed. Web never did — a grep
 * for `USER_SURFACES` found consumers in the package barrel and the mobile test
 * and nowhere else, so the registry's `web` column was enforced by nobody and a
 * route could be declared here, linked from a notification, and simply not
 * exist.
 *
 * Like the mobile one, this is not a diff between the two clients. It checks
 * one client against the single registry: adding a surface means editing the
 * registry first, and this says whether web has caught up.
 *
 * It mounts the Dashboard at each route and the renderer on each block. A
 * search of their source for `activeTab === '<route>'` and `case '<kind>':`
 * passes a route the build gates out and an arm that draws nothing, and fails
 * on a rename.
 */
const auth = vi.hoisted(() => ({ role: 'user' }));

/**
 * What the Dashboard mounts at each declared web route, replaced by a marker.
 * The surfaces themselves have their own tests; this one asks only whether
 * the route reaches one. A route the registry gains needs its entry here,
 * which is the point: someone has to say what serves it.
 */
const SERVED_BY: Record<string, string> = {
  home: 'surface-home',
  chat: 'surface-chat',
  discover: 'surface-discover',
  notifications: 'surface-notifications',
  usage: 'surface-usage',
  users: 'surface-users',
};

// Home is ChatTab's personal layout; the Groups tab (`chat`) is its shell.
vi.mock('../components/home/Home', () => ({ HomeBriefing: () => null }));
vi.mock('../components/home/TodayPeek', () => ({ TodayPeek: () => null }));
vi.mock('../components/ChatTab', () => ({
  default: ({ layout }: { layout?: string }) => (
    <div data-testid={layout === 'personal' ? 'surface-home' : 'surface-chat'} />
  ),
}));
// Home opens on the latest personal thread; these specs assert nothing about
// which one, and no #chat link here names a thread the list knows.
vi.mock('../hooks/useConversationList', () => ({
  useLatestPersonalConversation: () => ({ id: null, isLoading: false }),
  useConversationScope: () => null,
}));

vi.mock('../components/StoreScreen', () => ({ default: () => <div data-testid="surface-discover" /> }));
vi.mock('../components/notifications/NotificationsPanel', () => ({
  default: () => <div data-testid="surface-notifications" />,
}));
vi.mock('../components/BillingPage', () => ({ default: () => <div data-testid="surface-usage" /> }));
vi.mock('../components/UserManagement', () => ({ default: () => <div data-testid="surface-users" /> }));

// Usage is the registry's one build-gated row (its `why` says so): the gate
// ships false, and the surface must exist behind it for the day it opens. The
// registry is checked whole with the gate open; the closed state has its own
// test below. Which state a build ships in is not decided here: this mock
// stands in for the constant, and billingGate.test.ts evaluates the real one.
const gate = vi.hoisted(() => ({ billing: true }));
vi.mock('../constants/features', () => ({
  get BILLING_ENABLED() {
    return gate.billing;
  },
}));

vi.mock('../components/dashboard/index', () => ({
  ConversationList: () => null,
  useUnreadConversationsCount: () => 0,
  usePendingUsersCount: () => 0,
  useStoreStatsPendingCount: () => 0,
}));
vi.mock('../hooks/useNotifications', () => ({
  useUnreadCount: () => ({ unreadCount: 0, isLoading: false }),
}));
vi.mock('../components/ConnectProviderBanner', () => ({ ConnectProviderBanner: () => null }));
vi.mock('../hooks/useAuth', () => ({
  useAuth: () => ({
    user: { id: 'u-1', email: 'alice@acme.com', display_name: 'Alice', role: auth.role },
    logout: vi.fn(),
    isAuthenticated: true,
    isLoading: false,
  }),
}));
vi.mock('../services/api', () => ({
  // Read by the shared hook bindings at import; this spec asserts nothing they fetch.
  featureFlagsApi: {},
}));
vi.mock('../services/analytics', () => ({ track: vi.fn() }));

async function renderDashboardAt(hash: string, role: string) {
  auth.role = role;
  window.history.replaceState(null, '', hash === '' ? '/' : `/#${hash}`);
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  await act(async () => {
    render(
      <QueryClientProvider client={queryClient}>
        <Dashboard />
      </QueryClientProvider>,
    );
  });
}

const CHAT_MESSAGE: Message = {
  id: 'msg-1',
  role: 'assistant',
  content: 'Your load is climbing.',
  created_at: '2026-08-24T10:00:00Z',
};

const SCENE_BLOCKS = JSON.stringify([
  {
    kind: 'chart',
    view_box: { x: 0, y: 0, width: 320, height: 180 },
    nodes: [],
    legend: [],
    title: 'Weekly volume',
    source_tool: 'get_activities',
  },
]);

/**
 * One reply per block kind, and what the athlete must be able to see of it.
 *
 * `scene` is drawn where the prose's own marker puts it, so its reply carries
 * the prose that positions it. `notice` is the one kind the message draws
 * nothing for by decision — the conversation's usage banner owns it — so what
 * is asserted is that the reply around it still renders and the figure does
 * not appear twice.
 */
const BLOCK_CASES: Record<string, { blocks: ReplyBlock[]; shows: () => void }> = {
  prose: {
    blocks: [{ type: 'prose', text: 'Nice negative split.' }],
    shows: () => expect(screen.getByText('Nice negative split.')).toBeInTheDocument(),
  },
  activity_list: {
    blocks: [{ type: 'activity_list', text: '1. Long run - 24 km\n2. Threshold 3x10 - 14 km' }],
    shows: () => expect(screen.getByText('Your Activities (2)')).toBeInTheDocument(),
  },
  workout_plan: {
    blocks: [
      {
        type: 'workout_plan',
        plan: {
          goal_race: { name: 'Parkrun PB', date: '2026-11-14', discipline: 'run_5k', priority: 'A' },
          phases: [],
          weeks: [],
          weeks_deferred: 0,
        },
      },
    ],
    shows: () => expect(screen.getByText('Parkrun PB')).toBeInTheDocument(),
  },
  scene: {
    blocks: [
      { type: 'prose', text: 'Here is your month.\n\n⟦viz:0⟧' },
      { type: 'scene', scene_blocks: SCENE_BLOCKS },
    ],
    shows: () => expect(screen.getByRole('img', { name: 'Chart: Weekly volume' })).toBeInTheDocument(),
  },
  verdicts: {
    blocks: [
      { type: 'prose', text: 'Your VO2max is 82.' },
      { type: 'verdicts', chips: [{ claim: 'Your VO2max is 82.', contradicted: true }] },
    ],
    shows: () => expect(screen.getByTestId('verdict-chip')).toBeInTheDocument(),
  },
  reconnect: {
    blocks: [
      {
        type: 'reconnect',
        provider: 'whoop',
        display_name: 'WHOOP',
        url: 'https://app.dravr.ai/providers/sciotte/login?token=one-time-abc',
        text: 'Reconnect WHOOP to continue.',
      },
    ],
    shows: () => expect(screen.getByRole('link', { name: /Reconnect WHOOP/ })).toBeInTheDocument(),
  },
  actions: {
    blocks: [
      {
        type: 'actions',
        title: 'Pick a session',
        actions: [{ label: 'Seuil 3x10', action_type: 'postback', value: '/plan session seuil' }],
      },
    ],
    shows: () => expect(screen.getByRole('button', { name: 'Seuil 3x10' })).toBeInTheDocument(),
  },
  notice: {
    blocks: [
      { type: 'prose', text: 'Here is your week.' },
      {
        type: 'notice',
        notice: {
          kind: 'quota_warning',
          level: 'approaching',
          current: 45,
          limit: 50,
          resets_at: '2026-08-26T00:00:00Z',
        },
      },
    ],
    shows: () => {
      expect(screen.getByText('Here is your week.')).toBeInTheDocument();
      expect(screen.queryByText(/45\/50/)).not.toBeInTheDocument();
    },
  },
};

describe('surface parity — web', () => {
  const webSurfaces = surfacesFor('web');

  beforeEach(() => {
    vi.clearAllMocks();
    gate.billing = true;
  });

  it('declares at least the primary destinations', () => {
    // Guards against the registry itself being gutted to make this pass. The
    // floor dropped from 14 when the Chat-First Cutover retired Insights and
    // folded the Coach tab into Discover, and again to 12 when group
    // management moved into the group's own chat thread — each one surface
    // fewer by decision. It dropped to 6 when the settings destinations left
    // this registry for SETTINGS_PANES, their one declaration.
    expect(webSurfaces.length).toBeGreaterThanOrEqual(6);
  });

  it('no longer declares the retired Groups surface', () => {
    // Group management lives in the group's chat thread (`/group …` and the
    // header's Group info); a registry row pointing at a Groups tab would send
    // a deep link to a destination that no longer exists.
    expect(USER_SURFACES.find((s) => s.id === 'groups')).toBeUndefined();
  });

  it.each(webSurfaces.map((s) => [s.id, s.web as string, s.webNav]))(
    'implements %s at %s',
    async (_id, route, webNav) => {
      // A surface with no rail label is the operator's; the rest are the
      // athlete's. Each is opened the way a deep link opens it — by its hash.
      await renderDashboardAt(route as string, webNav === null ? 'admin' : 'user');

      const marker = SERVED_BY[route as string];
      // vitest's `expect` takes a single argument, so the explanation rides in
      // the compared value where the failure diff will actually print it.
      const outcome =
        marker === undefined
          ? `UNKNOWN — this test names no component for '${route}'. Add it to SERVED_BY with its mock.`
          : (await screen.findAllByTestId(marker).catch(() => [])).length === 1
            ? 'implemented'
            : `MISSING — the Dashboard mounts nothing at '#${route}'. Build the surface, or set ` +
              'web: null in the registry with a "why" if web should not have it.';
      expect(outcome).toBe('implemented');
      // And the route is its own: it was not resolved away to the role default.
      expect(window.location.hash).toBe(`#${route}`);
    },
  );

  it.each((USER_SURFACES.find((s) => s.id === 'chat')?.blocks ?? []).map((kind) => [kind]))(
    'renders the %s reply block the chat surface declares',
    (kind) => {
      // The registry's `blocks` column comes from the server's own capability
      // table. A kind the server will send this surface and the renderer has no
      // arm for is a block the athlete never sees — silently, because an
      // unmatched switch arm renders nothing and throws nothing.
      const blockCase = BLOCK_CASES[kind];
      expect(blockCase === undefined ? `no case for ${kind} — add one to BLOCK_CASES` : 'covered').toBe('covered');
      render(<MessageItem message={CHAT_MESSAGE} blocks={blockCase.blocks} />);
      blockCase.shows();
    },
  );

  it('declares the eight block kinds the chat surface is served', () => {
    const chat = USER_SURFACES.find((s) => s.id === 'chat');
    expect(chat?.blocks.length).toBeGreaterThanOrEqual(8);
  });

  it('serves usage only behind the billing gate, which ships closed', async () => {
    // The registry declares `usage` for web and says in its `why` that it is
    // gated. This is the build that ships (billingGate.test.ts pins that the
    // real constant defaults closed): no rail button, and the deep link
    // mounts nothing. Every other labelled surface is still offered.
    gate.billing = false;
    const gated = USER_SURFACES.filter((s) => s.id === 'usage');
    expect(gated.map((s) => s.why)).toEqual([expect.stringContaining('BILLING_ENABLED')]);

    await renderDashboardAt('usage', 'user');

    expect(screen.queryByTestId(SERVED_BY.usage)).not.toBeInTheDocument();
    const offered = within(screen.getByRole('list'))
      .getAllByRole('button')
      .map((button) => button.textContent?.trim());
    expect(offered).toEqual(
      USER_SURFACES.filter((s) => s.webNav !== null && s.id !== 'usage').map((s) => s.webNav),
    );
  });

  it('reads the same capability row as mobile', () => {
    // The registry publishes one `blocks` column for the chat surface. That is
    // only honest while both in-app rows resolve the same capabilities, so it
    // is asserted rather than assumed.
    expect(SURFACE_CAPABILITIES.web_chat.blocks).toEqual(SURFACE_CAPABILITIES.mobile_chat.blocks);
    expect(SURFACE_CAPABILITIES.web_chat.max_reply_chars).toBe(
      SURFACE_CAPABILITIES.mobile_chat.max_reply_chars,
    );
    expect(SURFACE_CAPABILITIES.web_chat.progressive).toBe('delta_channel');
  });

  it('pins every web nav label to a rail button that opens its surface', async () => {
    // The design sweep walks these labels; a label no sidebar button carries
    // makes the sweep skip a surface and still report success. So the rail is
    // rendered in English, as the sweep sees it, and each registry label is
    // pressed: it must be there, and it must lead to the surface it names.
    const labelled = USER_SURFACES.filter((s) => s.webNav !== null);
    // Home, Groups, Discover and Usage; Notifications is the header bell's
    // sheet, not a rail destination (carnet#820).
    expect(labelled.length).toBeGreaterThanOrEqual(4);

    await renderDashboardAt('', 'user');
    const rail = screen.getByRole('list');
    const offered = within(rail)
      .getAllByRole('button')
      .map((button) => button.textContent?.trim());
    expect(offered).toEqual(labelled.map((s) => s.webNav));

    for (const surface of labelled) {
      await act(async () => {
        within(rail).getByRole('button', { name: surface.webNav as string }).click();
      });
      expect(await screen.findByTestId(SERVED_BY[surface.web as string])).toBeInTheDocument();
    }
  });
});
