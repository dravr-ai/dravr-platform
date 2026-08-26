// ABOUTME: PHASE 2 e2e — the mobile client paints the TurnEnvelope's typed ReplyBlocks
// ABOUTME: Prose and the activity panel come off a live turn; the scene is drawn with react-native-svg

import React from 'react';
import { render, renderHook, act, fireEvent, waitFor } from '@testing-library/react-native';
import Svg, { Path, Line, Text as SvgText } from 'react-native-svg';
import type { Message, ReplyBlock } from '@pierre/shared-types';

import { installHttpStub, type HttpStub } from './helpers/httpStub';
import {
  ACTIVITY_LIST_TEXT,
  CHART_PATH_D,
  PROSE_CLOSING,
  PROSE_OPENING,
  SCENE_BLOCKS_JSON,
  assistantTurn,
  listedMessages,
} from './helpers/chatFixtures';

import { useMessages } from '../../src/screens/chat/useMessages';
import { MessageList } from '../../src/screens/chat/MessageList';

const CONVERSATION_ID = 'conv-e2e-1';
const MESSAGES_URL = `/api/chat/conversations/${CONVERSATION_ID}/messages`;
const VERDICTS_URL = `/api/chat/conversations/${CONVERSATION_ID}/verdicts`;

/** Renders the production list with the state the production hook produced. */
function renderList(messages: Message[], messageBlocks: Record<string, ReplyBlock[]> = {}) {
  return render(
    <MessageList
      messages={messages}
      coaches={[]}
      isLoading={false}
      isSending={false}
      isCoachConversation
      messageFeedback={{}}
      messageFeedbackComment={{}}
      messageBlocks={messageBlocks}
      flatListRef={React.createRef()}
      onScrollToBottom={jest.fn()}
      onCoachSelect={jest.fn()}
      onThumbsUp={jest.fn()}
      onThumbsDown={jest.fn()}
      onSubmitFeedbackReason={jest.fn()}
      onRetryMessage={jest.fn()}
      onOpenUrl={jest.fn()}
    />
  );
}

describe('PHASE 2 — TurnEnvelope blocks on mobile', () => {
  let stub: HttpStub;

  afterEach(() => {
    stub.restore();
  });

  it('paints a live turn from its blocks: prose plus its own activity panel', async () => {
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: { data: assistantTurn() },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, 'Comment se presente ma semaine ?');
    });

    // The turn's own prose block is the assistant text the athlete reads.
    const assistant = result.current.messages.find((message) => message.role === 'assistant');
    expect(assistant?.id).toBe('msg-assistant-1');
    expect(assistant?.content).toContain('Ta charge monte depuis trois semaines.');

    // The activity list arrived as its own block, keyed by assistant message id
    // — not sniffed back out of the prose.
    const list = result.current.messageBlocks['msg-assistant-1'].find(
      (block) => block.type === 'activity_list',
    );
    expect(list).toEqual({ type: 'activity_list', text: ACTIVITY_LIST_TEXT });

    const { getByText } = renderList(result.current.messages, result.current.messageBlocks);
    expect(getByText('Ta charge monte depuis trois semaines.')).toBeTruthy();
    expect(getByText('Your Activities (2)')).toBeTruthy();

    // The surface identifies itself so the server resolves SurfaceId::Mobile
    // — the in-app profile (Markdown prose, inline scenes, plan cards) under
    // the mobile identity, not the web one and not a messaging one.
    const post = stub.requestsFor('POST')[0];
    expect(post.url).toBe(MESSAGES_URL);
    expect(post.headers['x-client-platform']).toBe('mobile');
    // One transport for both clients: the same Accept the web composer sends.
    expect(post.headers.accept).toBe('text/event-stream, application/json');
    expect(post.body).toMatchObject({ content: 'Comment se presente ma semaine ?' });
    // `stream` left the request: the Accept header is what asks for frames now.
    expect(post.body).not.toHaveProperty('stream');
  });

  it('draws a loaded turn\'s scene with react-native-svg, interleaved with the prose', async () => {
    stub = installHttpStub({
      [`GET ${MESSAGES_URL}`]: { data: listedMessages() },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID);
    });

    await waitFor(() => expect(result.current.messages).toHaveLength(2));
    expect(result.current.messages[1].scene_blocks).toBe(SCENE_BLOCKS_JSON);

    const view = renderList(result.current.messages);

    // Prose on both sides of the ⟦viz:0⟧ marker survives the split.
    expect(view.getByText('Ta charge monte depuis trois semaines.')).toBeTruthy();
    expect(view.getByText('On garde le jeudi facile.')).toBeTruthy();

    // The chart is a real SVG, sized by the scene's own view box.
    const svg = view.UNSAFE_getByType(Svg);
    expect(svg.props.viewBox).toBe('0 0 640 360');

    // One element per scene node — the renderer draws primitives, not a chart
    // library's idea of a chart.
    expect(view.UNSAFE_getAllByType(Path)).toHaveLength(1);
    expect(view.UNSAFE_getAllByType(Line)).toHaveLength(1);
    const svgText = view.UNSAFE_getAllByType(SvgText);
    expect(svgText).toHaveLength(1);
    expect(svgText[0].props.children).toBe('Semaine 1');
    expect(svgText[0].props.fontSize).toBe(13);

    // The path is drawn from the scene's geometry verbatim.
    expect(view.UNSAFE_getAllByType(Path)[0].props.d).toBe(CHART_PATH_D);

    // Chrome the athlete sees around the chart.
    expect(view.getByText('Charge hebdomadaire, 4 dernieres semaines')).toBeTruthy();
    expect(view.getByText('TSS hebdo')).toBeTruthy();
    expect(view.getByText('source: get_activities')).toBeTruthy();
    expect(view.getByLabelText('Chart: Charge hebdomadaire, 4 dernieres semaines')).toBeTruthy();

    // Two reads, and only two: the transcript and the verdicts attached to it.
    // Nothing opens a second stream to correlate a turn with.
    expect(stub.requestsFor('GET').map((request) => request.url)).toEqual([
      MESSAGES_URL,
      VERDICTS_URL,
    ]);
  });

  it('draws the scene on the LIVE turn, not only after a reload', async () => {
    // The regression this pins: mobile lifted the activity list and the actions
    // off the envelope but never the scenes, so a chart asked for in
    // conversation showed the athlete a bare ⟦viz:0⟧ marker until the
    // conversation was reloaded and the persisted row supplied scene_blocks.
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        data: assistantTurn({
          // The prose keeps its ⟦viz:0⟧ marker to say where the scene lands,
          // exactly as the envelope carries it on a live turn.
          content: `${PROSE_OPENING}\n\n⟦viz:0⟧\n\n${PROSE_CLOSING}`,
          blocks: [
            { type: 'prose', text: `${PROSE_OPENING}\n\n⟦viz:0⟧\n\n${PROSE_CLOSING}` },
            { type: 'scene', scene_blocks: SCENE_BLOCKS_JSON },
          ],
        }),
      },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, 'Montre-moi ma charge.');
    });

    const assistant = result.current.messages.find((message) => message.role === 'assistant');
    expect(assistant?.scene_blocks).toBe(SCENE_BLOCKS_JSON);

    const view = renderList(result.current.messages);
    const svg = view.UNSAFE_getByType(Svg);
    expect(svg.props.viewBox).toBe('0 0 640 360');
    expect(view.UNSAFE_getAllByType(Path)[0].props.d).toBe(CHART_PATH_D);

    // And the raw marker never reaches the athlete.
    expect(view.queryByText(/⟦viz:0⟧/)).toBeNull();
  });

  it('renders a verdict chip with the status the block reported', async () => {
    // carnet #56: `rg verdict frontend-mobile/src` returned NOTHING before this
    // — the server sent a `verdicts` block to a client with no arm for it, so a
    // flagged claim reached the athlete unmarked. Deleting the arm turns this red.
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        data: assistantTurn({
          content: 'Ton VO2max est de 82.',
          blocks: [
            { type: 'prose', text: 'Ton VO2max est de 82.' },
            {
              type: 'verdicts',
              chips: [
                { claim: 'Ton VO2max est de 82.', contradicted: true },
                { claim: 'Six heures de sommeil suffisent.', contradicted: false },
              ],
            },
          ],
        }),
      },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, 'Quel est mon VO2max ?');
    });

    const view = renderList(result.current.messages, result.current.messageBlocks);
    // The shared rollup picked the worst of the two, and the chip says which.
    expect(view.getByText('2 verdicts · contradicted')).toBeTruthy();
    expect(view.getByLabelText('Claim verdicts: contradicted')).toBeTruthy();
  });

  it('renders the reconnect call to action from the block, not from a URL in the prose', async () => {
    // The regression this turns red: the deleted
    // `/https?:\/\/\S*\/providers\/sciotte\/login\?token=\S+/` scrape coming back.
    // The prose here carries no URL at all, so a regex-driven renderer draws
    // no button and the athlete has nothing to tap.
    const reconnectUrl = 'https://app.dravr.ai/providers/sciotte/login?token=one-time-abc';
    const onOpenUrl = jest.fn();
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        data: assistantTurn({
          content: 'Je dois te reconnecter avant de lire ca.',
          blocks: [
            { type: 'prose', text: 'Je dois te reconnecter avant de lire ca.' },
            {
              type: 'reconnect',
              provider: 'whoop',
              display_name: 'WHOOP',
              url: reconnectUrl,
              text: 'Reconnecte WHOOP pour continuer.',
            },
          ],
        }),
      },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, 'Quel est mon recovery ?');
    });

    const view = render(
      <MessageList
        messages={result.current.messages}
        coaches={[]}
        isLoading={false}
        isSending={false}
        isCoachConversation
        messageFeedback={{}}
        messageFeedbackComment={{}}
        messageBlocks={result.current.messageBlocks}
        flatListRef={React.createRef()}
        onScrollToBottom={jest.fn()}
        onCoachSelect={jest.fn()}
        onThumbsUp={jest.fn()}
        onThumbsDown={jest.fn()}
        onSubmitFeedbackReason={jest.fn()}
        onRetryMessage={jest.fn()}
        onOpenUrl={onOpenUrl}
      />
    );

    fireEvent.press(view.getByText('Reconnect WHOOP'));
    expect(onOpenUrl).toHaveBeenCalledWith(reconnectUrl);
    // And the one-time token URL is never printed beside the control.
    expect(view.queryByText(reconnectUrl)).toBeNull();
  });

  it('chips a conversation read back from history, from the verdict rows it fetched', async () => {
    // The history path has no block list on the wire. Without the verdict read
    // wired in, a flagged claim would be chipped on the turn that produced it
    // and bare forever after — so this fails if the fetch is removed.
    stub = installHttpStub({
      [`GET ${MESSAGES_URL}`]: { data: listedMessages() },
      [`GET ${VERDICTS_URL}`]: {
        data: {
          verdicts: [
            {
              id: 'verdict-1',
              conversation_id: CONVERSATION_ID,
              message_id: 'msg-assistant-1',
              coach_id: 'coach-tempo',
              claim_text: 'Ton VO2max est de 82.',
              category: 'physiological',
              status: 'contradicted',
              evidence_strength: 'none',
              confidence: 0.91,
              layer_fired: 'deterministic',
              explanation: null,
              evidence_refs: null,
              created_at: '2026-08-22T10:00:05Z',
            },
          ],
          total: 1,
        },
      },
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.loadMessages(CONVERSATION_ID);
    });

    await waitFor(() => expect(result.current.verdicts).toHaveLength(1));
    expect(result.current.verdicts[0].claim_text).toBe('Ton VO2max est de 82.');

    const view = render(
      <MessageList
        messages={result.current.messages}
        coaches={[]}
        isLoading={false}
        isSending={false}
        isCoachConversation
        messageFeedback={{}}
        messageFeedbackComment={{}}
        messageBlocks={result.current.messageBlocks}
        verdicts={result.current.verdicts}
        flatListRef={React.createRef()}
        onScrollToBottom={jest.fn()}
        onCoachSelect={jest.fn()}
        onThumbsUp={jest.fn()}
        onThumbsDown={jest.fn()}
        onSubmitFeedbackReason={jest.fn()}
        onRetryMessage={jest.fn()}
        onOpenUrl={jest.fn()}
      />
    );

    // The rows carry an evidence strength, so the chip qualifies with it
    // rather than with the status the block-only path shows.
    expect(view.getByText('1 verdict · none')).toBeTruthy();
  });
});
