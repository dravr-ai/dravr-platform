// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The catalogue keys a group's reply mode reads as, for the settings form and the agent's rows
// ABOUTME: Built at import time, where `t` does not exist: the table carries keys and the render resolves them

import type { GroupRespondMode } from '@pierre/shared-types';

/** When the agent answers in the room, as the settings select and the agent's row both say it. */
export const RESPOND_MODE_LABEL_KEY: Record<GroupRespondMode, string> = {
  all: 'groups.respondEvery',
  mentions: 'groups.respondMentioned',
};
