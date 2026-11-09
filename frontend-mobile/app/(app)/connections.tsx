// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Connections modal route presented over tabs, role-gated
// ABOUTME: Renders ConnectionsScreen for regular users; admins (platform operators) redirect to Settings

import { hiddenFromOperators } from '../../src/navigation/hiddenFromOperators';
import { ConnectionsScreen } from '../../src/screens/connections/ConnectionsScreen';

export default hiddenFromOperators(ConnectionsScreen);
