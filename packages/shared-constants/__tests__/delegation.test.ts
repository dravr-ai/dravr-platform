// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves every coaching-platform link refusal reads as a corpus key the en catalogue carries
// ABOUTME: An unknown reason reads as the generic failure; every platform the copy names reads as its brand

import { describe, it, expect } from 'vitest';
import en from '../../i18n/src/locales/en/translation.json';
import {
  COACH_CALENDAR_PLATFORM,
  COACH_CONSENTS_KEYS,
  DELEGATION_CONSENT_KEYS,
  DELEGATION_CONSENTS_NONE_KEY,
  DELEGATION_REFUSAL_KEY,
  DELEGATION_CONNECTION_REFUSALS,
  DELEGATION_REVOKE_CONFIRM_KEYS,
  WELLNESS_READ_SKIP_KEYS,
  coachPlatformName,
  confirmConsents,
  consentChangeConfirm,
  consentNoteKey,
  consentStateKeys,
  delegationRefusalKey,
  isDelegationRefusal,
  offeredConsents,
} from '../src/delegation';

function lookup(key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    en,
  );
}

describe('delegation refusal vocabulary', () => {
  it('names a key the catalogue carries for every reason', () => {
    const reasons = Object.keys(DELEGATION_REFUSAL_KEY);
    expect(reasons).toHaveLength(21);
    for (const reason of reasons) {
      const key = delegationRefusalKey(reason);
      expect(typeof lookup(key), `${reason} -> ${key}`).toBe('string');
    }
  });

  it('words the conflicts a member can meet on confirm', () => {
    expect(lookup(delegationRefusalKey('own_connection'))).toBe(
      'You already connect {{platform}} with your own account, and Dravr keeps reading that one.',
    );
    expect(lookup(delegationRefusalKey('already_linked'))).toBe(
      'Your {{platform}} is already linked through another group.',
    );
  });

  it('reads an unknown or missing reason as the generic failure', () => {
    expect(isDelegationRefusal('session_expired')).toBe(false);
    expect(isDelegationRefusal(undefined)).toBe(false);
    expect(delegationRefusalKey('session_expired')).toBe('delegation.actionFailed');
    expect(delegationRefusalKey(undefined)).toBe('delegation.actionFailed');
  });

  it('never reads a name the table inherits as a refusal', () => {
    for (const inherited of ['toString', 'constructor', 'hasOwnProperty', '__proto__']) {
      expect(isDelegationRefusal(inherited), inherited).toBe(false);
      expect(delegationRefusalKey(inherited), inherited).toBe('delegation.actionFailed');
    }
  });

  it('sends the coach to connections only for their own connection gaps', () => {
    expect([...DELEGATION_CONNECTION_REFUSALS].sort()).toEqual([
      'coach_platform_api_key_required',
      'coach_platform_email_mismatch',
      'coach_platform_email_missing',
      'coach_platform_not_connected',
      'coach_platform_reconnect_needed',
      'coach_platform_terms_outdated',
    ]);
  });

  it('says a roster athlete the coach has no access to cannot be linked, naming the platform, never a connection gap', () => {
    expect(isDelegationRefusal('athlete_no_access')).toBe(true);
    expect(delegationRefusalKey('athlete_no_access')).toBe('delegation.athleteNoAccess');
    expect(lookup('delegation.athleteNoAccess')).toBe(
      '{{platform}} gives your account no access to this athlete, so they cannot be linked.',
    );
    expect(DELEGATION_CONNECTION_REFUSALS.has('athlete_no_access')).toBe(false);
  });

  it('reads a consent grant refused outside the app as the generic failure', () => {
    expect(isDelegationRefusal('consent_grant_needs_app')).toBe(true);
    expect(delegationRefusalKey('consent_grant_needs_app')).toBe('delegation.actionFailed');
    expect(DELEGATION_CONNECTION_REFUSALS.has('consent_grant_needs_app')).toBe(false);
  });

  it('asks an Intervals.icu coach linked by OAuth for their API key, naming the platform', () => {
    expect(delegationRefusalKey('coach_platform_api_key_required')).toBe('delegation.apiKeyRequired');
    expect(lookup('delegation.apiKeyRequired')).toContain('{{platform}}');
    expect(lookup('delegation.apiKeyRequired')).toContain('API key');
  });
});

describe('coaching-platform names', () => {
  it('names a platform by its brand, from its own name or its connection backend', () => {
    expect(coachPlatformName('trainingpeaks')).toBe('TrainingPeaks');
    expect(coachPlatformName('sciotte_trainingpeaks')).toBe('TrainingPeaks');
    expect(coachPlatformName('intervals_icu')).toBe('Intervals.icu');
  });

  it('names every platform a coach can connect while none is known', () => {
    for (const unknown of [undefined, null, 'strava', 'toString', 42]) {
      expect(coachPlatformName(unknown), String(unknown)).toBe('TrainingPeaks / Intervals.icu');
    }
  });

  it('gives every delegation string that names a platform the placeholder it fills', () => {
    const delegation = (en as { delegation: Record<string, string> }).delegation;
    for (const [key, text] of Object.entries(delegation)) {
      expect(text, key).not.toContain('TrainingPeaks');
    }
  });
});

describe('link consents', () => {
  const TRAININGPEAKS = {
    calendar_write: false,
    calendar_write_supported: false,
    wellness_read: false,
    wellness_read_supported: false,
    wellness_read_skipped: null,
  };
  const INTERVALS = { ...TRAININGPEAKS, calendar_write_supported: true, wellness_read_supported: true };

  it('words every consent with keys the catalogue carries, naming the human coach and the platform', () => {
    for (const [consent, keys] of Object.entries(DELEGATION_CONSENT_KEYS)) {
      expect(typeof lookup(keys.label), `${consent} label`).toBe('string');
      expect(lookup(keys.label), `${consent} label`).toContain('{{platform}}');
      expect(lookup(keys.hint), `${consent} hint`).toContain('{{coach}}');
      expect(typeof lookup(keys.granted), `${consent} granted`).toBe('string');
    }
    expect(lookup(DELEGATION_CONSENTS_NONE_KEY)).toBe('Workouts only');
  });

  it('offers only what the platform takes, calendar first', () => {
    expect(offeredConsents(TRAININGPEAKS)).toEqual([]);
    expect(offeredConsents(INTERVALS)).toEqual(['calendar_write', 'wellness_read']);
    expect(offeredConsents({ ...TRAININGPEAKS, wellness_read_supported: true })).toEqual(['wellness_read']);
  });

  it('tells the coach what the member granted, workouts only for none, and nothing where nothing is offered', () => {
    expect(consentStateKeys(TRAININGPEAKS)).toEqual([]);
    expect(consentStateKeys(INTERVALS)).toEqual(['delegation.consentsWorkoutsOnly']);
    expect(consentStateKeys({ ...INTERVALS, calendar_write: true })).toEqual(['delegation.calendarWriteGranted']);
    expect(consentStateKeys({ ...INTERVALS, calendar_write: true, wellness_read: true })).toEqual([
      'delegation.calendarWriteGranted',
      'delegation.wellnessReadGranted',
    ]);
  });

  it('says where a granted wellness comes from when a source of the member own skips the link', () => {
    const ownSource = { ...INTERVALS, wellness_read_skipped: 'own_wellness_source' as const };
    expect(consentStateKeys({ ...ownSource, calendar_write: true, wellness_read: true })).toEqual([
      'delegation.calendarWriteGranted',
      'delegation.wellnessReadOwnSource',
    ]);
    // Not granted, nothing is claimed either way.
    expect(consentStateKeys(ownSource)).toEqual(['delegation.consentsWorkoutsOnly']);
    // A member's own connection to the platform has no coach line: nothing says it shares.
    const ownConnection = { ...INTERVALS, wellness_read: true, wellness_read_skipped: 'own_connection' as const };
    expect(consentStateKeys(ownConnection)).toEqual(['delegation.consentsWorkoutsOnly']);
  });

  it('notes under the wellness control why it syncs nothing, in words the catalogue carries', () => {
    expect(consentNoteKey(INTERVALS, 'wellness_read')).toBeUndefined();
    expect(consentNoteKey({ ...INTERVALS, wellness_read_skipped: 'own_wellness_source' as const }, 'wellness_read')).toBe(
      'delegation.wellnessReadOwnSourceNote',
    );
    expect(
      consentNoteKey({ ...INTERVALS, wellness_read_skipped: 'own_wellness_source' as const }, 'calendar_write'),
    ).toBeUndefined();
    for (const [skip, keys] of Object.entries(WELLNESS_READ_SKIP_KEYS)) {
      expect(typeof lookup(keys.note), `${skip} note`).toBe('string');
      if (keys.state) expect(typeof lookup(keys.state), `${skip} state`).toBe('string');
    }
  });

  it('asks first only before turning wellness off', () => {
    expect(consentChangeConfirm('wellness_read', false)).toEqual({
      title: 'delegation.wellnessRevokeTitle',
      body: 'delegation.wellnessRevokeBody',
      confirm: 'delegation.wellnessRevokeConfirm',
    });
    expect(consentChangeConfirm('wellness_read', true)).toBeUndefined();
    expect(consentChangeConfirm('calendar_write', false)).toBeUndefined();
    expect(consentChangeConfirm('calendar_write', true)).toBeUndefined();
    for (const [consent, keys] of Object.entries(DELEGATION_REVOKE_CONFIRM_KEYS)) {
      expect(typeof lookup(keys.title), `${consent} title`).toBe('string');
      expect(lookup(keys.body), `${consent} body`).toContain('{{platform}}');
      expect(lookup(keys.body), `${consent} body`).toContain('{{coach}}');
      expect(typeof lookup(keys.confirm), `${consent} confirm`).toBe('string');
    }
  });

  it('confirms with an answer for each offered consent, and with no body where none is offered', () => {
    expect(confirmConsents(TRAININGPEAKS, { calendar_write: true })).toBeUndefined();
    expect(confirmConsents(INTERVALS, {})).toEqual({ calendar_write: false, wellness_read: false });
    expect(confirmConsents(INTERVALS, { wellness_read: true })).toEqual({
      calendar_write: false,
      wellness_read: true,
    });
    // A grant the platform does not offer is never sent.
    expect(confirmConsents({ ...TRAININGPEAKS, calendar_write_supported: true }, { wellness_read: true })).toEqual({
      calendar_write: false,
    });
  });
});

describe('what a member lets their coach do', () => {
  it('names keys the catalogue carries, the step naming the coach and the platform', () => {
    expect(typeof lookup(COACH_CONSENTS_KEYS.title)).toBe('string');
    expect(lookup(COACH_CONSENTS_KEYS.hint)).toContain('{{coach}}');
    expect(lookup(COACH_CONSENTS_KEYS.calendarStep)).toContain('{{coach}}');
    expect(lookup(COACH_CONSENTS_KEYS.calendarStep)).toContain('{{platform}}');
    // The step is the one the athlete takes on the platform whose coach writes calendars.
    expect(coachPlatformName(COACH_CALENDAR_PLATFORM)).toBe('Intervals.icu');
    expect(lookup(COACH_CONSENTS_KEYS.calendarStep)).toContain('intervals.icu/athletes');
  });

  it('offers the member\'s coach consents as it offers a link\'s', () => {
    const consents = {
      calendar_write: true,
      calendar_write_at: '2026-10-20T12:00:00Z',
      calendar_write_supported: true,
      wellness_read: false,
      wellness_read_at: null,
      wellness_read_supported: false,
      wellness_read_skipped: null,
    };
    expect(offeredConsents(consents)).toEqual(['calendar_write']);
    expect(consentNoteKey({ ...consents, wellness_read_supported: true, wellness_read_skipped: 'own_wellness_source' }, 'wellness_read')).toBe(
      'delegation.wellnessReadOwnSourceNote',
    );
  });
});
