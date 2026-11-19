// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the data access request parsers — the request, its list and its letter as the routes serve them (carnet#1646)
// ABOUTME: A body of another shape reads as null, so a screen shows its error state instead of a request built from guesses

import { describe, it, expect } from 'vitest';
import {
  parseDataAccessLetter,
  parseDataAccessRequest,
  parseDataAccessRequestList,
} from '../src/data-access-request';

const SENT = {
  id: '6f1c0e4a-2b7d-4c49-9a43-0d6b1e5f2a10',
  reference: 'DRV-2026-AB3XZ7',
  controller: 'Acme Fitness',
  controller_contact: 'privacy@acme.example',
  athlete_identifier: 'ana@example.com',
  law: 'gdpr',
  scope: 'access_portability',
  letter_locale: 'fr',
  status: 'sent',
  derived_state: 'due_soon',
  sent_on: '2026-11-08',
  due_on: '2026-12-08',
  extended_until: null,
  answered_on: null,
  created_at: '2026-11-08T15:00:00Z',
  updated_at: '2026-11-08T15:00:00Z',
};

describe('parseDataAccessRequest', () => {
  it('reads a request as the routes serve it', () => {
    expect(parseDataAccessRequest(SENT)).toEqual(SENT);
  });

  it('reads a draft, whose dates and derived state are all null', () => {
    const draft = { ...SENT, status: 'drafted', derived_state: null, sent_on: null, due_on: null };
    expect(parseDataAccessRequest(draft)?.status).toBe('drafted');
  });

  it('refuses a law, scope, status, locale or derived state the server does not send', () => {
    expect(parseDataAccessRequest({ ...SENT, law: 'ccpa' })).toBeNull();
    expect(parseDataAccessRequest({ ...SENT, scope: 'erasure' })).toBeNull();
    expect(parseDataAccessRequest({ ...SENT, status: 'pending' })).toBeNull();
    expect(parseDataAccessRequest({ ...SENT, letter_locale: 'it' })).toBeNull();
    expect(parseDataAccessRequest({ ...SENT, derived_state: 'late' })).toBeNull();
  });

  it('refuses a date that is not a calendar day', () => {
    expect(parseDataAccessRequest({ ...SENT, due_on: '2026-12-08T00:00:00Z' })).toBeNull();
  });
});

describe('parseDataAccessRequestList', () => {
  it('reads every request of the list', () => {
    expect(parseDataAccessRequestList({ requests: [SENT, SENT] })).toHaveLength(2);
  });

  it('refuses the whole list when one request is of another shape', () => {
    expect(parseDataAccessRequestList({ requests: [SENT, { id: 'x' }] })).toBeNull();
    expect(parseDataAccessRequestList([SENT])).toBeNull();
  });
});

describe('parseDataAccessLetter', () => {
  const LETTER = {
    locale: 'en',
    subject: 'Data subject access request — reference DRV-2026-AB3XZ7',
    body: 'Date: 18 November 2026\n\nTo the data protection officer',
    text: 'Data subject access request — reference DRV-2026-AB3XZ7\n\nDate: 18 November 2026\n',
    filename: 'DRV-2026-AB3XZ7.txt',
    mailto: 'mailto:privacy@acme.example?subject=x&body=y',
  };

  it('reads a letter and its mailto link', () => {
    expect(parseDataAccessLetter(LETTER)).toEqual(LETTER);
  });

  it('reads a letter with no mailto link, for a postal contact', () => {
    expect(parseDataAccessLetter({ ...LETTER, mailto: null })?.mailto).toBeNull();
  });

  it('refuses a link that is not mailto, or a letter with no file name', () => {
    expect(parseDataAccessLetter({ ...LETTER, mailto: 'https://acme.example' })).toBeNull();
    expect(parseDataAccessLetter({ ...LETTER, filename: '' })).toBeNull();
  });
});
