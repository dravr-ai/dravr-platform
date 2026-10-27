// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: PHASE 5 e2e — the mobile turn trail is fed by the turn's own body, and a reopened app finds a turn still running
// ABOUTME: Covers the steps it folds, the reply streamed and retracted, the blocks drawn off frames, the run id and the restore

import React from 'react';
import { View } from 'react-native';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react-native';

import { renderHook } from './helpers/queryHook';
import {
  heldBody,
  installHttpStub,
  sseFrame,
  sseTurn,
  STAGE_PROGRESS,
  type HttpStub,
} from './helpers/httpStub';
import { CONVERSATION_ID, PROSE_OPENING, assistantTurn } from './helpers/chatFixtures';

import { useMessages } from '../../src/screens/chat/useMessages';
import { FoldedTurnTrail, LiveTurnIndicator } from '../../src/screens/chat/TurnTrail';
import { MessageList } from '../../src/screens/chat/MessageList';
import { AWAITING_REPLY_POLL_MS, TURN_STEP_KEYS, turnCurrentLine } from '@pierre/chat-utils';
import en from '@pierre/i18n/locales/en/translation.json';

const MESSAGES_URL = `/api/chat/conversations/${CONVERSATION_ID}/messages`;
const VERDICTS_URL = `/api/chat/conversations/${CONVERSATION_ID}/verdicts`;
const STOP_URL = `/api/chat/conversations/${CONVERSATION_ID}/stop`;
const QUESTION = 'Comment se presente ma semaine ?';
const QUESTION_ID = 'msg-question-awaited';
const ASKED_AT = '2026-10-26T08:00:00Z';
/** The notice the server closes a stopped question with. */
const STOPPED_NOTICE = 'Réponse arrêtée.';

/** The tool calls a plan-building turn makes, as the server frames them. */
const TOOL_PROGRESS = [
  { kind: 'tool', id: 'call-1', title: 'get_training_history', status: 'InProgress' },
  { kind: 'tool', id: 'call-1', title: 'get_training_history', status: 'Completed' },
  { kind: 'tool', id: 'call-2', title: 'recommend_plan_flavour', status: 'InProgress' },
  { kind: 'tool', id: 'call-2', title: 'recommend_plan_flavour', status: 'Completed' },
];

/** The thread as a reopened app reads it while the server still writes the reply. */
function awaitedThread() {
  return {
    messages: [{ id: QUESTION_ID, role: 'user', content: QUESTION, created_at: ASKED_AT }],
    awaiting_reply: { question_id: QUESTION_ID, asked_at: ASKED_AT },
  };
}

/** The same thread once the reply has landed. */
function answeredThread() {
  return {
    messages: [
      { id: QUESTION_ID, role: 'user', content: QUESTION, created_at: ASKED_AT },
      { id: 'msg-reply', role: 'assistant', content: PROSE_OPENING, created_at: '2026-10-26T08:07:40Z' },
    ],
    awaiting_reply: null,
  };
}

describe('PHASE 5 — one stream, and a turn trail fed off it', () => {
  let stub: HttpStub | null = null;

  afterEach(() => {
    stub?.restore();
    stub = null;
    jest.useRealTimers();
  });

  it('folds the tools the turn called above its reply, named in the athlete\'s words', async () => {
    // Nothing here is hand-fed: the frames are the ones the server writes,
    // the parser is `@pierre/api-client`, and the steps are the shared
    // reducer's. A transport that delivered no frame folds nothing.
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        data: sseTurn(assistantTurn(), [], { progress: [...STAGE_PROGRESS, ...TOOL_PROGRESS] }),
      },
      [`GET ${VERDICTS_URL}`]: { data: { verdicts: [] } },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    expect(result.current.liveTrail).toBeNull();
    const trail = result.current.turnTrails.get('msg-assistant-1');
    expect(trail?.steps.map(step => [step.label.key, step.done])).toEqual([
      ['chat.turn.step.readHistory', true],
      ['chat.turn.step.choosePlanApproach', true],
    ]);

    // And the fold opens on the athlete's own language (the catalogue, never a tool name).
    render(<FoldedTurnTrail trail={trail!} />);
    act(() => {
      fireEvent.press(screen.getByTestId('turn-trail-toggle'));
    });
    expect(screen.getByText(en.chat.turn.step.readHistory)).toBeTruthy();
    expect(screen.getByText(en.chat.turn.step.choosePlanApproach)).toBeTruthy();
  });

  it('draws a step on the phone while the turn runs, before its reply lands', async () => {
    // The body is held open after the first tool call: the step must be on
    // screen while the server is still working. A client that read the body
    // only once it was complete (React Native's XHR fetch) shows the step
    // and the reply together, and the first assertion fails.
    const body = heldBody();
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: { data: body.stream },
      [`GET ${VERDICTS_URL}`]: { data: { verdicts: [] } },
    });

    const { result } = renderHook(() => useMessages());
    let sent: Promise<unknown> = Promise.resolve();
    act(() => {
      sent = result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });
    body.write(
      [...STAGE_PROGRESS, TOOL_PROGRESS[0]].map(progress => sseFrame('progress', progress)).join(''),
    );

    await waitFor(() => {
      expect(result.current.liveTrail?.steps.map(step => [step.label.key, step.done])).toEqual([
        ['chat.turn.step.readHistory', false],
      ]);
    });
    expect(result.current.messages.some(message => message.role === 'assistant')).toBe(false);
    render(
      <LiveTurnIndicator
        trail={result.current.liveTrail}
        startedAt={Date.now()}
        glyph={<View testID="thinking-indicator" />}
      />,
    );
    expect(screen.getByText(en.chat.turn.step.readHistory)).toBeTruthy();

    body.write(sseFrame('progress', TOOL_PROGRESS[1]));
    body.write(sseFrame('done', assistantTurn()));
    body.close();
    await act(async () => {
      await sent;
    });

    expect(result.current.liveTrail).toBeNull();
    expect(result.current.messages.find(message => message.role === 'assistant')?.content).toBe(
      PROSE_OPENING,
    );
    expect(
      result.current.turnTrails.get('msg-assistant-1')?.steps.map(step => [step.label.key, step.done]),
    ).toEqual([['chat.turn.step.readHistory', true]]);
  });

  it('shows the reply on the phone as it is written, and takes it back on a retract frame', async () => {
    // The body is held open between frames: the words must be on screen
    // while the server is still writing, and gone the moment it takes them
    // back. A hook that rendered only at `done` fails the first assertion.
    const body = heldBody();
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: { data: body.stream },
      [`GET ${VERDICTS_URL}`]: { data: { verdicts: [] } },
    });

    const { result } = renderHook(() => useMessages());
    let sent: Promise<unknown> = Promise.resolve();
    act(() => {
      sent = result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });
    body.write(
      [
        ...STAGE_PROGRESS.map(progress => sseFrame('progress', progress)),
        sseFrame('delta', { delta: 'Je regarde ' }),
        sseFrame('delta', { delta: 'tes sorties.' }),
      ].join(''),
    );

    await waitFor(() => {
      expect(result.current.streamingContent).toBe('Je regarde tes sorties.');
    });
    expect(result.current.messages.some(message => message.role === 'assistant')).toBe(false);
    const list = render(
      <MessageList
        messages={result.current.messages}
        isLoading={false}
        isSending={result.current.turnInFlight}
        liveTrail={result.current.liveTrail}
        streamingContent={result.current.streamingContent}
        messageFeedback={{}}
        messageFeedbackComment={{}}
        flatListRef={React.createRef()}
        onScrollToBottom={jest.fn()}
        onThumbsUp={jest.fn()}
        onThumbsDown={jest.fn()}
        onSubmitFeedbackReason={jest.fn()}
        onRetryMessage={jest.fn()}
        onOpenUrl={jest.fn()}
        onReconnectProvider={jest.fn()}
      />,
    );
    expect(list.getByTestId('streaming-reply')).toHaveTextContent('Je regarde tes sorties.');
    expect(list.getByTestId('thinking-indicator')).toBeTruthy();
    expect(list.getByText(en.chat.turn.step.writing)).toBeTruthy();
    list.unmount();

    // A tool call followed the words: the server takes them back.
    body.write(sseFrame('retract', {}));
    await waitFor(() => {
      expect(result.current.streamingContent).toBe('');
    });
    expect(turnCurrentLine(result.current.liveTrail!)).toEqual({ key: TURN_STEP_KEYS.thinking });

    body.write(sseFrame('delta', { delta: PROSE_OPENING }));
    await waitFor(() => {
      expect(result.current.streamingContent).toBe(PROSE_OPENING);
    });
    body.write(sseFrame('done', assistantTurn()));
    body.close();
    await act(async () => {
      await sent;
    });

    expect(result.current.streamingContent).toBe('');
    expect(result.current.messages.find(message => message.role === 'assistant')?.content).toBe(
      PROSE_OPENING,
    );
  });

  it('draws the live steps, the timer after ten seconds and the leave note after forty-five', () => {
    jest.useFakeTimers({ now: Date.parse(ASKED_AT) });
    render(
      <LiveTurnIndicator
        trail={null}
        startedAt={Date.parse(ASKED_AT)}
        glyph={<View testID="thinking-indicator" />}
      />,
    );
    expect(screen.getByText(en.chat.turn.step.thinking)).toBeTruthy();
    expect(screen.queryByTestId('turn-elapsed')).toBeNull();
    expect(screen.queryByTestId('turn-safe-to-leave')).toBeNull();

    act(() => {
      jest.advanceTimersByTime(12_000);
    });
    expect(screen.getByTestId('turn-elapsed')).toHaveTextContent('· 12 s');
    expect(screen.queryByTestId('turn-safe-to-leave')).toBeNull();

    act(() => {
      jest.advanceTimersByTime(60_000);
    });
    expect(screen.getByTestId('turn-elapsed')).toHaveTextContent('· 1 min 12 s');
    expect(screen.getByText(en.chat.turn.safeToLeave)).toBeTruthy();
  });

  it('restores a turn still running after the app reopened, and re-reads until its reply lands', async () => {
    jest.useFakeTimers({ now: Date.parse('2026-10-26T08:05:00Z'), doNotFake: ['nextTick', 'setImmediate'] });
    let reads = 0;
    stub = installHttpStub({
      // Two reads still waiting, then the reply.
      [`GET ${MESSAGES_URL}`]: () => {
        reads += 1;
        return { data: reads < 3 ? awaitedThread() : answeredThread() };
      },
      [`GET ${VERDICTS_URL}`]: { data: { verdicts: [] } },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID);
    });

    // The indicator stands from when the question was asked, and the stop
    // button with it — there is no stream here to hold either up.
    expect(result.current.awaitingSince).toBe(Date.parse(ASKED_AT));
    expect(result.current.turnInFlight).toBe(true);
    expect(result.current.isSending).toBe(false);

    await act(async () => {
      await jest.advanceTimersByTimeAsync(AWAITING_REPLY_POLL_MS);
    });
    expect(reads).toBe(2);
    expect(result.current.turnInFlight).toBe(true);

    await act(async () => {
      await jest.advanceTimersByTimeAsync(AWAITING_REPLY_POLL_MS);
    });
    expect(reads).toBe(3);
    expect(result.current.awaitingSince).toBeNull();
    expect(result.current.turnInFlight).toBe(false);
    expect(result.current.messages.map(m => m.content)).toEqual([QUESTION, PROSE_OPENING]);

    // Landed: nothing re-reads the thread any more.
    await act(async () => {
      await jest.advanceTimersByTimeAsync(AWAITING_REPLY_POLL_MS * 3);
    });
    expect(reads).toBe(3);
  });

  it('stops a restored turn on the open thread and re-reads it for the stopped notice', async () => {
    jest.useFakeTimers({ now: Date.parse('2026-10-26T08:05:00Z'), doNotFake: ['nextTick', 'setImmediate'] });
    let stopped = false;
    stub = installHttpStub({
      [`GET ${MESSAGES_URL}`]: () => ({
        data: stopped
          ? {
              ...answeredThread(),
              messages: [awaitedThread().messages[0], { id: 'msg-stop', role: 'assistant', content: STOPPED_NOTICE, created_at: ASKED_AT }],
            }
          : awaitedThread(),
      }),
      [`POST ${STOP_URL}`]: () => {
        stopped = true;
        return { data: { stopped: true } };
      },
      [`GET ${VERDICTS_URL}`]: { data: { verdicts: [] } },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID);
    });
    expect(result.current.turnInFlight).toBe(true);

    await act(async () => {
      await result.current.stopTurn();
    });

    expect(stub.requestsFor('POST').map(request => request.url)).toEqual([STOP_URL]);
    expect(result.current.turnInFlight).toBe(false);
    expect(result.current.isStopping).toBe(false);
    expect(result.current.messages).toHaveLength(2);
  });

  it('ignores a stale flag when the reply is already in the thread', async () => {
    jest.useFakeTimers({ now: Date.parse('2026-10-26T08:05:00Z'), doNotFake: ['nextTick', 'setImmediate'] });
    let reads = 0;
    stub = installHttpStub({
      [`GET ${MESSAGES_URL}`]: () => {
        reads += 1;
        return { data: { ...answeredThread(), awaiting_reply: awaitedThread().awaiting_reply } };
      },
      [`GET ${VERDICTS_URL}`]: { data: { verdicts: [] } },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID);
    });
    expect(result.current.awaitingSince).toBeNull();
    expect(result.current.turnInFlight).toBe(false);

    await act(async () => {
      await jest.advanceTimersByTimeAsync(AWAITING_REPLY_POLL_MS * 2);
    });
    expect(reads).toBe(1);
  });

  it('draws the reply from the block frames, not from the envelope list', async () => {
    // The `done` envelope lists no blocks at all; every renderable piece
    // arrives as its own frame. A client that ignored the frames and walked
    // the envelope shows no activity panel, and this fails.
    const turn = assistantTurn();
    const framed = turn.assistant.blocks;
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        data: sseTurn(
          { ...turn, assistant: { ...turn.assistant, blocks: [] } },
          [],
          { progress: STAGE_PROGRESS, blocks: framed },
        ),
      },
      [`GET ${VERDICTS_URL}`]: { data: { verdicts: [] } },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    expect(
      result.current.messageBlocks['msg-assistant-1'].some(
        (block) => block.type === 'activity_list' && block.text.includes('1. Sortie longue'),
      ),
    ).toBe(true);
  });

  it('sends the message and nothing else — no run id to correlate a second stream with', async () => {
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: { data: sseTurn(assistantTurn()) },
      [`GET ${VERDICTS_URL}`]: { data: [] },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    const posts = stub.requestsFor('POST');
    expect(posts).toHaveLength(1);
    expect(Object.keys(posts[0].body as Record<string, unknown>)).toEqual(['content']);
    // And no parallel subscription was opened: the only other request is the
    // read of the finished turn's verdict rows.
    expect(stub.requestsFor('GET').map((request) => request.url)).toEqual([VERDICTS_URL]);
    expect(stub.requests).toHaveLength(2);
  });
});
