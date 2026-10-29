// ABOUTME: Unit tests for a coach's private lane in a one-athlete room — the divider, the audience, the quote and the composer words
// ABOUTME: Red if a private stretch loses its divider, a non-athlete row becomes quotable, or the switch names the wrong audience

import { describe, expect, it } from 'vitest';
import type { GroupTranscriptEntry, Message } from '@pierre/shared-types';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import { composeRoomThread } from '../src/room-thread';
import {
  PRIVATE_LANE_KEYS,
  audienceOf,
  opensPrivateStretch,
  privateLaneComposerText,
  roomQuoteOf,
} from '../src/private-lane';
import type { Translate } from '../src/text';

const ALICE = 'user-alice';
const COACH = 'user-coach';

/** A catalogue, resolved the way a client's `t` resolves a key and its params. */
function translatorFor(locale: string, catalogue: unknown): Translate {
  return (key, params = {}) => {
    const template = key
      .split('.')
      .reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], catalogue);
    if (typeof template !== 'string') throw new Error(`${locale} has no ${key}`);
    return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(params[name]));
  };
}

const t = translatorFor('en', en);
const CATALOGUES = { en, fr, es, de, pt };

function entry(overrides: Partial<GroupTranscriptEntry> & Pick<GroupTranscriptEntry, 'id' | 'created_at'>): GroupTranscriptEntry {
  return {
    speaker: 'member',
    withheld: false,
    own: false,
    author_user_id: ALICE,
    author_display_name: 'Alice Runner',
    content: 'Legs felt heavy this morning. Is Thursday still a good idea?',
    message_id: null,
    ...overrides,
  };
}

/** The room the coach reads: the athlete's words, the agent's reply to them, the coach's own line, and the private lane. */
function coachThread() {
  const room: GroupTranscriptEntry[] = [
    entry({ id: 'e-athlete', created_at: '2026-10-28T07:12:00Z' }),
    entry({ id: 'e-reply', created_at: '2026-10-28T07:13:00Z', speaker: 'coach', content: "I'd keep Thursday easy." }),
    entry({ id: 'e-withheld', created_at: '2026-10-28T07:14:00Z', withheld: true, author_user_id: null, author_display_name: null, content: null }),
    entry({ id: 'e-correction', created_at: '2026-10-28T07:15:00Z', speaker: 'correction', content: 'Thursday stays.' }),
    entry({ id: 'e-other', created_at: '2026-10-28T07:16:00Z', author_user_id: 'user-bob', author_display_name: 'Bob', content: 'see you saturday' }),
    entry({ id: 'e-coach', created_at: '2026-10-28T07:17:00Z', own: true, author_user_id: COACH, author_display_name: 'Casey', content: 'On it.' }),
  ];
  const own: Message[] = [{ id: 'own-1', role: 'user', content: "Agreed. I'll rework the week tonight.", created_at: '2026-10-28T07:30:00Z' }];
  const work: Message[] = [
    { id: 'work-1', role: 'user', content: 'Move Thursday to Saturday.', created_at: '2026-10-28T20:40:00Z' },
    { id: 'work-2', role: 'assistant', content: 'Done. Two days change.', created_at: '2026-10-28T20:41:00Z' },
  ];
  return composeRoomThread(own, room, work);
}

describe('audienceOf', () => {
  it('reads a row as private only when it is tagged so', () => {
    expect(audienceOf({ audience: 'private' })).toBe('private');
    expect(audienceOf({ audience: 'room' })).toBe('room');
    expect(audienceOf({})).toBe('room');
    expect(audienceOf({ audience: null })).toBe('room');
  });
});

describe('opensPrivateStretch', () => {
  it('draws one divider above each stretch of private rows, none inside it', () => {
    const thread = coachThread();
    const dividers = thread
      .map((row, index) => (opensPrivateStretch(thread[index - 1], row) ? row.id : null))
      .filter((id) => id !== null);

    expect(dividers).toEqual(['work-1']);
  });

  it('opens a stretch at the top of the thread, and again after the room speaks in between', () => {
    const privateRow = { audience: 'private' as const };
    expect(opensPrivateStretch(null, privateRow)).toBe(true);
    expect(opensPrivateStretch(undefined, privateRow)).toBe(true);
    expect(opensPrivateStretch({}, privateRow)).toBe(true);
    expect(opensPrivateStretch(privateRow, privateRow)).toBe(false);
    expect(opensPrivateStretch(privateRow, {})).toBe(false);
  });
});

describe('roomQuoteOf', () => {
  it("quotes the athlete's own room words, naming the transcript entry", () => {
    const athleteRow = coachThread().find((row) => row.id === 'room-e-athlete');
    expect(athleteRow).toBeDefined();

    expect(athleteRow && roomQuoteOf(athleteRow, ALICE)).toEqual({
      entry_id: 'e-athlete',
      author_user_id: ALICE,
      author_display_name: 'Alice Runner',
      content: 'Legs felt heavy this morning. Is Thursday still a good idea?',
      created_at: '2026-10-28T07:12:00Z',
    });
  });

  it("quotes nothing else: the agent's reply or a correction about the athlete, a withheld entry, another member, the coach's own words, a private row", () => {
    const thread = coachThread();
    const quotable = thread.filter((row) => roomQuoteOf(row, ALICE) !== null).map((row) => row.id);

    expect(quotable).toEqual(['room-e-athlete']);
  });

  it("is null when the room's one athlete is someone else", () => {
    const athleteRow = coachThread().find((row) => row.id === 'room-e-athlete');
    expect(athleteRow && roomQuoteOf(athleteRow, 'user-bob')).toBeNull();
  });

  it('is null for a blank line, which carries nothing to adjust from', () => {
    const [blank] = composeRoomThread([], [entry({ id: 'e-blank', created_at: '2026-10-28T08:00:00Z', content: '   ' })]);
    expect(roomQuoteOf(blank, ALICE)).toBeNull();
  });
});

describe('privateLaneComposerText', () => {
  const names = { agent: 'Dravr', athlete: 'Camille' };

  it('says the next send reaches the agent alone, and that the athlete will not see it', () => {
    expect(privateLaneComposerText('private', names, t)).toEqual({
      label: 'Only Dravr',
      aria: 'Only Dravr. Switch to everyone',
      placeholder: "Ask Dravr to change Camille's plan. Camille won't see this.",
    });
  });

  it('says the next send reaches the room', () => {
    expect(privateLaneComposerText('room', names, t)).toEqual({
      label: 'Everyone',
      aria: 'Everyone. Switch to only Dravr',
      placeholder: 'Message Camille and Dravr. Type / for commands',
    });
  });

  it('names a key the catalogue holds for every word of the lane', () => {
    const params = { agent: 'Dravr', name: 'Camille' };
    for (const key of Object.values(PRIVATE_LANE_KEYS)) {
      expect(t(key, params).length).toBeGreaterThan(0);
    }
  });

  // WCAG 2.5.3 Label in Name: a voice user says what they see, so the switch's
  // name starts with its label in every language — and the name carries the
  // state, which is why the switch announces no pressed state.
  it.each(Object.entries(CATALOGUES))('in %s, names the switch by the label it shows', (locale, catalogue) => {
    const translate = translatorFor(locale, catalogue);
    for (const audience of ['private', 'room'] as const) {
      const { label, aria } = privateLaneComposerText(audience, names, translate);
      expect(aria.startsWith(`${label}.`)).toBe(true);
    }
  });
});
