// ABOUTME: Unit tests for the unified conversation-list row model both clients render
// ABOUTME: Pins kind precedence, initials, the avatar hash, preview prefixes, timestamp buckets, sort and search

import { describe, it, expect } from 'vitest';
import type { Conversation } from '@pierre/shared-types';
import {
  AVATAR_SLOTS,
  UNTITLED_CONVERSATION,
  avatarSlot,
  buildConversationRow,
  deriveKind,
  filterRows,
  formatListTimestamp,
  initialsFor,
  previewFor,
  sortRowsByActivity,
} from '../src/conversation-row';

function conversation(overrides: Partial<Conversation> = {}): Conversation {
  return {
    id: 'conv-1',
    title: 'Marathon plan',
    coach_id: null,
    group_id: null,
    channel_type: 'web',
    message_count: 4,
    created_at: '2026-08-20T10:00:00Z',
    updated_at: '2026-08-26T10:00:00Z',
    ...overrides,
  };
}

/** A fixed "now": Thursday 2026-08-27 at 15:00, in the test runner's own zone. */
const NOW = new Date(2026, 7, 27, 15, 0, 0);

function localIso(year: number, month: number, day: number, hour = 9, minute = 50): string {
  return new Date(year, month - 1, day, hour, minute, 0).toISOString();
}

describe('deriveKind', () => {
  it('ranks group over channel over coach over plain', () => {
    expect(
      deriveKind(conversation({ group_id: 'g1', channel_type: 'telegram', coach_id: 'c1' })),
    ).toBe('group');
    expect(deriveKind(conversation({ channel_type: 'telegram', coach_id: 'c1' }))).toBe('channel');
    expect(deriveKind(conversation({ coach_id: 'c1' }))).toBe('coach');
    expect(deriveKind(conversation())).toBe('plain');
  });

  it('reads a channel from the legacy "Messaging: <channel>" title too', () => {
    expect(deriveKind(conversation({ title: 'Messaging: whatsapp', channel_type: 'web' }))).toBe(
      'channel',
    );
  });
});

describe('initialsFor', () => {
  it('takes the first letter of the first two words, upper-cased', () => {
    expect(initialsFor('marathon plan')).toBe('MP');
    expect(initialsFor('Sunday long run')).toBe('SL');
  });

  it('yields one letter for a single word and skips opening punctuation', () => {
    expect(initialsFor('Tempo')).toBe('T');
    expect(initialsFor('«Plan» "B"')).toBe('PB');
    expect(initialsFor('@tempo-coach')).toBe('T');
  });

  it('yields ? when there is nothing to take a letter from', () => {
    expect(initialsFor('')).toBe('?');
    expect(initialsFor('   ')).toBe('?');
  });
});

describe('avatarSlot', () => {
  it('is deterministic and stays inside the palette', () => {
    const ids = Array.from({ length: 200 }, (_, i) => `conv-${i}`);
    const slots = ids.map((id) => avatarSlot({ id, coach_id: null, group_id: null }));
    expect(slots).toEqual(ids.map((id) => avatarSlot({ id, coach_id: null, group_id: null })));
    expect(slots.every((slot) => Number.isInteger(slot) && slot >= 0 && slot < AVATAR_SLOTS)).toBe(
      true,
    );
    // Two hundred ids over six colours must actually spread; a hash that
    // collapsed onto one slot would pass a range check and still be useless.
    expect(new Set(slots).size).toBe(AVATAR_SLOTS);
  });

  it('keys on the group, then the coach, then the conversation', () => {
    const byGroup = avatarSlot({ id: 'a', coach_id: 'coach-x', group_id: 'group-z' });
    expect(byGroup).toBe(avatarSlot({ id: 'b', coach_id: 'coach-y', group_id: 'group-z' }));

    const byCoach = avatarSlot({ id: 'a', coach_id: 'coach-x', group_id: null });
    expect(byCoach).toBe(avatarSlot({ id: 'b', coach_id: 'coach-x', group_id: null }));

    expect(avatarSlot({ id: 'a', coach_id: null, group_id: null })).toBe(
      avatarSlot({ id: 'a', coach_id: null, group_id: null }),
    );
  });
});

describe('previewFor', () => {
  const assistant = { preview: 'Ease off this week.', role: 'assistant' as const, created_at: '2026-08-26T10:00:00Z' };
  const user = { preview: 'How was my week?', role: 'user' as const, created_at: '2026-08-26T10:00:00Z' };

  it('prefixes the athlete\'s own rows with You: on every kind of row', () => {
    expect(previewFor(conversation({ last_message: user }))).toBe('You: How was my week?');
    expect(previewFor(conversation({ group_id: 'g1', coach_title: 'Phil', last_message: user }))).toBe(
      'You: How was my week?',
    );
  });

  it('names the coach only in a group row', () => {
    expect(
      previewFor(conversation({ group_id: 'g1', coach_title: 'Marathon Coach', last_message: assistant })),
    ).toBe('Marathon Coach: Ease off this week.');
    expect(previewFor(conversation({ coach_id: 'c1', coach_title: 'Marathon Coach', last_message: assistant }))).toBe(
      'Ease off this week.',
    );
    expect(previewFor(conversation({ channel_type: 'telegram', last_message: assistant }))).toBe(
      'Ease off this week.',
    );
  });

  it('falls to "Coach" in a group whose coach no longer exists', () => {
    expect(previewFor(conversation({ group_id: 'g1', coach_title: null, last_message: assistant }))).toBe(
      'Coach: Ease off this week.',
    );
  });

  it('is empty for a thread with no message yet', () => {
    expect(previewFor(conversation())).toBe('');
    expect(previewFor(conversation({ last_message: null }))).toBe('');
  });
});

describe('formatListTimestamp', () => {
  it('shows the clock time for today', () => {
    expect(formatListTimestamp(localIso(2026, 8, 27, 9, 50), NOW)).toBe('09:50');
    expect(formatListTimestamp(localIso(2026, 8, 27, 0, 5), NOW)).toBe('00:05');
  });

  it('shows the weekday inside the last week', () => {
    expect(formatListTimestamp(localIso(2026, 8, 26), NOW)).toBe('Wed');
    expect(formatListTimestamp(localIso(2026, 8, 24), NOW)).toBe('Mon');
    expect(formatListTimestamp(localIso(2026, 8, 21), NOW)).toBe('Fri');
  });

  it('shows the date from seven days back, with the year once it is another year', () => {
    expect(formatListTimestamp(localIso(2026, 8, 20), NOW)).toBe('Aug 20');
    expect(formatListTimestamp(localIso(2026, 1, 3), NOW)).toBe('Jan 3');
    expect(formatListTimestamp(localIso(2025, 12, 31), NOW)).toBe('Dec 31, 2025');
  });

  it('is empty for a stamp that does not parse', () => {
    expect(formatListTimestamp('not a date', NOW)).toBe('');
  });
});

describe('buildConversationRow', () => {
  it('assembles every field a row draws', () => {
    const row = buildConversationRow(
      conversation({
        id: 'conv-9',
        title: 'Harricana 80',
        group_id: 'group-1',
        group_name: 'Harricana crew',
        coach_id: 'coach-1',
        coach_handle: 'trail-coach',
        coach_title: 'Trail Coach',
        unread_count: 3,
        last_message: {
          preview: 'Long run Sunday, 28 km.',
          role: 'assistant',
          created_at: localIso(2026, 8, 27, 9, 50),
        },
      }),
      NOW,
    );

    expect(row).toEqual({
      id: 'conv-9',
      kind: 'group',
      title: 'Harricana 80',
      coachHandle: 'trail-coach',
      coachTitle: 'Trail Coach',
      groupName: 'Harricana crew',
      channel: null,
      initials: 'H8',
      avatarSlot: avatarSlot({ id: 'conv-9', coach_id: 'coach-1', group_id: 'group-1' }),
      preview: 'Trail Coach: Long run Sunday, 28 km.',
      timestamp: '09:50',
      unreadCount: 3,
      lastActivityAt: localIso(2026, 8, 27, 9, 50),
    });
  });

  it('names an untitled thread and stamps it from updated_at when it has no message', () => {
    const row = buildConversationRow(
      conversation({ title: null, updated_at: localIso(2026, 8, 24, 8, 0) }),
      NOW,
    );
    expect(row.title).toBe(UNTITLED_CONVERSATION);
    expect(row.initials).toBe('UC');
    expect(row.preview).toBe('');
    expect(row.timestamp).toBe('Mon');
    expect(row.lastActivityAt).toBe(localIso(2026, 8, 24, 8, 0));
    expect(row.unreadCount).toBe(0);
  });

  it('badges a messaging-origin thread from its channel_type', () => {
    const row = buildConversationRow(conversation({ channel_type: 'telegram' }), NOW);
    expect(row.kind).toBe('channel');
    expect(row.channel).toEqual({ channel: 'telegram', label: 'Telegram' });
  });
});

describe('sortRowsByActivity', () => {
  it('orders newest activity first, by the last message or else updated_at', () => {
    const quiet = buildConversationRow(
      conversation({ id: 'quiet', updated_at: '2026-08-25T10:00:00Z' }),
      NOW,
    );
    const busy = buildConversationRow(
      conversation({
        id: 'busy',
        updated_at: '2026-08-20T10:00:00Z',
        last_message: { preview: 'hi', role: 'user', created_at: '2026-08-27T10:00:00Z' },
      }),
      NOW,
    );
    const middle = buildConversationRow(
      conversation({ id: 'middle', updated_at: '2026-08-26T10:00:00Z' }),
      NOW,
    );

    const input = [quiet, busy, middle];
    expect(sortRowsByActivity(input).map((row) => row.id)).toEqual(['busy', 'middle', 'quiet']);
    // The caller's array is not reordered in place.
    expect(input.map((row) => row.id)).toEqual(['quiet', 'busy', 'middle']);
  });
});

describe('filterRows', () => {
  const rows = [
    buildConversationRow(
      conversation({ id: 'a', title: 'Marathon plan', coach_id: 'c1', coach_handle: 'marathon-coach' }),
      NOW,
    ),
    buildConversationRow(
      conversation({
        id: 'b',
        title: 'Tuesday',
        last_message: { preview: 'Deadlift form check', role: 'user', created_at: '2026-08-27T10:00:00Z' },
      }),
      NOW,
    ),
    buildConversationRow(conversation({ id: 'c', title: 'Recovery week' }), NOW),
  ];

  it('matches the title, the coach handle and the preview, case-insensitively', () => {
    expect(filterRows(rows, 'MARATHON').map((row) => row.id)).toEqual(['a']);
    expect(filterRows(rows, 'marathon-co').map((row) => row.id)).toEqual(['a']);
    expect(filterRows(rows, '@marathon').map((row) => row.id)).toEqual(['a']);
    expect(filterRows(rows, 'deadlift').map((row) => row.id)).toEqual(['b']);
    expect(filterRows(rows, 'kettlebell')).toEqual([]);
  });

  it('keeps every row for a blank query', () => {
    expect(filterRows(rows, '   ').map((row) => row.id)).toEqual(['a', 'b', 'c']);
  });
});
