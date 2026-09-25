// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The billing shapes both clients read — subscription, invoices, plan catalogue, quota snapshot, checkout/portal
// ABOUTME: Mirrors pierre_routes_billing::billing and pierre_server::routes::user_profile's quota response

/** A plan tier slug, as the server's `UserTier::as_str` spells it. */
export type PlanTier = 'starter' | 'professional' | 'enterprise';

/** A tier a checkout can move the athlete to. */
export type PaidPlanTier = Exclude<PlanTier, 'starter'>;

/** `GET /api/billing/subscription`: the caller's own subscription row. */
export interface SubscriptionView {
  id: string;
  tenant_id: string;
  user_id: string;
  /** Provider slug — `stripe`, `revenuecat`, `dummy`, … */
  provider: string;
  provider_customer_id: string;
  provider_subscription_id: string | null;
  /** Lifecycle status mirrored from the provider (`active`, `past_due`, …). */
  status: string;
  /** The tier slug; a string on the wire, so an unknown tier still arrives. */
  plan_tier: string;
  /** RFC 3339. */
  current_period_start: string | null;
  /** RFC 3339. */
  current_period_end: string | null;
  /** True when cancellation is scheduled at the period end. */
  cancel_at_period_end: boolean;
}

/** One invoice row, as the billing provider reports it. */
export interface BillingInvoice {
  id?: string;
  number?: string;
  /** Smallest currency unit (cents for USD). */
  amount_paid?: number;
  /** Smallest currency unit. */
  amount_due?: number;
  /** ISO-4217, lowercase. */
  currency?: string;
  /** `paid`, `open`, `void`, `uncollectible`. */
  status?: string;
  /** Unix epoch seconds. */
  created?: number;
  hosted_invoice_url?: string;
  invoice_pdf?: string;
}

/** `GET /api/billing/invoices`: newest first. */
export interface InvoicesResponse {
  invoices: BillingInvoice[];
}

/** One counter in the caller's quota snapshot. */
export interface QuotaCounter {
  /** `daily_messages`, `daily_tokens`, `weekly_tokens`, `daily_tool_calls`, … */
  counter_type: string;
  current: number;
  limit: number;
  warning: boolean;
  burst_zone: boolean;
  /** ISO 8601. */
  resets_at: string;
}

/** `GET /api/users/me/quota`: the effective tier and every counter the chat write path checks. */
export interface MyQuotaResponse {
  tier: string;
  counters: QuotaCounter[];
}

/** One plan in the comparison catalogue, derived server-side from the caps it enforces. */
export interface PlanView {
  tier: string;
  label: string;
  /** True when the caps are effectively unlimited (Enterprise). */
  unlimited: boolean;
  daily_messages: number;
  daily_tokens: number;
  monthly_tokens: number;
  max_active_agents: number;
  daily_tool_calls: number;
  /** Monthly USD included before metered overage; `null` for a tier without overage. */
  included_usd: number | null;
}

/** `GET /api/billing/plans`: Starter → Professional → Enterprise. */
export interface PlansResponse {
  plans: PlanView[];
}

/**
 * `POST /api/billing/checkout` body. The user and tenant come from the bearer
 * token, never from the body: it carries only the plan and the redirect targets.
 */
export interface CheckoutRequest {
  tier: PaidPlanTier;
  success_url: string;
  cancel_url: string;
}

/** Where the provider's hosted checkout lives. */
export interface CheckoutResponse {
  checkout_url: string;
}

/** `POST /api/billing/portal` body. The portal opens on the caller's own subscription row. */
export interface PortalRequest {
  return_url: string;
}

/** Where the provider's customer portal lives. */
export interface PortalResponse {
  portal_url: string;
}
