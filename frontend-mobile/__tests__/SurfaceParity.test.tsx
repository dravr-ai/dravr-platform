// ABOUTME: Asserts mobile implements every surface the shared registry declares for it
// ABOUTME: Checks real expo-router files on disk and mounts the real chat renderer on each declared block

import fs from 'fs';
import path from 'path';
import React from 'react';
import { fireEvent, render, type RenderResult } from '@testing-library/react-native';
import type { ChatMessageAction, ReplyBlock } from '@pierre/shared-types';
import { SURFACE_CAPABILITIES, USER_SURFACES, surfacesFor } from '@pierre/shared-constants';
import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

/**
 * Mobile silently lacked Profile, Privacy and Messaging while rendering rows
 * that pointed at them, and shipped Memory and Billing screens nothing
 * navigated to. None of that failed anything — it was visible only by driving
 * both apps side by side.
 *
 * This is not a diff between web's routes and mobile's. It checks one client
 * against the single registry, which is the declaration of what the product
 * offers. Adding a surface means editing the registry first; this test then
 * says whether mobile has caught up.
 *
 * Why the routes are checked on disk: expo-router's route table IS the file
 * tree under app/ — a route exists exactly when its file does, and there is no
 * other artefact to ask. That is a check of the tree's shape, not of what any
 * file says. The chat renderer is different: a search of its text for
 * `case '<kind>':` passes an arm that draws nothing, so it is mounted.
 */
const APP_DIR = path.join(__dirname, '..', 'app');

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

const onReconnectProvider = jest.fn();
const onActionClick = jest.fn();

function renderReply(blocks: ReplyBlock[]): RenderResult {
  return render(
    <MessageList
      messages={[CHAT_MESSAGE]}
      isLoading={false}
      isSending={false}
      messageFeedback={{}}
      messageFeedbackComment={{}}
      messageBlocks={{ [CHAT_MESSAGE.id]: blocks }}
      flatListRef={React.createRef()}
      onScrollToBottom={jest.fn()}
      onThumbsUp={jest.fn()}
      onThumbsDown={jest.fn()}
      onSubmitFeedbackReason={jest.fn()}
      onRetryMessage={jest.fn()}
      onOpenUrl={jest.fn()}
      onReconnectProvider={onReconnectProvider}
      onActionClick={onActionClick}
    />,
  );
}

const SESSION: ChatMessageAction = { label: 'Seuil 3x10', action_type: 'postback', value: '/plan session seuil' };

/**
 * One reply per block kind, and what the athlete must be able to see or do.
 *
 * `scene` is drawn where the prose's own marker puts it, so its reply carries
 * the prose that positions it. `notice` is the one kind the message draws
 * nothing for by decision — the usage banner owns it — so what is asserted is
 * that the reply around it still renders and the figure appears nowhere in it.
 */
const BLOCK_CASES: Record<string, { blocks: ReplyBlock[]; shows: (view: RenderResult) => void }> = {
  prose: {
    blocks: [{ type: 'prose', text: 'Nice negative split.' }],
    shows: (view) => expect(view.getByText('Nice negative split.')).toBeTruthy(),
  },
  activity_list: {
    blocks: [{ type: 'activity_list', text: '1. Long run - 24 km\n2. Threshold 3x10 - 14 km' }],
    shows: (view) => expect(view.getByText('Your Activities (2)')).toBeTruthy(),
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
    shows: (view) => expect(view.getByText('Parkrun PB')).toBeTruthy(),
  },
  scene: {
    blocks: [
      { type: 'prose', text: 'Here is your month.\n\n⟦viz:0⟧' },
      { type: 'scene', scene_blocks: SCENE_BLOCKS },
    ],
    shows: (view) => expect(view.getByText('Weekly volume')).toBeTruthy(),
  },
  verdicts: {
    blocks: [
      { type: 'prose', text: 'Your VO2max is 82.' },
      { type: 'verdicts', chips: [{ claim: 'Your VO2max is 82.', contradicted: true }] },
    ],
    shows: (view) => expect(view.getByTestId('verdict-chip')).toBeTruthy(),
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
    shows: (view) => {
      fireEvent.press(view.getByText(/Reconnect WHOOP/));
      expect(onReconnectProvider).toHaveBeenCalledWith('whoop');
    },
  },
  actions: {
    blocks: [{ type: 'actions', title: 'Pick a session', actions: [SESSION] }],
    shows: (view) => {
      expect(view.getByText('Pick a session')).toBeTruthy();
      fireEvent.press(view.getByText(SESSION.label));
      expect(onActionClick).toHaveBeenCalledWith(SESSION);
    },
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
    shows: (view) => {
      expect(view.getByText('Here is your week.')).toBeTruthy();
      expect(view.queryByText(/45\/50/)).toBeNull();
    },
  },
};

/** Turn an expo-router path into the file that should serve it. */
function routeFileCandidates(route: string): string[] {
  const rel = route.replace(/^\//, '');
  return [
    path.join(APP_DIR, `${rel}.tsx`),
    path.join(APP_DIR, rel, 'index.tsx'),
    path.join(APP_DIR, rel, '_layout.tsx'),
  ];
}

describe('surface parity — mobile', () => {
  const mobileSurfaces = surfacesFor('mobile');

  it('declares at least the primary destinations', () => {
    // Guards against the registry itself being gutted to make this pass. The
    // floor moved from 14 to 13 when the Chat-First Cutover folded the Coaches
    // tab into Discover, to 12 when group management moved into the group's
    // own chat thread, and to 11 when the per-athlete AI-provider screen was
    // removed from both clients: each one destination fewer by decision. It
    // rose to 12 when Home became the landing on both clients, and dropped to
    // 4 when the settings destinations left this registry for SETTINGS_PANES,
    // their one declaration, which SettingsPaneParity.test.tsx checks.
    expect(mobileSurfaces.length).toBeGreaterThanOrEqual(4);
  });

  it('lands on Home, served by the first tab', () => {
    // The registry's first row is the landing, and mobile serves it as a tab
    // group whose index is a real screen, not only a layout.
    const home = USER_SURFACES[0];
    expect(home.id).toBe('home');
    expect(home.mobile).toBe('/(app)/(tabs)/(home)');
    expect(fs.existsSync(path.join(APP_DIR, '(app)', '(tabs)', '(home)', 'index.tsx'))).toBe(true);
    expect(fs.existsSync(path.join(APP_DIR, '(app)', '(tabs)', '(home)', '_layout.tsx'))).toBe(true);
  });

  it('no longer declares the retired Coaches surface', () => {
    // The (coaches) tab folded into Discover; a row pointing at it would send
    // a deep link into a route group that no longer exists.
    expect(USER_SURFACES.find((s) => s.id === 'coaches')).toBeUndefined();
    expect(USER_SURFACES.find((s) => s.id === 'insights')).toBeUndefined();
  });

  it('no longer declares the retired AI-provider surface', () => {
    // Nobody brings their own model. The screen took provider API keys and
    // changed nothing about the coaching, so a row pointing at it would open a
    // route that no longer exists.
    expect(USER_SURFACES.find((s) => s.id === 'ai-provider')).toBeUndefined();
  });

  it('no longer declares the retired Groups surface', () => {
    // Group management lives in the group's chat thread (`/group …` and the
    // header's Group info); a row pointing at a (groups) tab would send a deep
    // link into a route group that no longer exists.
    expect(USER_SURFACES.find((s) => s.id === 'groups')).toBeUndefined();
  });

  it.each(mobileSurfaces.map((s) => [s.id, s.mobile as string]))(
    'implements %s at %s',
    (_id, route) => {
      const found = routeFileCandidates(route).some((candidate) => fs.existsSync(candidate));
      // jest's `expect` takes a single argument, so the explanation rides in the
      // compared value where the failure diff will actually print it.
      const outcome = found
        ? 'implemented'
        : `MISSING — no expo-router file serves ${route}. Build the screen, or set ` +
          'mobile: null in the registry with a "why" if mobile should not have it.';
      expect(outcome).toBe('implemented');
    },
  );

  it.each((USER_SURFACES.find((s) => s.id === 'chat')?.blocks ?? []).map((kind) => [kind]))(
    'renders the %s reply block the chat surface declares',
    (kind) => {
      // The registry's `blocks` column comes from the server's own capability
      // table. A kind the server will send this surface and the renderer has no
      // arm for is a block the athlete never sees — silently, because an
      // unmatched switch arm renders nothing and throws nothing.
      jest.clearAllMocks();
      const blockCase = BLOCK_CASES[kind];
      expect(blockCase === undefined ? `no case for ${kind} — add one to BLOCK_CASES` : 'covered').toBe('covered');
      blockCase.shows(renderReply(blockCase.blocks));
    },
  );

  it('declares the eight block kinds the chat surface is served', () => {
    const chat = USER_SURFACES.find((s) => s.id === 'chat');
    expect(chat?.blocks.length).toBeGreaterThanOrEqual(8);
  });

  it('reads the same capability row as web', () => {
    // The registry publishes one `blocks` column for the chat surface. That is
    // only honest while both in-app rows resolve the same capabilities, so it
    // is asserted rather than assumed.
    expect(SURFACE_CAPABILITIES.mobile_chat.blocks).toEqual(SURFACE_CAPABILITIES.web_chat.blocks);
    expect(SURFACE_CAPABILITIES.mobile_chat.max_reply_chars).toBe(
      SURFACE_CAPABILITIES.web_chat.max_reply_chars,
    );
    expect(SURFACE_CAPABILITIES.mobile_chat.progressive).toBe('delta_channel');
  });

  it('records a reason whenever a platform deliberately lacks a surface', () => {
    // A null without a reason is indistinguishable from an oversight, which is
    // exactly how the original gaps went unnoticed.
    const unexplained = USER_SURFACES.filter(
      (s) => (s.web === null || s.mobile === null) && !s.why,
    );
    expect(unexplained.map((s) => s.id)).toEqual([]);
  });
});
