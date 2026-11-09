// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Super-admin impersonation log — every session in which an operator acted as another user, newest first
// ABOUTME: One row per session (who, as whom, when, how long); a row's details read the one session and show its reason

import { useState } from 'react';
import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { formatDateTime } from '@pierre/chat-utils';
import {
  adminApi,
  type ImpersonationSessionsCursor,
  type ImpersonationSessionsResponse,
  type ImpersonationSessionSummary,
} from '../services/api/admin';
import { Button, Section } from './ui';
import { ADMIN_LOCALE, describeAdminError } from '../utils/adminLocale';

const SESSIONS_KEY = ['admin', 'impersonation', 'sessions'] as const;

function SessionDetails({ sessionId }: { sessionId: string }) {
  const session = useQuery({
    queryKey: [...SESSIONS_KEY, sessionId],
    queryFn: () => adminApi.getImpersonationSession(sessionId),
  });
  if (session.isPending) {
    return <div className="pierre-spinner h-4 w-4 my-2" role="status" aria-label="Loading" />;
  }
  if (session.isError) {
    return (
      <p role="alert" className="text-xs text-error">
        {describeAdminError(session.error, "Couldn't load the impersonation log.")}
      </p>
    );
  }
  return (
    <dl className="mt-2 grid gap-1 text-xs text-on-surface-variant" data-testid={`impersonation-details-${sessionId}`}>
      <div>
        <dt className="font-medium text-on-surface">Reason</dt>
        <dd className="whitespace-pre-wrap break-words">{session.data.reason || 'No reason given'}</dd>
      </div>
      {session.data.ended_at && (
        <dd>Ended {formatDateTime(session.data.ended_at, ADMIN_LOCALE)}</dd>
      )}
    </dl>
  );
}

function SessionRow({ session }: { session: ImpersonationSessionSummary }) {
  const [open, setOpen] = useState(false);
  const unknown = 'Unknown user';
  return (
    <li className="py-3" data-testid={`impersonation-session-${session.id}`}>
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <p className="text-sm font-medium text-on-surface break-words">
            {session.impersonator_email ?? unknown} as {session.target_user_email ?? unknown}
          </p>
          <p className="mt-0.5 flex flex-wrap gap-x-3 text-xs text-on-surface-variant">
            <span>Started {formatDateTime(session.started_at, ADMIN_LOCALE)}</span>
            <span>{Math.max(1, Math.round(session.duration_seconds / 60))} min</span>
            <span className={session.is_active ? 'font-medium text-on-warning-container' : undefined}>
              {session.is_active ? 'Active' : 'Ended'}
            </span>
          </p>
        </div>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          data-testid={`impersonation-details-toggle-${session.id}`}
        >
          {open ? 'Hide details' : 'Details'}
        </Button>
      </div>
      {open && <SessionDetails sessionId={session.id} />}
    </li>
  );
}

function nextCursor(page: ImpersonationSessionsResponse): ImpersonationSessionsCursor | undefined {
  return page.next_before && page.next_before_id
    ? { before: page.next_before, before_id: page.next_before_id }
    : undefined;
}

export default function ImpersonationLogTab() {
  const sessions = useInfiniteQuery({
    queryKey: SESSIONS_KEY,
    queryFn: ({ pageParam }) => adminApi.listImpersonationSessions(pageParam),
    initialPageParam: undefined as ImpersonationSessionsCursor | undefined,
    getNextPageParam: nextCursor,
  });

  const renderBody = () => {
    if (sessions.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label="Loading" />;
    }
    if (sessions.isError) {
      return (
        <p role="alert" className="text-sm text-error">
          {describeAdminError(sessions.error, "Couldn't load the impersonation log.")}
        </p>
      );
    }
    const rows = sessions.data.pages.flatMap((page) => page.sessions);
    if (rows.length === 0) {
      return (
        <p className="text-sm text-on-surface-variant" data-testid="impersonation-log-empty">
          No impersonation sessions yet.
        </p>
      );
    }
    return (
      <>
        <ul className="divide-y divide-outline-variant" data-testid="impersonation-log-list">
          {rows.map((session) => (
            <SessionRow key={session.id} session={session} />
          ))}
        </ul>
        {sessions.hasNextPage && (
          <div className="mt-3">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void sessions.fetchNextPage()}
              disabled={sessions.isFetchingNextPage}
            >
              {sessions.isFetchingNextPage ? 'Loading…' : 'Load older'}
            </Button>
          </div>
        )}
      </>
    );
  };

  return (
    <Section
      title="Impersonation log"
      description="Every session in which a super admin acted as another user, newest first."
      data-testid="impersonation-log"
    >
      {renderBody()}
    </Section>
  );
}
