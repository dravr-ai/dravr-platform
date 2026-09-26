// ABOUTME: Fingerprints the real project to prove fingerprint.config.js is read and does what it says
// ABOUTME: package.json scripts and eas.json stay out of the runtime version; the native inputs stay in

import path from 'path';
import { createFingerprintAsync, type Fingerprint, type FingerprintSource } from 'expo/fingerprint';

// The loader drops unknown config keys and unknown skip names without a word,
// so a typo in fingerprint.config.js would leave the runtime version exactly as
// sensitive as before and nothing else would notice. Only the computed sources
// show whether the config took effect.
const projectRoot = path.resolve(__dirname, '..');

function sourceKey(source: FingerprintSource): string {
  return source.type === 'contents' ? source.id : source.filePath;
}

function hashedSources(fingerprint: Fingerprint): FingerprintSource[] {
  return fingerprint.sources.filter((source) => source.hash != null);
}

describe('fingerprint.config.js', () => {
  let fingerprint: Fingerprint;

  beforeAll(async () => {
    fingerprint = await createFingerprintAsync(projectRoot, { platforms: ['ios', 'android'], silent: true });
  }, 60_000);

  it('leaves the package.json scripts block out of the runtime version', () => {
    const keys = hashedSources(fingerprint).map(sourceKey);

    expect(keys).not.toContain('packageJson:scripts');
  });

  it('leaves eas.json out of the runtime version', () => {
    const keys = hashedSources(fingerprint).map(sourceKey);

    expect(keys).not.toContain('eas.json');
  });

  it('still hashes the expo config and both autolinking graphs', () => {
    const keys = hashedSources(fingerprint).map(sourceKey);

    expect(keys).toEqual(
      expect.arrayContaining([
        'expoConfig',
        'expoAutolinkingConfig:ios',
        'expoAutolinkingConfig:android',
        'rncoreAutolinkingConfig:ios',
        'rncoreAutolinkingConfig:android',
      ]),
    );
  });

  it('still hashes the native code of installed modules', () => {
    const nativeModuleDirs = hashedSources(fingerprint).filter(
      (source) =>
        source.type === 'dir' &&
        source.reasons.some((reason) => reason === 'expoAutolinkingIos' || reason === 'rncoreAutolinkingIos'),
    );

    expect(nativeModuleDirs.length).toBeGreaterThan(0);
  });
});
