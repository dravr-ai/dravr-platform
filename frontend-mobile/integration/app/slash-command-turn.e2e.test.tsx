// ABOUTME: PHASE 2 e2e — a slash-command turn is told apart by finish_reason, not the deleted is_command_response
// ABOUTME: The command turn's actions block becomes buttons whose postback value re-enters the same send path

import React from 'react';
import { render, act, fireEvent } from '@testing-library/react-native';
import { COMMAND_DRAFTS } from '@pierre/shared-constants';
import type { Message, TurnEnvelope } from '@pierre/shared-types';

import { renderHook } from './helpers/queryHook';
import { installHttpStub, type HttpStub } from './helpers/httpStub';
import { CONVERSATION_ID, assistantTurn } from './helpers/chatFixtures';

import { chatApi } from '../../src/services/api';
import { useMessages } from '../../src/screens/chat/useMessages';
import type { ChatMessageAction, ReplyBlock } from '@pierre/shared-types';
import { MessageList } from '../../src/screens/chat/MessageList';

const MESSAGES_URL = `/api/chat/conversations/${CONVERSATION_ID}/messages`;

/**
 * Send one turn through the shared `sendTurn` and hand back the envelope.
 *
 * `sendTurn` reports through callbacks rather than a return value — that is
 * how the same method serves the web client, which renders deltas as they
 * land. A spec that only wants the finished turn collects it here.
 */
async function takeTurn(content: string): Promise<TurnEnvelope> {
  let turn: TurnEnvelope | null = null;
  let failure: Error | null = null;
  await chatApi.sendTurn(CONVERSATION_ID, content, {
    onDone: (done) => {
      turn = done;
    },
    onError: (error) => {
      failure = error;
    },
  });
  if (failure) throw failure;
  if (!turn) throw new Error(`sendTurn reported neither a turn nor an error for ${content}`);
  return turn;
}

const COACH_PICKER_PROSE = 'Choisis ton coach :';
// The postback is the command an athlete could type, spelled by the shared
// drafts — `/coach select <uuid>` is gone from the tree.
const ADD_TEMPO = COMMAND_DRAFTS.coachAdd('coach-tempo');
const ADD_RECUP = COMMAND_DRAFTS.coachAdd('coach-recup');

/** What `/coach list` answers with: prose plus one button per coach. */
const commandTurn = () =>
  assistantTurn({
    content: COACH_PICKER_PROSE,
    finishReason: 'command',
    blocks: [
      { type: 'prose', text: COACH_PICKER_PROSE },
      {
        type: 'actions',
        title: 'Tes coachs',
        actions: [
          { label: 'Coach Tempo', action_type: 'postback', value: ADD_TEMPO },
          { label: 'Coach Recup', action_type: 'postback', value: ADD_RECUP },
        ],
      },
    ],
  });

function renderList(
  messages: Message[],
  messageBlocks: Record<string, ReplyBlock[]>,
  onActionClick: (action: ChatMessageAction) => void
) {
  return render(
    <MessageList
      bottomInset={0}
      messages={messages}
      isLoading={false}
      isSending={false}
      messageFeedback={{}}
      messageFeedbackComment={{}}
      messageBlocks={messageBlocks}
      flatListRef={React.createRef()}
      onScrollToBottom={jest.fn()}
      onThumbsUp={jest.fn()}
      onThumbsDown={jest.fn()}
      onSubmitFeedbackReason={jest.fn()}
      onRetryMessage={jest.fn()}
      onOpenUrl={jest.fn()}
      onReconnectProvider={jest.fn()}
      onActionClick={onActionClick}
    />
  );
}

describe('PHASE 2 — slash-command turns', () => {
  let stub: HttpStub;

  afterEach(() => {
    stub.restore();
  });

  it('marks the command turn with finish_reason "command" and the LLM turn with "stop"', async () => {
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: (request) => ({
        data:
          (request.body as { content: string }).content === '/coach list'
            ? commandTurn()
            : assistantTurn(),
      }),
    });

    const command = await takeTurn('/coach list');
    const llm = await takeTurn('Comment se presente ma semaine ?');

    expect(command.assistant.finish_reason).toBe('command');
    expect(llm.assistant.finish_reason).toBe('stop');

    // The flag the clients used to branch on is gone from the wire; the turn
    // carries nothing else that names a command.
    expect(Object.keys(command.assistant).sort()).toEqual(['blocks', 'finish_reason', 'message']);
    expect('is_command_response' in command).toBe(false);
    expect('card_title' in command.assistant).toBe(false);

    // The command turn's controls ride a typed block, titled by the server.
    const actions = command.assistant.blocks.find((block) => block.type === 'actions');
    expect(actions).toEqual({
      type: 'actions',
      title: 'Tes coachs',
      actions: [
        { label: 'Coach Tempo', action_type: 'postback', value: ADD_TEMPO },
        { label: 'Coach Recup', action_type: 'postback', value: ADD_RECUP },
      ],
    });
    // The LLM turn carries no controls at all.
    expect(llm.assistant.blocks.some((block) => block.type === 'actions')).toBe(false);
  });

  it('renders the command turn as buttons and posts the pressed value back', async () => {
    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: (request) => ({
        data:
          (request.body as { content: string }).content === '/coach list'
            ? commandTurn()
            : assistantTurn({ content: 'Coach Tempo est actif.' }),
      }),
    });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, '/coach list');
    });

    // The actions block reached the renderer as one of the turn's own blocks.
    const actions = result.current.messageBlocks['msg-assistant-1'].find(
      (block) => block.type === 'actions',
    );
    expect(actions).toEqual({
      type: 'actions',
      title: 'Tes coachs',
      actions: [
        { label: 'Coach Tempo', action_type: 'postback', value: ADD_TEMPO },
        { label: 'Coach Recup', action_type: 'postback', value: ADD_RECUP },
      ],
    });

    const view = renderList(result.current.messages, result.current.messageBlocks, (action) => {
      void result.current.sendTurn(CONVERSATION_ID, action.value);
    });
    expect(view.getByText(COACH_PICKER_PROSE)).toBeTruthy();
    expect(view.getByText('Coach Tempo')).toBeTruthy();
    expect(view.getByText('Coach Recup')).toBeTruthy();

    // Pressing one re-posts its value through the same send path.
    await act(async () => {
      fireEvent.press(view.getByText('Coach Tempo'));
    });

    const posted = stub.requestsFor('POST').map((request) => (request.body as { content: string }).content);
    expect(posted).toEqual(['/coach list', ADD_TEMPO]);
  });

  it('leaves an LLM turn without any action buttons', async () => {
    stub = installHttpStub({ [`POST ${MESSAGES_URL}`]: { data: assistantTurn() } });

    const { result } = renderHook(() => useMessages());
    await act(async () => {
      await result.current.sendTurn(CONVERSATION_ID, 'Comment se presente ma semaine ?');
    });

    const blocks = result.current.messageBlocks['msg-assistant-1'] ?? [];
    expect(blocks.some((block) => block.type === 'actions')).toBe(false);

    const view = renderList(result.current.messages, result.current.messageBlocks, jest.fn());
    expect(view.queryByText('Coach Tempo')).toBeNull();
  });
});
