// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Admin console view of the standing pre-approval allow-list — add, list, remove
// ABOUTME: Same endpoints pierre-cli user allow / disallow / list-allowed drives, cookie-authenticated

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { adminApi } from '../services/api';
import type { PreApprovedEmail } from '../services/api/admin';
import { Button, Card, Badge, Input } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';

/**
 * Pre-approving an address is how an operator adds someone who has not
 * registered yet: their registration lands active, attributed to the operator,
 * instead of queueing. An address that already has a pending account is
 * approved on the spot by the same action.
 */
export default function PreApprovedEmails() {
  const queryClient = useQueryClient();
  const [email, setEmail] = useState('');
  const [note, setNote] = useState('');
  const [result, setResult] = useState<{ message: string; ok: boolean } | null>(null);

  const {
    data: entries = [],
    isLoading,
    error,
    refetch,
  } = useQuery<PreApprovedEmail[]>({
    queryKey: QUERY_KEYS.adminUsers.preApproved(),
    queryFn: () => adminApi.getPreApprovedEmails(),
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminUsers.preApproved() });
    // An allow can approve a pending account, which changes both user listings.
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminUsers.pending() });
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminUsers.list() });
  };

  const allowMutation = useMutation({
    mutationFn: () => adminApi.allowEmail(email.trim(), note.trim() || undefined),
    onSuccess: (data) => {
      setResult({ message: data.message, ok: true });
      setEmail('');
      setNote('');
      invalidate();
    },
    onError: (err: unknown) => {
      setResult({ message: errorMessage(err, 'Could not pre-approve that address'), ok: false });
    },
  });

  const disallowMutation = useMutation({
    mutationFn: (target: string) => adminApi.disallowEmail(target),
    onSuccess: (data) => {
      setResult({ message: data.message, ok: true });
      invalidate();
    },
    onError: (err: unknown) => {
      setResult({ message: errorMessage(err, 'Could not remove that pre-approval'), ok: false });
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!email.trim()) return;
    allowMutation.mutate();
  };

  return (
    <div className="space-y-6">
      <Card variant="dark" className="p-4">
        <h3 className="text-lg font-medium text-on-surface mb-1">Pre-approve an email</h3>
        <p className="text-sm text-on-surface-variant mb-4">
          They register themselves as usual — the allow only spares them the approval queue, and
          records you as the approver. A pending account for the same address is approved now.
        </p>
        <form onSubmit={handleSubmit} className="flex flex-col sm:flex-row gap-3">
          <div className="flex-1">
            <Input
              type="email"
              placeholder="person@example.com"
              aria-label="Email address to pre-approve"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              required
            />
          </div>
          <div className="flex-1">
            <Input
              type="text"
              placeholder="Note (cohort, reason)"
              aria-label="Note recorded with the pre-approval"
              value={note}
              onChange={(e) => setNote(e.target.value)}
            />
          </div>
          <Button type="submit" disabled={!email.trim() || allowMutation.isPending}>
            {allowMutation.isPending ? 'Allowing…' : 'Allow'}
          </Button>
        </form>
        {result && (
          <p
            role="status"
            className={`mt-3 text-sm ${result.ok ? 'text-on-surface-variant' : 'text-error'}`}
          >
            {result.message}
          </p>
        )}
      </Card>

      <div className="flex justify-between items-center">
        <h3 className="text-lg font-medium text-on-surface">
          Pre-approved emails ({entries.length})
        </h3>
        <Button onClick={() => void refetch()} variant="outline" size="sm">
          Refresh
        </Button>
      </div>

      {isLoading ? (
        <div className="space-y-4">
          {[...Array(3)].map((_, i) => (
            <Card key={i} variant="dark" className="p-4 animate-pulse">
              <div className="h-4 bg-surface-container-high rounded w-48 mb-2"></div>
              <div className="h-3 bg-surface-container-high rounded w-32"></div>
            </Card>
          ))}
        </div>
      ) : error ? (
        <Card variant="dark" className="p-6 text-center">
          <p className="text-lg font-medium text-on-surface mb-4">
            Failed to load pre-approved emails
          </p>
          <Button onClick={() => void refetch()} variant="outline">
            Retry
          </Button>
        </Card>
      ) : entries.length === 0 ? (
        <Card variant="dark" className="p-6 text-center">
          <p className="text-lg font-medium text-on-surface">No pre-approved emails</p>
          <p className="text-on-surface-variant">
            Allow an address above and it will appear here until they register.
          </p>
        </Card>
      ) : (
        <div className="space-y-4">
          {entries.map((entry) => (
            <Card key={entry.email} variant="dark" className="p-4">
              <div className="flex justify-between items-start">
                <div className="flex-1">
                  <div className="flex items-center space-x-2 mb-1">
                    <h4 className="font-medium text-on-surface">{entry.email}</h4>
                    <Badge
                      variant={entry.account_status ? statusVariant(entry.account_status) : 'secondary'}
                      className="text-xs"
                    >
                      {entry.account_status ?? 'not registered'}
                    </Badge>
                  </div>
                  {entry.note && (
                    <p className="text-sm text-on-surface-variant mb-1">{entry.note}</p>
                  )}
                  <div className="flex items-center space-x-4 text-xs text-outline">
                    <span>Allowed: {formatDate(entry.created_at)}</span>
                    <span>By: {entry.allowed_by_email ?? 'unattributed'}</span>
                  </div>
                </div>
                <Button
                  onClick={() => disallowMutation.mutate(entry.email)}
                  disabled={disallowMutation.isPending}
                  size="sm"
                  variant="outline"
                  className="border-error/50 text-error hover:bg-error/10 ml-4"
                >
                  Remove
                </Button>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}

function statusVariant(status: 'pending' | 'active' | 'suspended') {
  switch (status) {
    case 'active':
      return 'success' as const;
    case 'suspended':
      return 'destructive' as const;
    default:
      return 'warning' as const;
  }
}

function formatDate(value: string) {
  return new Date(value).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

/**
 * Surface the server's own rejection (a malformed address, a permission
 * failure) rather than a generic line — the operator needs to know which of
 * the two it was.
 */
function errorMessage(err: unknown, fallback: string): string {
  const response = (err as { response?: { data?: { message?: string; error?: string } } })?.response;
  return response?.data?.message ?? response?.data?.error ?? fallback;
}
