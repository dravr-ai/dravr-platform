// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The invite deep link — dravr://groups/join/<code> lands in a fresh thread that runs /group join
// ABOUTME: There is no Groups tab to land on any more, so the link redeems the code the way an athlete would

import React from 'react';
import { Redirect, useLocalSearchParams } from 'expo-router';
import { COMMAND_DRAFTS } from '@pierre/shared-constants';
import { CHAT_LIST_ROUTE, NEW_CONVERSATION_ID, threadHref } from '../../../src/navigation/routes';

/**
 * Redeem a group invite from a link.
 *
 * The redemption is `/group join <code>` — the same command the athlete could
 * type, the same one messaging runs — sent in a new thread, which is where the
 * server puts the group conversation it creates. A link with no code has
 * nothing to redeem, so it lands on the conversation list.
 *
 * The screen sits outside `(auth)`, so the root layout's guard bounces an
 * unauthenticated visitor to login and brings them back here after sign-in.
 */
export default function JoinGroupByInviteLink() {
  const { code } = useLocalSearchParams<{ code?: string }>();
  const trimmed = typeof code === 'string' ? code.trim() : '';

  if (!trimmed) {
    return <Redirect href={CHAT_LIST_ROUTE} />;
  }

  return (
    <Redirect
      href={threadHref(NEW_CONVERSATION_ID, { send: COMMAND_DRAFTS.groupJoin(trimmed) })}
    />
  );
}
