// ABOUTME: Server-shaped chat payloads for the mobile app e2e specs — one TurnEnvelope, one history read
// ABOUTME: Mirrors crates/pierre-server/src/routes/chat/turn_response.rs, including its omitted fields

import type { MessagesResponse, ReplyBlock, TurnEnvelope } from '@pierre/api-client';
import type { Message } from '@pierre/shared-types';
import type { RenderBlock } from '@pierre/scene-types';

export const CONVERSATION_ID = 'conv-e2e-1';
export const ASSISTANT_MESSAGE_ID = 'msg-assistant-1';
export const USER_MESSAGE_ID = 'msg-user-1';

/** The coach's opening sentence, the reply's `prose` block. */
export const PROSE_OPENING = 'Ta charge monte depuis trois semaines.';
/** The sentence after the chart marker, in the same prose block. */
export const PROSE_CLOSING = 'On garde le jeudi facile.';

/** The series path of the resolved chart, asserted verbatim against the SVG. */
export const CHART_PATH_D = 'M40 300 L200 240 L360 180 L520 120';

/**
 * A chart already resolved into positioned primitives.
 *
 * Three nodes, one of each kind the assertions count: an axis line, the series
 * path, and one tick label. Nothing here is computed on the client.
 */
export const CHART_BLOCK: RenderBlock = {
  kind: 'chart',
  view_box: { width: 640, height: 360 },
  nodes: [
    { node: 'line', x1: 40, y1: 320, x2: 600, y2: 320, stroke: 'axis', width: 1 },
    { node: 'path', d: CHART_PATH_D, stroke: 'activity', width: 2 },
    {
      node: 'text',
      x: 40,
      y: 340,
      content: 'Semaine 1',
      anchor: 'start',
      baseline: 'hanging',
      role: 'axis_tick',
      color: 'label',
    },
  ],
  legend: [{ label: 'TSS hebdo', color: 'activity' }],
  title: 'Charge hebdomadaire, 4 dernieres semaines',
  source_tool: 'get_activities',
};

/** `scene_blocks` as the wire carries it: a JSON-encoded `RenderBlock[]`. */
export const SCENE_BLOCKS_JSON = JSON.stringify([CHART_BLOCK]);

/** The athlete's activities, which the in-app surface draws as its own panel. */
export const ACTIVITY_LIST_TEXT = [
  '1. Sortie longue - 24 km - 2h02',
  '2. Seuil 3x10 - 14 km - 1h05',
].join('\n');

const USER_MESSAGE: Message = {
  id: USER_MESSAGE_ID,
  conversation_id: CONVERSATION_ID,
  role: 'user',
  content: 'Comment se presente ma semaine ?',
  created_at: '2026-08-22T10:00:00Z',
};

/**
 * One completed turn, shaped like `TurnResponse`.
 *
 * `assistant.message.scene_blocks` is deliberately absent: on a live turn the
 * server sends charts as a `scene` block and leaves the persisted row's field
 * empty, so a client that read both would draw every chart twice.
 */
export function assistantTurn(overrides: {
  content?: string;
  blocks?: ReplyBlock[];
  finishReason?: string;
} = {}): TurnEnvelope {
  const {
    content = PROSE_OPENING,
    blocks = [
      { type: 'prose', text: content },
      { type: 'activity_list', text: ACTIVITY_LIST_TEXT },
    ],
    finishReason = 'stop',
  } = overrides;

  return {
    turn_id: 'turn-e2e-1',
    user_message: USER_MESSAGE,
    assistant: {
      message: {
        id: ASSISTANT_MESSAGE_ID,
        conversation_id: CONVERSATION_ID,
        role: 'assistant',
        content,
        created_at: '2026-08-22T10:00:04Z',
        token_count: 180,
      },
      blocks,
      finish_reason: finishReason,
    },
    conversation_updated_at: '2026-08-22T10:00:04Z',
    telemetry: {
      model: 'claude-sonnet-4-6',
      provider_name: 'anthropic',
      tool_calls_count: 1,
      tools_called: ['get_activities'],
      execution_time_ms: 4210,
    },
  };
}

/**
 * The same turn read back from history.
 *
 * Here the assistant row does carry `scene_blocks` — there is no block list on
 * the history path to position the charts against, so the field is where they
 * live, and the prose keeps its ⟦viz:0⟧ marker to say where.
 */
export function listedMessages(): MessagesResponse {
  return {
    messages: [
      USER_MESSAGE,
      {
        id: ASSISTANT_MESSAGE_ID,
        conversation_id: CONVERSATION_ID,
        role: 'assistant',
        content: `${PROSE_OPENING}\n\n⟦viz:0⟧\n\n${PROSE_CLOSING}`,
        created_at: '2026-08-22T10:00:04Z',
        token_count: 180,
        scene_blocks: SCENE_BLOCKS_JSON,
      },
    ],
  };
}
