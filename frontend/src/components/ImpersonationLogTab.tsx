// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Super-admin impersonation log — every session in which an operator acted as another user, newest first
// ABOUTME: One row per session (who, as whom, when, how long); a row's details read the one session and show its reason

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { formatDateTime } from '@pierre/chat-utils';
import { describeApiError } from '@pierre/ui-logic';
import { adminApi, type ImpersonationSessionSummary } from '../services/api/admin';
import { Button, Section } from './ui';

const SESSIONS_KEY = ['admin', 'impersonation', 'sessions'] as const;

function SessionDetails({ sessionId }: { sessionId: string }) {
  const { t, i18n } = useTranslation();
  const session = useQuery({
    queryKey: [...SESSIONS_KEY, sessionId],
    queryFn: () => adminApi.getImpersonationSession(sessionId),
  });
  if (session.isPending) {
    return <div className="pierre-spinner h-4 w-4 my-2" role="status" aria-label={t('common.loading')} />;
  }
  if (session.isError) {
    return (
      <p role="alert" className="text-xs text-error">
        {describeApiError(session.error, { t, fallbackKey: 'impersonationLog.loadFailed' })}
      </p>
    );
  }
  return (
    <dl className="mt-2 grid gap-1 text-xs text-on-surface-variant" data-testid={`impersonation-details-${sessionId}`}>
      <div>
        <dt className="font-medium text-on-surface">{t('impersonationLog.reason')}</dt>
        <dd className="whitespace-pre-wrap break-words">{session.data.reason || t('impersonationLog.noReason')}</dd>
      </div>
      {session.data.ended_at && (
        <dd>{t('impersonationLog.endedAt', { date: formatDateTime(session.data.ended_at, i18n.language) })}</dd>
      )}
    </dl>
  );
}

function SessionRow({ session }: { session: ImpersonationSessionSummary }) {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const unknown = t('impersonationLog.unknownUser');
  return (
    <li className="py-3" data-testid={`impersonation-session-${session.id}`}>
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <p className="text-sm font-medium text-on-surface break-words">
            {t('impersonationLog.pair', {
              impersonator: session.impersonator_email ?? unknown,
              target: session.target_user_email ?? unknown,
            })}
          </p>
          <p className="mt-0.5 flex flex-wrap gap-x-3 text-xs text-on-surface-variant">
            <span>{t('impersonationLog.started', { date: formatDateTime(session.started_at, i18n.language) })}</span>
            <span>{t('impersonationLog.duration', { minutes: Math.max(1, Math.round(session.duration_seconds / 60)) })}</span>
            <span className={session.is_active ? 'font-medium text-warning' : undefined}>
              {session.is_active ? t('impersonationLog.active') : t('impersonationLog.ended')}
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
          {open ? t('impersonationLog.hideDetails') : t('impersonationLog.details')}
        </Button>
      </div>
      {open && <SessionDetails sessionId={session.id} />}
    </li>
  );
}

export default function ImpersonationLogTab() {
  const { t } = useTranslation();
  const sessions = useQuery({ queryKey: SESSIONS_KEY, queryFn: () => adminApi.listImpersonationSessions() });

  const renderBody = () => {
    if (sessions.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label={t('common.loading')} />;
    }
    if (sessions.isError) {
      return (
        <p role="alert" className="text-sm text-error">
          {describeApiError(sessions.error, { t, fallbackKey: 'impersonationLog.loadFailed' })}
        </p>
      );
    }
    if (sessions.data.sessions.length === 0) {
      return (
        <p className="text-sm text-on-surface-variant" data-testid="impersonation-log-empty">
          {t('impersonationLog.empty')}
        </p>
      );
    }
    return (
      <ul className="divide-y divide-outline-variant" data-testid="impersonation-log-list">
        {sessions.data.sessions.map((session) => (
          <SessionRow key={session.id} session={session} />
        ))}
      </ul>
    );
  };

  return (
    <Section
      title={t('shell.navImpersonationLog')}
      description={t('impersonationLog.hint')}
      data-testid="impersonation-log"
    >
      {renderBody()}
    </Section>
  );
}
