// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Data access requests domain API — write a statutory access request to a company, read its letter, and track its legal deadline
// ABOUTME: Each answer is checked with its shared-types parser; only the athlete's own session may call these routes; Dravr never sends the letter

import type { AxiosInstance } from 'axios';
import {
  parseDataAccessLetter,
  parseDataAccessRequest,
  parseDataAccessRequestList,
  type CreateDataAccessRequest,
  type DataAccessLetter,
  type DataAccessLetterLocale,
  type DataAccessRequest,
  type EditDataAccessRequest,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';
import { requireShape } from '../core/shape';

/**
 * Creates the data access request API methods bound to an axios instance.
 *
 * Every date is a `YYYY-MM-DD` calendar day. The server computes the due date
 * from the day the athlete says they sent the letter; the client never does.
 */
export function createDataAccessRequestsApi(axios: AxiosInstance) {
  /** POST a step's body and read back the request it changed. */
  async function step(endpoint: string, body: Record<string, string>): Promise<DataAccessRequest> {
    const response = await axios.post<unknown>(endpoint, body);
    return requireShape(parseDataAccessRequest(response.data), endpoint);
  }

  return {
    /** The athlete's requests, newest first. */
    async list(): Promise<DataAccessRequest[]> {
      const endpoint = ENDPOINTS.USER.DATA_ACCESS_REQUESTS;
      const response = await axios.get<unknown>(endpoint);
      return requireShape(parseDataAccessRequestList(response.data), endpoint);
    },

    /** Write a draft. */
    async create(request: CreateDataAccessRequest): Promise<DataAccessRequest> {
      const endpoint = ENDPOINTS.USER.DATA_ACCESS_REQUESTS;
      const response = await axios.post<unknown>(endpoint, request);
      return requireShape(parseDataAccessRequest(response.data), endpoint);
    },

    /** One request, as it stands now. */
    async get(id: string): Promise<DataAccessRequest> {
      const endpoint = ENDPOINTS.USER.DATA_ACCESS_REQUEST(id);
      const response = await axios.get<unknown>(endpoint);
      return requireShape(parseDataAccessRequest(response.data), endpoint);
    },

    /** Change a draft's fields; a request already sent answers 409. */
    async edit(id: string, edit: EditDataAccessRequest): Promise<DataAccessRequest> {
      const endpoint = ENDPOINTS.USER.DATA_ACCESS_REQUEST(id);
      const response = await axios.patch<unknown>(endpoint, edit);
      return requireShape(parseDataAccessRequest(response.data), endpoint);
    },

    /** The letter, in `locale` or the request's own language. */
    async letter(id: string, locale?: DataAccessLetterLocale): Promise<DataAccessLetter> {
      const endpoint = ENDPOINTS.USER.DATA_ACCESS_REQUEST_LETTER(id);
      const response = await axios.get<unknown>(endpoint, locale ? { params: { locale } } : undefined);
      return requireShape(parseDataAccessLetter(response.data), endpoint);
    },

    /** The athlete sent it on `sentOn`; also corrects the date of one already sent. */
    markSent(id: string, sentOn: string): Promise<DataAccessRequest> {
      return step(ENDPOINTS.USER.DATA_ACCESS_REQUEST_SENT(id), { sent_on: sentOn });
    },

    /** A GDPR company said it would answer by `extendedUntil`. */
    recordExtension(id: string, extendedUntil: string): Promise<DataAccessRequest> {
      return step(ENDPOINTS.USER.DATA_ACCESS_REQUEST_EXTENSION(id), { extended_until: extendedUntil });
    },

    /** The company answered on `answeredOn`. */
    markAnswered(id: string, answeredOn: string): Promise<DataAccessRequest> {
      return step(ENDPOINTS.USER.DATA_ACCESS_REQUEST_ANSWERED(id), { answered_on: answeredOn });
    },

    /** The athlete gave up, or the request no longer matters. */
    close(id: string): Promise<DataAccessRequest> {
      return step(ENDPOINTS.USER.DATA_ACCESS_REQUEST_CLOSE(id), {});
    },
  };
}

export type DataAccessRequestsApi = ReturnType<typeof createDataAccessRequestsApi>;
