// ABOUTME: The support reference a verdict copies — its own id, then the message and conversation it came from
// ABOUTME: A row missing either id copies only what it carries, never an empty "message " line

import { describe, it, expect } from 'vitest';
import { verdictSupportReference } from '../src/verdict-support';

describe('verdictSupportReference', () => {
  it('names the verdict, its message and its conversation, one line each', () => {
    expect(
      verdictSupportReference({ id: 'verdict-3', message_id: 'm1', conversation_id: 'conv-1' }),
    ).toBe('verdict verdict-3\nmessage m1\nconversation conv-1');
  });

  it('leaves out the ids a row did not carry', () => {
    expect(verdictSupportReference({ id: 'verdict-4', message_id: null, conversation_id: null })).toBe(
      'verdict verdict-4',
    );
  });
});
