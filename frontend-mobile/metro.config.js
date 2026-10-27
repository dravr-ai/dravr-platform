// ABOUTME: Metro bundler configuration for Expo with NativeWind v4 and npm workspaces
// ABOUTME: Enables CSS support for Tailwind-style classes and resolves workspace packages

const { getDefaultConfig } = require('expo/metro-config');
const path = require('path');

// Load NativeWind's metro plugin with fallback for CI environments
// In some CI configurations (like xcodebuild's bundle phase), module resolution may fail
let withNativeWind;
try {
  withNativeWind = require('nativewind/metro').withNativeWind;
} catch (loadError) {
  // Fallback for environments where NativeWind can't be loaded
  // This allows the build to proceed; styles will use fallback rendering
  withNativeWind = (config, nativeWindOptions) => config;
  console.warn('NativeWind metro plugin not available:', loadError.message);
}

// Get the project root (monorepo root)
const projectRoot = __dirname;
const monorepoRoot = path.resolve(projectRoot, '..');

// Expo Router: set app directory for monorepo where expo-router is hoisted to root node_modules
process.env.EXPO_ROUTER_APP_ROOT = path.resolve(projectRoot, 'app');

const config = getDefaultConfig(projectRoot);

// Watch all files in the monorepo
config.watchFolders = [...(config.watchFolders || []), monorepoRoot];

// Let Metro know where to resolve packages from
config.resolver.nodeModulesPaths = [
  path.resolve(projectRoot, 'node_modules'),
  path.resolve(monorepoRoot, 'node_modules'),
];

// Resolve packages from the monorepo root
config.resolver.disableHierarchicalLookup = false;

// Ensure @pierre/* packages resolve correctly
config.resolver.extraNodeModules = {
  '@pierre/shared-types': path.resolve(monorepoRoot, 'packages/shared-types'),
  '@pierre/shared-constants': path.resolve(monorepoRoot, 'packages/shared-constants'),
  '@pierre/api-client': path.resolve(monorepoRoot, 'packages/api-client'),
  '@pierre/i18n': path.resolve(monorepoRoot, 'packages/i18n'),
};

// Use resolveRequest to redirect @pierre/api-client to mobile-specific entry
// This avoids importing web.ts which uses import.meta (not supported by Hermes)
config.resolver.resolveRequest = (context, moduleName, platform) => {
  if (moduleName === '@pierre/api-client') {
    return {
      filePath: path.resolve(monorepoRoot, 'packages/api-client/src/index-mobile.ts'),
      type: 'sourceFile',
    };
  }
  // @pierre/i18n keeps its AsyncStorage-backed hook behind a `./native`
  // subpath so a web bundle never pulls React Native in. extraNodeModules maps
  // only the bare package name, so the subpath is resolved to its file here —
  // the same treatment @pierre/api-client's mobile entry gets.
  if (moduleName === '@pierre/i18n/native') {
    return {
      filePath: path.resolve(monorepoRoot, 'packages/i18n/src/native.ts'),
      type: 'sourceFile',
    };
  }
  if (moduleName === '@pierre/i18n') {
    return {
      filePath: path.resolve(monorepoRoot, 'packages/i18n/src/index.ts'),
      type: 'sourceFile',
    };
  }
  // Force single React instance across all workspace packages
  // Bun workspace hoisting can place react in both local and root node_modules,
  // causing "Cannot read property 'useState' of null" hooks crash at runtime
  if (moduleName === 'react' || moduleName.startsWith('react/')) {
    return {
      filePath: require.resolve(moduleName, { paths: [projectRoot] }),
      type: 'sourceFile',
    };
  }
  // Workspace TS packages re-export with ESM-style `.js` extensions on relative
  // paths (e.g. shared-types/src/index.ts `export { parseWorkoutPlan } from
  // './workout-plan.js'`). tsc (moduleResolution: bundler), Vite and Jest map that
  // `.js` specifier back to the `.ts` source, but Metro treats `.js` as a literal
  // on-disk extension and fails to resolve it. Only value re-exports reach Metro
  // (type-only re-exports are erased by the TS transform), so a single
  // `export { fn } from './x.js'` breaks the whole native bundle. Resolve the
  // `.js` literally when it exists; otherwise strip it so Metro's sourceExts find
  // the `.ts` source.
  if (moduleName.startsWith('.') && moduleName.endsWith('.js')) {
    try {
      return context.resolveRequest(context, moduleName, platform);
    } catch (jsResolutionError) {
      return context.resolveRequest(context, moduleName.slice(0, -3), platform);
    }
  }
  return context.resolveRequest(context, moduleName, platform);
};

// The E2E seeded-session route (app/(auth)/e2e/, carnet#1201) signs the app in
// with a session the Maestro control process holds, so it belongs only in the
// bundle the mobile E2E lanes serve to Expo Go. Metro keeps the directory only
// when all three hold:
//   - EXPO_PUBLIC_E2E_SEEDED_SESSIONS=1, which only the E2E lanes, the local
//     E2E script and the dev stack's --maestro run set;
//   - NODE_ENV is 'development', which `expo start` sets and every export
//     (`expo export`, `expo export:embed`, so every store, TestFlight and EAS
//     update bundle) replaces with 'production';
//   - EXPO_PUBLIC_API_URL names a local server (the simulator's loopback or the
//     Android emulator's 10.0.2.2), so a bundle pointed at a real server never
//     carries a link that could sign someone into another account.
// A file on the block list is not in Metro's file map, so Expo Router's route
// context never sees it: the link has no route to land on, rather than one that
// refuses. The route checks the same at runtime (`__DEV__`, the flag, the API
// host), and scripts/ci/check-seeded-session-route-not-bundled.sh exports a
// production bundle and fails if the route is in it.
const SEEDED_SESSION_ROUTES = /(^|[\\/])app[\\/]\(auth\)[\\/]e2e([\\/]|$)/;
const LOCAL_API_URL = /^https?:\/\/(127\.0\.0\.1|localhost|10\.0\.2\.2)(:\d+)?(\/|$)/i;
const seededSessionsEnabled =
  process.env.EXPO_PUBLIC_E2E_SEEDED_SESSIONS === '1' &&
  process.env.NODE_ENV === 'development' &&
  LOCAL_API_URL.test(process.env.EXPO_PUBLIC_API_URL || '');
if (!seededSessionsEnabled) {
  config.resolver.blockList = [].concat(config.resolver.blockList || [], SEEDED_SESSION_ROUTES);
}

// Use port 8082 to avoid conflict with Pierre MCP server on 8081
config.server = {
  ...config.server,
  port: 8082,
};

module.exports = withNativeWind(config, { input: './global.css' });
