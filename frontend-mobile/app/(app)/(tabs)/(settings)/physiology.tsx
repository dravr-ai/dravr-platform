// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Physiology route for Expo Router file-based routing, role-gated
// ABOUTME: Renders PhysiologyScreen for athletes; admins (platform operators) redirect to Settings

import { hiddenFromOperators } from '../../../../src/navigation/hiddenFromOperators';
import { PhysiologyScreen } from '../../../../src/screens/settings/PhysiologyScreen';

export default hiddenFromOperators(PhysiologyScreen);
