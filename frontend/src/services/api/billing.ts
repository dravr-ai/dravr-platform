// ABOUTME: Phase 5E billing endpoints — checkout, portal, subscription, invoices, quota
// ABOUTME: All routes auth-bound on the server; frontend just relays the bearer token
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { axios } from './client';

export interface SubscriptionView {
  id: string;
  tenant_id: string;
  user_id: string;
  /** Provider slug — `stripe`, `revenuecat`, `dummy`, … */
  provider: string;
  provider_customer_id: string;
  provider_subscription_id: string | null;
  status: string;
  plan_tier: string;
  current_period_start: string | null;
  current_period_end: string | null;
  cancel_at_period_end: boolean;
}

export interface InvoicesResponse {
  invoices: Array<{
    id?: string;
    number?: string;
    amount_paid?: number;
    amount_due?: number;
    currency?: string;
    status?: string;
    created?: number;
    hosted_invoice_url?: string;
    invoice_pdf?: string;
    [k: string]: unknown;
  }>;
}

export interface QuotaCounter {
  counter_type: string;
  current: number;
  limit: number;
  warning: boolean;
  burst_zone: boolean;
  resets_at: string;
}

export interface MyQuotaResponse {
  tier: string;
  counters: QuotaCounter[];
}

export const billingApi = {
  async startCheckout(req: {
    tier: 'starter' | 'professional' | 'enterprise';
    tenant_id: string;
    user_id: string;
    success_url: string;
    cancel_url: string;
  }): Promise<{ checkout_url: string }> {
    const response = await axios.post('/api/billing/checkout', req);
    return response.data;
  },

  async openPortal(req: { provider_customer_id: string; return_url: string }): Promise<{ portal_url: string }> {
    const response = await axios.post('/api/billing/portal', req);
    return response.data;
  },

  async getSubscription(): Promise<SubscriptionView | null> {
    try {
      const response = await axios.get('/api/billing/subscription');
      return response.data;
    } catch (e) {
      const err = e as { response?: { status?: number } };
      if (err.response?.status === 404) return null;
      throw e;
    }
  },

  async listInvoices(): Promise<InvoicesResponse> {
    const response = await axios.get('/api/billing/invoices');
    return response.data;
  },

  async getMyQuota(): Promise<MyQuotaResponse> {
    const response = await axios.get('/api/users/me/quota');
    return response.data;
  },
};
