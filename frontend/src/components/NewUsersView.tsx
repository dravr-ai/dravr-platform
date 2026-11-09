// ABOUTME: Admin console view of the accounts created in a window: the activation funnel and one row per account
// ABOUTME: Reads /api/admin/analytics/new-users (Postgres, complete whatever the analytics consent); 7/30/90-day windows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { formatDate, formatDateTime } from '@pierre/chat-utils';
import { adminApi } from '../services/api';
import { newUsersQueryKey, type NewUserCohort, type NewUserRow } from '../services/api/admin';
import { percentOf, sinceSignup } from '../utils/newUserCohort';
import { Button, Section } from './ui';
import { ADMIN_LOCALE } from '../utils/adminLocale';

/** The windows the view offers, in days. */
const NEW_USER_WINDOWS = [7, 30, 90] as const;
type Window = (typeof NEW_USER_WINDOWS)[number];

/** Status is a dot and a word (DESIGN.md §8), never a tinted chip. */
const STATUS_DOT: Record<string, string> = {
  pending: 'bg-warning',
  active: 'bg-success',
  suspended: 'bg-error',
};

const STATUS_LABEL: Record<string, string> = {
  pending: 'Pending',
  active: 'Active',
  suspended: 'Suspended',
};

const SOURCE_LABEL: Record<string, string> = {
  email: 'Email',
  google: 'Google',
  apple: 'Apple',
};

function FunnelTile({
  testId,
  label,
  count,
  percent,
  hint,
}: {
  testId: string;
  label: string;
  count: number;
  percent: string | null;
  hint?: string;
}) {
  return (
    <div data-testid={testId} className="p-4 rounded-lg bg-surface-container-low border ghost-border">
      <p className="break-words text-xs font-medium text-on-surface-variant">{label}</p>
      <p className="mt-1 font-mono text-xl text-on-surface" data-testid={`${testId}-count`}>
        {count}
      </p>
      {percent !== null && <p className="text-xs text-outline">{percent}</p>}
      {hint && <p className="mt-1 text-xs text-outline">{hint}</p>}
    </div>
  );
}

/** A milestone cell: how long after sign-up, the full date on hover, a dash when not reached. */
function MilestoneCell({ row, at }: { row: NewUserRow; at: string | null }) {
  if (at === null) {
    return <span className="text-outline" aria-label="Not yet">—</span>;
  }
  return (
    <span className="text-on-surface" title={formatDateTime(at, ADMIN_LOCALE)}>
      {sinceSignup(row.signed_up_at, at)}
    </span>
  );
}

function Funnel({ cohort }: { cohort: NewUserCohort }) {
  const { funnel } = cohort;
  const ofSignups = (n: number) => {
    const percent = percentOf(n, funnel.signed_up);
    return percent === null ? null : `${percent}% of sign-ups`;
  };
  const returnedPercent = percentOf(funnel.active_after_day_7, funnel.day_7_eligible);
  return (
    <div className="grid grid-cols-2 sm:grid-cols-4 xl:grid-cols-7 gap-3" data-testid="new-users-funnel">
      <FunnelTile testId="funnel-signed-up" label="Signed up" count={funnel.signed_up} percent={null} />
      <FunnelTile
        testId="funnel-approved"
        label="Approved"
        count={funnel.approved_or_active}
        percent={ofSignups(funnel.approved_or_active)}
      />
      <FunnelTile
        testId="funnel-verified"
        label="Email verified"
        count={funnel.email_verified}
        percent={ofSignups(funnel.email_verified)}
      />
      <FunnelTile
        testId="funnel-connected"
        label="Provider connected"
        count={funnel.provider_connected}
        percent={ofSignups(funnel.provider_connected)}
      />
      <FunnelTile
        testId="funnel-synced"
        label="Data synced"
        count={funnel.first_sync}
        percent={ofSignups(funnel.first_sync)}
      />
      <FunnelTile
        testId="funnel-first-turn"
        label="First message"
        count={funnel.first_turn}
        percent={ofSignups(funnel.first_turn)}
      />
      <FunnelTile
        testId="funnel-returned"
        label="Back after a week"
        count={funnel.active_after_day_7}
        percent={returnedPercent === null ? null : `${returnedPercent}% of those a week old`}
        hint={
          funnel.day_7_eligible === 1
            ? 'of 1 account a week old or more'
            : `of ${funnel.day_7_eligible} accounts a week old or more`
        }
      />
    </div>
  );
}

function UsersTable({ cohort }: { cohort: NewUserCohort }) {
  // Whole class names, never `text-${align}`: Tailwind only emits the classes
  // it finds spelled out in the source.
  const header = (label: string, align: 'left' | 'right' = 'left') => (
    <th
      className={`px-3 py-2 ${align === 'right' ? 'text-right' : 'text-left'} text-xs font-medium text-on-surface-variant whitespace-nowrap`}
    >
      {label}
    </th>
  );
  return (
    <div className="overflow-x-auto">
      <table className="min-w-full divide-y divide-outline-variant" data-testid="new-users-table">
        <thead className="bg-surface-container">
          <tr>
            {header('Account')}
            {header('Signed up')}
            {header('Source')}
            {header('Status')}
            {header('Verified')}
            {header('Connected')}
            {header('Synced')}
            {header('First message')}
            {header('Messages, week 1', 'right')}
            {header('Last seen')}
          </tr>
        </thead>
        <tbody className="divide-y divide-outline-variant">
          {cohort.users.map((row) => (
            <tr key={row.user_id} data-testid="new-user-row">
              <td className="px-3 py-2 text-sm">
                <div className="text-on-surface">{row.display_name ?? row.email}</div>
                {row.display_name && <div className="text-xs text-outline">{row.email}</div>}
              </td>
              <td className="px-3 py-2 text-sm text-on-surface-variant whitespace-nowrap" title={formatDateTime(row.signed_up_at, ADMIN_LOCALE)}>
                {formatDate(row.signed_up_at, ADMIN_LOCALE)}
              </td>
              <td className="px-3 py-2 text-sm text-on-surface-variant">
                {SOURCE_LABEL[row.signup_source] ?? row.signup_source}
              </td>
              <td className="px-3 py-2 text-sm">
                <span className="inline-flex items-center gap-2 text-on-surface-variant">
                  <span className={`w-2 h-2 rounded-full ${STATUS_DOT[row.status] ?? 'bg-outline-variant'}`} aria-hidden="true" />
                  {STATUS_LABEL[row.status] ?? row.status}
                </span>
              </td>
              <td className="px-3 py-2 text-sm text-on-surface-variant">
                {row.email_verified ? 'Yes' : 'No'}
              </td>
              <td className="px-3 py-2 text-sm whitespace-nowrap">
                <MilestoneCell row={row} at={row.provider_connected_at} />
              </td>
              <td className="px-3 py-2 text-sm whitespace-nowrap">
                <MilestoneCell row={row} at={row.first_sync_at} />
              </td>
              <td className="px-3 py-2 text-sm whitespace-nowrap">
                <MilestoneCell row={row} at={row.first_turn_at} />
              </td>
              <td className="px-3 py-2 text-sm text-right font-mono text-on-surface" data-testid="turns-first-week">
                {row.turns_first_week}
              </td>
              <td className="px-3 py-2 text-sm text-on-surface-variant whitespace-nowrap">
                {row.last_active_at ? formatDate(row.last_active_at, ADMIN_LOCALE) : '—'}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * The accounts created in the last 7, 30 or 90 days: the activation funnel
 * and, under it, one row per account with the time each milestone came after
 * sign-up.
 */
export default function NewUsersView() {
  const [days, setDays] = useState<Window>(30);
  const { data, isLoading, isError, refetch } = useQuery({
    queryKey: newUsersQueryKey(days),
    queryFn: () => adminApi.getNewUsers(days),
  });

  const windowPicker = (
    <div className="flex gap-1" role="group" aria-label="Window">
      {NEW_USER_WINDOWS.map((option) => (
        <button
          key={option}
          type="button"
          aria-pressed={days === option}
          data-testid={`new-users-window-${option}`}
          onClick={() => setDays(option)}
          className={`px-3 py-1 rounded-full text-xs font-medium border ${
            days === option
              ? 'bg-primary text-on-primary border-primary'
              : 'ghost-border text-on-surface-variant hover:text-on-surface'
          }`}
        >
          {option} days
        </button>
      ))}
    </div>
  );

  return (
    <Section
      title="New users"
      description={`Accounts created in the last ${days} days and how far each got.`}
      actions={windowPicker}
    >
      {isLoading ? (
        <div className="space-y-3" aria-busy="true" data-testid="new-users-loading">
          {[0, 1, 2].map((i) => (
            <div key={i} className="h-12 rounded-lg bg-surface-container-low animate-pulse" />
          ))}
        </div>
      ) : isError || !data ? (
        <div className="p-6 text-center" data-testid="new-users-error">
          <p className="text-sm font-medium text-on-surface mb-4">Couldn't load new users.</p>
          <Button variant="secondary" onClick={() => void refetch()}>
            Try again
          </Button>
        </div>
      ) : (
        <div className="space-y-6">
          <Funnel cohort={data} />
          {data.truncated && (
            <p className="text-xs text-on-surface-variant" data-testid="new-users-truncated">
              Showing the newest {data.users.length} accounts.
            </p>
          )}
          {data.users.length === 0 ? (
            <p className="p-6 text-center text-sm text-on-surface-variant" data-testid="new-users-empty">
              No accounts were created in this window.
            </p>
          ) : (
            <UsersTable cohort={data} />
          )}
        </div>
      )}
    </Section>
  );
}
