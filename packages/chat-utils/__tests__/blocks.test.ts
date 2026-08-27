// ABOUTME: Unit tests for decoding a persisted transcript row into a live turn's ReplyBlock shape
// ABOUTME: Red when a history read stops producing the block a live turn carries for the same reply

import { describe, it, expect } from 'vitest';
import type { ClaimVerdict, Message } from '@pierre/shared-types';
import { transcriptBlocks } from '../src/blocks';

const CHART = {
  kind: 'chart',
  view_box: { width: 640, height: 360 },
  nodes: [{ node: 'path', d: 'M40 300 L520 120', stroke: 'activity', width: 2 }],
  title: 'Weekly load',
};

const PLAN = { kind: 'workout_plan', plan: { plan_window: { start: '2026-09-01', end: '2026-09-07' } } };

function assistantRow(overrides: Partial<Message> = {}): Message {
  return {
    id: 'msg-1',
    role: 'assistant',
    content: 'Your load is climbing.',
    created_at: '2026-08-24T10:00:00Z',
    ...overrides,
  };
}

function verdictRow(overrides: Partial<ClaimVerdict> = {}): ClaimVerdict {
  return {
    id: 'v1',
    conversation_id: 'conv-1',
    message_id: 'msg-1',
    coach_id: 'coach-1',
    claim_text: 'Your VO2max is 82.',
    category: 'physiological',
    status: 'contradicted',
    evidence_strength: 'none',
    confidence: 0.9,
    layer_fired: 'deterministic',
    explanation: null,
    evidence_refs: null,
    created_at: '2026-08-24T10:00:01Z',
    ...overrides,
  };
}

describe('transcriptBlocks', () => {
  it('decodes a plain assistant row into exactly one prose block', () => {
    expect(transcriptBlocks(assistantRow())).toEqual([
      { type: 'prose', text: 'Your load is climbing.' },
    ]);
  });

  it('lifts a baked-in activity list into its own block, ahead of the prose', () => {
    const row = assistantRow({
      content:
        'Your Activities:\n1. Long run - 24 km - 2h02\n2. Threshold 3x10 - 14 km - 1h05' +
        '\n\n---\n\n**Analysis:**\n\n' +
        'Your load is climbing.',
    });

    const blocks = transcriptBlocks(row);
    expect(blocks[0].type).toBe('activity_list');
    expect(blocks[0]).toHaveProperty('text');
    const list = blocks[0] as { type: 'activity_list'; text: string };
    expect(list.text).toContain('1. Long run - 24 km - 2h02');
    expect(blocks[1]).toEqual({ type: 'prose', text: 'Your load is climbing.' });
  });

  it('keeps scene_blocks verbatim so the prose markers still index them', () => {
    // The plan sits at index 1 of the stored array. Filtering it out of the
    // `scene` block would shift ⟦viz:1⟧ onto the wrong chart.
    const sceneBlocks = JSON.stringify([CHART, PLAN]);
    const blocks = transcriptBlocks(
      assistantRow({ content: 'Look: ⟦viz:0⟧', scene_blocks: sceneBlocks }),
    );

    expect(blocks).toEqual([
      { type: 'prose', text: 'Look: ⟦viz:0⟧' },
      { type: 'scene', scene_blocks: sceneBlocks },
      { type: 'workout_plan', plan: PLAN.plan },
    ]);
  });

  it('turns the conversation\'s verdict rows into the reply\'s verdicts block', () => {
    const blocks = transcriptBlocks(assistantRow(), [
      verdictRow(),
      verdictRow({ id: 'v2', claim_text: 'Sleep 6h is plenty.', status: 'unsupported' }),
    ]);

    expect(blocks[1]).toEqual({
      type: 'verdicts',
      chips: [
        { claim: 'Your VO2max is 82.', contradicted: true },
        { claim: 'Sleep 6h is plenty.', contradicted: false },
      ],
    });
  });

  it('strips residual tool scaffolding before anything else reads the content', () => {
    const blocks = transcriptBlocks(
      assistantRow({ content: 'Recovery is fine. <tool_result>{"hrv":65}</tool_result> Keep going.' }),
    );

    expect(blocks).toHaveLength(1);
    const prose = blocks[0] as { type: 'prose'; text: string };
    expect(prose.text).not.toContain('tool_result');
    expect(prose.text).toContain('Recovery is fine.');
    expect(prose.text).toContain('Keep going.');
  });

  it('re-emits the controls a persisted command reply carried, after the prose', () => {
    const actions = [
      { label: 'Marathon Coach', action_type: 'postback', value: '/coach add @marathon-coach' },
      { label: 'Trail Coach', action_type: 'postback', value: '/coach add @trail-coach' },
    ];
    const blocks = transcriptBlocks(
      assistantRow({
        content: 'Your installed coaches:',
        finish_reason: 'command',
        actions: { title: 'Pick a coach', actions },
      }),
    );

    expect(blocks).toEqual([
      { type: 'prose', text: 'Your installed coaches:' },
      { type: 'actions', title: 'Pick a coach', actions },
    ]);
  });

  it('emits the actions block without a title key when the reply had none', () => {
    const actions = [{ label: 'Yes', action_type: 'postback', value: '/coach create confirm abc' }];
    const blocks = transcriptBlocks(
      assistantRow({ content: 'Create this coach?', actions: { actions } }),
    );

    expect(blocks[1]).toEqual({ type: 'actions', actions });
    expect(blocks[1]).not.toHaveProperty('title');
  });

  it('draws no controls for a persisted reply whose action list is empty', () => {
    expect(transcriptBlocks(assistantRow({ actions: { title: 'Nothing', actions: [] } }))).toEqual([
      { type: 'prose', text: 'Your load is climbing.' },
    ]);
  });

  it('gives a user row a single prose block and nothing else', () => {
    expect(
      transcriptBlocks({
        id: 'msg-0',
        role: 'user',
        content: 'How was my week?',
        created_at: '2026-08-24T09:59:00Z',
      }),
    ).toEqual([{ type: 'prose', text: 'How was my week?' }]);
  });
});
