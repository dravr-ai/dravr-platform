// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the route card loader — it resolves only a module with a card in it, and rejects everything else
// ABOUTME: Under Metro a failing require yields undefined rather than throwing, so an empty module must reject, not resolve

import { NativeModules } from 'react-native';

import {
  MAP_NATIVE_MODULE,
  RouteCardUnavailableError,
  loadRouteCard,
  mapLibreLinked,
} from '../routeCardLoader';

function Card() {
  return null;
}

describe('loadRouteCard', () => {
  it('resolves the module whose default export is the card', async () => {
    const loaded = await loadRouteCard(
      () => true,
      () => ({ default: Card }),
    );
    expect(loaded.default).toBe(Card);
  });

  it('never requires the card where MapLibre is not linked in', async () => {
    const requireCard = jest.fn(() => ({ default: Card }));
    await expect(loadRouteCard(() => false, requireCard)).rejects.toMatchObject({
      reason: 'native_module_not_linked',
    });
    expect(requireCard).not.toHaveBeenCalled();
  });

  it('rejects when the require throws', async () => {
    await expect(
      loadRouteCard(
        () => true,
        () => {
          throw new Error("TurboModuleRegistry.getEnforcing(...): 'MLRNCameraModule' could not be found");
        },
      ),
    ).rejects.toThrow("'MLRNCameraModule' could not be found");
  });

  it.each([
    ['undefined, as Metro returns after reporting the throw', undefined],
    ['null', null],
    ['a module without a default export', { RouteView: Card }],
    ['a module whose default export is undefined', { default: undefined }],
  ])('rejects when the require yields %s', async (_label, yielded) => {
    const load = loadRouteCard(
      () => true,
      () => yielded,
    );
    await expect(load).rejects.toBeInstanceOf(RouteCardUnavailableError);
    await expect(load).rejects.toMatchObject({ reason: 'card_module_empty' });
  });
});

describe('mapLibreLinked', () => {
  const linked = NativeModules[MAP_NATIVE_MODULE];

  afterEach(() => {
    NativeModules[MAP_NATIVE_MODULE] = linked;
  });

  it('answers from the native module registry without throwing', () => {
    expect(mapLibreLinked()).toBe(true);
    delete NativeModules[MAP_NATIVE_MODULE];
    expect(mapLibreLinked()).toBe(false);
  });
});
