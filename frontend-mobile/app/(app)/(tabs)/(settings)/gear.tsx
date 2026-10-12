// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Shoes route for Expo Router file-based routing, role-gated
// ABOUTME: Renders GearScreen for athletes; admins (platform operators) redirect to Settings

import { Redirect } from 'expo-router';
import { useAuth } from '../../../../src/contexts/AuthContext';
import { GearScreen } from '../../../../src/screens/settings/GearScreen';

export default function GearRoute() {
  const { user } = useAuth();
  // The gear ledger is an athlete-account surface (ADMIN_HIDDEN_PANES hides
  // it from the operator's settings list), so deep links redirect away.
  if (user?.role === 'admin' || user?.role === 'super_admin') {
    return <Redirect href="/(app)/(tabs)/(settings)" />;
  }
  return <GearScreen />;
}
