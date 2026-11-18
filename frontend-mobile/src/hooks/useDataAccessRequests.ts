// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile binding of the data access requests — the shared ui-logic hooks over this client's API
// ABOUTME: The list, one letter, and every step on a request; a step settles with an outcome rather than throwing

import { createDataAccessRequestHooks } from '@pierre/ui-logic';
import { dataAccessRequestsApi } from '../services/api';

/**
 * The hooks over this client's data access requests API. The definitions
 * live in `@pierre/ui-logic`, one for web and mobile; the API is reached when
 * a hook runs, so importing this touches no client.
 */
export const { useDataAccessRequests, useDataAccessLetter, useDataAccessAction } = createDataAccessRequestHooks({
  list: () => dataAccessRequestsApi.list(),
  create: (body) => dataAccessRequestsApi.create(body),
  get: (id) => dataAccessRequestsApi.get(id),
  edit: (id, edit) => dataAccessRequestsApi.edit(id, edit),
  letter: (id, locale) => dataAccessRequestsApi.letter(id, locale),
  markSent: (id, date) => dataAccessRequestsApi.markSent(id, date),
  recordExtension: (id, date) => dataAccessRequestsApi.recordExtension(id, date),
  markAnswered: (id, date) => dataAccessRequestsApi.markAnswered(id, date),
  close: (id) => dataAccessRequestsApi.close(id),
});
