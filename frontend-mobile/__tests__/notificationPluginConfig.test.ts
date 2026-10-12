// ABOUTME: Pins the expo-notifications entry of app.config.js: the Android status-bar icon it names and its tint
// ABOUTME: A missing or mis-sized icon fails the native prebuild, and a drifted tint stops matching the design system

import fs from 'fs';
import path from 'path';
import { BOREAL_LIGHT } from '@pierre/shared-constants';

const projectRoot = path.resolve(__dirname, '..');

// The icon is cut into the five Android densities from a 96px (xxxhdpi) source.
const ICON_PX = 96;
const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const PNG_COLOR_TYPE_RGBA = 6;

type PluginEntry = string | [string, Record<string, unknown>];

function notificationsPluginOptions(): Record<string, unknown> {
  // app.config.js is CommonJS and read by Expo's tooling, so jest loads it the same way.
  const { plugins } = require('../app.config.js') as { plugins: PluginEntry[] };
  const entry = plugins.find((plugin) => (Array.isArray(plugin) ? plugin[0] : plugin) === 'expo-notifications');
  if (!Array.isArray(entry)) {
    throw new Error('app.config.js lists expo-notifications without options');
  }
  return entry[1];
}

describe('the expo-notifications plugin entry in app.config.js', () => {
  it('names an Android icon that exists in the project', () => {
    const { icon } = notificationsPluginOptions();

    expect(typeof icon).toBe('string');
    expect(fs.existsSync(path.resolve(projectRoot, icon as string))).toBe(true);
  });

  it('names a 96px PNG with an alpha channel, the shape Android draws a status-bar icon from', () => {
    const { icon } = notificationsPluginOptions();
    // The image is the data under test: the native prebuild reads this file,
    // and no runtime seam exposes what it found there.
    const png = fs.readFileSync(path.resolve(projectRoot, icon as string));

    expect(Array.from(png.subarray(0, 8))).toEqual(PNG_SIGNATURE);
    // IHDR: width and height are the first two big-endian words after the chunk
    // header; the colour type is the byte after the bit depth.
    expect(png.readUInt32BE(16)).toBe(ICON_PX);
    expect(png.readUInt32BE(20)).toBe(ICON_PX);
    expect(png[25]).toBe(PNG_COLOR_TYPE_RGBA);
  });

  it("tints the icon the design system's primary", () => {
    expect(notificationsPluginOptions().color).toBe(BOREAL_LIGHT.primary);
  });
});
