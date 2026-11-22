// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile client's binding of the shared "build from family" editor hooks — the families, the live preview, the draft-day save
// ABOUTME: The hooks themselves live in @pierre/ui-logic, one definition for web and mobile (carnet#1515)

import { createSessionEditorHooks } from '@pierre/ui-logic';
import { chatApi } from '../services/api';

/**
 * The editor's hooks over the mobile chat API. The API is reached when a
 * hook calls it, so importing this touches no client.
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
