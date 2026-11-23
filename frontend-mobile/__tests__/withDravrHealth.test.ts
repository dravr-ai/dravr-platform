// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the withDravrHealth config plugin — read-only HealthKit and Health Connect, and the privacy policy the rationale opens
// ABOUTME: Runs the plugin's mods over in-memory native files, and holds app.config.js's policy address to the shared LEGAL_URL

import { LEGAL_URL } from '@pierre/shared-constants';

type PluginEntry = string | [string, Record<string, unknown>];
type Mod = (config: Record<string, unknown>) => Promise<{ modResults: unknown }>;
type ManifestNode = { $: Record<string, string> } & Record<string, unknown>;
interface Manifest {
  manifest: ManifestNode & { application: ManifestNode[]; 'uses-permission'?: ManifestNode[]; queries?: ManifestNode[] };
}

// A CommonJS config plugin, loaded as Expo's tooling loads it.
const withDravrHealth = require('../plugins/withDravrHealth') as (
  config: Record<string, unknown>,
  options?: Record<string, unknown>,
) => { mods: Record<string, Record<string, Mod>> };

// The pinned catalogue copy the purpose string is read from (packages/i18n, written by the contremaitre bump lane).
const enCatalogue = require('../../packages/i18n/src/locales/en/translation.json') as {
  providers: { deviceHealth: { permissionCopy: string } };
};

function pluginOptions(): Record<string, unknown> {
  // app.config.js is CommonJS and read by Expo's tooling, so jest loads it the same way.
  const { plugins } = require('../app.config.js') as { plugins: PluginEntry[] };
  const entry = plugins.find((plugin) => (Array.isArray(plugin) ? plugin[0] : plugin) === './plugins/withDravrHealth');
  if (!Array.isArray(entry)) throw new Error('app.config.js lists withDravrHealth without options');
  return entry[1];
}

async function run(platform: 'ios' | 'android', name: string, modResults: unknown): Promise<unknown> {
  const config = withDravrHealth(
    { name: 'Dravr', slug: 'dravr-app', android: { package: 'ai.dravr.app' }, ios: { bundleIdentifier: 'ai.dravr.app' } },
    { privacyPolicyUrl: LEGAL_URL },
  );
  const mod = config.mods[platform][name];
  const out = await mod({ ...config, modResults, modRequest: { platform, modName: name, introspect: true } });
  return out.modResults;
}

function emptyManifest(): Manifest {
  return {
    manifest: {
      $: {},
      application: [{ $: { 'android:name': '.MainApplication' }, activity: [{ $: { 'android:name': '.MainActivity' } }] }],
    },
  };
}

describe('the withDravrHealth plugin', () => {
  it('is given the privacy policy both clients link', () => {
    expect(pluginOptions().privacyPolicyUrl).toBe(LEGAL_URL);
  });

  it('refuses to run without a policy address', () => {
    expect(() => withDravrHealth({ name: 'Dravr', slug: 'dravr-app' })).toThrow('privacyPolicyUrl');
  });

  it('turns HealthKit on with no clinical records', async () => {
    expect(await run('ios', 'entitlements', {})).toEqual({
      'com.apple.developer.healthkit': true,
      'com.apple.developer.healthkit.access': [],
    });
  });

  it('asks HealthKit to read only: a share string, never an update string', async () => {
    const plist = (await run('ios', 'infoPlist', { NSHealthUpdateUsageDescription: 'left over' })) as Record<string, string>;
    expect(plist.NSHealthShareUsageDescription).toBe(enCatalogue.providers.deviceHealth.permissionCopy);
    expect(plist).not.toHaveProperty('NSHealthUpdateUsageDescription');
  });

  it('gives every other app locale its own Health purpose string, iOS-only', () => {
    const { locales, ios } = require('../app.config.js') as {
      locales: Record<string, { ios?: { NSHealthShareUsageDescription?: string } } & Record<string, unknown>>;
      ios: { infoPlist: Record<string, unknown> };
    };
    expect(ios.infoPlist.CFBundleAllowMixedLocalizations).toBe(true);
    const en = enCatalogue.providers.deviceHealth.permissionCopy;
    expect(Object.keys(locales).sort()).toEqual(['de', 'es', 'fr', 'pt']);
    for (const [locale, entry] of Object.entries(locales)) {
      const purpose = entry.ios?.NSHealthShareUsageDescription;
      expect({ locale, purpose: typeof purpose === 'string' && purpose.trim().length > 0 }).toEqual({ locale, purpose: true });
      expect({ locale, differsFromEn: purpose !== en }).toEqual({ locale, differsFromEn: true });
      // Nothing at the top level: Android would write it into strings.xml.
      expect(Object.keys(entry)).toEqual(['ios']);
    }
  });

  it('refuses a catalogue copy InfoPlist.strings cannot carry, or one with no purpose string', () => {
    // healthPurpose.js reads the catalogue by absolute path, so a doMock of that path stands in for a bad copy.
    const frCatalogue = require.resolve('../../packages/i18n/src/locales/fr/translation.json');
    // The catalogue is required when the purpose is read, so the read runs inside the isolated registry.
    const purposeWith = (catalogue: unknown) => () => {
      try {
        jest.isolateModules(() => {
          jest.doMock(frCatalogue, () => catalogue);
          const { healthSharePurpose } = require('../plugins/healthPurpose') as { healthSharePurpose: (locale: string) => string };
          healthSharePurpose('fr');
        });
      } finally {
        jest.dontMock(frCatalogue);
      }
    };
    expect(purposeWith({ providers: { deviceHealth: { permissionCopy: 'Dravr lit "Santé".' } } })).toThrow('quote, backslash or line break');
    expect(purposeWith({ providers: { deviceHealth: { permissionCopy: 'Une ligne\nDeux' } } })).toThrow('quote, backslash or line break');
    expect(purposeWith({ providers: { deviceHealth: { permissionCopy: '  ' } } })).toThrow('fr/translation.json has no providers.deviceHealth.permissionCopy');
    expect(purposeWith({ providers: {} })).toThrow('fr/translation.json has no providers.deviceHealth.permissionCopy');
  });

  it('asks Health Connect for read permissions only, and names the policy its rationale opens', async () => {
    const { manifest } = (await run('android', 'manifest', emptyManifest())) as Manifest;
    const permissions = (manifest['uses-permission'] ?? []).map((node) => node.$['android:name']);
    expect(permissions).toEqual(expect.arrayContaining(['android.permission.health.READ_EXERCISE', 'android.permission.health.READ_SLEEP']));
    expect(permissions.filter((name) => name.startsWith('android.permission.health.'))).toHaveLength(9);
    expect(permissions.some((name) => name.includes('WRITE'))).toBe(false);

    const queries = (manifest.queries?.[0]?.package ?? []) as ManifestNode[];
    expect(queries.map((node) => node.$['android:name'])).toEqual(['com.google.android.apps.healthdata']);

    const metaData = (manifest.application[0]['meta-data'] ?? []) as ManifestNode[];
    expect(metaData.map((node) => node.$)).toContainEqual({
      'android:name': 'ai.dravr.health.PRIVACY_POLICY_URL',
      'android:value': LEGAL_URL,
    });
  });
});
