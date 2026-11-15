// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The catalogue keys a group's reply mode and weekly-recap mode read as, for the settings form and the agent's rows
// ABOUTME: Built at import time, where `t` does not exist: each table carries keys and the render resolves them

import type { GroupDigestMode, GroupRespondMode } from '@pierre/shared-types';

/** Each weekly-digest mode's option label and the one-line hint shown under the select while it is chosen. */
export const DIGEST_MODES: Record<GroupDigestMode, { labelKey: string; hintKey: string }> = {
  off: { labelKey: 'groups.digestOff', hintKey: 'groups.digestOffHint' },
  chat: { labelKey: 'groups.digestChat', hintKey: 'groups.digestChatHint' },
  managers: { labelKey: 'groups.digestManagers', hintKey: 'groups.digestManagersHint' },
};

/** When the agent answers in the room, as the settings select and the agent's row both say it. */
export const RESPOND_MODE_LABEL_KEY: Record<GroupRespondMode, string> = {
  all: 'groups.respondEvery',
  mentions: 'groups.respondMentioned',
};
