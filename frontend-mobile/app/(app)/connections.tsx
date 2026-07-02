// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Connections modal route presented over tabs, role-gated
// ABOUTME: Renders ConnectionsScreen for regular users; admins (platform operators) redirect to Settings

import { Redirect } from 'expo-router';
import { useAuth } from '../../src/contexts/AuthContext';
import { ConnectionsScreen } from '../../src/screens/connections/ConnectionsScreen';

export default function ConnectionsRoute() {
  const { user } = useAuth();
  // Admins are platform operators: provider connections are a user-account
  // surface (mirrors the web's ADMIN_HIDDEN_TABS), so deep links redirect away.
  if (user?.role === 'admin' || user?.role === 'super_admin') {
    return <Redirect href="/(app)/(tabs)/(settings)" />;
  }
  return <ConnectionsScreen />;
}
