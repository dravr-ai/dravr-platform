// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach-access request with no group — on Home and in Settings' coaching pane, for a coach who has no room to ask from
// ABOUTME: Shown to someone who coaches others without coach access; Settings also keeps a waiting or declined request and the granted state

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
 * Home's group-less request (carnet#1522): a coach who tapped "Later" on the
 * onboarding group step, or never made a group, still has the one way to ask
 * on the page they land on. Shown only to someone who said they coach others
 * and holds no coach access; it leaves the page once the access is on, when
 * Home becomes their coach Home.
 */
export function CoachAccessHome() {
  const { user } = useAuth();
  const managesRoster = user?.manages_roster === true;
  const { data: onboarding } = useOnboardingStatus(!!user && !managesRoster);
  if (!canRequestCoachAccessWithoutGroup({ managesRoster, coachesOthers: onboarding?.coaches_others === true })) {
    return null;
  }
  return (
    <div data-testid="home-coach-access">
      <CoachAccessRequest variant="panel" />
    </div>
  );
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
