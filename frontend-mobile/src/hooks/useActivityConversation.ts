// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Binds the shared activity-conversation link to this client's athlete API
// ABOUTME: The thread an activity's view opened, linked on the server so any device resumes it

import { createActivityConversationHook } from '@pierre/ui-logic';
import { athleteApi } from '../services/api';

export const useActivityConversation = createActivityConversationHook(athleteApi);
