// ABOUTME: Whether a group thread offers its coach the chat app — the owning coach only, once, read from server records
// ABOUTME: « Pas maintenant » is the messaging_configure onboarding record, so the answer holds on every device (carnet#1525)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery, useQueryClient } from '@tanstack/react-query';
import type { AvailableChannel, ChannelLink } from '@pierre/api-client';
import type { CoachingGroup } from '@pierre/shared-types';
import {
  CHAT_APP_OFFER_STEP_ID,
  QUERY_KEYS,
  chatAppOfferAnswered,
  offeredChatApp,
  withStepRecorded,
} from '@pierre/shared-constants';
import { messagingLinkApi, userApi } from '../../services/api';
import { useAuth } from '../../hooks/useAuth';
import { useOnboardingStatus } from '../../hooks/useOnboardingStatus';
import { useAvailableChannels } from '../../hooks/useAvailableChannels';

/** The onboarding status as the shared query caches it. */
type OnboardingStatus = Awaited<ReturnType<typeof userApi.getOnboardingStatus>>;

/** What the card needs once it is shown: the app it names, and its two answers. */
export interface GroupChatAppOfferState {
  app: AvailableChannel;
  /** « Pas maintenant »: store the answer, then hide the card. Rejects, card kept, when it was not stored. */
  decline: () => Promise<void>;
}

/**
 * Whether the group thread offers its coach a chat app, once.
 *
 * Only the coach who owns the group meets it (`coaches_others` and the
 * group's `owner_id`), only while the workspace offers an app, the coach has
 * none linked, and the offer was never answered — on any device: the answer
 * is the `messaging_configure` onboarding record (`CHAT_APP_OFFER_STEP_ID`),
 * the one the wizard's chat-app screen wrote before it moved here. Nothing is
 * read for anyone else.
 */
export function useGroupChatAppOffer(group: CoachingGroup | null): GroupChatAppOfferState | null {
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const status = useOnboardingStatus(Boolean(group));
  const candidate =
    group !== null &&
    user?.id === group.owner_id &&
    status.data?.coaches_others === true &&
    !chatAppOfferAnswered(status.data.steps);
  const { channels } = useAvailableChannels({ enabled: candidate });
  const links = useQuery({
    queryKey: ['messaging-links'],
    queryFn: () => messagingLinkApi.listLinks(),
    enabled: candidate,
  });
  const linked: ChannelLink[] = Array.isArray(links.data) ? links.data : [];
  const app = offeredChatApp(channels);

  if (!candidate || app === undefined || links.data === undefined || linked.length > 0) return null;

  const decline = async () => {
    // Stored first: a card hidden on an answer that never landed would come
    // back on the next load, so a failure keeps it up with the error.
    await userApi.setOnboardingStep(CHAT_APP_OFFER_STEP_ID, 'skipped');
    const key = QUERY_KEYS.user.onboardingStatus();
    queryClient.setQueryData<OnboardingStatus>(key, (previous) =>
      previous
        ? { ...previous, steps: withStepRecorded(previous.steps, CHAT_APP_OFFER_STEP_ID, 'skipped') }
        : previous,
    );
    void queryClient.invalidateQueries({ queryKey: key });
  };

  return { app, decline };
}
