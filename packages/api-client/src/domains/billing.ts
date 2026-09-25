// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Billing domain API — subscription, invoices, plans, quota snapshot, hosted checkout and portal
// ABOUTME: All six routes are auth-bound; the user and tenant come from the bearer token, never from a body

import type { AxiosInstance } from 'axios';
import type {
  CheckoutRequest,
  CheckoutResponse,
  InvoicesResponse,
  MyQuotaResponse,
  PlansResponse,
  PortalRequest,
  PortalResponse,
  SubscriptionView,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

export type {
  CheckoutRequest,
  CheckoutResponse,
  InvoicesResponse,
  MyQuotaResponse,
  PlansResponse,
  PortalRequest,
  PortalResponse,
  SubscriptionView,
};

/**
 * Creates the billing API bound to an axios instance.
 */
export function createBillingApi(axios: AxiosInstance) {
  return {
    /**
     * Start a hosted checkout. The body carries only the plan and where the
     * provider sends the athlete back; each client supplies its own targets.
     */
    async startCheckout(request: CheckoutRequest): Promise<CheckoutResponse> {
      const response = await axios.post<CheckoutResponse>(ENDPOINTS.BILLING.CHECKOUT, request);
      return response.data;
    },

    /** Open the provider's customer portal on the caller's own subscription row. */
    async openPortal(request: PortalRequest): Promise<PortalResponse> {
      const response = await axios.post<PortalResponse>(ENDPOINTS.BILLING.PORTAL, request);
      return response.data;
    },

    /** The caller's subscription, or `null` when they have none (the server's 404). */
    async getSubscription(): Promise<SubscriptionView | null> {
      try {
        const response = await axios.get<SubscriptionView>(ENDPOINTS.BILLING.SUBSCRIPTION);
        return response.data;
      } catch (error) {
        if ((error as { response?: { status?: number } }).response?.status === 404) return null;
        throw error;
      }
    },

    /** The caller's invoices, newest first. */
    async listInvoices(): Promise<InvoicesResponse> {
      const response = await axios.get<InvoicesResponse>(ENDPOINTS.BILLING.INVOICES);
      return response.data;
    },

    /** The effective tier and every quota counter, for the plan page's gauges. */
    async getMyQuota(): Promise<MyQuotaResponse> {
      const response = await axios.get<MyQuotaResponse>(ENDPOINTS.BILLING.MY_QUOTA);
      return response.data;
    },

    /** The plan comparison catalogue. */
    async getPlans(): Promise<PlansResponse> {
      const response = await axios.get<PlansResponse>(ENDPOINTS.BILLING.PLANS);
      return response.data;
    },
  };
}

export type BillingApi = ReturnType<typeof createBillingApi>;
