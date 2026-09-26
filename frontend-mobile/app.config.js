// ABOUTME: Expo configuration for Dravr mobile app — identity, native modules and the OTA update channel
// ABOUTME: Uses Expo Go for development; native builds only needed for speech recognition testing

module.exports = {
  name: 'Dravr',
  slug: 'dravr-app',
  version: '1.0.0',
  // Over-the-air updates. Without these, everything the JS bundle decides is
  // frozen into each binary for the life of that install — and one of those
  // decisions is EXPO_PUBLIC_API_URL, https://app.dravr.ai, which the EAS
  // `production` and `preview` environments hold (not eas.json, so a build and
  // an `eas update --environment` inline the same value). If that host ever
  // moves, every phone already carrying the old one would talk to nothing,
  // with the only remedy an App Store submission and a forced upgrade. An
  // update channel is what turns that from a recall into a publish, and
  // .github/workflows/mobile-ota.yml is the one way to publish to it.
  updates: {
    url: 'https://u.expo.dev/74a36e57-41ac-4c07-95bc-89a1cde64bc7',
  },
  // `fingerprint`, not `sdkVersion`. The runtime version decides which builds
  // an update is allowed to land on, and this app carries real native code —
  // MapLibre, speech recognition, Google sign-in, and expo-updates itself. On
  // the `sdkVersion` policy every SDK 55 build accepts every SDK 55 update, so
  // a JS bundle calling a native module the installed binary does not have
  // would be delivered and crash on launch: a worse outage than the one OTA is
  // here to prevent. A fingerprint is a hash of the native project, so a
  // JS-only fix ships over the air and anything that moves native code is
  // withheld from old binaries and waits for a store build, which is the
  // distinction that makes shipping an update safe.
  runtimeVersion: {
    policy: 'fingerprint',
  },
  // 'default' rather than 'portrait': supportsTablet is true below, and Apple
  // expects a tablet-capable app to rotate and to support Split View. A phone
  // still opens portrait because that is how it is held; it is no longer
  // FORBIDDEN from rotating.
  orientation: 'default',
  icon: './assets/icon.png',
  // Boreal Editorial is a light-first system; follow the OS so mobile falls
  // back to the tuned dark variant on OLED night use. See ThemeProvider in
  // app/_layout.tsx for the runtime switch.
  userInterfaceStyle: 'automatic',
  scheme: 'dravr',
  ios: {
    supportsTablet: true,
    // Apple freezes the bundle id once an app record ships, so this is set
    // before the first TestFlight submission. Deep links are unaffected: they
    // come from `scheme` above, not from the bundle id.
    bundleIdentifier: 'ai.dravr.app',
    infoPlist: {
      ITSAppUsesNonExemptEncryption: false,
      NSMicrophoneUsageDescription:
        'Dravr needs microphone access to capture your voice for speech-to-text transcription.',
      NSSpeechRecognitionUsageDescription:
        'Dravr uses speech recognition to transcribe your voice messages into text queries.',
      NSPhotoLibraryUsageDescription:
        'Dravr needs access to your photo library to allow selecting images for profile customization and activity attachments.',
      // Dravr never calls a location API — RouteView.tsx frames a route's own
      // bounds, never the device's position (see its Camera usage). This key
      // exists only because @maplibre/maplibre-react-native links CoreLocation
      // for its (unused) live-position display capability; Apple's static
      // binary scan requires the purpose string regardless (ITMS-90683).
      NSLocationWhenInUseUsageDescription:
        'Dravr may use your location to show your position on a training route map.',
    },
  },
  android: {
    adaptiveIcon: {
      foregroundImage: './assets/adaptive-icon.png',
      backgroundColor: '#f9f9f6', // surface — DESIGN.md §2
    },
    package: 'ai.dravr.app',
    permissions: ['android.permission.RECORD_AUDIO'],
  },
  web: {
    favicon: './assets/favicon.png',
  },
  owner: 'dravr',
  extra: {
    eas: {
      projectId: '74a36e57-41ac-4c07-95bc-89a1cde64bc7',
    },
  },
  plugins: [
    'expo-router',
    [
      // The launch screen on both platforms. Prebuild reads it from this
      // plugin entry only: a root `splash` or `android.splash` key reaches no
      // native file, and the build falls back to a white screen with no logo.
      'expo-splash-screen',
      {
        image: './assets/splash-icon.png',
        imageWidth: 200,
        resizeMode: 'contain',
        backgroundColor: '#f9f9f6', // surface — DESIGN.md §2
        // userInterfaceStyle is 'automatic', so a dark-mode launch flashed the
        // light surface before the app painted its own dark canvas. The dark
        // variant is the tuned surface from index.css, not black.
        dark: {
          image: './assets/splash-icon.png',
          backgroundColor: '#11130f',
        },
      },
    ],
    [
      'expo-build-properties',
      {
        android: {
          minSdkVersion: 24,
          // 36 is what androidx.core 1.17 demands, and @maplibre/maplibre-react-native
          // pulls that in. Compiling against 35 fails checkReleaseAarMetadata before
          // a single source file is built; the EAS log names the requirement.
          compileSdkVersion: 36,
          // No targetSdkVersion: the target comes from React Native's version
          // catalog (targetSdk 36 in react-native/gradle/libs.versions.toml),
          // which the Expo root-project Gradle plugin applies to the app.
          // Google Play takes new apps and updates only at target API 36 or
          // later from 2026-08-31, which opts the app into Android 16's
          // runtime behaviour: edge-to-edge with no opt-out, predictive back,
          // and orientation and resizability locks ignored on large screens.
          enableProguardInReleaseBuilds: true,
          enableShrinkResourcesInReleaseBuilds: true,
        },
        ios: {
          useFrameworks: 'static',
          // Apps linked against the iOS 27 SDK must adopt the UIKit scene
          // life cycle or UIKit traps at launch. This makes prebuild emit a
          // UIApplicationSceneManifest pointing at Expo's scene delegate and
          // moves React Native startup out of AppDelegate, which keeps
          // forwarding URL and lifecycle events to Expo's app-delegate
          // subscribers, Google Sign-In's among them.
          enableSceneSupport: true,
        },
      },
    ],
    [
      // Registers the reversed iOS OAuth client id as a URL scheme so Google can
      // hand the sign-in result back to the app. Bound to bundle id ai.dravr.app.
      '@react-native-google-signin/google-signin',
      {
        iosUrlScheme:
          'com.googleusercontent.apps.629001562818-fqu15igkvlj6jt1ftusktilq7rpg5imn',
      },
    ],
    [
      // The route card in a coach reply draws a real MapLibre map, which is a
      // native view: this plugin is what puts the MapLibre SDK into the build.
      // It pins the iOS distribution through the Podfile and the Android
      // artifact through gradle.properties, so a dev-client rebuild is what
      // makes a route block renderable at all — Expo Go cannot carry it.
      '@maplibre/maplibre-react-native',
      {
        android: {
          // F-Droid-compatible, and the only location engine this app needs:
          // a route is history the platform hydrates, never the phone's own
          // position, so nothing here asks for a fix.
          locationEngine: 'default',
        },
      },
    ],
    [
      'expo-speech-recognition',
      {
        microphonePermission:
          'Dravr needs microphone access to capture your voice for speech-to-text transcription.',
        speechRecognitionPermission:
          'Dravr uses speech recognition to transcribe your voice messages into text queries.',
        androidSpeechServicePackages: ['com.google.android.googlequicksearchbox'],
      },
    ],
  ],
};
