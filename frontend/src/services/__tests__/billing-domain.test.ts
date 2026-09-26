// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the shared @pierre/api-client billing domain both plan pages call
// ABOUTME: Pins each of the six routes, the bodies the checkout and portal send, and "no subscription" as null

import { describe, it, expect, vi } from 'vitest'
import type { AxiosInstance } from 'axios'
import { createBillingApi } from '@pierre/api-client'
import type { MyQuotaResponse, PlansResponse, SubscriptionView } from '@pierre/shared-types'

function axiosStub(handlers: { get?: (url: string) => Promise<unknown>; post?: (url: string, body: unknown) => Promise<unknown> }) {
  const stub = {
    get: vi.fn(handlers.get ?? (() => Promise.resolve({ data: {} }))),
    post: vi.fn(handlers.post ?? (() => Promise.resolve({ data: {} }))),
  }
  return { stub, api: createBillingApi(stub as unknown as AxiosInstance) }
}

const subscription: SubscriptionView = {
  id: 'sub-1',
  tenant_id: 't-1',
  user_id: 'u-1',
  provider: 'stripe',
  provider_customer_id: 'cus_1',
  provider_subscription_id: 'sub_stripe_1',
  status: 'past_due',
  plan_tier: 'professional',
  current_period_start: '2026-09-01T00:00:00Z',
  current_period_end: '2026-10-01T00:00:00Z',
  cancel_at_period_end: false,
}

describe('shared billing domain', () => {
  it('starts a checkout with only the plan and the return links', async () => {
    const { stub, api } = axiosStub({
      post: () => Promise.resolve({ data: { checkout_url: 'https://checkout.example/s/1' } }),
    })

    const result = await api.startCheckout({
      tier: 'professional',
      success_url: 'https://app.example/billing?upgrade=success',
      cancel_url: 'https://app.example/billing?upgrade=cancel',
    })

    expect(stub.post).toHaveBeenCalledWith('/api/billing/checkout', {
      tier: 'professional',
      success_url: 'https://app.example/billing?upgrade=success',
      cancel_url: 'https://app.example/billing?upgrade=cancel',
    })
    expect(result.checkout_url).toBe('https://checkout.example/s/1')
  })

  it('opens the portal with only the return link', async () => {
    const { stub, api } = axiosStub({
      post: () => Promise.resolve({ data: { portal_url: 'https://portal.example/p/1' } }),
    })

    const result = await api.openPortal({ return_url: 'dravr://billing' })

    expect(stub.post).toHaveBeenCalledWith('/api/billing/portal', { return_url: 'dravr://billing' })
    expect(result.portal_url).toBe('https://portal.example/p/1')
  })

  it('reads the subscription row from /api/billing/subscription', async () => {
    const { stub, api } = axiosStub({ get: () => Promise.resolve({ data: subscription }) })

    const result = await api.getSubscription()

    expect(stub.get).toHaveBeenCalledWith('/api/billing/subscription')
    expect(result?.status).toBe('past_due')
    expect(result?.plan_tier).toBe('professional')
  })

  it('reads the server 404 as "no subscription", not as a failure', async () => {
    const { api } = axiosStub({ get: () => Promise.reject({ response: { status: 404 } }) })

    await expect(api.getSubscription()).resolves.toBeNull()
  })

  it('lets any other subscription failure through', async () => {
    const refusal = { response: { status: 500 } }
    const { api } = axiosStub({ get: () => Promise.reject(refusal) })

    await expect(api.getSubscription()).rejects.toBe(refusal)
  })

  it('reads invoices, plans and the quota snapshot from their routes', async () => {
    const plans: PlansResponse = {
      plans: [
        {
          tier: 'starter',
          label: 'Starter',
          unlimited: false,
          daily_messages: 50,
          daily_tokens: 500_000,
          monthly_tokens: 5_000_000,
          max_active_agents: 3,
          daily_tool_calls: 200,
          included_usd: null,
        },
      ],
    }
    const quota: MyQuotaResponse = {
      tier: 'starter',
      counters: [
        { counter_type: 'daily_messages', current: 12, limit: 50, warning: false, burst_zone: false, resets_at: '2026-09-26T00:00:00Z' },
      ],
    }
    const byUrl: Record<string, unknown> = {
      '/api/billing/invoices': { invoices: [{ id: 'in_1', amount_paid: 1900, currency: 'usd' }] },
      '/api/billing/plans': plans,
      '/api/users/me/quota': quota,
    }
    const { stub, api } = axiosStub({ get: (url) => Promise.resolve({ data: byUrl[url] }) })

    const invoices = await api.listInvoices()
    const catalogue = await api.getPlans()
    const snapshot = await api.getMyQuota()

    expect(stub.get.mock.calls.map(([url]) => url)).toEqual([
      '/api/billing/invoices',
      '/api/billing/plans',
      '/api/users/me/quota',
    ])
    expect(invoices.invoices[0].amount_paid).toBe(1900)
    expect(catalogue.plans[0].daily_tokens).toBe(500_000)
    expect(snapshot.counters[0].current).toBe(12)
  })
})
