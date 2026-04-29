// ABOUTME: Phase 5E user-facing billing page — subscription, plan picker, invoices, quota gauges
// ABOUTME: Drives /api/billing/checkout, /portal, /subscription, /invoices, /users/me/quota
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery, useMutation } from '@tanstack/react-query';
import { billingApi } from '../services/api';
import { useAuth } from '../hooks/useAuth';
import { Button, Card } from './ui';
import { Badge } from './ui/Badge';

const TIER_LABELS: Record<string, string> = {
  starter: 'Starter',
  professional: 'Professional',
  enterprise: 'Enterprise',
};

function formatCurrency(amount: number | undefined, currency: string | undefined): string {
  if (amount == null) return '—';
  const value = amount / 100;
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: (currency ?? 'usd').toUpperCase(),
  }).format(value);
}

function formatDate(epochSecs: number | undefined): string {
  if (!epochSecs) return '—';
  return new Date(epochSecs * 1000).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

export default function BillingPage() {
  const { user } = useAuth();
  const [error, setError] = useState<string | null>(null);

  const subscriptionQuery = useQuery({
    queryKey: ['billing', 'subscription'],
    queryFn: () => billingApi.getSubscription(),
  });

  const invoicesQuery = useQuery({
    queryKey: ['billing', 'invoices'],
    queryFn: () => billingApi.listInvoices(),
    enabled: subscriptionQuery.data != null,
  });

  const quotaQuery = useQuery({
    queryKey: ['billing', 'quota'],
    queryFn: () => billingApi.getMyQuota(),
  });

  const checkoutMutation = useMutation({
    mutationFn: (tier: 'professional' | 'enterprise') => {
      if (!user) throw new Error('not authenticated');
      const successUrl = `${window.location.origin}/billing?upgrade=success`;
      const cancelUrl = `${window.location.origin}/billing?upgrade=cancel`;
      return billingApi.startCheckout({
        tier,
        tenant_id: '', // server resolves from auth context for self-checkout
        user_id: user.id,
        success_url: successUrl,
        cancel_url: cancelUrl,
      });
    },
    onSuccess: (data) => {
      window.location.href = data.checkout_url;
    },
    onError: (e) => setError(e instanceof Error ? e.message : 'Checkout failed'),
  });

  const portalMutation = useMutation({
    mutationFn: () => {
      const sub = subscriptionQuery.data;
      if (!sub) throw new Error('no subscription on file');
      return billingApi.openPortal({
        provider_customer_id: sub.provider_customer_id,
        return_url: `${window.location.origin}/billing`,
      });
    },
    onSuccess: (data) => {
      window.location.href = data.portal_url;
    },
    onError: (e) => setError(e instanceof Error ? e.message : 'Portal open failed'),
  });

  const sub = subscriptionQuery.data;
  const tier = sub?.plan_tier ?? user?.tier ?? 'starter';
  const tierLabel = TIER_LABELS[tier] ?? tier;

  return (
    <div className="space-y-6 max-w-5xl mx-auto">
      <Card variant="dark" className="p-6">
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-lg font-semibold text-on-surface">Current Plan</h2>
            <p className="text-sm text-on-surface-variant">
              Manage your subscription, payment method, and invoices.
            </p>
          </div>
          <Badge variant={tier === 'starter' ? 'warning' : 'success'}>{tierLabel}</Badge>
        </div>

        {subscriptionQuery.isLoading ? (
          <div className="animate-pulse h-6 bg-surface-container-high rounded w-1/3" />
        ) : sub ? (
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-sm">
            <div>
              <div className="text-on-surface-variant">Status</div>
              <div className="font-medium text-on-surface">{sub.status}</div>
            </div>
            <div>
              <div className="text-on-surface-variant">Period End</div>
              <div className="font-medium text-on-surface">
                {sub.current_period_end
                  ? new Date(sub.current_period_end).toLocaleDateString()
                  : '—'}
              </div>
            </div>
            <div>
              <div className="text-on-surface-variant">Billing Customer ({sub.provider})</div>
              <div className="font-mono text-xs text-on-surface">{sub.provider_customer_id}</div>
            </div>
            <div>
              <div className="text-on-surface-variant">Cancel at Period End</div>
              <div className="font-medium text-on-surface">{sub.cancel_at_period_end ? 'Yes' : 'No'}</div>
            </div>
          </div>
        ) : (
          <p className="text-sm text-on-surface-variant">
            No subscription on file. You're on the Starter plan — pick a paid tier below to upgrade.
          </p>
        )}

        <div className="mt-6 flex items-center space-x-3">
          {tier !== 'professional' && (
            <Button
              onClick={() => checkoutMutation.mutate('professional')}
              disabled={checkoutMutation.isPending}
              className="bg-pierre-activity hover:bg-pierre-activity/80 text-on-primary"
            >
              {checkoutMutation.isPending ? 'Redirecting…' : 'Upgrade to Professional'}
            </Button>
          )}
          {tier !== 'enterprise' && (
            <Button
              onClick={() => checkoutMutation.mutate('enterprise')}
              disabled={checkoutMutation.isPending}
              variant="secondary"
            >
              Talk to Sales (Enterprise)
            </Button>
          )}
          {sub != null && (
            <Button
              onClick={() => portalMutation.mutate()}
              disabled={portalMutation.isPending}
              variant="secondary"
            >
              {portalMutation.isPending ? 'Opening…' : 'Manage Subscription'}
            </Button>
          )}
        </div>

        {error && <p className="mt-4 text-sm text-pierre-red-400">Error: {error}</p>}
      </Card>

      <Card variant="dark" className="p-6">
        <h2 className="text-lg font-semibold text-on-surface mb-4">Usage Quota</h2>
        {quotaQuery.isLoading ? (
          <div className="space-y-3 animate-pulse">
            {[1, 2, 3].map((k) => (
              <div key={k} className="h-4 bg-surface-container-high rounded w-2/3" />
            ))}
          </div>
        ) : quotaQuery.data ? (
          <div className="space-y-3">
            {quotaQuery.data.counters.map((c) => {
              const pct = c.limit > 0 ? Math.min(100, (c.current / c.limit) * 100) : 0;
              const color = c.burst_zone
                ? 'bg-pierre-red-400'
                : c.warning
                ? 'bg-pierre-nutrition'
                : 'bg-pierre-activity';
              return (
                <div key={c.counter_type}>
                  <div className="flex justify-between text-sm mb-1">
                    <span className="text-on-surface-variant capitalize">
                      {c.counter_type.replace(/_/g, ' ')}
                    </span>
                    <span className="font-medium text-on-surface">
                      {c.current.toLocaleString()} / {c.limit === Number.MAX_SAFE_INTEGER ? '∞' : c.limit.toLocaleString()}
                    </span>
                  </div>
                  <div className="w-full bg-surface-container-high rounded-full h-2">
                    <div
                      className={`h-2 rounded-full transition-all ${color}`}
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <p className="text-sm text-on-surface-variant">Quota information unavailable.</p>
        )}
      </Card>

      <Card variant="dark" className="p-6">
        <h2 className="text-lg font-semibold text-on-surface mb-4">Invoices</h2>
        {invoicesQuery.isLoading ? (
          <div className="animate-pulse h-4 bg-surface-container-high rounded w-1/2" />
        ) : invoicesQuery.data && invoicesQuery.data.invoices.length > 0 ? (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-on-surface-variant border-b ghost-border">
                  <th className="text-left py-2">Date</th>
                  <th className="text-left py-2">Number</th>
                  <th className="text-left py-2">Amount</th>
                  <th className="text-left py-2">Status</th>
                  <th className="text-left py-2">Link</th>
                </tr>
              </thead>
              <tbody>
                {invoicesQuery.data.invoices.map((inv, idx) => (
                  <tr key={inv.id ?? idx} className="border-b ghost-border last:border-none">
                    <td className="py-2 text-on-surface">{formatDate(inv.created)}</td>
                    <td className="py-2 text-on-surface font-mono text-xs">{inv.number ?? '—'}</td>
                    <td className="py-2 text-on-surface">
                      {formatCurrency(inv.amount_paid ?? inv.amount_due, inv.currency)}
                    </td>
                    <td className="py-2 text-on-surface capitalize">{inv.status ?? '—'}</td>
                    <td className="py-2">
                      {inv.hosted_invoice_url ? (
                        <a
                          href={inv.hosted_invoice_url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="text-pierre-violet hover:underline"
                        >
                          View
                        </a>
                      ) : (
                        '—'
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="text-sm text-on-surface-variant">
            {sub ? 'No invoices yet.' : 'Invoices appear after your first paid period.'}
          </p>
        )}
      </Card>
    </div>
  );
}
