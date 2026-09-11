// ABOUTME: Expo configuration for Dravr mobile app — identity, native modules and the OTA update channel
// ABOUTME: Uses Expo Go for development; native builds only needed for speech recognition testing

module.exports = {
  name: 'Dravr',
  slug: 'dravr-app',
  version: '1.0.0',
  // Over-the-air updates. Without these, everything the JS bundle decides is
  // frozen into each binary for the life of that install — and one of those
  // decisions is EXPO_PUBLIC_API_URL, which eas.json still points at a raw
  // Cloud Run hostname. Recreating that service changes the hostname, and
  // every phone already carrying the old one would have talked to nothing,
  // with the only remedy an App Store submission and a forced upgrade. An
  // update channel is what turns that from a recall into a publish.
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
  splash: {
    image: './assets/splash-icon.png',
    resizeMode: 'contain',
    backgroundColor: '#f9f9f6', // surface — DESIGN.md §2
    // userInterfaceStyle is 'automatic', so a dark-mode launch flashed this
    // light surface before the app painted its own dark canvas. The dark
    // variant is the tuned surface from index.css, not black.
    dark: {
      image: './assets/splash-icon.png',
      resizeMode: 'contain',
      backgroundColor: '#11130f',
    },
  },
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
    },
  },
  android: {
    adaptiveIcon: {
      foregroundImage: './assets/adaptive-icon.png',
      backgroundColor: '#f9f9f6', // surface — DESIGN.md §2
    },
    // Same reasoning as splash.dark above: an Android dark-mode launch got the
    // light surface first.
    splash: {
      image: './assets/splash-icon.png',
      resizeMode: 'contain',
      backgroundColor: '#f9f9f6',
      dark: {
        image: './assets/splash-icon.png',
        resizeMode: 'contain',
        backgroundColor: '#11130f',
      },
    },
    edgeToEdgeEnabled: true,
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
      'expo-build-properties',
      {
        android: {
          minSdkVersion: 24,
          // 36 is what androidx.core 1.17 demands, and @maplibre/maplibre-react-native
          // pulls that in. Compiling against 35 fails checkReleaseAarMetadata before
          // a single source file is built; the EAS log names the requirement.
          compileSdkVersion: 36,
          targetSdkVersion: 35,
          enableProguardInReleaseBuilds: true,
          enableShrinkResourcesInReleaseBuilds: true,
        },
        ios: {
          useFrameworks: 'static',
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
