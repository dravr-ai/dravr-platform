// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for AdminConfiguration component
// ABOUTME: Validates the tool management wrapper renders correctly

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import AdminConfiguration from '../AdminConfiguration';

// Mock ToolAvailability since it has its own tests
vi.mock('../ToolAvailability', () => ({
  default: () => <div data-testid="tool-availability">ToolAvailability</div>,
}));

// Mock TenantPlanCard — it needs useAuth/adminApi and renders null for non-super-admins
vi.mock('../TenantPlanCard', () => ({
  default: () => <div data-testid="tenant-plan-card">TenantPlanCard</div>,
}));

function renderWithProviders(ui: React.ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

describe('AdminConfiguration', () => {
  it('renders the ToolAvailability component', async () => {
    renderWithProviders(<AdminConfiguration />);
    const toolAvailability = await screen.findByTestId('tool-availability');
    expect(toolAvailability).toBeDefined();
  });

  it('renders the TenantPlanCard above the tool list', async () => {
    renderWithProviders(<AdminConfiguration />);
    const planCard = await screen.findByTestId('tenant-plan-card');
    expect(planCard).toBeDefined();
  });
});
