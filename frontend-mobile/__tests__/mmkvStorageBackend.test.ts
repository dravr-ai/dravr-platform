// ABOUTME: Pins where the query cache lives — an MMKV instance from createMMKV everywhere but Expo Go
// ABOUTME: Red if the persister goes back to a v3-style constructor that throws into an in-memory fallback

import type { PersistedClient } from '@tanstack/react-query-persist-client';

type Environment = 'bare' | 'standalone' | 'storeClient';

/** A stand-in MMKV instance that records what reaches it. */
function recordingInstance() {
  const values = new Map<string, string>();
  return {
    values,
    getString: jest.fn((key: string) => values.get(key)),
    set: jest.fn((key: string, value: string) => {
      values.set(key, value);
    }),
    remove: jest.fn((key: string) => values.delete(key)),
    clearAll: jest.fn(() => values.clear()),
  };
}

/**
 * Load mmkvStorage fresh under one execution environment, with createMMKV
 * spied, and hand back the module and what createMMKV was asked for.
 */
function loadUnder(environment: Environment) {
  const instance = recordingInstance();
  const createMMKV = jest.fn(() => instance);
  let storage: typeof import('../src/utils/mmkvStorage') | undefined;
  jest.isolateModules(() => {
    jest.doMock('react-native-mmkv', () => ({ createMMKV }));
    jest.doMock('expo-constants', () => ({
      __esModule: true,
      default: { executionEnvironment: environment },
      ExecutionEnvironment: { Bare: 'bare', Standalone: 'standalone', StoreClient: 'storeClient' },
    }));
    storage = require('../src/utils/mmkvStorage');
  });
  if (storage === undefined) throw new Error('mmkvStorage did not load');
  return { storage, createMMKV, instance };
}

const CLIENT: PersistedClient = {
  timestamp: 1_790_000_000_000,
  buster: 'v1',
  clientState: { queries: [], mutations: [] },
};

afterEach(() => {
  jest.dontMock('react-native-mmkv');
  jest.dontMock('expo-constants');
});

describe.each<Environment>(['standalone', 'bare'])('a native build (%s)', (environment) => {
  it('keeps the cache in the pierre-query-cache MMKV instance', async () => {
    const { storage, createMMKV, instance } = loadUnder(environment);

    expect(createMMKV).toHaveBeenCalledTimes(1);
    expect(createMMKV).toHaveBeenCalledWith({ id: 'pierre-query-cache' });

    await storage.mmkvPersister.persistClient(CLIENT);
    expect(instance.set).toHaveBeenCalledWith('REACT_QUERY_CACHE', JSON.stringify(CLIENT));
    expect(await storage.mmkvPersister.restoreClient()).toEqual(CLIENT);

    storage.clearQueryCache();
    expect(instance.remove).toHaveBeenCalledWith('REACT_QUERY_CACHE');
    expect(await storage.mmkvPersister.restoreClient()).toBeUndefined();
  });

  it('drops a cache it cannot parse and restores nothing', async () => {
    const { storage, instance } = loadUnder(environment);
    instance.values.set('REACT_QUERY_CACHE', '{not json');

    expect(await storage.mmkvPersister.restoreClient()).toBeUndefined();
    expect(instance.remove).toHaveBeenCalledWith('REACT_QUERY_CACHE');
    expect(instance.values.has('REACT_QUERY_CACHE')).toBe(false);
  });

  it('lets a missing native module fail loudly instead of forgetting the cache', () => {
    const createMMKV = jest.fn(() => {
      throw new Error('NitroModules are not available');
    });
    jest.isolateModules(() => {
      jest.doMock('react-native-mmkv', () => ({ createMMKV }));
      jest.doMock('expo-constants', () => ({
        __esModule: true,
        default: { executionEnvironment: environment },
        ExecutionEnvironment: { Bare: 'bare', Standalone: 'standalone', StoreClient: 'storeClient' },
      }));
      expect(() => require('../src/utils/mmkvStorage')).toThrow('NitroModules are not available');
    });
  });
});

describe('Expo Go', () => {
  it('never asks for the native module and keeps the session cache in memory', async () => {
    const { storage, createMMKV } = loadUnder('storeClient');

    expect(createMMKV).not.toHaveBeenCalled();
    await storage.mmkvPersister.persistClient(CLIENT);
    expect(await storage.mmkvPersister.restoreClient()).toEqual(CLIENT);
    await storage.mmkvPersister.removeClient();
    expect(await storage.mmkvPersister.restoreClient()).toBeUndefined();
  });
});
