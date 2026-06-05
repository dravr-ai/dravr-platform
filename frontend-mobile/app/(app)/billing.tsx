// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Billing modal route presented over tabs, gated by the BILLING_ENABLED flag
// ABOUTME: Renders BillingScreen when enabled, otherwise redirects away (billing is out of scope)

import { Redirect } from 'expo-router';
import { BILLING_ENABLED } from '../../src/constants/features';
import { BillingScreen } from '../../src/screens/settings/BillingScreen';

export default function BillingRoute() {
  if (!BILLING_ENABLED) {
    return <Redirect href="/(app)/(tabs)/(settings)" />;
  }
  return <BillingScreen />;
}
