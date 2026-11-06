// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Messaging channels route for Expo Router file-based routing, role-gated
// ABOUTME: Renders the screen for athletes; admins (platform operators) redirect to Settings

import { hiddenFromOperators } from '../../../../src/navigation/hiddenFromOperators';
import { MessagingChannelsScreen } from '../../../../src/screens/settings/MessagingChannelsScreen';

export default hiddenFromOperators(MessagingChannelsScreen);
