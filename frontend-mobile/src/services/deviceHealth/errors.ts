// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The device-health lane's internal failures, as codes: the screen shows its own translated line
// ABOUTME: Never rendered; a code names the broken invariant for logs and tests

/** What went wrong inside the lane. */
export type DeviceHealthFault = 'not_a_page' | 'no_store' | 'window_too_large';

/** A failure inside the device-health lane, carried by its code. */
export class DeviceHealthError extends Error {
  readonly code: DeviceHealthFault;

  constructor(code: DeviceHealthFault) {
    super(`device_health.${code}`);
    this.name = 'DeviceHealthError';
    this.code = code;
  }
}
