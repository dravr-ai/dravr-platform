// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The topics of the athlete's personal thread, newest first, at the head of Home's history
// ABOUTME: Each is titled with the athlete's first words in it; picking one opens the thread where that topic starts

import { useQuery } from '@tanstack/react-query';
import { Minus } from 'lucide-react';
import { formatListTimestamp } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { topicsApi, type ThreadTopic } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { useConversationList, usePersonalConversation } from '../../hooks/useConversationList';

interface PersonalTopicsProps {
  /** Open the thread where a topic starts: its divider, or `null` for the thread's opening. */
  onOpenTopic: (conversationId: string, dividerId: string | null) => void;
}

/**
 * "+" and `/reset` divide the personal thread rather than start another
 * (carnet#840), so the stretches the athlete would once have found as
 * separate threads are its topics. A thread with one topic has nothing to
 * list beyond itself.
 */
export default function PersonalTopics({ onOpenTopic }: PersonalTopicsProps) {
  const { t, language } = useTranslation();
  const { id: conversationId } = usePersonalConversation();
  const { conversations } = useConversationList();
  const threadTitle = conversations.find(c => c.id === conversationId)?.title;
  const { data: topics } = useQuery<ThreadTopic[]>({
    queryKey: QUERY_KEYS.chat.topics(conversationId),
    queryFn: () => topicsApi.getConversationTopics(conversationId!),
    enabled: !!conversationId,
  });
  if (!conversationId || !topics || topics.length < 2) return null;

  const label = threadTitle ?? t('home.personal.history');
  return (
    <section aria-label={label} data-testid="personal-topics" className="border-b ghost-border pb-2">
      <h3 className="px-4 pb-1 pt-3 text-xs font-medium text-outline">{label}</h3>
      <ul>
        {topics.map(topic => (
          <li key={topic.divider_id ?? 'opening'}>
            <button
              type="button"
              data-testid="personal-topic"
              data-divider-id={topic.divider_id ?? ''}
              onClick={() => onOpenTopic(conversationId, topic.divider_id ?? null)}
              className="flex touch-target w-full items-center gap-3 px-4 py-2 text-left transition-colors hover:bg-surface-container-low focus-ring"
            >
              <Minus className="h-4 w-4 flex-shrink-0 text-outline" aria-hidden="true" />
              <span className="min-w-0 flex-1 truncate text-sm text-on-surface">
                {topic.title ?? t('chat.newChat')}
              </span>
              <span className="flex-shrink-0 text-xs text-outline">
                {formatListTimestamp(topic.started_at, language)}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
