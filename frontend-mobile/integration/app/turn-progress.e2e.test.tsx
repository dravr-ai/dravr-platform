// ABOUTME: PHASE 5 e2e — the mobile progress strip is fed by the turn's own stream, not a second rail
// ABOUTME: Covers the frames it renders, the blocks it draws off frames, and the run id it no longer sends

import { renderHook, act, render, screen } from '@testing-library/react-native';

import { installHttpStub, sseTurn, STAGE_PROGRESS, type HttpStub } from './helpers/httpStub';
import { CONVERSATION_ID, PROSE_OPENING, assistantTurn } from './helpers/chatFixtures';

import { chatApi } from '../../src/services/api';
import { useMessages } from '../../src/screens/chat/useMessages';
import { ChatProgressStrip } from '../../src/screens/chat/ChatProgressStrip';
import { statusTextForProgress } from '@pierre/chat-utils';

const MESSAGES_URL = `/api/chat/conversations/${CONVERSATION_ID}/messages`;
const QUESTION = 'Comment se presente ma semaine ?';

describe('PHASE 5 — one stream, and a progress strip that finally renders', () => {
  let stub: HttpStub | null = null;

  afterEach(() => {
    stub?.restore();
    stub = null;
  });

  it('carries the pipeline stages to the strip\'s own words, off the real turn body', async () => {
    // This is the bug the deleted rail shipped with: the mobile
    // `useAgUiProgress` registered a listener for `'message'` while the server
    // named every payload frame `'agui'` and the handshake `'connection'`, so
    // no event ever reached it and the strip never rendered once. Nothing here
    // is hand-fed: the frames are the ones the server writes, the parser is
    // `@pierre/api-client`, and the wording is the shared mapper the strip is
    // given. A rail that delivered nothing produces an empty list.
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        data: sseTurn(assistantTurn(), [], { progress: STAGE_PROGRESS }),
      },
    });

    const lines: string[] = [];
    await chatApi.sendTurn(CONVERSATION_ID, QUESTION, {
      onProgress: (progress) => {
        const text = statusTextForProgress(progress);
        if (text !== null) lines.push(text);
      },
    });

    expect(lines).toEqual(['reading your question…', 'generating response…']);
    // And those exact words are what the strip puts on screen.
    render(<ChatProgressStrip statusText={lines[1]} />);
    expect(screen.getByText('generating response…')).toBeTruthy();
  });

  it('feeds the hook that drives the strip, and collapses it once the reply lands', async () => {
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        data: sseTurn(assistantTurn(), [], { progress: STAGE_PROGRESS }),
      },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    // The reply is the source of truth now, so the strip has nothing to say.
    // (The mid-turn text is pinned in `__tests__/useMessages.test.tsx`, where
    // the turn can be held open across two commits; React Native hands the
    // whole body over at once, so a completed turn commits only its end state.)
    expect(result.current.progressText).toBeNull();
    const assistant = result.current.messages.find((m) => m.role === 'assistant');
    expect(assistant?.content).toBe(PROSE_OPENING);
  });

  it('draws the strip for a status line, and nothing at all without one', () => {
    // The whole visible surface of the progress rail, in the two states it
    // has. A component that returned markup for `null` would put an empty
    // spinner row above the composer on every idle chat screen.
    const { unmount } = render(<ChatProgressStrip statusText="calling get_activities…" />);
    expect(screen.getByText('calling get_activities…')).toBeTruthy();
    unmount();

    render(<ChatProgressStrip statusText={null} />);
    expect(screen.queryByText('calling get_activities…')).toBeNull();
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
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    const posts = stub.requestsFor('POST');
    expect(posts).toHaveLength(1);
    expect(Object.keys(posts[0].body as Record<string, unknown>)).toEqual(['content']);
    // And exactly one request in total: no parallel subscription was opened.
    expect(stub.requests).toHaveLength(1);
  });
});
