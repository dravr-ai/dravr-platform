// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web client's binding of the coach's "build from family" editor hooks — the families, the live preview, the save into the draft
// ABOUTME: The hooks live in @pierre/ui-logic, one definition for web and mobile (carnet#1515)

import { createSessionEditorHooks } from '@pierre/ui-logic';
import { chatApi } from '../services/api';

/**
 * The editor's hooks over the web chat API. The API is reached when a hook
 * calls it, so importing this touches no client.
 */
export const { useSubjectFamilies, useSessionPreview, useSaveDraftDay } = createSessionEditorHooks({
  chatApi: {
    getSubjectFamilies: (conversationId, locale) => chatApi.getSubjectFamilies(conversationId, locale),
    previewSubjectSession: (conversationId, request, locale) =>
      chatApi.previewSubjectSession(conversationId, request, locale),
    saveSubjectDraftDay: (conversationId, date, request, locale) =>
      chatApi.saveSubjectDraftDay(conversationId, date, request, locale),
  },
});

