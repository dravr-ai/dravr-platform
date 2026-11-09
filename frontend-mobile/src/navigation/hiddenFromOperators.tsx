// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Route wrapper for a settings pane that ADMIN_HIDDEN_PANES keeps from platform operators
// ABOUTME: The menu already omits the pane, so this sends a deep link back to Settings

import type { ComponentType } from 'react';
import { Redirect } from 'expo-router';
import { useAuth } from '../contexts/AuthContext';

/** `Screen` for athletes; a redirect to Settings for an admin or super admin. */
export function hiddenFromOperators(Screen: ComponentType): ComponentType {
  return function HiddenFromOperators() {
    const { user } = useAuth();
    if (user?.role === 'admin' || user?.role === 'super_admin') {
      return <Redirect href="/(app)/(tabs)/(settings)" />;
    }
    return <Screen />;
  };
}
