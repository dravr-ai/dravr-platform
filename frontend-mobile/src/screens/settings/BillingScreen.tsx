// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile billing screen — current plan, upgrade CTA, manage portal, invoices, drawn on the theme tokens
// ABOUTME: Reads the shared api-client billing domain; the dravr:// return links are mobile's own

import React, { useState } from 'react';
import {
  ActivityIndicator,
  ScrollView,
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
} from 'react-native';
import { useQuery, useMutation } from '@tanstack/react-query';
import type { PaidPlanTier } from '@pierre/shared-types';
import {
  PLAN_INCLUDED_USAGE_LABEL_KEY,
  PLAN_LIMIT_ROWS,
  PLAN_PER_MONTH_KEY,
  PLAN_UNLIMITED_KEY,
  PLAN_INCLUDED_CUSTOM_KEY,
  QUERY_KEYS,
  billingLabel,
  hasPaymentProblem,
  planTierLabelKey,
  quotaCounterLabelKey,
  subscriptionStatusLabelKey,
} from '@pierre/shared-constants';
import { formatCompactNumber, formatCount, formatDate, formatMajorCurrency, formatMinorCurrency } from '@pierre/chat-utils';
import { ALERT_TONE, Badge, Button, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useAuth } from '../../contexts/AuthContext';
import { billingApi } from '../../services/api';
import { trackMobile } from '../../services/analytics';
import { useFeatureFlags, FEATURE_KEYS } from '../../hooks/useFeatureFlags';
import { openExternal } from '../../utils/openExternal';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';

// The page's recurring lines, as whole class strings so NativeWind generates
// each one; every colour is a theme token, so the page follows the athlete's
// appearance setting.
const HEADER_ROW_CLASS = 'flex-row items-center justify-between';
const PAIR_ROW_CLASS = 'flex-row justify-between';
/** A section pays no inset for its content; plain text, bars and buttons pay the pane's 16 themselves. */
const SECTION_BODY_CLASS = 'px-4 gap-3';
const PLAN_TITLE_CLASS = 'text-base font-semibold text-text-primary';
const LABEL_CLASS = 'text-sm text-text-secondary';
const VALUE_CLASS = 'text-sm text-text-primary';
const MUTED_CLASS = 'text-sm text-text-secondary';

export function BillingScreen(): React.ReactElement {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
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
      void openExternal(checkout_url, t);
    },
    onError: (e) => setError(describeApiError(e, { t, fallbackKey: 'app.checkoutFailed' })),
  });

  const portalMutation = useMutation({
    mutationFn: () => {
      if (!subscriptionQuery.data) throw new Error('no subscription');
      return billingApi.openPortal({ return_url: 'dravr://billing' });
    },
    onSuccess: ({ portal_url }) => {
      void openExternal(portal_url, t);
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
  const statusLabel = sub ? billingLabel(sub.status, subscriptionStatusLabelKey, t) : '';

  return (
    <ScrollView className="flex-1 bg-background-primary" contentContainerClassName="py-4 gap-8">
      {paymentProblem && (
        <View className={`mx-4 rounded-xl border p-4 gap-3 ${ALERT_TONE.error.edge} ${ALERT_TONE.error.fill}`}>
          <Text className={`text-sm font-semibold ${ALERT_TONE.error.ink}`}>{t('app.paymentProblem')}</Text>
          <Text className={`text-sm ${ALERT_TONE.error.ink}`}>
            {t('app.lastPaymentFailed', {
              plan: tierLabel,
              status: statusLabel,
            })}
          </Text>
          <Button
            variant="danger"
            title={portalMutation.isPending ? t('app.opening') : t('app.updatePayment')}
            disabled={portalMutation.isPending}
            onPress={() => portalMutation.mutate()}
          />
        </View>
      )}
      {showBillingHeader && (
      <Section title={t('app.currentPlan')} actions={<Badge variant="primary" label={tierLabel} />}>
        <View className={SECTION_BODY_CLASS}>
        {subscriptionQuery.isLoading ? (
          <ActivityIndicator color={colors.tokens.primary} />
        ) : sub ? (
          <View className={PAIR_ROW_CLASS}>
            <Text className={LABEL_CLASS}>{t('app.status')}</Text>
            <Text className={VALUE_CLASS}>{statusLabel}</Text>
          </View>
        ) : (
          <Text className={MUTED_CLASS}>
            {t('app.onStarterUpgrade')}
          </Text>
        )}

        {tier !== 'professional' && (
          <Button
            title={checkoutMutation.isPending ? t('app.openingCheckout') : t('app.upgradeToProfessional')}
            disabled={checkoutMutation.isPending}
            onPress={() => checkoutMutation.mutate('professional')}
          />
        )}

        {sub != null && (
          <Button
            variant="secondary"
            title={portalMutation.isPending ? t('app.opening') : t('app.manageSubscription')}
            disabled={portalMutation.isPending}
            onPress={() => portalMutation.mutate()}
          />
        )}

        {sub != null && (
          <Text className="text-xs text-text-secondary">
            {sub.cancel_at_period_end
              ? t('app.planCancelScheduled')
              : t('app.useManageSubscription')}
          </Text>
        )}

        {error != null && <Text className="text-sm text-error">{t('app.errorWithReason', { reason: error })}</Text>}
        </View>
      </Section>
      )}

      <Section title={t('app.plans')} description={t('app.planCompareBlurb')}>
        <View className={SECTION_BODY_CLASS}>
        {plansQuery.isLoading ? (
          <ActivityIndicator color={colors.tokens.primary} />
        ) : plansQuery.data ? (
          plansQuery.data.plans.map((plan) => {
            const cap = (v: number): string =>
              plan.unlimited ? t(PLAN_UNLIMITED_KEY) : formatCompactNumber(v, t, language);
            const includedUsage =
              plan.included_usd != null
                ? t(PLAN_PER_MONTH_KEY, { amount: formatMajorCurrency(plan.included_usd, 'usd', language) })
                : plan.unlimited
                ? t(PLAN_INCLUDED_CUSTOM_KEY)
                : '—';
            const isCurrent = plan.tier === tier;
            return (
              <View
                key={plan.tier}
                className={`rounded-lg border p-3 gap-2 ${isCurrent ? 'border-primary bg-primary/5' : 'border-border'}`}
              >
                <View className={HEADER_ROW_CLASS}>
                  <Text className={PLAN_TITLE_CLASS}>{plan.label}</Text>
                  {isCurrent && <Badge variant="primary" label={t('app.current')} />}
                </View>
                {[
                  ...PLAN_LIMIT_ROWS.map((row): [string, string] => [t(row.labelKey), cap(plan[row.field])]),
                  [t(PLAN_INCLUDED_USAGE_LABEL_KEY), includedUsage] as [string, string],
                ].map(([planLabel, planValue]) => (
                  <View key={planLabel} className={PAIR_ROW_CLASS}>
                    <Text className={LABEL_CLASS}>{planLabel}</Text>
                    <Text className={VALUE_CLASS}>{planValue}</Text>
                  </View>
                ))}
                {!isCurrent && plan.tier === 'professional' && (
                  <Button
                    title={checkoutMutation.isPending ? t('app.openingCheckout') : t('app.upgrade')}
                    disabled={checkoutMutation.isPending}
                    onPress={() => checkoutMutation.mutate('professional')}
                  />
                )}
                {!isCurrent && plan.tier === 'enterprise' && (
                  <Button
                    variant="secondary"
                    title={t('app.talkToSales')}
                    disabled={checkoutMutation.isPending}
                    onPress={() => checkoutMutation.mutate('enterprise')}
                  />
                )}
              </View>
            );
          })
        ) : (
          <Text className={MUTED_CLASS}>{t('app.planInfoUnavailable')}</Text>
        )}
        </View>
      </Section>

      <Section title={t('app.usageQuota')}>
        <View className={SECTION_BODY_CLASS}>
        {quotaQuery.isLoading ? (
          <ActivityIndicator color={colors.tokens.primary} />
        ) : quotaQuery.data ? (
          quotaQuery.data.counters.map((c) => {
            const ratio = c.limit > 0 ? Math.min(1, c.current / c.limit) : 0;
            const barClass = c.burst_zone ? 'bg-error' : c.warning ? 'bg-warning' : 'bg-success';
            return (
              <View key={c.counter_type} className="gap-1.5">
                <View className={PAIR_ROW_CLASS}>
                  <Text className={LABEL_CLASS}>{billingLabel(c.counter_type, quotaCounterLabelKey, t)}</Text>
                  <Text className={VALUE_CLASS}>
                    {formatCount(c.current, language)} / {c.limit === Number.MAX_SAFE_INTEGER ? '∞' : formatCount(c.limit, language)}
                  </Text>
                </View>
                <View className="h-1.5 overflow-hidden rounded bg-surface-container-high">
                  <View
                    className={`h-1.5 ${barClass}`}
                    style={{ width: `${Math.round(ratio * 100)}%` }}
                    testID={`quota-bar-${c.counter_type}`}
                  />
                </View>
              </View>
            );
          })
        ) : (
          <Text className={MUTED_CLASS}>{t('app.quotaInfoUnavailable')}</Text>
        )}
        </View>
      </Section>

      <Section title={t('app.invoices')}>
        <View className={SECTION_BODY_CLASS}>
        {invoicesQuery.isLoading ? (
          <ActivityIndicator color={colors.tokens.primary} />
        ) : invoicesQuery.data && invoicesQuery.data.invoices.length > 0 ? (
          invoicesQuery.data.invoices.map((inv, idx) => (
            <TouchableOpacity
              key={inv.id ?? idx}
              className="flex-row justify-between border-b border-border-faint py-2"
              style={{ borderBottomWidth: StyleSheet.hairlineWidth }}
              onPress={() => {
                if (inv.hosted_invoice_url) void openExternal(inv.hosted_invoice_url, t);
              }}
              disabled={!inv.hosted_invoice_url}
            >
              <Text className={VALUE_CLASS}>
                {inv.created ? formatDate(new Date(inv.created * 1000).toISOString(), language) : '—'}
              </Text>
              <Text className={LABEL_CLASS}>{inv.number ?? '—'}</Text>
              <Text className={VALUE_CLASS}>
                {inv.amount_paid != null
                  ? formatMinorCurrency(inv.amount_paid, inv.currency ?? 'usd', language)
                  : '—'}
              </Text>
            </TouchableOpacity>
          ))
        ) : (
          <Text className={MUTED_CLASS}>
            {sub ? t('app.noInvoicesYet') : t('app.invoicesAppearAfter')}
          </Text>
        )}
        </View>
      </Section>
    </ScrollView>
  );
}
