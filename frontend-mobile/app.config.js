// ABOUTME: Expo configuration for Dravr mobile app
// ABOUTME: Uses Expo Go for development; native builds only needed for speech recognition testing

module.exports = {
  name: 'Dravr',
  slug: 'dravr-app',
  version: '1.0.0',
  runtimeVersion: {
    policy: 'sdkVersion',
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
      // Registers the reversed iOS OAuth client id as a URL scheme so Google can
      // hand the sign-in result back to the app. Bound to bundle id ai.dravr.app.
      '@react-native-google-signin/google-signin',
      {
        iosUrlScheme:
          'com.googleusercontent.apps.629001562818-fqu15igkvlj6jt1ftusktilq7rpg5imn',
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
