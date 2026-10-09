// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Provides the signed-in athlete's resolved unit system to every screen that prints a distance
// ABOUTME: Mounted inside the signed-in app group only: the units read needs a session (carnet#835)

import React from 'react';
import { UnitsContext } from '@pierre/ui-logic';
import { useUnitPreferences } from '../hooks/useUnits';

/**
 * Read the athlete's units once and hand them down. Until the server has
 * answered, screens print metric, the platform's own units.
 */
export function UnitsProvider({ children }: { children: React.ReactNode }): React.JSX.Element {
  const { units } = useUnitPreferences();
  return <UnitsContext.Provider value={units ?? 'metric'}>{children}</UnitsContext.Provider>;
}
