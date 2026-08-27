// ABOUTME: Phase 5E user-facing billing page — subscription, plan picker, invoices, quota gauges
// ABOUTME: Drives /api/billing/checkout, /portal, /subscription, /invoices, /users/me/quota
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useQuery, useMutation } from '@tanstack/react-query';
import { billingApi } from '../services/api';
import { track } from '../services/analytics';
import type { PlanView } from '../services/api';
import { useAuth } from '../hooks/useAuth';
import { useFeatureFlags, FEATURE_KEYS } from '../hooks/useFeatureFlags';
import { Button, Card } from './ui';
import { Badge } from './ui/Badge';
import { useTranslation } from '@pierre/i18n';

const TIER_LABEL_KEYS: Record<string, string> = {
  starter: 'plan.starter',
  professional: 'plan.professional',
  enterprise: 'plan.enterprise',
};

/** Subscription statuses that mean the user must fix their payment. */
const PAYMENT_PROBLEM_STATUSES = new Set(['past_due', 'unpaid', 'incomplete', 'incomplete_expired']);

/** Compact integer formatting for quota caps (500000 → "500K", 5000000 → "5M"). */
function formatCompact(value: number): string {
  return new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 }).format(
    value
  );
}

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
  const { t } = useTranslation();
  const { user } = useAuth();
  const { flags: featureFlags } = useFeatureFlags();
  const showBillingHeader = featureFlags[FEATURE_KEYS.billingHeader];
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

  const plansQuery = useQuery({
    queryKey: ['billing', 'plans'],
    queryFn: () => billingApi.getPlans(),
  });

  const checkoutMutation = useMutation({
    mutationFn: (tier: 'professional' | 'enterprise') => {
      if (!user) throw new Error('not authenticated');
      track({ name: 'checkout_started', props: { tier } });
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
    onError: (e) => setError(e instanceof Error ? e.message : t('shell.billingCheckoutFailed')),
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
    onError: (e) => setError(e instanceof Error ? e.message : t('shell.billingPortalOpenFailed')),
  });

  const sub = subscriptionQuery.data;
  // Prefer the subscription row, then the quota endpoint's tier (resolved
  // server-side from users.tier — authoritative), then the auth-context
  // user as a last resort. The stored auth user may omit `tier`, so relying
  // on it alone mislabels a paid user as Starter.
  const tier = sub?.plan_tier ?? quotaQuery.data?.tier ?? user?.tier ?? 'starter';
  const tierLabel = TIER_LABEL_KEYS[tier] ? t(TIER_LABEL_KEYS[tier]) : tier;
  const hasPaymentProblem = sub != null && PAYMENT_PROBLEM_STATUSES.has(sub.status);

  // Checkout success return path: the provider redirects to
  // `/billing?upgrade=success` once payment completes. Fire the funnel-close
  // events once the subscription row has resolved to the new paid tier, then
  // strip the query param so a refresh doesn't double-count. Gated on the
  // resolved `tier` so we report the real destination plan, not a guess.
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    if (params.get('upgrade') !== 'success') return;
    if (subscriptionQuery.isLoading) return;
    track({ name: 'checkout_completed', props: { tier } });
    track({ name: 'tier_changed', props: { from: 'starter', to: tier } });
    params.delete('upgrade');
    const query = params.toString();
    const cleaned = `${window.location.pathname}${query ? `?${query}` : ''}${window.location.hash}`;
    window.history.replaceState(null, '', cleaned);
  }, [subscriptionQuery.isLoading, tier]);

  return (
    <div className="space-y-6 max-w-5xl mx-auto">
      {hasPaymentProblem && (
        <Card variant="dark" className="p-4 border border-error/60 bg-error/10">
          <div className="flex items-start justify-between gap-4">
            <div>
              <h3 className="text-sm font-semibold text-error">
                {t('shell.billingPaymentProblem')}
              </h3>
              <p className="mt-1 text-sm text-on-surface-variant">
                {t('frag.lastPaymentFor')} {tierLabel} plan didn&apos;t go through (status:{' '}
                {sub?.status}). Update your payment method to keep your plan — otherwise it will
                revert to Starter.
              </p>
            </div>
            <Button
              onClick={() => portalMutation.mutate()}
              disabled={portalMutation.isPending}
              className="shrink-0 bg-error hover:bg-error/80 text-on-primary"
            >
              {portalMutation.isPending ? t('shell.billingPortalOpening') : t('shell.billingUpdatePayment')}
            </Button>
          </div>
        </Card>
      )}
      {showBillingHeader && (
      <Card variant="dark" className="p-6">
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-lg font-semibold text-on-surface">{t('shell.billingCurrentPlanHeading')}</h2>
            <p className="text-sm text-on-surface-variant">
              {t('shell.billingSubtitle')}
            </p>
          </div>
          <Badge variant={tier === 'starter' ? 'warning' : 'success'}>{tierLabel}</Badge>
        </div>

        {subscriptionQuery.isLoading ? (
          <div className="animate-pulse h-6 bg-surface-container-high rounded w-1/3" />
        ) : sub ? (
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-sm">
            <div>
              <div className="text-on-surface-variant">{t('auth.statusFieldLabel')}</div>
              <div className="font-medium text-on-surface">{sub.status}</div>
            </div>
            <div>
              <div className="text-on-surface-variant">{t('shell.billingPeriodEnd')}</div>
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
              <div className="text-on-surface-variant">{t('shell.billingCancelAtPeriodEnd')}</div>
              <div className="font-medium text-on-surface">{sub.cancel_at_period_end ? 'Yes' : 'No'}</div>
            </div>
          </div>
        ) : (
          <p className="text-sm text-on-surface-variant">
            {t('shell.billingNoSubscription')}
          </p>
        )}

        <div className="mt-6 flex items-center space-x-3">
          {tier !== 'professional' && (
            <Button
              onClick={() => checkoutMutation.mutate('professional')}
              disabled={checkoutMutation.isPending}
              className="bg-activity hover:bg-activity/80 text-on-primary"
            >
              {checkoutMutation.isPending ? t('shell.billingRedirecting') : t('shell.billingUpgradeProfessional')}
            </Button>
          )}
          {tier !== 'enterprise' && (
            <Button
              onClick={() => checkoutMutation.mutate('enterprise')}
              disabled={checkoutMutation.isPending}
              variant="secondary"
            >
              {t('shell.billingTalkToSalesEnterprise')}
            </Button>
          )}
          {sub != null && (
            <Button
              onClick={() => portalMutation.mutate()}
              disabled={portalMutation.isPending}
              variant="secondary"
            >
              {portalMutation.isPending ? t('shell.billingPortalOpening') : t('shell.billingManageSubscription')}
            </Button>
          )}
        </div>

        {sub != null && (
          <p className="mt-3 text-xs text-on-surface-variant">
            {sub.cancel_at_period_end
              ? t('shell.billingCancelScheduled')
              : t('shell.billingManageHint')}
          </p>
        )}

        {error && <p className="mt-4 text-sm text-error">{t('frag.errorLabel')} {error}</p>}
      </Card>
      )}

      <Card variant="dark" className="p-6">
        <h2 className="text-lg font-semibold text-on-surface mb-1">{t('shell.billingPlans')}</h2>
        <p className="text-sm text-on-surface-variant mb-4">
          {t('shell.billingPlanCompareHint')}
        </p>
        {plansQuery.isLoading ? (
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4 animate-pulse">
            {[1, 2, 3].map((k) => (
              <div key={k} className="h-48 bg-surface-container-high rounded-xl" />
            ))}
          </div>
        ) : plansQuery.data?.plans ? (
          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            {plansQuery.data.plans.map((plan) => (
              <PlanCard
                key={plan.tier}
                plan={plan}
                isCurrent={plan.tier === tier}
                onUpgrade={() =>
                  checkoutMutation.mutate(plan.tier as 'professional' | 'enterprise')
                }
                onManage={() => (sub != null ? portalMutation.mutate() : undefined)}
                checkoutPending={checkoutMutation.isPending}
                hasSubscription={sub != null}
              />
            ))}
          </div>
        ) : (
          <p className="text-sm text-on-surface-variant">{t('shell.billingPlanUnavailable')}</p>
        )}
      </Card>

      <Card variant="dark" className="p-6">
        <h2 className="text-lg font-semibold text-on-surface mb-4">{t('shell.billingUsageQuota')}</h2>
        {quotaQuery.isLoading ? (
          <div className="space-y-3 animate-pulse">
            {[1, 2, 3].map((k) => (
              <div key={k} className="h-4 bg-surface-container-high rounded w-2/3" />
            ))}
          </div>
        ) : quotaQuery.data?.counters ? (
          <div className="space-y-3">
            {quotaQuery.data.counters.map((c) => {
              const pct = c.limit > 0 ? Math.min(100, (c.current / c.limit) * 100) : 0;
              const color = c.burst_zone
                ? 'bg-error'
                : c.warning
                ? 'bg-nutrition'
                : 'bg-activity';
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
          <p className="text-sm text-on-surface-variant">{t('shell.billingQuotaUnavailable')}</p>
        )}
      </Card>

      <Card variant="dark" className="p-6">
        <h2 className="text-lg font-semibold text-on-surface mb-4">{t('shell.billingInvoices')}</h2>
        {invoicesQuery.isLoading ? (
          <div className="animate-pulse h-4 bg-surface-container-high rounded w-1/2" />
        ) : invoicesQuery.data?.invoices?.length ? (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-on-surface-variant border-b ghost-border">
                  <th className="text-left py-2">{t('shell.billingInvoiceDate')}</th>
                  <th className="text-left py-2">{t('shell.billingInvoiceNumber')}</th>
                  <th className="text-left py-2">{t('shell.billingAmount')}</th>
                  <th className="text-left py-2">{t('auth.statusFieldLabel')}</th>
                  <th className="text-left py-2">{t('shell.billingInvoiceLink')}</th>
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
                          className="text-primary hover:underline"
                        >
                          {t('shell.billingViewInvoice')}
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
            {sub ? t('shell.billingNoInvoices') : t('shell.billingInvoicesAfterFirstPeriod')}
          </p>
        )}
      </Card>
    </div>
  );
}

/** One plan column in the comparison grid. */
function PlanCard({
  plan,
  isCurrent,
  onUpgrade,
  onManage,
  checkoutPending,
  hasSubscription,
}: {
  plan: PlanView;
  isCurrent: boolean;
  onUpgrade: () => void;
  onManage: () => void;
  checkoutPending: boolean;
  hasSubscription: boolean;
}) {
  const { t } = useTranslation();
  const cap = (value: number): string => (plan.unlimited ? t('shell.billingUnlimited') : formatCompact(value));
  const includedUsage = plan.included_usd != null
    ? `$${plan.included_usd}/mo`
    : plan.unlimited
    ? t('shell.billingIncludedCustom')
    : '—';
  const features: Array<[string, string]> = [
    ['Messages / day', cap(plan.daily_messages)],
    ['Tokens / day', cap(plan.daily_tokens)],
    ['Coaches', cap(plan.max_active_coaches)],
    ['Tool calls / day', cap(plan.daily_tool_calls)],
    ['Included usage', includedUsage],
  ];

  return (
    <div
      className={`rounded-xl border p-5 flex flex-col ${
        isCurrent ? 'border-activity bg-activity/5' : 'ghost-border'
      }`}
    >
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-base font-semibold text-on-surface">{plan.label}</h3>
        {isCurrent && <Badge variant="success">{t('shell.billingCurrentBadge')}</Badge>}
      </div>
      <ul className="space-y-2 text-sm flex-1">
        {features.map(([label, value]) => (
          <li key={label} className="flex justify-between gap-2">
            <span className="text-on-surface-variant">{label}</span>
            <span className="font-medium text-on-surface">{value}</span>
          </li>
        ))}
      </ul>
      <div className="mt-4">
        {isCurrent ? (
          <Button disabled variant="secondary" className="w-full">
            {t('shell.billingCurrentPlanLabel')}
          </Button>
        ) : plan.tier === 'enterprise' ? (
          <Button onClick={onUpgrade} disabled={checkoutPending} variant="secondary" className="w-full">
            {t('shell.billingTalkToSales')}
          </Button>
        ) : plan.tier === 'professional' ? (
          <Button
            onClick={onUpgrade}
            disabled={checkoutPending}
            className="w-full bg-activity hover:bg-activity/80 text-on-primary"
          >
            {checkoutPending ? t('shell.billingRedirecting') : t('shell.billingUpgrade')}
          </Button>
        ) : (
          <Button onClick={onManage} disabled={!hasSubscription} variant="secondary" className="w-full">
            {t('shell.billingDowngrade')}
          </Button>
        )}
      </div>
    </div>
  );
}
