// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Privacy route for Expo Router file-based routing, role-gated
// ABOUTME: Renders the screen for athletes; admins (platform operators) redirect to Settings

import { hiddenFromOperators } from '../../../../src/navigation/hiddenFromOperators';
import { PrivacySettingsScreen } from '../../../../src/screens/settings/PrivacySettingsScreen';

export default hiddenFromOperators(PrivacySettingsScreen);
