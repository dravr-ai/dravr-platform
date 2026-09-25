// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web composer's @handle mention palette — the shared palette bound to the web coaches API
// ABOUTME: Sibling of useCommandPalette: the composer renders what this decides and nothing more

import { createMentionPaletteHook } from '@pierre/ui-logic';
import { coachesApi } from '../services/api';

export const useMentionPalette = createMentionPaletteHook(coachesApi);
