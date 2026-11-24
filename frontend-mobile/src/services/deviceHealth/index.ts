// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The device health sync's public face — turn the phone's health store on or off, sync it, delete what it sent
// ABOUTME: The catalogue, sample checks and window arithmetic stay inside the folder

export {
  deleteDeviceHealthData,
  deviceHealthAvailable,
  disableDeviceHealth,
  enableDeviceHealth,
  isDeviceHealthSyncing,
  readDeviceHealthState,
  subscribeDeviceHealth,
  syncDeviceHealth,
  type EnableOutcome,
  type SyncOutcome,
} from './sync';
export { deviceHealthSource } from './catalogue';
export type { DeviceHealthState } from './state';
