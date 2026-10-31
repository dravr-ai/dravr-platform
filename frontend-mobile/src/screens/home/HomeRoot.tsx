// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What the Home tab opens: an athlete's Home, or for a coach who manages a roster the switch between "Mes athlètes" and "Mon entraînement"
// ABOUTME: A coach lands on their athletes (D2, carnet#1253); the other side of the switch is the same Home every athlete has

import React, { useState } from 'react';
import { useAuth } from '../../contexts/AuthContext';
import { CoachHomeScreen } from '../coach-home/CoachHomeScreen';
import { CoachHomeSwitch, type HomeScope } from '../coach-home/CoachHomeSwitch';
import { HomeScreen } from './HomeScreen';

export function HomeRoot() {
  const { user } = useAuth();
  const [scope, setScope] = useState<HomeScope>('athletes');

  if (user?.manages_roster !== true) return <HomeScreen />;

  const scopeSwitch = <CoachHomeSwitch scope={scope} onChange={setScope} />;
  return scope === 'athletes' ? <CoachHomeScreen scopeSwitch={scopeSwitch} /> : <HomeScreen scopeSwitch={scopeSwitch} />;
}
