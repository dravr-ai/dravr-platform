// ABOUTME: PHASE 4 e2e — the mobile app reads a chat turn off the ONE sendTurn transport, frames and all
// ABOUTME: Covers the SSE body it actually receives, the deltas it declines, and the real retry

import { act } from '@testing-library/react-native';
import type { Message } from '@pierre/shared-types';

import { renderHook } from './helpers/queryHook';
import { installHttpStub, sseTurn, type HttpStub } from './helpers/httpStub';
import { CONVERSATION_ID, PROSE_OPENING, assistantTurn } from './helpers/chatFixtures';

import { chatApi } from '../../src/services/api';
import { useMessages } from '../../src/screens/chat/useMessages';

const MESSAGES_URL = `/api/chat/conversations/${CONVERSATION_ID}/messages`;
const QUESTION = 'Comment se presente ma semaine ?';

describe('PHASE 4 — one transport, read by the mobile client', () => {
  let stub: HttpStub | null = null;

  afterEach(() => {
    stub?.restore();
    stub = null;
  });

  it('exposes exactly one send method on the real chat API', () => {
    // Two send methods for one endpoint is the parallel system every
    // server-side capability used to die in — is_command_response, card_title
    // and the X-Usage-* headers each reached one client and not the other.
    // This runs against the real `@pierre/api-client`, not a hand-written mock.
    const senders = Object.keys(chatApi).filter((name) => /^send/.test(name));
    expect(senders).toEqual(['sendTurn']);
    expect(typeof chatApi.sendTurn).toBe('function');
  });

  it('reads an SSE body — the shape the server actually answers with — into the transcript', async () => {
    // Regression this turns red: point the mobile send back at a JSON-only
    // transport (an axios POST, a second send method) and this framed body
    // parses to nothing — the athlete's reply never lands.
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: { data: sseTurn(assistantTurn()) },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    const assistant = result.current.messages.find((message) => message.role === 'assistant');
    expect(assistant?.id).toBe('msg-assistant-1');
    expect(assistant?.content).toBe(PROSE_OPENING);
    expect(assistant?.model).toBe('claude-sonnet-4-6');
    expect(assistant?.execution_time_ms).toBe(4210);
    // The activity list rode its own block through the same parser.
    expect(
      result.current.messageBlocks['msg-assistant-1'].some(
        (block) => block.type === 'activity_list' && block.text.includes('1. Sortie longue'),
      ),
    ).toBe(true);
    expect(result.current.error).toBeNull();
    expect(result.current.isSending).toBe(false);
  });

  it('takes a turn that streamed deltas and shows the finished reply, not the fragments', async () => {
    // React Native's fetch hands the body over complete, so the app declares
    // no partial-text rendering. The delta frames must still be consumed
    // without corrupting the turn — a parser that mistook them for the reply
    // would show 'Ta ' + 'charge ' twice over.
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        data: sseTurn(assistantTurn(), ['Ta charge ', 'monte ', 'depuis trois semaines.']),
      },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    const assistant = result.current.messages.find((message) => message.role === 'assistant');
    expect(assistant?.content).toBe(PROSE_OPENING);
    expect(result.current.messages.filter((m) => m.role === 'assistant')).toHaveLength(1);
  });

  it('surfaces the message carried by an error frame, not a bare HTTP status', async () => {
    // Regression this turns red: an error frame read as an ordinary frame
    // leaves the turn "successful but empty" and the athlete stares at a
    // spinner that stopped for no stated reason.
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        data: 'event: failed\ndata: {"error":"Daily message limit reached."}\n\n',
      },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    expect(result.current.error).toBe('Daily message limit reached.');
    const failed = result.current.messages.find((message) => message.isError);
    expect(failed?.content).toContain('Daily message limit reached.');
    expect(result.current.isSending).toBe(false);
  });

  it('retries by dropping the failed row and re-sending the user message', async () => {
    // The real retry, which the web client now shares: one attempt is left
    // behind, not a failure followed by its replacement.
    let answered = 0;
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: () => {
        answered += 1;
        return answered === 1
          ? { data: 'event: failed\ndata: {"error":"Upstream timed out."}\n\n' }
          : { data: sseTurn(assistantTurn({ content: 'Voici ta semaine.' })) };
      },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    const failed = result.current.messages.find((message) => message.isError) as Message;
    expect(failed).toBeDefined();

    await act(async () => {
      await result.current.retryMessage(failed.id, CONVERSATION_ID);
    });

    // The failed row is gone and the successful reply took its place.
    expect(result.current.messages.some((message) => message.isError)).toBe(false);
    const assistants = result.current.messages.filter((message) => message.role === 'assistant');
    expect(assistants).toHaveLength(1);
    expect(assistants[0].content).toBe('Voici ta semaine.');
    // The retry re-sent the athlete's own words, not the failure text.
    const posts = stub.requestsFor('POST');
    expect(posts).toHaveLength(2);
    expect((posts[1].body as { content: string }).content).toBe(QUESTION);
  });
});
