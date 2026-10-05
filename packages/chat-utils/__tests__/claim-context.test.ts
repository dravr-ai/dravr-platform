// ABOUTME: The window a verdict's source preview shows — the claim found in its reply, context either side
// ABOUTME: Markdown and chart markers never reach the preview, and a missing claim still shows the reply

import { describe, it, expect } from 'vitest';
import { claimInContext } from '../src/claim-context';

const REPLY = [
  'Je n’ai pas tes zones de FC enregistrées, donc c’est une **estimation**.',
  '',
  '- Si tu voulais un vrai footing facile : vise 5 à 10 bpm de moins.',
  '- Si tu t’es senti fluide : tu peux considérer la séance comme bonne.',
].join('\n');

describe('claimInContext', () => {
  it('splits the reply around the claim, as plain prose', () => {
    const context = claimInContext(
      REPLY,
      'Si tu t’es senti fluide : tu peux considérer la séance comme bonne.',
    );

    expect(context.match).toBe('Si tu t’es senti fluide : tu peux considérer la séance comme bonne.');
    // The bold and the list bullets are gone; the lines read as one paragraph.
    expect(context.before).toBe(
      'Je n’ai pas tes zones de FC enregistrées, donc c’est une estimation. Si tu voulais un vrai footing facile : vise 5 à 10 bpm de moins. ',
    );
    expect(context.after).toBe('');
  });

  it('matches a claim the verifier stored with its list bullet', () => {
    // The verifier can store the claim with the reply's own "- " bullet.
    const context = claimInContext(REPLY, '- Si tu voulais un vrai footing facile : vise 5 à 10 bpm de moins.');

    expect(context.match).toBe('Si tu voulais un vrai footing facile : vise 5 à 10 bpm de moins.');
    expect(context.after.startsWith(' Si tu t’es senti fluide')).toBe(true);
  });

  it('falls back to a case-insensitive match', () => {
    const context = claimInContext('Easy runs: KEEP IT CONVERSATIONAL. Then rest.', 'keep it conversational.');

    expect(context.match).toBe('KEEP IT CONVERSATIONAL.');
  });

  it('cuts long context on word boundaries and marks each cut', () => {
    const long = `${'warm up well '.repeat(30)}The claim sits here.${' cool down after'.repeat(30)}`;
    const context = claimInContext(long, 'The claim sits here.', 40);

    expect(context.before.startsWith('…')).toBe(true);
    expect(context.before.length).toBeLessThanOrEqual(41);
    expect(context.before.startsWith('… ')).toBe(false);
    expect(context.after.endsWith('…')).toBe(true);
    expect(context.after.length).toBeLessThanOrEqual(41);
  });

  it('drops chart markers and tool scaffolding', () => {
    const context = claimInContext(
      'Your week:\n⟦viz:0⟧\n<tool_call>{"name":"x"}</tool_call>Load is steady.',
      'Load is steady.',
    );

    expect(context.before).toBe('Your week: ');
    expect(context.match).toBe('Load is steady.');
  });

  it('still shows the opening of the reply when the claim is not in it', () => {
    const context = claimInContext('A **short** reply.', 'Something the reply never said.');

    expect(context).toEqual({ before: 'A short reply.', match: '', after: '' });
  });
});
