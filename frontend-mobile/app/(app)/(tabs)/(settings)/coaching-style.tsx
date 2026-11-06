// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coaching-style route for Expo Router file-based routing, role-gated
// ABOUTME: Renders the screen for athletes; admins (platform operators) redirect to Settings

import { hiddenFromOperators } from '../../../../src/navigation/hiddenFromOperators';
import { CoachingStyleScreen } from '../../../../src/screens/settings/CoachingStyleScreen';

export default hiddenFromOperators(CoachingStyleScreen);
