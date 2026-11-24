// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Expo config plugin for the dravr-health module — the HealthKit entitlement and read purpose string, the Health Connect read permissions
// ABOUTME: Read-only on both (no update string, no write permission, no clinical records); names the privacy policy Health Connect's rationale opens

const { AndroidConfig, withAndroidManifest, withEntitlementsPlist, withInfoPlist } = require('@expo/config-plugins');
const { BASE_LOCALE, healthSharePurpose } = require('./healthPurpose');

/** One read permission per record type the module reads (DravrHealthModule.kt RECORD_TYPES), plus history past 30 days. */
const HEALTH_CONNECT_READ_PERMISSIONS = [
  'android.permission.health.READ_EXERCISE',
  'android.permission.health.READ_SLEEP',
  'android.permission.health.READ_STEPS',
  'android.permission.health.READ_ACTIVE_CALORIES_BURNED',
  'android.permission.health.READ_DISTANCE',
  'android.permission.health.READ_RESTING_HEART_RATE',
  'android.permission.health.READ_HEART_RATE_VARIABILITY',
  'android.permission.health.READ_WEIGHT',
  'android.permission.health.READ_HEALTH_DATA_HISTORY',
];

const HEALTH_CONNECT_PACKAGE = 'com.google.android.apps.healthdata';

/** Read by the module's PermissionsRationaleActivity (modules/dravr-health/android). */
const PRIVACY_POLICY_URL_KEY = 'ai.dravr.health.PRIVACY_POLICY_URL';

function withHealthKit(config) {
  config = withEntitlementsPlist(config, (mod) => {
    mod.modResults['com.apple.developer.healthkit'] = true;
    // Empty: no health-records (clinical) access, which is a separate review.
    mod.modResults['com.apple.developer.healthkit.access'] = [];
    return mod;
  });
  // The base (English) sentence; app.config.js `locales` gives every other
  // locale its own InfoPlist.strings from the same catalogue key.
  const healthShareUsage = healthSharePurpose(BASE_LOCALE);
  return withInfoPlist(config, (mod) => {
    mod.modResults.NSHealthShareUsageDescription = healthShareUsage;
    delete mod.modResults.NSHealthUpdateUsageDescription;
    return mod;
  });
}

function withHealthConnect(config, privacyPolicyUrl) {
  config = AndroidConfig.Permissions.withPermissions(config, HEALTH_CONNECT_READ_PERMISSIONS);
  return withAndroidManifest(config, (mod) => {
    const manifest = mod.modResults.manifest;

    // Android 11+ package visibility: the app must see Health Connect to bind to it.
    manifest.queries = manifest.queries ?? [{}];
    const queries = manifest.queries[0];
    queries.package = queries.package ?? [];
    if (!queries.package.some((pkg) => pkg.$['android:name'] === HEALTH_CONNECT_PACKAGE)) {
      queries.package.push({ $: { 'android:name': HEALTH_CONNECT_PACKAGE } });
    }

    // The rationale activity and its Android 14 alias ship in the module's own
    // manifest; what they open is named here.
    const application = AndroidConfig.Manifest.getMainApplicationOrThrow(mod.modResults);
    AndroidConfig.Manifest.addMetaDataItemToMainApplication(application, PRIVACY_POLICY_URL_KEY, privacyPolicyUrl);
    return mod;
  });
}

/**
 * HealthKit and Health Connect, read-only, for `modules/dravr-health`.
 * `privacyPolicyUrl` is what Health Connect's permissions rationale opens.
 */
module.exports = function withDravrHealth(config, { privacyPolicyUrl } = {}) {
  if (typeof privacyPolicyUrl !== 'string' || !privacyPolicyUrl.startsWith('https://')) {
    throw new Error('withDravrHealth needs privacyPolicyUrl, the https address Health Connect shows as the rationale');
  }
  return withHealthConnect(withHealthKit(config), privacyPolicyUrl);
};
