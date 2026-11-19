// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The data access request wire shapes — a statutory access request to a company (Quebec P-39.1 or GDPR) and its letter
// ABOUTME: With their parsers: a body of any other shape reads as null, never as a half-filled request

/** The law a request is made under. */
export type DataAccessLaw = 'quebec_p39_1' | 'gdpr';

/** What the request asks for: a copy, or a copy plus a reusable file. */
export type DataAccessScope = 'access' | 'access_portability';

/** The stored state of a request. */
export type DataAccessStatus = 'drafted' | 'sent' | 'answered' | 'closed';

/** Read from the due date at read time, and only while the request is sent. */
export type DataAccessDerivedState = 'due_soon' | 'overdue';

/** The languages a letter can be written in. */
export type DataAccessLetterLocale = 'fr' | 'en' | 'es' | 'de' | 'pt';

export const DATA_ACCESS_LAWS: readonly DataAccessLaw[] = ['quebec_p39_1', 'gdpr'];
export const DATA_ACCESS_SCOPES: readonly DataAccessScope[] = ['access', 'access_portability'];
export const DATA_ACCESS_STATUSES: readonly DataAccessStatus[] = ['drafted', 'sent', 'answered', 'closed'];
export const DATA_ACCESS_LETTER_LOCALES: readonly DataAccessLetterLocale[] = ['fr', 'en', 'es', 'de', 'pt'];

/** One request as `/api/user/data-access-requests` serves it. Dates are `YYYY-MM-DD`. */
export interface DataAccessRequest {
  id: string;
  /** `DRV-<year>-<6 base32 characters>`, quoted in the letter. */
  reference: string;
  /** The company asked. */
  controller: string;
  /** Its privacy address: an email, or a postal address over several lines. */
  controller_contact: string;
  /** The identifier the athlete uses there. */
  athlete_identifier: string;
  law: DataAccessLaw;
  scope: DataAccessScope;
  letter_locale: DataAccessLetterLocale;
  status: DataAccessStatus;
  derived_state: DataAccessDerivedState | null;
  sent_on: string | null;
  due_on: string | null;
  extended_until: string | null;
  answered_on: string | null;
  created_at: string;
  updated_at: string;
}

/** What `POST /api/user/data-access-requests` takes. */
export interface CreateDataAccessRequest {
  controller: string;
  controller_contact: string;
  athlete_identifier: string;
  law: DataAccessLaw;
  scope: DataAccessScope;
  /** The user's locale when left out. */
  letter_locale?: DataAccessLetterLocale;
}

/** What `PATCH /api/user/data-access-requests/{id}` takes: a draft's fields, any subset. */
export type EditDataAccessRequest = Partial<CreateDataAccessRequest>;

/** The letter, written out (`GET …/{id}/letter`). */
export interface DataAccessLetter {
  locale: DataAccessLetterLocale;
  subject: string;
  /** The paragraphs, joined by a blank line. */
  body: string;
  /** Subject, blank line, body: what is copied and downloaded. */
  text: string;
  /** `DRV-2026-AB3XZ7.txt`. */
  filename: string;
  /** A `mailto:` link, or `null` when the contact is not a plain email address. */
  mailto: string | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isOneOf<T extends string>(value: unknown, allowed: readonly T[]): value is T {
  return typeof value === 'string' && (allowed as readonly string[]).includes(value);
}

const CIVIL_DATE = /^\d{4}-\d{2}-\d{2}$/;

function isNullableDate(value: unknown): value is string | null | undefined {
  return value === null || value === undefined || (typeof value === 'string' && CIVIL_DATE.test(value));
}

const DERIVED_STATES: readonly DataAccessDerivedState[] = ['due_soon', 'overdue'];

/** Read one request, or `null` for any other shape. */
export function parseDataAccessRequest(body: unknown): DataAccessRequest | null {
  if (!isRecord(body)) return null;
  const derived = body.derived_state;
  if (
    typeof body.id !== 'string' ||
    typeof body.reference !== 'string' ||
    typeof body.controller !== 'string' ||
    typeof body.controller_contact !== 'string' ||
    typeof body.athlete_identifier !== 'string' ||
    !isOneOf(body.law, DATA_ACCESS_LAWS) ||
    !isOneOf(body.scope, DATA_ACCESS_SCOPES) ||
    !isOneOf(body.letter_locale, DATA_ACCESS_LETTER_LOCALES) ||
    !isOneOf(body.status, DATA_ACCESS_STATUSES) ||
    !(derived === null || derived === undefined || isOneOf(derived, DERIVED_STATES)) ||
    !isNullableDate(body.sent_on) ||
    !isNullableDate(body.due_on) ||
    !isNullableDate(body.extended_until) ||
    !isNullableDate(body.answered_on) ||
    typeof body.created_at !== 'string' ||
    typeof body.updated_at !== 'string'
  ) {
    return null;
  }
  return {
    id: body.id,
    reference: body.reference,
    controller: body.controller,
    controller_contact: body.controller_contact,
    athlete_identifier: body.athlete_identifier,
    law: body.law,
    scope: body.scope,
    letter_locale: body.letter_locale,
    status: body.status,
    derived_state: derived ?? null,
    sent_on: body.sent_on ?? null,
    due_on: body.due_on ?? null,
    extended_until: body.extended_until ?? null,
    answered_on: body.answered_on ?? null,
    created_at: body.created_at,
    updated_at: body.updated_at,
  };
}

/** Read the list (`{"requests": [...]}`), or `null` when it or any request is of another shape. */
export function parseDataAccessRequestList(body: unknown): DataAccessRequest[] | null {
  if (!isRecord(body) || !Array.isArray(body.requests)) return null;
  const requests: DataAccessRequest[] = [];
  for (const entry of body.requests) {
    const request = parseDataAccessRequest(entry);
    if (request === null) return null;
    requests.push(request);
  }
  return requests;
}

/** Read a letter, or `null` for any other shape. */
export function parseDataAccessLetter(body: unknown): DataAccessLetter | null {
  if (!isRecord(body)) return null;
  if (
    !isOneOf(body.locale, DATA_ACCESS_LETTER_LOCALES) ||
    typeof body.subject !== 'string' ||
    typeof body.body !== 'string' ||
    typeof body.text !== 'string' ||
    typeof body.filename !== 'string' ||
    body.filename === '' ||
    !(body.mailto === null || body.mailto === undefined || (typeof body.mailto === 'string' && body.mailto.startsWith('mailto:')))
  ) {
    return null;
  }
  return {
    locale: body.locale,
    subject: body.subject,
    body: body.body,
    text: body.text,
    filename: body.filename,
    mailto: body.mailto ?? null,
  };
}
