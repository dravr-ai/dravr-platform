// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web composer's slash-command palette — the shared palette bound to the web chat API
// ABOUTME: The composer passes the open conversation and hands the palette each key event's name

import { createCommandPaletteHook } from '@pierre/ui-logic';
import { chatApi } from '../services/api';

export const useCommandPalette = createCommandPaletteHook(chatApi);
