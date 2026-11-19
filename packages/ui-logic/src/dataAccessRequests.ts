// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Statutory access requests, shared by both clients — the words and tones of each state, the form's checks, the dates' bounds
// ABOUTME: And the React Query hooks bound to each client's API: the list, a letter, and every step, which settle instead of throwing

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { PROVIDER_BRAND, QUERY_KEYS } from '@pierre/shared-constants';
import type { DataAccessRequestsApi } from '@pierre/api-client';
import {
  DATA_ACCESS_LETTER_LOCALES,
  type CreateDataAccessRequest,
  type DataAccessLaw,
  type DataAccessLetter,
  type DataAccessLetterLocale,
  type DataAccessRequest,
  type DataAccessScope,
  type EditDataAccessRequest,
} from '@pierre/shared-types';
import { apiErrorStatus } from './apiError';

const CIVIL_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const DAY_MS = 24 * 60 * 60 * 1000;

/** The server's field ceilings, so the form refuses what the server would. */
export const DATA_ACCESS_FIELD_LIMITS = {
  controller: 200,
  controller_contact: 500,
  athlete_identifier: 320,
} as const;

/** A company the form offers by name: the ones Dravr works with. The athlete may type any other. */
export const DATA_ACCESS_COMPANY_SUGGESTIONS: readonly string[] = [
  ...Object.values(PROVIDER_BRAND),
  'Apple',
];

/** The two laws, with the words each is offered under. */
export const DATA_ACCESS_LAW_OPTIONS: readonly { value: DataAccessLaw; labelKey: string; hintKey: string }[] = [
  { value: 'quebec_p39_1', labelKey: 'dataAccess.form.lawQuebec', hintKey: 'dataAccess.form.lawQuebecHint' },
  { value: 'gdpr', labelKey: 'dataAccess.form.lawGdpr', hintKey: 'dataAccess.form.lawGdprHint' },
];

/** The two scopes, with the words each is offered under. */
export const DATA_ACCESS_SCOPE_OPTIONS: readonly { value: DataAccessScope; labelKey: string; hintKey: string }[] = [
  { value: 'access', labelKey: 'dataAccess.form.scopeAccess', hintKey: 'dataAccess.form.scopeAccessHint' },
  {
    value: 'access_portability',
    labelKey: 'dataAccess.form.scopeAccessPortability',
    hintKey: 'dataAccess.form.scopeAccessPortabilityHint',
  },
];

/** The line naming where the athlete turns when the deadline passes unanswered. */
export const DATA_ACCESS_RECOURSE_KEY: Readonly<Record<DataAccessLaw, string>> = {
  quebec_p39_1: 'dataAccess.recourse.quebec',
  gdpr: 'dataAccess.recourse.gdpr',
};

/** The letter's language a new request starts in: the app's, or English for one the letter is not written in. */
export function defaultLetterLocale(language: string): DataAccessLetterLocale {
  const base = language.slice(0, 2).toLowerCase();
  return (DATA_ACCESS_LETTER_LOCALES as readonly string[]).includes(base) ? (base as DataAccessLetterLocale) : 'en';
}

/** `ms` as a `YYYY-MM-DD` day, read in UTC. */
function civilOfUtcMs(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10);
}

/** A `YYYY-MM-DD` day as midnight UTC, or `null` for anything else (31 February included). */
function civilToUtcMs(date: string): number | null {
  const match = CIVIL_DATE.exec(date);
  if (match === null) return null;
  const ms = Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  return civilOfUtcMs(ms) === date ? ms : null;
}

/**
 * Today, as the date fields default to and are bounded by: the earlier of the
 * device's day and UTC's. The server refuses a date after its own (UTC) day,
 * so a device ahead of UTC must not offer one; a device behind it keeps its
 * own day, which is the day the athlete lived.
 */
export function dataAccessToday(now: Date = new Date()): string {
  const local = civilOfUtcMs(Date.UTC(now.getFullYear(), now.getMonth(), now.getDate()));
  const utc = civilOfUtcMs(now.getTime());
  return local < utc ? local : utc;
}

/** `date` plus `months` calendar months, clamped to the month's end (31 Jan + 1 → 28/29 Feb), as the server counts. */
export function addCivilMonths(date: string, months: number): string | null {
  const ms = civilToUtcMs(date);
  if (ms === null) return null;
  const start = new Date(ms);
  const year = start.getUTCFullYear();
  const month = start.getUTCMonth() + months;
  const lastDay = new Date(Date.UTC(year, month + 1, 0)).getUTCDate();
  return civilOfUtcMs(Date.UTC(year, month, Math.min(start.getUTCDate(), lastDay)));
}

/** `date` plus `days` calendar days. */
export function addCivilDays(date: string, days: number): string | null {
  const ms = civilToUtcMs(date);
  return ms === null ? null : civilOfUtcMs(ms + days * DAY_MS);
}

/**
 * The days a GDPR extension may name: after the one-month due date, and no
 * later than three months from the day it was sent (Art. 12(3)). `null`
 * before the request is sent.
 */
export function gdprExtensionWindow(sentOn: string | null): { after: string; latest: string } | null {
  if (sentOn === null) return null;
  const after = addCivilMonths(sentOn, 1);
  const latest = addCivilMonths(sentOn, 3);
  return after !== null && latest !== null ? { after, latest } : null;
}

/** The badge tone a state is drawn in; both clients map it to their own badge. */
export type DataAccessTone = 'neutral' | 'primary' | 'warning' | 'error' | 'success' | 'secondary';

/** The state the athlete reads: a sent request is "due soon" or "overdue" when the server says so. */
export function dataAccessStatus(request: DataAccessRequest): { key: string; tone: DataAccessTone } {
  if (request.status === 'sent' && request.derived_state === 'overdue') {
    return { key: 'dataAccess.status.overdue', tone: 'error' };
  }
  if (request.status === 'sent' && request.derived_state === 'due_soon') {
    return { key: 'dataAccess.status.dueSoon', tone: 'warning' };
  }
  switch (request.status) {
    case 'drafted':
      return { key: 'dataAccess.status.drafted', tone: 'neutral' };
    case 'sent':
      return { key: 'dataAccess.status.sent', tone: 'primary' };
    case 'answered':
      return { key: 'dataAccess.status.answered', tone: 'success' };
    case 'closed':
      return { key: 'dataAccess.status.closed', tone: 'secondary' };
  }
}

/** One date line of a request: its key and the `YYYY-MM-DD` day it names. */
export interface DataAccessDateLine {
  key: string;
  date: string;
}

/**
 * The dates a request shows, in the order they happened: sent, then due (or
 * extended), then answered. A closed request keeps the ones it reached.
 */
export function dataAccessDateLines(request: DataAccessRequest): DataAccessDateLine[] {
  const lines: DataAccessDateLine[] = [];
  if (request.sent_on) lines.push({ key: 'dataAccess.dates.sentOn', date: request.sent_on });
  if (request.due_on && request.status !== 'answered') {
    lines.push({
      key: request.extended_until ? 'dataAccess.dates.dueOnExtended' : 'dataAccess.dates.dueOn',
      date: request.due_on,
    });
  }
  if (request.answered_on) lines.push({ key: 'dataAccess.dates.answeredOn', date: request.answered_on });
  return lines;
}

/** Whether the request still waits on the company, so the deadline's recourse line applies. */
export function dataAccessShowsRecourse(request: DataAccessRequest): boolean {
  return request.status === 'sent' && request.derived_state === 'overdue';
}

/** The form's fields, as typed. */
export interface DataAccessDraftForm {
  controller: string;
  controller_contact: string;
  athlete_identifier: string;
  law: DataAccessLaw;
  scope: DataAccessScope;
  letter_locale: DataAccessLetterLocale;
}

/** A form field's error key. */
export type DataAccessDraftErrors = Partial<
  Record<'controller' | 'controller_contact' | 'athlete_identifier', string>
>;

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/**
 * The form's errors, as keys; empty when it can be sent. The contact may be
 * a postal address over several lines, so only a one-line contact that looks
 * meant as an email (it holds an `@`) must be one: that is the case a typo
 * would otherwise turn into a letter with no `mailto:` link.
 */
export function validateDataAccessDraft(form: DataAccessDraftForm): DataAccessDraftErrors {
  const errors: DataAccessDraftErrors = {};
  const controller = form.controller.trim();
  const contact = form.controller_contact.trim();
  const identifier = form.athlete_identifier.trim();
  if (controller === '' || controller.length > DATA_ACCESS_FIELD_LIMITS.controller || /[\r\n]/.test(controller)) {
    errors.controller = 'dataAccess.error.companyRequired';
  }
  if (contact === '' || contact.length > DATA_ACCESS_FIELD_LIMITS.controller_contact) {
    errors.controller_contact = 'dataAccess.error.contactRequired';
  } else if (!/[\r\n]/.test(contact) && contact.includes('@') && !EMAIL.test(contact)) {
    errors.controller_contact = 'dataAccess.error.contactInvalid';
  }
  if (
    identifier === '' ||
    identifier.length > DATA_ACCESS_FIELD_LIMITS.athlete_identifier ||
    /[\r\n]/.test(identifier)
  ) {
    errors.athlete_identifier = 'dataAccess.error.identifierRequired';
  }
  return errors;
}

/** The form's fields trimmed, as the create route takes them. */
export function dataAccessDraftBody(form: DataAccessDraftForm): CreateDataAccessRequest {
  return {
    controller: form.controller.trim(),
    controller_contact: form.controller_contact.trim(),
    athlete_identifier: form.athlete_identifier.trim(),
    law: form.law,
    scope: form.scope,
    letter_locale: form.letter_locale,
  };
}

/** Whether `date` is a real `YYYY-MM-DD` calendar day (no 31 February). */
export function isCivilDate(date: string): boolean {
  return civilToUtcMs(date) !== null;
}

/** The earliest day a request can be marked sent: the server keeps a year. */
export function earliestSentOn(today: string): string {
  return addCivilDays(today, -365) ?? today;
}

/** A sent date's error key, or `null`. A field bounded by `earliestSentOn` cannot reach the past limit. */
export function validateSentOn(date: string, today: string): string | null {
  return date > today ? 'dataAccess.error.sentOnFuture' : null;
}

/**
 * An answer date's error key, or `null`. The field is bounded by today, so
 * the one mistake left to name is a day before the letter was sent.
 */
export function validateAnsweredOn(date: string, sentOn: string | null): string | null {
  return sentOn !== null && date < sentOn ? 'dataAccess.error.answeredBeforeSent' : null;
}

/** An extension's error key, or `null`: a GDPR request, after its one-month date, within three months of sending. */
export function validateExtension(date: string, request: DataAccessRequest): string | null {
  if (request.law !== 'gdpr') return 'dataAccess.error.extensionGdprOnly';
  const window = gdprExtensionWindow(request.sent_on);
  if (window === null || civilToUtcMs(date) === null || date <= window.after || date > window.latest) {
    return 'dataAccess.error.extensionInvalid';
  }
  return null;
}

/** A step the athlete takes on a request. */
export type DataAccessAction =
  | { kind: 'create'; body: CreateDataAccessRequest }
  | { kind: 'edit'; id: string; edit: EditDataAccessRequest }
  | { kind: 'sent'; id: string; date: string }
  | { kind: 'extension'; id: string; date: string }
  | { kind: 'answered'; id: string; date: string }
  | { kind: 'close'; id: string };

/** How a step settled: the request as it now stands, or the key of the sentence that says why not. */
export type DataAccessOutcome = { ok: true; request: DataAccessRequest } | { ok: false; errorKey: string };

/** The sentence a refused step or read is named by: a request gone is said so; anything else is `fallbackKey`. */
export function dataAccessFailureKey(err: unknown, fallbackKey: string): string {
  return apiErrorStatus(err) === 404 ? 'dataAccess.error.notFound' : fallbackKey;
}

/** The list with `request` in its place, or first when it is new (the list is newest first). */
function withRequest(list: DataAccessRequest[] | undefined, request: DataAccessRequest): DataAccessRequest[] {
  const current = list ?? [];
  return current.some((entry) => entry.id === request.id)
    ? current.map((entry) => (entry.id === request.id ? request : entry))
    : [request, ...current];
}

/** The hooks a client binds over its data access requests API. */
export interface DataAccessRequestHooks {
  useDataAccessRequests: () => {
    requests: DataAccessRequest[] | undefined;
    isLoading: boolean;
    isError: boolean;
    refetch: () => Promise<void>;
  };
  useDataAccessLetter: (
    id: string | null,
    locale: DataAccessLetterLocale | null,
  ) => { letter: DataAccessLetter | undefined; isLoading: boolean; errorKey: string | null };
  /** Take a step. It settles with an outcome rather than throwing, so the screen says what went wrong in place. */
  useDataAccessAction: () => { run: (action: DataAccessAction) => Promise<DataAccessOutcome>; isPending: boolean };
}

function perform(api: DataAccessRequestsApi, action: DataAccessAction): Promise<DataAccessRequest> {
  switch (action.kind) {
    case 'create':
      return api.create(action.body);
    case 'edit':
      return api.edit(action.id, action.edit);
    case 'sent':
      return api.markSent(action.id, action.date);
    case 'extension':
      return api.recordExtension(action.id, action.date);
    case 'answered':
      return api.markAnswered(action.id, action.date);
    case 'close':
      return api.close(action.id);
  }
}

/**
 * Build the data access request hooks over one client's API.
 *
 * Every step writes the request it answered into the cached list, and drops
 * that request's cached letters: the letter is dated by the sent day and
 * carries the draft's fields, so a step can change it. A conflict (the
 * request moved on elsewhere) or a request gone rereads the list, so the
 * screen shows what the server holds.
 */
export function createDataAccessRequestHooks(api: DataAccessRequestsApi): DataAccessRequestHooks {
  function useDataAccessRequests() {
    const query = useQuery({
      queryKey: QUERY_KEYS.dataAccessRequests.list(),
      queryFn: () => api.list(),
    });
    return {
      requests: query.data,
      isLoading: query.isLoading,
      isError: query.isError && query.data === undefined,
      refetch: async () => {
        await query.refetch();
      },
    };
  }

  function useDataAccessLetter(id: string | null, locale: DataAccessLetterLocale | null) {
    const query = useQuery({
      queryKey: QUERY_KEYS.dataAccessRequests.letter(id ?? '', locale ?? ''),
      queryFn: () => api.letter(id ?? '', locale ?? undefined),
      enabled: id !== null && locale !== null,
    });
    return {
      letter: query.data,
      isLoading: query.isLoading,
      errorKey: query.isError ? dataAccessFailureKey(query.error, 'dataAccess.error.letterFailed') : null,
    };
  }

  function useDataAccessAction() {
    const queryClient = useQueryClient();
    const mutation = useMutation({
      mutationFn: async (action: DataAccessAction): Promise<DataAccessOutcome> => {
        try {
          const request = await perform(api, action);
          queryClient.setQueryData<DataAccessRequest[]>(QUERY_KEYS.dataAccessRequests.list(), (list) =>
            withRequest(list, request),
          );
          queryClient.removeQueries({ queryKey: QUERY_KEYS.dataAccessRequests.letters(request.id) });
          return { ok: true, request };
        } catch (err: unknown) {
          const status = apiErrorStatus(err);
          if (status === 404 || status === 409) {
            void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.dataAccessRequests.list() });
          }
          return { ok: false, errorKey: dataAccessFailureKey(err, 'dataAccess.error.saveFailed') };
        }
      },
    });
    return { run: (action: DataAccessAction) => mutation.mutateAsync(action), isPending: mutation.isPending };
  }

  return { useDataAccessRequests, useDataAccessLetter, useDataAccessAction };
}
