// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared data access request logic — state words and tones, the form's checks, the date bounds (carnet#1646)
// ABOUTME: And the hooks: a step writes its request into the cached list and settles with an outcome instead of throwing

import { describe, it, expect, vi } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { DataAccessRequestsApi } from '@pierre/api-client';
import type { DataAccessRequest } from '@pierre/shared-types';
import {
  addCivilMonths,
  createDataAccessRequestHooks,
  dataAccessDateLines,
  dataAccessStatus,
  dataAccessToday,
  defaultLetterLocale,
  earliestSentOn,
  gdprExtensionWindow,
  isCivilDate,
  validateAnsweredOn,
  validateDataAccessDraft,
  validateExtension,
  validateSentOn,
  type DataAccessDraftForm,
} from '../src/dataAccessRequests';

const SENT: DataAccessRequest = {
  id: 'r1',
  reference: 'DRV-2026-AB3XZ7',
  controller: 'Acme Fitness',
  controller_contact: 'privacy@acme.example',
  athlete_identifier: 'ana@example.com',
  law: 'gdpr',
  scope: 'access',
  letter_locale: 'en',
  status: 'sent',
  derived_state: null,
  sent_on: '2026-01-31',
  due_on: '2026-02-28',
  extended_until: null,
  answered_on: null,
  created_at: '2026-01-31T15:00:00Z',
  updated_at: '2026-01-31T15:00:00Z',
};

const FORM: DataAccessDraftForm = {
  controller: 'Acme Fitness',
  controller_contact: 'privacy@acme.example',
  athlete_identifier: 'ana@example.com',
  law: 'gdpr',
  scope: 'access',
  letter_locale: 'en',
};

describe('dates', () => {
  it('adds calendar months clamped to the month end, as the server counts', () => {
    expect(addCivilMonths('2026-01-31', 1)).toBe('2026-02-28');
    expect(addCivilMonths('2028-01-31', 1)).toBe('2028-02-29');
    expect(addCivilMonths('2026-11-30', 3)).toBe('2027-02-28');
    expect(addCivilMonths('2026-12-15', 1)).toBe('2027-01-15');
  });

  it('knows a real calendar day from a string that only looks like one', () => {
    expect(isCivilDate('2026-02-28')).toBe(true);
    expect(isCivilDate('2026-02-31')).toBe(false);
    expect(isCivilDate('')).toBe(false);
  });

  it("takes the earlier of the device's day and UTC's as today", () => {
    // 23:30 UTC on 18 Nov is 19 Nov on a device east of UTC; the server still says 18 Nov.
    const now = new Date(Date.UTC(2026, 10, 18, 23, 30));
    expect(dataAccessToday(now) <= '2026-11-18').toBe(true);
  });

  it('bounds an extension after the one-month date and within three months of sending', () => {
    expect(gdprExtensionWindow('2026-01-31')).toEqual({ after: '2026-02-28', latest: '2026-04-30' });
    expect(gdprExtensionWindow(null)).toBeNull();
  });

  it('keeps a year of sent dates', () => {
    expect(earliestSentOn('2026-11-18')).toBe('2025-11-18');
  });
});

describe('dataAccessStatus', () => {
  it('names a sent request by its deadline when the server derived one', () => {
    expect(dataAccessStatus(SENT)).toEqual({ key: 'dataAccess.status.sent', tone: 'primary' });
    expect(dataAccessStatus({ ...SENT, derived_state: 'due_soon' })).toEqual({
      key: 'dataAccess.status.dueSoon',
      tone: 'warning',
    });
    expect(dataAccessStatus({ ...SENT, derived_state: 'overdue' })).toEqual({
      key: 'dataAccess.status.overdue',
      tone: 'error',
    });
  });

  it('names the stored states', () => {
    expect(dataAccessStatus({ ...SENT, status: 'drafted' }).key).toBe('dataAccess.status.drafted');
    expect(dataAccessStatus({ ...SENT, status: 'answered' }).key).toBe('dataAccess.status.answered');
    expect(dataAccessStatus({ ...SENT, status: 'closed' }).key).toBe('dataAccess.status.closed');
  });
});

describe('dataAccessDateLines', () => {
  it('shows the sent and due days of a sent request', () => {
    expect(dataAccessDateLines(SENT)).toEqual([
      { key: 'dataAccess.dates.sentOn', date: '2026-01-31' },
      { key: 'dataAccess.dates.dueOn', date: '2026-02-28' },
    ]);
  });

  it('says an extended due date is extended', () => {
    const extended = { ...SENT, extended_until: '2026-04-15', due_on: '2026-04-15' };
    expect(dataAccessDateLines(extended)[1]).toEqual({ key: 'dataAccess.dates.dueOnExtended', date: '2026-04-15' });
  });

  it('drops the due day once the company answered', () => {
    const answered = { ...SENT, status: 'answered' as const, answered_on: '2026-02-10' };
    expect(dataAccessDateLines(answered).map((line) => line.key)).toEqual([
      'dataAccess.dates.sentOn',
      'dataAccess.dates.answeredOn',
    ]);
  });
});

describe('validateDataAccessDraft', () => {
  it('passes a complete form', () => {
    expect(validateDataAccessDraft(FORM)).toEqual({});
  });

  it('names each empty field', () => {
    expect(validateDataAccessDraft({ ...FORM, controller: ' ', controller_contact: '', athlete_identifier: '' })).toEqual({
      controller: 'dataAccess.error.companyRequired',
      controller_contact: 'dataAccess.error.contactRequired',
      athlete_identifier: 'dataAccess.error.identifierRequired',
    });
  });

  it('refuses a one-line contact meant as an email that is not one', () => {
    expect(validateDataAccessDraft({ ...FORM, controller_contact: 'privacy@acme' }).controller_contact).toBe(
      'dataAccess.error.contactInvalid',
    );
  });

  it('takes a postal address over several lines', () => {
    const postal = 'Acme Fitness\nPrivacy officer\n1 Main St, Montréal QC';
    expect(validateDataAccessDraft({ ...FORM, controller_contact: postal })).toEqual({});
  });

  it('refuses a company name over one line or past the server limit', () => {
    expect(validateDataAccessDraft({ ...FORM, controller: 'Acme\nFitness' }).controller).toBeDefined();
    expect(validateDataAccessDraft({ ...FORM, controller: 'a'.repeat(201) }).controller).toBeDefined();
  });
});

describe('date checks', () => {
  it('refuses a sent day after today', () => {
    expect(validateSentOn('2026-11-19', '2026-11-18')).toBe('dataAccess.error.sentOnFuture');
    expect(validateSentOn('2026-11-18', '2026-11-18')).toBeNull();
  });

  it('refuses an answer before the letter was sent', () => {
    expect(validateAnsweredOn('2026-01-30', '2026-01-31')).toBe('dataAccess.error.answeredBeforeSent');
    expect(validateAnsweredOn('2026-02-01', '2026-01-31')).toBeNull();
  });

  it('takes an extension only for GDPR, inside its window', () => {
    expect(validateExtension('2026-03-15', { ...SENT, law: 'quebec_p39_1' })).toBe('dataAccess.error.extensionGdprOnly');
    expect(validateExtension('2026-02-28', SENT)).toBe('dataAccess.error.extensionInvalid');
    expect(validateExtension('2026-05-01', SENT)).toBe('dataAccess.error.extensionInvalid');
    expect(validateExtension('2026-04-30', SENT)).toBeNull();
  });
});

describe('defaultLetterLocale', () => {
  it("starts in the app's language, or English for one the letter is not written in", () => {
    expect(defaultLetterLocale('fr-CA')).toBe('fr');
    expect(defaultLetterLocale('it')).toBe('en');
  });
});

describe('createDataAccessRequestHooks', () => {
  function setup(api: Partial<DataAccessRequestsApi>) {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client }, children);
    return { client, wrapper, hooks: createDataAccessRequestHooks(api as DataAccessRequestsApi) };
  }

  it('writes a step into the cached list and drops that request’s letters', async () => {
    const answered = { ...SENT, status: 'answered' as const, answered_on: '2026-02-10' };
    const markAnswered = vi.fn().mockResolvedValue(answered);
    const { client, wrapper, hooks } = setup({ markAnswered });
    client.setQueryData(QUERY_KEYS.dataAccessRequests.list(), [SENT]);
    client.setQueryData(QUERY_KEYS.dataAccessRequests.letter('r1', 'en'), { text: 'old' });
    const { result } = renderHook(() => hooks.useDataAccessAction(), { wrapper });

    const outcome = await act(() => result.current.run({ kind: 'answered', id: 'r1', date: '2026-02-10' }));

    expect(outcome).toEqual({ ok: true, request: answered });
    expect(markAnswered).toHaveBeenCalledWith('r1', '2026-02-10');
    expect(client.getQueryData(QUERY_KEYS.dataAccessRequests.list())).toEqual([answered]);
    expect(client.getQueryData(QUERY_KEYS.dataAccessRequests.letter('r1', 'en'))).toBeUndefined();
  });

  it('puts a new request first', async () => {
    const draft = { ...SENT, id: 'r2', status: 'drafted' as const };
    const { client, wrapper, hooks } = setup({ create: vi.fn().mockResolvedValue(draft) });
    client.setQueryData(QUERY_KEYS.dataAccessRequests.list(), [SENT]);
    const { result } = renderHook(() => hooks.useDataAccessAction(), { wrapper });

    await act(() => result.current.run({ kind: 'create', body: FORM }));

    expect(client.getQueryData<DataAccessRequest[]>(QUERY_KEYS.dataAccessRequests.list())?.map((r) => r.id)).toEqual([
      'r2',
      'r1',
    ]);
  });

  it('settles a refused step with its sentence, and rereads the list on a conflict', async () => {
    const conflict = Object.assign(new Error('409'), { response: { status: 409 } });
    const list = vi.fn().mockResolvedValue([SENT]);
    const { client, wrapper, hooks } = setup({ close: vi.fn().mockRejectedValue(conflict), list });
    const invalidate = vi.spyOn(client, 'invalidateQueries');
    const { result } = renderHook(() => hooks.useDataAccessAction(), { wrapper });

    const outcome = await act(() => result.current.run({ kind: 'close', id: 'r1' }));

    expect(outcome).toEqual({ ok: false, errorKey: 'dataAccess.error.saveFailed' });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: QUERY_KEYS.dataAccessRequests.list() });
  });

  it('names a request that is gone', async () => {
    const gone = Object.assign(new Error('404'), { response: { status: 404 } });
    const { wrapper, hooks } = setup({ markSent: vi.fn().mockRejectedValue(gone) });
    const { result } = renderHook(() => hooks.useDataAccessAction(), { wrapper });

    const outcome = await act(() => result.current.run({ kind: 'sent', id: 'r1', date: '2026-11-18' }));

    expect(outcome).toEqual({ ok: false, errorKey: 'dataAccess.error.notFound' });
  });

  it('reads a letter only once it has a request and a language', async () => {
    const letter = vi.fn().mockResolvedValue({ text: 'x' });
    const { wrapper, hooks } = setup({ letter });
    const { result, rerender } = renderHook(
      ({ id }: { id: string | null }) => hooks.useDataAccessLetter(id, 'de'),
      { wrapper, initialProps: { id: null as string | null } },
    );
    expect(letter).not.toHaveBeenCalled();

    rerender({ id: 'r1' });

    await waitFor(() => expect(result.current.letter).toEqual({ text: 'x' }));
    expect(letter).toHaveBeenCalledWith('r1', 'de');
  });
});
