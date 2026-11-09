// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Notification preferences route for Expo Router file-based routing, role-gated
// ABOUTME: Renders the screen for athletes; admins (platform operators) redirect to Settings

import { hiddenFromOperators } from '../../../../src/navigation/hiddenFromOperators';
import { NotificationPreferencesScreen } from '../../../../src/screens/settings/NotificationPreferencesScreen';

export default hiddenFromOperators(NotificationPreferencesScreen);
