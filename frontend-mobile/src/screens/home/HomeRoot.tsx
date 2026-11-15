// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What the Home tab opens: an athlete's Home, a coach's — with access or still waiting for it — and, for a coach who trains too, the switch to their own training
// ABOUTME: A coach lands on their athletes (D2, carnet#1253); one still waiting lands on « Préparer ton coaching » (carnet#1529)

import React, { useState } from 'react';
import { coachTrainsToo, landsOnPendingCoachHome } from '@pierre/shared-constants';
import { useAuth } from '../../contexts/AuthContext';
import { useOnboardingStatus } from '../../hooks/useOnboardingStatus';
import { CoachHomeScreen } from '../coach-home/CoachHomeScreen';
import { CoachHomeSwitch, type HomeScope } from '../coach-home/CoachHomeSwitch';
import { PendingCoachHomeScreen } from '../coach-home/PendingCoachHomeScreen';
import { HomeScreen } from './HomeScreen';

/**
 * The Home tab, by who is reading. An athlete has their Home. A coach with
 * coach access lands on their athletes; a coach still waiting for it
 * (carnet#1529) lands on the pending coach Home — the checklist that gets
 * them there, never the athlete's Home. Either coach who trains too has the
 * switch to their own training; one who does not has no other Home and no
 * athlete nudge, their own thread staying in the Chat tab. The onboarding
 * status is the gate's own read, already cached; a grant patches the
 * session, and the tab becomes the coach Home with no relaunch.
 */
export function HomeRoot() {
  const { user } = useAuth();
  const [scope, setScope] = useState<HomeScope>('athletes');
  const managesRoster = user?.manages_roster === true;
  const { data: onboarding } = useOnboardingStatus(!!user);
  const pending = landsOnPendingCoachHome({ managesRoster, coachesOthers: onboarding?.coaches_others === true });

  if (!managesRoster && !pending) return <HomeScreen />;
  if (!coachTrainsToo(onboarding?.steps)) {
    return managesRoster ? <CoachHomeScreen /> : <PendingCoachHomeScreen />;
  }

  const scopeSwitch = <CoachHomeSwitch scope={scope} onChange={setScope} />;
  if (scope === 'me') return <HomeScreen scopeSwitch={scopeSwitch} />;
  return managesRoster ? (
    <CoachHomeScreen scopeSwitch={scopeSwitch} />
  ) : (
    <PendingCoachHomeScreen scopeSwitch={scopeSwitch} />
  );
}
