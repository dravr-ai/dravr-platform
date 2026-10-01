// ABOUTME: A failed chat turn on mobile reads in the athlete's language and keeps their question with a retry
// ABOUTME: Covers the fail-fast 503 and the mid-stream failed frame, both keyed on the server's error code (carnet#680)

import { act } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import type { Message } from '@pierre/shared-types';

import { renderHook } from './helpers/queryHook';
import { installHttpStub, sseTurn, type HttpStub } from './helpers/httpStub';
import { CONVERSATION_ID, assistantTurn } from './helpers/chatFixtures';

import { useMessages } from '../../src/screens/chat/useMessages';

const MESSAGES_URL = `/api/chat/conversations/${CONVERSATION_ID}/messages`;
const QUESTION = 'Comment se presente ma semaine ?';
/** A sentence from the French catalogue, read through the app's own i18n instance. */
const fr = (key: string): string => i18n.getFixedT('fr')(key);

/** What the server says, in English, for the fail-fast refusal. */
const SERVER_TEXT = 'The resource is temporarily unavailable';

describe('a failed turn, in French', () => {
  let stub: HttpStub | null = null;

  beforeEach(async () => {
    await i18n.changeLanguage('fr');
  });

  afterEach(async () => {
    stub?.restore();
    stub = null;
    await i18n.changeLanguage('en');
  });

  it('words a fail-fast 503 from its code and keeps the question in the thread', async () => {
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        status: 503,
        data: { code: 'ResourceUnavailable', message: SERVER_TEXT },
      },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    // The French sentence, not the English one the same key reads as.
    expect(fr('errors.generic')).not.toBe(i18n.getFixedT('en')('errors.generic'));
    expect(result.current.error).toBe(fr('errors.generic'));
    const failed = result.current.messages.find((message) => message.isError);
    expect(failed?.content).toContain(fr('errors.generic'));
    expect(failed?.content).not.toContain(SERVER_TEXT);
    expect(
      result.current.messages.filter((message) => message.role === 'user').map((m) => m.content),
    ).toEqual([QUESTION]);
  });

  it('words a mid-stream failure from the failed frame code, and the retry re-sends the question', async () => {
    let answered = 0;
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: () => {
        answered += 1;
        return answered === 1
          ? {
              data:
                `event: delta\ndata: ${JSON.stringify({ delta: 'Je regarde ' })}\n\n` +
                `event: failed\ndata: ${JSON.stringify({ error: SERVER_TEXT, code: 'ResourceUnavailable' })}\n\n`,
            }
          : { data: sseTurn(assistantTurn({ content: 'Voici ta semaine.' })) };
      },
      // A landed reply re-reads its claim verdicts.
      [`GET ${MESSAGES_URL.replace('/messages', '/verdicts')}`]: { data: { verdicts: [] } },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, QUESTION);
    });

    const failed = result.current.messages.find((message) => message.isError) as Message;
    expect(failed.content).toContain(fr('errors.generic'));
    expect(failed.content).toContain(fr('chat.turnTryAgain'));
    expect(failed.content).not.toContain(SERVER_TEXT);
    expect(result.current.messages.some((m) => m.role === 'user' && m.content === QUESTION)).toBe(true);

    await act(async () => {
      await result.current.retryMessage(failed.id, CONVERSATION_ID);
    });

    const posts = stub.requestsFor('POST');
    expect(posts).toHaveLength(2);
    expect((posts[1].body as { content: string }).content).toBe(QUESTION);
    expect(result.current.messages.some((message) => message.isError)).toBe(false);
    expect(result.current.messages.find((m) => m.role === 'assistant')?.content).toBe('Voici ta semaine.');
  });
});
