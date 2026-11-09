// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Shoes route for Expo Router file-based routing, role-gated
// ABOUTME: Renders GearScreen for athletes; admins (platform operators) redirect to Settings

import { hiddenFromOperators } from '../../../../src/navigation/hiddenFromOperators';
import { GearScreen } from '../../../../src/screens/settings/GearScreen';

export default hiddenFromOperators(GearScreen);
