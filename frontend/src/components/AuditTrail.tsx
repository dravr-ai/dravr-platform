// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Admin console audit trail — what operators did, newest first, from the console, pierre-cli or a service token
// ABOUTME: Filters by outcome and action prefix, pages older events by the server's cursor; a row opens before → after and the raw metadata

import { useState } from 'react';
import { useInfiniteQuery } from '@tanstack/react-query';
import { formatDateTime } from '@pierre/chat-utils';
import { adminApi } from '../services/api';
import {
  auditEventsQueryKey,
  type AuditEvent,
  type AuditEventsPage,
  type AuditEventsQuery,
  type AuditOutcome,
} from '../services/api/admin';
import { Badge, Button, EmptyState, Input, Section, Select } from './ui';
import { ADMIN_LOCALE } from '../utils/adminLocale';

const PAGE_SIZE = 50;

type OutcomeFilter = AuditOutcome | 'all';

const OUTCOME_OPTIONS: { value: OutcomeFilter; label: string }[] = [
  { value: 'all', label: 'All outcomes' },
  { value: 'success', label: 'Succeeded' },
  { value: 'refused', label: 'Refused' },
  { value: 'failed', label: 'Failed' },
];

const OUTCOME_BADGE: Record<AuditOutcome, 'success' | 'warning' | 'error'> = {
  success: 'success',
  refused: 'warning',
  failed: 'error',
};

const SOURCE_LABEL: Record<AuditEvent['source'], string> = {
  console: 'console',
  cli: 'pierre-cli',
  cli_database: 'pierre-cli (database)',
  service: 'service token',
};

/**
 * Who acted. An event with no email is either a door that carries no account
 * (pierre-cli on the database, a service token) or an operator whose account
 * has since been deleted — the row must say which, not show a blank.
 */
function actorLabel(event: AuditEvent): string {
  if (event.actor_email) return event.actor_email;
  if (event.source === 'cli_database') return 'pierre-cli (database)';
  if (event.source === 'service') return 'service token';
  return 'deleted account';
}

function compact(value: unknown): string {
  if (value === null || value === undefined) return '';
  if (typeof value === 'string') return value;
  return JSON.stringify(value);
}

/** What the action touched, in one line: the route parameters, then the recorded target. */
function targetLine(event: AuditEvent): string {
  const parts: string[] = [];
  const params = event.metadata?.params;
  if (params && typeof params === 'object') {
    for (const [key, value] of Object.entries(params)) {
      parts.push(`${key}: ${compact(value)}`);
    }
  }
  const target = event.metadata?.target;
  if (target !== undefined && target !== null) parts.push(compact(target));
  return parts.join(' · ');
}

function pretty(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function EventRow({ event }: { event: AuditEvent }) {
  const [open, setOpen] = useState(false);
  const target = targetLine(event);
  const before = event.metadata?.before;
  const after = event.metadata?.after;
  const hasChange = before !== undefined || after !== undefined;
  return (
    <li className="py-3" data-testid={`audit-event-${event.id}`}>
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={OUTCOME_BADGE[event.outcome]} className="text-xs">
              {event.outcome}
            </Badge>
            <p className="text-sm font-medium text-on-surface break-words">{event.summary}</p>
          </div>
          <p className="mt-1 flex flex-wrap gap-x-3 text-xs text-on-surface-variant">
            <time dateTime={event.occurred_at}>{formatDateTime(event.occurred_at, ADMIN_LOCALE)}</time>
            <span>By: {actorLabel(event)}</span>
            <span>Via: {SOURCE_LABEL[event.source] ?? event.source}</span>
            <span className="font-mono">{event.action}</span>
          </p>
          {target && (
            <p className="mt-0.5 text-xs text-outline font-mono break-all" data-testid={`audit-target-${event.id}`}>
              {target}
            </p>
          )}
        </div>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          data-testid={`audit-details-toggle-${event.id}`}
        >
          {open ? 'Hide details' : 'Details'}
        </Button>
      </div>
      {open && (
        <div className="mt-2 grid gap-2 text-xs text-on-surface-variant" data-testid={`audit-details-${event.id}`}>
          {hasChange && (
            <div className="grid gap-2 sm:grid-cols-2">
              <div className="min-w-0">
                <p className="font-medium text-on-surface">Before</p>
                <pre className="mt-1 overflow-x-auto rounded bg-surface-container p-2 font-mono">
                  {before === undefined ? '—' : pretty(before)}
                </pre>
              </div>
              <div className="min-w-0">
                <p className="font-medium text-on-surface">After</p>
                <pre className="mt-1 overflow-x-auto rounded bg-surface-container p-2 font-mono">
                  {after === undefined ? '—' : pretty(after)}
                </pre>
              </div>
            </div>
          )}
          <div className="min-w-0">
            <p className="font-medium text-on-surface">Recorded details</p>
            <pre className="mt-1 overflow-x-auto rounded bg-surface-container p-2 font-mono">
              {pretty(event.metadata ?? {})}
            </pre>
          </div>
        </div>
      )}
    </li>
  );
}

/**
 * Every change an operator made — through this console, pierre-cli, or a
 * service token — and every one the server refused, so "who deleted that
 * account?" has an answer that is not someone's memory.
 */
export default function AuditTrail() {
  const [outcome, setOutcome] = useState<OutcomeFilter>('all');
  const [actionDraft, setActionDraft] = useState('');
  const [action, setAction] = useState('');

  const filters = {
    limit: PAGE_SIZE,
    ...(outcome === 'all' ? {} : { outcome }),
    ...(action ? { action } : {}),
  };

  const query = useInfiniteQuery({
    queryKey: auditEventsQueryKey(filters),
    queryFn: ({ pageParam }) => adminApi.getAuditEvents({ ...filters, ...pageParam }),
    initialPageParam: undefined as Pick<AuditEventsQuery, 'before' | 'before_id'> | undefined,
    // The cursor is the last row's instant and id: rows sharing that instant
    // past the page boundary are read next, not skipped.
    getNextPageParam: (page: AuditEventsPage) =>
      page.next_before && page.next_before_id
        ? { before: page.next_before, before_id: page.next_before_id }
        : undefined,
  });

  const events = query.data?.pages.flatMap((page) => page.events) ?? [];

  const applyAction = (e: React.FormEvent) => {
    e.preventDefault();
    setAction(actionDraft.trim());
  };

  const renderBody = () => {
    if (query.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label="Loading" />;
    }
    if (query.isError) {
      return (
        <p role="alert" className="text-sm text-error">
          Could not load the audit trail.{' '}
          <button
            type="button"
            onClick={() => void query.refetch()}
            className="rounded font-medium text-primary hover:text-primary-hover focus-ring"
          >
            Retry
          </button>
        </p>
      );
    }
    if (events.length === 0) {
      return (
        <EmptyState data-testid="audit-empty">
          {outcome === 'all' && !action
            ? 'No operator actions recorded yet. Each change made from this console, pierre-cli or a service token — deleting a user, changing a setting, removing a pre-approval — will appear here, along with every attempt the server refused.'
            : 'No recorded actions match these filters.'}
        </EmptyState>
      );
    }
    return (
      <>
        <ul className="divide-y divide-outline-variant" data-testid="audit-list">
          {events.map((event) => (
            <EventRow key={event.id} event={event} />
          ))}
        </ul>
        {query.hasNextPage && (
          <div className="mt-3">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void query.fetchNextPage()}
              disabled={query.isFetchingNextPage}
            >
              {query.isFetchingNextPage ? 'Loading…' : 'Load older'}
            </Button>
          </div>
        )}
      </>
    );
  };

  return (
    <Section
      title="Audit"
      description="What operators did, newest first — from this console, pierre-cli and service tokens, including what the server refused."
      data-testid="audit-trail"
    >
      <form onSubmit={applyAction} className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-end">
        <div className="sm:w-48">
          <Select
            aria-label="Outcome"
            size="sm"
            value={outcome}
            onChange={(e) => setOutcome(e.target.value as OutcomeFilter)}
            options={OUTCOME_OPTIONS}
          />
        </div>
        <div className="flex-1">
          <Input
            type="text"
            aria-label="Action starts with"
            placeholder="Action starts with (user., settings.)"
            value={actionDraft}
            onChange={(e) => setActionDraft(e.target.value)}
          />
        </div>
        <Button type="submit" variant="secondary" size="sm">
          Filter
        </Button>
      </form>
      {renderBody()}
    </Section>
  );
}
