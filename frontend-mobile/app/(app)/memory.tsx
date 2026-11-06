// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Memory route presented over tabs, role-gated: user-facing pierre-memory inspector
// ABOUTME: Renders the screen for athletes; admins (platform operators) redirect to Settings

import { hiddenFromOperators } from '../../src/navigation/hiddenFromOperators';
import { MemoryScreen } from '../../src/screens/memory/MemoryScreen';

export default hiddenFromOperators(MemoryScreen);
