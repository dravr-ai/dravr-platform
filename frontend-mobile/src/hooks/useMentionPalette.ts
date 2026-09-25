// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile composer's @handle mention palette — the shared palette bound to the mobile coaches API
// ABOUTME: Sibling of useCommandPalette: the composer renders what this decides and inserts what it drafts

import { createMentionPaletteHook } from '@pierre/ui-logic';
import { coachesApi } from '../services/api';

export const useMentionPalette = createMentionPaletteHook(coachesApi);
