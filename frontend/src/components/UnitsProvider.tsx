// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Provides the signed-in athlete's resolved unit system to every surface that prints a distance
// ABOUTME: Mounted inside the signed-in app only: the units read needs a session, and a signed-out read would be refused

import type { ReactNode } from 'react';
import { UnitsContext } from '@pierre/ui-logic';
import { useUnitPreferences } from '../hooks/useUnits';

/**
 * Read the athlete's units once and hand them down. Until the server has
 * answered, surfaces print metric, the platform's own units.
 */
export default function UnitsProvider({ children }: { children: ReactNode }) {
  const { units } = useUnitPreferences();
  return <UnitsContext.Provider value={units ?? 'metric'}>{children}</UnitsContext.Provider>;
}
