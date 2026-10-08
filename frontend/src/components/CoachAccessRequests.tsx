// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Admin console queue of coach-access requests — who asked, for which group, grant or decline (carnet#738)
// ABOUTME: A grant gives manages_roster and makes the coach their onboarding group's coach; super-admin only server-side

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { CoachAccessRequestView } from '@pierre/shared-types';
import { formatDate } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { coachAccessAdminApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';
import { Button, EmptyState, Section } from './ui';

/**
 * A coach whose onboarding group came back coachless asks for coach access in
 * one tap; the request waits here. Granting is the same decision as turning
 * on "May coach a group" in the user drawer, and it also makes the coach the
 * coach of the group they asked from when it still has none — so they need
 * no coach invite. Declining changes nothing else; they may ask again.
 */
export default function CoachAccessRequests() {
  const { language } = useTranslation();
  const queryClient = useQueryClient();
  const [result, setResult] = useState<{ message: string; ok: boolean } | null>(null);

  const {
    data: requests = [],
    isLoading,
    error,
    refetch,
  } = useQuery<CoachAccessRequestView[]>({
    queryKey: QUERY_KEYS.adminUsers.coachAccess('pending'),
    queryFn: () => coachAccessAdminApi.list('pending'),
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminUsers.coachAccess('pending') });
    // A grant turns on manages_roster, which the user listing and drawer show.
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminUsers.list() });
  };

  const grantMutation = useMutation({
    mutationFn: (id: string) => coachAccessAdminApi.grant(id),
    onSuccess: (data) => {
      setResult({ message: data.message, ok: true });
      invalidate();
    },
    onError: (err: unknown) => {
      setResult({ message: errorMessage(err, 'Could not grant coach access'), ok: false });
      invalidate();
    },
  });

  const declineMutation = useMutation({
    mutationFn: (id: string) => coachAccessAdminApi.decline(id),
    onSuccess: (data) => {
      setResult({ message: data.message, ok: true });
      invalidate();
    },
    onError: (err: unknown) => {
      setResult({ message: errorMessage(err, 'Could not decline the request'), ok: false });
      invalidate();
    },
  });

  const busy = grantMutation.isPending || declineMutation.isPending;

  return (
    <Section
      title={`Coach access requests (${requests.length})`}
      description="Coaches asking to coach their group. Granting lets them coach and makes them their group's coach."
      actions={
        <Button onClick={() => void refetch()} variant="outline" size="sm">
          Refresh
        </Button>
      }
      data-testid="coach-access-requests"
    >
      {result && (
        <p
          role="status"
          className={`mb-3 text-sm ${result.ok ? 'text-on-surface-variant' : 'text-error'}`}
        >
          {result.message}
        </p>
      )}
      {isLoading ? (
        <EmptyState>Loading requests…</EmptyState>
      ) : error ? (
        <EmptyState action={{ label: 'Retry', onClick: () => void refetch() }}>
          Failed to load coach access requests.
        </EmptyState>
      ) : requests.length === 0 ? (
        <EmptyState>No coach is waiting for access.</EmptyState>
      ) : (
        <ul className="divide-y divide-outline-variant">
          {requests.map((request) => (
            <li
              key={request.id}
              className="flex items-start justify-between gap-4 py-3"
              data-testid={`coach-access-row-${request.id}`}
            >
              <div className="min-w-0 flex-1">
                <p className="truncate font-medium text-on-surface">
                  {request.display_name
                    ? `${request.display_name} · ${request.email ?? 'account deleted'}`
                    : (request.email ?? 'account deleted')}
                </p>
                <p className="text-sm text-on-surface-variant">
                  {request.group_name ? `Group: ${request.group_name}` : 'No group named'}
                </p>
                <p className="text-xs text-outline">
                  Requested {formatDate(request.created_at, language)}
                </p>
              </div>
              <div className="flex shrink-0 gap-2">
                <Button
                  onClick={() => grantMutation.mutate(request.id)}
                  disabled={busy}
                  size="sm"
                >
                  Grant
                </Button>
                <Button
                  onClick={() => declineMutation.mutate(request.id)}
                  disabled={busy}
                  size="sm"
                  variant="outline"
                >
                  Decline
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

/** Surface the server's own refusal (already decided, permission) over a generic line. */
function errorMessage(err: unknown, fallback: string): string {
  const response = (err as { response?: { data?: { message?: string; error?: string } } })?.response;
  return response?.data?.message ?? response?.data?.error ?? fallback;
}
