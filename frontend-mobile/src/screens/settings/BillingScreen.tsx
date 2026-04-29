// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Phase 5E mobile billing surface — current plan, upgrade CTA, manage portal, invoices
// ABOUTME: Calls /api/billing/{checkout,portal,subscription,invoices,users/me/quota}

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
import { useAuth } from '../../contexts/AuthContext';
import { apiClient } from '../../services/api';

interface SubscriptionView {
  id: string;
  tenant_id: string;
  user_id: string;
  provider: string;
  provider_customer_id: string;
  provider_subscription_id: string | null;
  status: string;
  plan_tier: string;
  current_period_end: string | null;
  cancel_at_period_end: boolean;
}

interface QuotaCounter {
  counter_type: string;
  current: number;
  limit: number;
  warning: boolean;
  burst_zone: boolean;
  resets_at: string;
}

interface MyQuotaResponse {
  tier: string;
  counters: QuotaCounter[];
}

interface InvoiceRow {
  id?: string;
  number?: string;
  amount_paid?: number;
  amount_due?: number;
  currency?: string;
  status?: string;
  created?: number;
  hosted_invoice_url?: string;
}

const TIER_LABEL: Record<string, string> = {
  starter: 'Starter',
  professional: 'Professional',
  enterprise: 'Enterprise',
};

export function BillingScreen(): React.ReactElement {
  const { user } = useAuth();
  const [error, setError] = useState<string | null>(null);

  const subscriptionQuery = useQuery({
    queryKey: ['mobile-billing', 'subscription'],
    queryFn: async (): Promise<SubscriptionView | null> => {
      try {
        const r = await apiClient.get('/api/billing/subscription');
        return r.data as SubscriptionView;
      } catch (e) {
        const err = e as { response?: { status?: number } };
        if (err.response?.status === 404) return null;
        throw e;
      }
    },
  });

  const quotaQuery = useQuery({
    queryKey: ['mobile-billing', 'quota'],
    queryFn: async (): Promise<MyQuotaResponse> => {
      const r = await apiClient.get('/api/users/me/quota');
      return r.data as MyQuotaResponse;
    },
  });

  const invoicesQuery = useQuery({
    queryKey: ['mobile-billing', 'invoices'],
    queryFn: async (): Promise<{ invoices: InvoiceRow[] }> => {
      const r = await apiClient.get('/api/billing/invoices');
      return r.data as { invoices: InvoiceRow[] };
    },
    enabled: subscriptionQuery.data != null,
  });

  const checkoutMutation = useMutation({
    mutationFn: async (tier: 'professional' | 'enterprise'): Promise<{ checkout_url: string }> => {
      if (!user) throw new Error('not authenticated');
      const r = await apiClient.post('/api/billing/checkout', {
        tier,
        tenant_id: '',
        user_id: user.id,
        success_url: 'dravr://billing?upgrade=success',
        cancel_url: 'dravr://billing?upgrade=cancel',
      });
      return r.data as { checkout_url: string };
    },
    onSuccess: ({ checkout_url }) => {
      void Linking.openURL(checkout_url);
    },
    onError: (e) => setError(e instanceof Error ? e.message : 'Checkout failed'),
  });

  const portalMutation = useMutation({
    mutationFn: async (): Promise<{ portal_url: string }> => {
      const sub = subscriptionQuery.data;
      if (!sub) throw new Error('no subscription');
      const r = await apiClient.post('/api/billing/portal', {
        provider_customer_id: sub.provider_customer_id,
        return_url: 'dravr://billing',
      });
      return r.data as { portal_url: string };
    },
    onSuccess: ({ portal_url }) => {
      void Linking.openURL(portal_url);
    },
    onError: (e) => setError(e instanceof Error ? e.message : 'Portal open failed'),
  });

  const sub = subscriptionQuery.data;
  const tier = sub?.plan_tier ?? user?.tier ?? 'starter';

  return (
    <ScrollView style={styles.scroll} contentContainerStyle={styles.container}>
      <View style={styles.card}>
        <View style={styles.cardHeader}>
          <Text style={styles.cardTitle}>Current Plan</Text>
          <Text style={styles.tierBadge}>{TIER_LABEL[tier] ?? tier}</Text>
        </View>
        {subscriptionQuery.isLoading ? (
          <ActivityIndicator />
        ) : sub ? (
          <View style={styles.row}>
            <Text style={styles.label}>Status</Text>
            <Text style={styles.value}>{sub.status}</Text>
          </View>
        ) : (
          <Text style={styles.muted}>
            You're on Starter. Upgrade to Professional for higher daily caps and overage billing.
          </Text>
        )}

        {tier !== 'professional' && (
          <TouchableOpacity
            style={[styles.button, styles.buttonPrimary]}
            disabled={checkoutMutation.isPending}
            onPress={() => checkoutMutation.mutate('professional')}
          >
            <Text style={styles.buttonText}>
              {checkoutMutation.isPending ? 'Opening Checkout…' : 'Upgrade to Professional'}
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
              {portalMutation.isPending ? 'Opening…' : 'Manage Subscription'}
            </Text>
          </TouchableOpacity>
        )}

        {error != null && <Text style={styles.error}>Error: {error}</Text>}
      </View>

      <View style={styles.card}>
        <Text style={styles.cardTitle}>Usage Quota</Text>
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
          <Text style={styles.muted}>Quota information unavailable.</Text>
        )}
      </View>

      <View style={styles.card}>
        <Text style={styles.cardTitle}>Invoices</Text>
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
          <Text style={styles.muted}>
            {sub ? 'No invoices yet.' : 'Invoices appear after your first paid period.'}
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
  cardTitle: { color: '#f5f5f5', fontSize: 16, fontWeight: '600' },
  tierBadge: { color: '#10b981', fontWeight: '600' },
  row: { flexDirection: 'row', justifyContent: 'space-between' },
  label: { color: '#a1a1aa', textTransform: 'capitalize' },
  value: { color: '#f5f5f5' },
  muted: { color: '#a1a1aa', fontSize: 14 },
  error: { color: '#ef4444', fontSize: 14 },
  button: {
    paddingVertical: 12,
    borderRadius: 10,
    alignItems: 'center',
  },
  buttonPrimary: { backgroundColor: '#10b981' },
  buttonSecondary: { backgroundColor: '#2a2a2a' },
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
