// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach-access request with no group (mobile) in Settings' coaching screen, for a coach with no room to ask from
// ABOUTME: Mirrors the web CoachAccessPanel: a waiting or declined request and the granted state; Home's is the pending coach Home's seat row

import React from 'react';
import { useQuery } from '@tanstack/react-query';
import { useRouter } from 'expo-router';
import {
  COACH_SEAT_KEYS,
  QUERY_KEYS,
  canRequestCoachAccessWithoutGroup,
  coachSeatState,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Section } from './ui';
import { CoachAccessRequest } from './CoachAccessRequest';
import { useAuth } from '../contexts/AuthContext';
import { useOnboardingStatus } from '../hooks/useOnboardingStatus';
import { userApi } from '../services/api';
import { HOME_ROUTE } from '../navigation/routes';

/**
 * Settings' coaching screen keeps the request too (carnet#1522): for someone
 * who coaches others without coach access, for anyone whose request waits or
 * was declined (so it can be withdrawn or asked again), and, once granted,
 * the way to their coach Home.
 */
export function CoachAccessSettings() {
  const { t } = useTranslation();
  const router = useRouter();
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
  if (!user || !offered) return null;
  return (
    <Section title={t(COACH_SEAT_KEYS.settingsTitle)} testID="settings-coach-access">
      <CoachAccessRequest
        variant="panel"
        managesRoster={managesRoster}
        onOpenHome={() => router.navigate(HOME_ROUTE)}
      />
    </Section>
  );
}
