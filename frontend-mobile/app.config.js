// ABOUTME: Expo configuration for Dravr mobile app
// ABOUTME: Uses Expo Go for development; native builds only needed for speech recognition testing

module.exports = {
  name: 'Dravr',
  slug: 'pierre-mobile',
  version: '1.0.0',
  runtimeVersion: {
    policy: 'sdkVersion',
  },
  orientation: 'portrait',
  icon: './assets/icon.png',
  userInterfaceStyle: 'dark',
  scheme: 'pierre',
  splash: {
    image: './assets/splash-icon.png',
    resizeMode: 'contain',
    backgroundColor: '#0f0f0f',
  },
  ios: {
    supportsTablet: true,
    bundleIdentifier: 'com.pierre.fitness',
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
      backgroundColor: '#0f0f0f',
    },
    edgeToEdgeEnabled: true,
    package: 'com.pierre.fitness',
    permissions: ['android.permission.RECORD_AUDIO'],
  },
  web: {
    favicon: './assets/favicon.png',
  },
  owner: 'dravr',
  extra: {
    eas: {
      projectId: '6c63325e-a0da-49cf-b4bd-e6aeae6fd981',
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
