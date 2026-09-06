// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The shared room transcript of a coaching group, read from the surface-neutral model
// ABOUTME: Renders exactly what the coach's ambient context sees - one visibility rule, every surface

import { MessageCircle } from 'lucide-react';
import { useGroupTranscript } from '../../hooks/useGroups';
import { useTranslation } from '@pierre/i18n';

interface GroupTranscriptPanelProps {
  groupId: string;
}

/**
 * The room, as every member shares it.
 *
 * Entries come consent-filtered from the server: an unconsented member stays
 * on the roster while their words are withheld, which is the same rule the
 * pipeline applies before the coach reasons over the room. A messaging turn,
 * a web turn and ambient room chatter all land in this one transcript.
 */
export default function GroupTranscriptPanel({ groupId }: GroupTranscriptPanelProps) {
  const { t } = useTranslation();
  const { transcript, isLoading, isError } = useGroupTranscript(groupId, true);

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner" />
      </div>
    );
  }

  if (isError || !transcript) {
    return (
      <p className="px-3 py-6 text-sm text-outline text-center">
        {t('groups.roomLoadFailed')}
      </p>
    );
  }

  if (transcript.entries.length === 0) {
    return (
      <div className="px-3 py-8 text-center">
        <MessageCircle className="w-8 h-8 mx-auto mb-2 text-outline" />
        <p className="text-sm text-on-surface-variant">{t('groups.roomEmpty')}</p>
        <p className="text-xs text-outline mt-1">
          {t('groups.roomHint')}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3" data-testid="group-transcript">
      {transcript.entries.map((entry) => (
        <div key={entry.id} className="flex gap-3">
          <div
            className={`flex-1 rounded-lg px-3 py-2 ${
              entry.speaker === 'coach' ? 'bg-surface-container-high' : 'bg-surface-container-low'
            }`}
          >
            <div className="flex items-baseline justify-between gap-2">
              <span className="text-xs font-medium text-on-surface-variant">
                {entry.author_display_name ?? entry.author_user_id}
                {entry.speaker === 'coach' ? ' · agent' : ''}
              </span>
              <span className="text-xs text-outline">
                {new Date(entry.created_at).toLocaleString()}
              </span>
            </div>
            <p className="text-sm text-on-surface whitespace-pre-wrap mt-0.5">{entry.content}</p>
          </div>
        </div>
      ))}
    </div>
  );
}
