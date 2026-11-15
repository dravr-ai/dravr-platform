// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach-access request with no group in Settings' coaching pane, for a coach who has no room to ask from
// ABOUTME: Shown to someone who coaches others without coach access, a waiting or declined request, and the granted state; Home's is the pending coach Home's seat row

import { useQuery } from '@tanstack/react-query';
import {
  COACH_SEAT_KEYS,
  QUERY_KEYS,
  canRequestCoachAccessWithoutGroup,
  coachSeatState,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { userApi } from '../services/api';
import { useAuth } from '../hooks/useAuth';
import { useOnboardingStatus } from '../hooks/useOnboardingStatus';
import CoachAccessRequest from './CoachAccessRequest';
import { Section } from './ui';

/** Open the coach's Home from anywhere in the shell: the Dashboard routes on the hash. */
function openCoachHome(): void {
  window.location.hash = 'home';
}

/**
 * Settings' coaching pane keeps the request too (carnet#1522): for someone
 * who coaches others without coach access, for anyone whose request waits or
 * was declined (so it can be withdrawn or asked again), and, once granted,
 * the way to their coach Home.
 */
export function CoachAccessSettings() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const managesRoster = user?.manages_roster === true;
  const { data: onboarding } = useOnboardingStatus(!!user && !managesRoster);
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
    enabled: !!user && !managesRoster,
  });
  const asked = coachSeatState(latest.data?.request) !== 'to_request';
  const offered =
    managesRoster ||
    asked ||
    canRequestCoachAccessWithoutGroup({ managesRoster, coachesOthers: onboarding?.coaches_others === true });
  if (!offered) return null;
  return (
    <Section title={t(COACH_SEAT_KEYS.settingsTitle)} data-testid="settings-coach-access">
      <CoachAccessRequest variant="panel" managesRoster={managesRoster} onOpenHome={openCoachHome} />
    </Section>
  );
}
