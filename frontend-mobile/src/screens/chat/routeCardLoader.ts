// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Loads the MapLibre route card on demand, and rejects — never resolves empty — when the runtime cannot draw it
// ABOUTME: Probes MapLibre's native side before requiring the card, so Expo Go gets a sentence instead of a fatal redbox

import type React from 'react';
import { TurboModuleRegistry } from 'react-native';
import type { RouteView as RouteBlock } from '@pierre/scene-types';
import type { DistanceUnit } from '@pierre/chat-utils';

/** What `React.lazy` needs back: the module's default export, the card itself. */
export interface RouteCardModule {
  default: React.ComponentType<{ route: RouteBlock; markerUnit?: DistanceUnit | null }>;
}

/**
 * The native module the map view is backed by. MapLibre registers every one
 * of its modules under an `MLRN` name and asks for them with
 * `TurboModuleRegistry.getEnforcing` the moment the package is evaluated;
 * the map view's is the one a route card cannot be drawn without.
 */
export const MAP_NATIVE_MODULE = 'MLRNMapViewModule';

/**
 * Why a route card could not load: the native module is not linked into this
 * runtime, or the card's module came back without a component. A code rather
 * than a sentence — it is read in logs and tests, never shown to an athlete,
 * who sees the translated "map could not be loaded" instead.
 */
export type RouteCardUnavailableReason = 'native_module_not_linked' | 'card_module_empty';

/** The error every failed load rejects with, carrying its reason code. */
export class RouteCardUnavailableError extends Error {
  public readonly reason: RouteCardUnavailableReason;

  public constructor(reason: RouteCardUnavailableReason) {
    super(reason);
    this.name = RouteCardUnavailableError.name;
    this.reason = reason;
  }
}

/**
 * Whether MapLibre's native side is linked into this runtime.
 *
 * `get`, unlike the `getEnforcing` the package itself calls, answers `null`
 * for a module that is not there instead of throwing.
 */
export function mapLibreLinked(): boolean {
  return TurboModuleRegistry.get(MAP_NATIVE_MODULE) != null;
}

/**
 * Load the route card, or reject with why it cannot be drawn.
 *
 * The probe comes first because a throw is not what a missing native module
 * produces under Metro. A `require` made outside a module factory — this one
 * runs in a promise callback — goes through Metro's guard, which hands the
 * throw to `ErrorUtils.reportFatalError` and returns `undefined`: the athlete
 * gets a fatal error screen over the tab, and `React.lazy` gets an empty
 * module and throws a `TypeError` of its own. So the card is required only
 * where its native side is known to be there, and whatever the require
 * yields is checked before `React.lazy` sees it: a throw, `undefined`, or a
 * module with no default export each reject, which is what the route
 * boundary catches.
 */
export function loadRouteCard(
  linked: () => boolean,
  requireCard: () => unknown,
): Promise<RouteCardModule> {
  return Promise.resolve().then(() => {
    if (!linked()) {
      throw new RouteCardUnavailableError('native_module_not_linked');
    }
    const loaded: unknown = requireCard();
    if (
      loaded === null ||
      typeof loaded !== 'object' ||
      !('default' in loaded) ||
      (loaded as { default: unknown }).default == null
    ) {
      throw new RouteCardUnavailableError('card_module_empty');
    }
    return loaded as RouteCardModule;
  });
}
