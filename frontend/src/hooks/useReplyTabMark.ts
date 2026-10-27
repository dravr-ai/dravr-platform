// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Marks the browser tab's title with a count when a reply lands while the tab is hidden
// ABOUTME: The mark comes off the moment the athlete looks at the tab again

import { useCallback, useEffect, useRef } from 'react';
import { TURN_TRAIL_KEYS } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';

/**
 * A long turn invites the athlete to leave the tab; when its reply lands in a
 * hidden tab, the title says so — "(1) Dravr" — until the tab is shown again.
 *
 * Returns the call a thread makes when a reply lands. It does nothing while
 * the tab is visible: the athlete is already looking at the reply.
 */
export function useReplyTabMark(): () => void {
  const { t } = useTranslation();
  // The title before the first mark, restored when the tab is shown again.
  const baseTitleRef = useRef<string | null>(null);
  const countRef = useRef(0);

  const clear = useCallback(() => {
    if (baseTitleRef.current !== null) document.title = baseTitleRef.current;
    baseTitleRef.current = null;
    countRef.current = 0;
  }, []);

  useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState === 'visible') clear();
    };
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      document.removeEventListener('visibilitychange', onVisibility);
      clear();
    };
  }, [clear]);

  return useCallback(() => {
    if (document.visibilityState !== 'hidden') return;
    if (baseTitleRef.current === null) baseTitleRef.current = document.title;
    countRef.current += 1;
    document.title = t(TURN_TRAIL_KEYS.tabTitle, { count: countRef.current, title: baseTitleRef.current });
  }, [t]);
}
