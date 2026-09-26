// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Phase 5E mobile billing surface — current plan, upgrade CTA, manage portal, invoices
// ABOUTME: Reads the shared api-client billing domain; the dravr:// return links are mobile's own

import React, { useState } from 'react';
import {
  ActivityIndicator,
  Linking,
  ScrollView,
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
} from 'react-native';
import { useQuery, useMutation } from '@tanstack/react-query';
import type { PaidPlanTier } from '@pierre/shared-types';
import { QUERY_KEYS, hasPaymentProblem, planTierLabelKey } from '@pierre/shared-constants';
import { formatCompactNumber } from '@pierre/chat-utils';
import { useAuth } from '../../contexts/AuthContext';
import { billingApi } from '../../services/api';
import { trackMobile } from '../../services/analytics';
import { useFeatureFlags, FEATURE_KEYS } from '../../hooks/useFeatureFlags';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';

export function BillingScreen(): React.ReactElement {
  const { t } = useTranslation();
  const { user } = useAuth();
  const { flags: featureFlags } = useFeatureFlags();
  const showBillingHeader = featureFlags[FEATURE_KEYS.billingHeader];
  const [error, setError] = useState<string | null>(null);

  const subscriptionQuery = useQuery({
    queryKey: QUERY_KEYS.billing.subscription(),
    queryFn: () => billingApi.getSubscription(),
  });

  const quotaQuery = useQuery({
    queryKey: QUERY_KEYS.billing.quota(),
    queryFn: () => billingApi.getMyQuota(),
  });

  const invoicesQuery = useQuery({
    queryKey: QUERY_KEYS.billing.invoices(),
    queryFn: () => billingApi.listInvoices(),
    enabled: subscriptionQuery.data != null,
  });

  const plansQuery = useQuery({
    queryKey: QUERY_KEYS.billing.plans(),
    queryFn: () => billingApi.getPlans(),
  });

  const checkoutMutation = useMutation({
    mutationFn: (tier: PaidPlanTier) => {
      if (!user) throw new Error('not authenticated');
      trackMobile({ name: 'checkout_started', props: { tier } });
      // The provider sends the athlete back into the app on these links.
      return billingApi.startCheckout({
        tier,
        success_url: 'dravr://billing?upgrade=success',
        cancel_url: 'dravr://billing?upgrade=cancel',
      });
    },
    onSuccess: ({ checkout_url }) => {
      void Linking.openURL(checkout_url);
    },
    onError: (e) => setError(describeApiError(e, { t, fallbackKey: 'app.checkoutFailed' })),
  });

  const portalMutation = useMutation({
    mutationFn: () => {
      if (!subscriptionQuery.data) throw new Error('no subscription');
      return billingApi.openPortal({ return_url: 'dravr://billing' });
    },
    onSuccess: ({ portal_url }) => {
      void Linking.openURL(portal_url);
    },
    onError: (e) => setError(describeApiError(e, { t, fallbackKey: 'app.portalOpenFailed' })),
  });

  const sub = subscriptionQuery.data;
  // Prefer the subscription row, then the quota endpoint's tier (resolved
  // server-side from users.tier — authoritative), then the auth-context user.
  // The stored auth user may omit `tier`, which would mislabel a paid user.
  const tier = sub?.plan_tier ?? quotaQuery.data?.tier ?? user?.tier ?? 'starter';
  const tierLabelKey = planTierLabelKey(tier);
  const tierLabel = tierLabelKey ? t(tierLabelKey) : tier;
  const paymentProblem = hasPaymentProblem(sub?.status);

  return (
    <ScrollView style={styles.scroll} contentContainerStyle={styles.container}>
      {paymentProblem && (
        <View style={[styles.card, styles.dunningCard]}>
          <Text className="text-sm font-semibold" style={styles.dunningTitle}>{t('app.paymentProblem')}</Text>
          <Text className="text-sm" style={styles.muted}>
            {t('app.lastPaymentFailed', {
              plan: tierLabel,
              status: sub?.status ?? '',
            })}
          </Text>
          <TouchableOpacity
            style={[styles.button, styles.buttonDanger]}
            disabled={portalMutation.isPending}
            onPress={() => portalMutation.mutate()}
          >
            <Text style={styles.buttonText}>
              {portalMutation.isPending ? t('app.opening') : t('app.updatePayment')}
            </Text>
          </TouchableOpacity>
        </View>
      )}
      {showBillingHeader && (
      <View style={styles.card}>
        <View style={styles.cardHeader}>
          <Text className="text-base font-semibold" style={styles.cardTitle}>{t('app.currentPlan')}</Text>
          <Text style={styles.tierBadge}>{tierLabel}</Text>
        </View>
        {subscriptionQuery.isLoading ? (
          <ActivityIndicator />
        ) : sub ? (
          <View style={styles.row}>
            <Text style={styles.label}>{t('app.status')}</Text>
            <Text style={styles.value}>{sub.status}</Text>
          </View>
        ) : (
          <Text className="text-sm" style={styles.muted}>
            {t('app.onStarterUpgrade')}
          </Text>
        )}

        {tier !== 'professional' && (
          <TouchableOpacity
            style={[styles.button, styles.buttonPrimary]}
            disabled={checkoutMutation.isPending}
            onPress={() => checkoutMutation.mutate('professional')}
          >
            <Text style={styles.buttonText}>
              {checkoutMutation.isPending ? t('app.openingCheckout') : t('app.upgradeToProfessional')}
            </Text>
          </TouchableOpacity>
        )}

        {sub != null && (
          <TouchableOpacity
            style={[styles.button, styles.buttonSecondary]}
            disabled={portalMutation.isPending}
            onPress={() => portalMutation.mutate()}
          >
            <Text style={styles.buttonText}>
              {portalMutation.isPending ? t('app.opening') : t('app.manageSubscription')}
            </Text>
          </TouchableOpacity>
        )}

        {sub != null && (
          <Text className="text-xs" style={styles.fineprint}>
            {sub.cancel_at_period_end
              ? t('app.planCancelScheduled')
              : t('app.useManageSubscription')}
          </Text>
        )}

        {error != null && <Text className="text-sm" style={styles.error}>{t('app.errorWithReason', { reason: error })}</Text>}
      </View>
      )}

      <View style={styles.card}>
        <Text className="text-base font-semibold" style={styles.cardTitle}>{t('app.plans')}</Text>
        <Text className="text-sm" style={styles.muted}>
          {t('app.planCompareBlurb')}
        </Text>
        {plansQuery.isLoading ? (
          <ActivityIndicator />
        ) : plansQuery.data ? (
          plansQuery.data.plans.map((plan) => {
            const cap = (v: number): string => (plan.unlimited ? t('app.unlimited') : formatCompactNumber(v));
            const includedUsage =
              plan.included_usd != null
                ? `$${plan.included_usd}/mo`
                : plan.unlimited
                ? t('app.custom')
                : '—';
            const isCurrent = plan.tier === tier;
            return (
              <View
                key={plan.tier}
                style={[styles.planCard, isCurrent && styles.planCardCurrent]}
              >
                <View style={styles.cardHeader}>
                  <Text className="text-base font-semibold" style={styles.planTitle}>{plan.label}</Text>
                  {isCurrent && <Text style={styles.tierBadge}>{t('app.current')}</Text>}
                </View>
                {(
                  [
                    ['Messages / day', cap(plan.daily_messages)],
                    ['Tokens / day', cap(plan.daily_tokens)],
                    ['Agents', cap(plan.max_active_agents)],
                    ['Tool calls / day', cap(plan.daily_tool_calls)],
                    ['Included usage', includedUsage],
                  ] as Array<[string, string]>
                ).map(([planLabel, planValue]) => (
                  <View key={planLabel} style={styles.row}>
                    <Text style={styles.label}>{planLabel}</Text>
                    <Text style={styles.value}>{planValue}</Text>
                  </View>
                ))}
                {!isCurrent && plan.tier === 'professional' && (
                  <TouchableOpacity
                    style={[styles.button, styles.buttonPrimary]}
                    disabled={checkoutMutation.isPending}
                    onPress={() => checkoutMutation.mutate('professional')}
                  >
                    <Text style={styles.buttonText}>
                      {checkoutMutation.isPending ? t('app.openingCheckout') : t('app.upgrade')}
                    </Text>
                  </TouchableOpacity>
                )}
                {!isCurrent && plan.tier === 'enterprise' && (
                  <TouchableOpacity
                    style={[styles.button, styles.buttonSecondary]}
                    disabled={checkoutMutation.isPending}
                    onPress={() => checkoutMutation.mutate('enterprise')}
                  >
                    <Text style={styles.buttonText}>{t('app.talkToSales')}</Text>
                  </TouchableOpacity>
                )}
              </View>
            );
          })
        ) : (
          <Text className="text-sm" style={styles.muted}>{t('app.planInfoUnavailable')}</Text>
        )}
      </View>

      <View style={styles.card}>
        <Text className="text-base font-semibold" style={styles.cardTitle}>{t('app.usageQuota')}</Text>
        {quotaQuery.isLoading ? (
          <ActivityIndicator />
        ) : quotaQuery.data ? (
          quotaQuery.data.counters.map((c) => {
            const ratio = c.limit > 0 ? Math.min(1, c.current / c.limit) : 0;
            const barColor = c.burst_zone ? '#ef4444' : c.warning ? '#f59e0b' : '#10b981';
            return (
              <View key={c.counter_type} style={styles.quotaRow}>
                <View style={styles.quotaTopRow}>
                  <Text style={styles.label}>{c.counter_type.replace(/_/g, ' ')}</Text>
                  <Text style={styles.value}>
                    {c.current.toLocaleString()} / {c.limit === Number.MAX_SAFE_INTEGER ? '∞' : c.limit.toLocaleString()}
                  </Text>
                </View>
                <View style={styles.barTrack}>
                  <View
                    style={[
                      styles.barFill,
                      { width: `${Math.round(ratio * 100)}%`, backgroundColor: barColor },
                    ]}
                  />
                </View>
              </View>
            );
          })
        ) : (
          <Text className="text-sm" style={styles.muted}>{t('app.quotaInfoUnavailable')}</Text>
        )}
      </View>

      <View style={styles.card}>
        <Text className="text-base font-semibold" style={styles.cardTitle}>{t('app.invoices')}</Text>
        {invoicesQuery.isLoading ? (
          <ActivityIndicator />
        ) : invoicesQuery.data && invoicesQuery.data.invoices.length > 0 ? (
          invoicesQuery.data.invoices.map((inv, idx) => (
            <TouchableOpacity
              key={inv.id ?? idx}
              style={styles.invoiceRow}
              onPress={() => {
                if (inv.hosted_invoice_url) void Linking.openURL(inv.hosted_invoice_url);
              }}
              disabled={!inv.hosted_invoice_url}
            >
              <Text style={styles.value}>
                {inv.created ? new Date(inv.created * 1000).toLocaleDateString() : '—'}
              </Text>
              <Text style={styles.label}>{inv.number ?? '—'}</Text>
              <Text style={styles.value}>
                {inv.amount_paid != null
                  ? new Intl.NumberFormat('en-US', {
                      style: 'currency',
                      currency: (inv.currency ?? 'usd').toUpperCase(),
                    }).format(inv.amount_paid / 100)
                  : '—'}
              </Text>
            </TouchableOpacity>
          ))
        ) : (
          <Text className="text-sm" style={styles.muted}>
            {sub ? t('app.noInvoicesYet') : t('app.invoicesAppearAfter')}
          </Text>
        )}
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: { flex: 1, backgroundColor: '#0a0a0a' },
  container: { padding: 16, gap: 16 },
  card: {
    backgroundColor: '#1c1c1c',
    borderRadius: 12,
    padding: 16,
    gap: 12,
    borderWidth: 1,
    borderColor: '#2a2a2a',
  },
  cardHeader: { flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center' },
  cardTitle: { color: '#f5f5f5' },
  tierBadge: { color: '#10b981', fontWeight: '600' },
  row: { flexDirection: 'row', justifyContent: 'space-between' },
  label: { color: '#a1a1aa', textTransform: 'capitalize' },
  value: { color: '#f5f5f5' },
  muted: { color: '#a1a1aa' },
  fineprint: { color: '#a1a1aa' },
  error: { color: '#ef4444' },
  dunningCard: { borderColor: 'rgba(239,68,68,0.6)', backgroundColor: 'rgba(239,68,68,0.1)' },
  dunningTitle: { color: '#ef4444' },
  planCard: {
    borderWidth: 1,
    borderColor: '#2a2a2a',
    borderRadius: 10,
    padding: 12,
    gap: 8,
  },
  planCardCurrent: { borderColor: '#10b981', backgroundColor: 'rgba(16,185,129,0.05)' },
  planTitle: { color: '#f5f5f5' },
  button: {
    paddingVertical: 12,
    borderRadius: 10,
    alignItems: 'center',
  },
  buttonPrimary: { backgroundColor: '#10b981' },
  buttonSecondary: { backgroundColor: '#2a2a2a' },
  buttonDanger: { backgroundColor: '#ef4444' },
  buttonText: { color: '#f5f5f5', fontWeight: '600' },
  quotaRow: { gap: 6 },
  quotaTopRow: { flexDirection: 'row', justifyContent: 'space-between' },
  barTrack: { backgroundColor: '#2a2a2a', height: 6, borderRadius: 4, overflow: 'hidden' },
  barFill: { height: 6 },
  invoiceRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: '#2a2a2a',
  },
});
