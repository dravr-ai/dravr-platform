// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile client's binding of the shared day-editor hooks — the families, the live preview, the saves, the restore, the remove, the add, a date's revert, the history
// ABOUTME: The hooks themselves live in @pierre/ui-logic, one definition for web and mobile (carnet#1515, #1714, #1716)

import { createSessionEditorHooks } from '@pierre/ui-logic';
import { chatApi } from '../services/api';

/**
 * The editor's hooks over the mobile chat API. The API is reached when a
 * hook calls it, so importing this touches no client.
 */
export const {
  useSubjectFamilies,
  useSessionPreview,
  useSaveDraftDay,
  useSaveDraftHandDay,
  useRestoreDraftDay,
  useSetCoachCall,
  useRemoveDraftSession,
  useAddDraftSession,
  useRevertDraftDate,
  useSubjectPlanHistory,
  useRereadPlan,
} = createSessionEditorHooks({
  chatApi: {
    getSubjectFamilies: (conversationId, locale) => chatApi.getSubjectFamilies(conversationId, locale),
    previewSubjectSession: (conversationId, request, locale) =>
      chatApi.previewSubjectSession(conversationId, request, locale),
    saveSubjectDraftDay: (conversationId, date, position, build, expect, locale) =>
      chatApi.saveSubjectDraftDay(conversationId, date, position, build, expect, locale),
    saveSubjectDraftHandDay: (conversationId, date, position, day, expect, locale) =>
      chatApi.saveSubjectDraftHandDay(conversationId, date, position, day, expect, locale),
    restoreSubjectDraftDay: (conversationId, date, position, expect, locale) =>
      chatApi.restoreSubjectDraftDay(conversationId, date, position, expect, locale),
    setSubjectDraftDayCoachCall: (conversationId, date, position, on, expect, locale) =>
      chatApi.setSubjectDraftDayCoachCall(conversationId, date, position, on, expect, locale),
    removeSubjectDraftDay: (conversationId, date, position, expect, locale) =>
      chatApi.removeSubjectDraftDay(conversationId, date, position, expect, locale),
    addSubjectDraftSession: (conversationId, date, session, locale) =>
      chatApi.addSubjectDraftSession(conversationId, date, session, locale),
    revertSubjectDraftDate: (conversationId, date, locale) =>
      chatApi.revertSubjectDraftDate(conversationId, date, locale),
    getSubjectPlanHistory: (conversationId, query) => chatApi.getSubjectPlanHistory(conversationId, query),
  },
});
