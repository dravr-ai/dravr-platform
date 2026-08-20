// ABOUTME: Expo configuration for Dravr mobile app
// ABOUTME: Uses Expo Go for development; native builds only needed for speech recognition testing

module.exports = {
  name: 'Dravr',
  slug: 'dravr-app',
  version: '1.0.0',
  runtimeVersion: {
    policy: 'sdkVersion',
  },
  orientation: 'portrait',
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
    },
  },
  android: {
    adaptiveIcon: {
      foregroundImage: './assets/adaptive-icon.png',
      backgroundColor: '#f9f9f6', // surface — DESIGN.md §2
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
          compileSdkVersion: 35,
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
