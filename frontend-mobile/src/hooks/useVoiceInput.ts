// ABOUTME: Custom hook for speech-to-text voice input functionality
// ABOUTME: Wraps expo-speech-recognition with state management and error handling

import { useState, useEffect, useCallback, useRef } from 'react';
import Constants, { ExecutionEnvironment } from 'expo-constants';
import { getLocales } from 'expo-localization';
import { useAuth } from '../contexts/AuthContext';
import type {
  ExpoSpeechRecognitionErrorCode,
  ExpoSpeechRecognitionErrorEvent,
  ExpoSpeechRecognitionResultEvent,
} from 'expo-speech-recognition';

// Check if running in Expo Go (native module won't be available)
const isExpoGo = Constants.executionEnvironment === ExecutionEnvironment.StoreClient;

// Lazy-load native speech module — top-level import crashes in Expo Go
// because the native module is not bundled in the Expo Go client
let ExpoSpeechRecognitionModule: typeof import('expo-speech-recognition').ExpoSpeechRecognitionModule | null = null;
let useSpeechRecognitionEvent: typeof import('expo-speech-recognition').useSpeechRecognitionEvent | null = null;

if (!isExpoGo) {
  try {
    const speechModule = require('expo-speech-recognition');
    ExpoSpeechRecognitionModule = speechModule.ExpoSpeechRecognitionModule;
    useSpeechRecognitionEvent = speechModule.useSpeechRecognitionEvent;
  } catch {
    // Native module not available — speech recognition will be disabled
  }
}

// No-op hook for when native module isn't available (must always call hooks)
// eslint-disable-next-line @typescript-eslint/no-unused-vars
const noopEventHook = (_event: string, _callback: (...args: never[]) => void): void => {};
const safeUseSpeechEvent = useSpeechRecognitionEvent ?? noopEventHook;

// Voice recognition error types for consumer handling
export type VoiceErrorType =
  | 'permission_denied'
  | 'no_speech'
  | 'network_error'
  | 'timeout'
  | 'not_available'
  | 'unknown';

export interface VoiceError {
  type: VoiceErrorType;
  /**
   * Catalogue key for the detail line the toast shows.
   *
   * A key rather than a sentence: this hook runs outside any component and
   * the wording it used to return was English, which the chat toast rendered
   * verbatim under French chrome (carnet#207).
   */
  messageKey: string;
  /**
   * The platform's own words, when the recognizer failed in a way we have no
   * wording for. Shown in place of the generic key so a device-specific
   * reason is not swallowed.
   */
  detail?: string;
}

interface VoiceInputState {
  isListening: boolean;
  transcript: string;
  partialTranscript: string;
  error: VoiceError | null;
  isAvailable: boolean;
}

interface UseVoiceInputResult extends VoiceInputState {
  startListening: () => Promise<void>;
  stopListening: () => Promise<void>;
  cancelListening: () => Promise<void>;
  clearTranscript: () => void;
  clearError: () => void;
}

// Timeout duration for voice input (30 seconds)
const VOICE_TIMEOUT_MS = 30000;

// Used only when neither the athlete nor the device names a language.
const FALLBACK_RECOGNITION_LOCALE = 'en-US';

// Map expo-speech-recognition error codes to our typed errors
function mapErrorCode(code: ExpoSpeechRecognitionErrorCode, message: string): VoiceError {
  switch (code) {
    case 'not-allowed':
      return { type: 'permission_denied', messageKey: 'voice.micAccessDenied' };
    case 'no-speech':
    case 'speech-timeout':
      return { type: 'no_speech', messageKey: 'voice.noSpeech' };
    case 'network':
      return { type: 'network_error', messageKey: 'voice.networkError' };
    case 'service-not-allowed':
      return { type: 'not_available', messageKey: 'voice.notAvailable' };
    case 'aborted':
      return { type: 'timeout', messageKey: 'voice.cancelled' };
    default:
      return { type: 'unknown', messageKey: 'voice.unknown', detail: message };
  }
}

/**
 * The BCP-47 tag the recognizer listens in.
 *
 * The athlete's stored `users.locale` wins: it is the language every coach
 * turn, notification and messaging reply is already written in, so dictation
 * must listen in it too. That column holds a short code (`fr`), so when the
 * device carries a tag in the same language its regional form (`fr-CA`) is
 * used instead — same language, better pronunciation model. With no stored
 * locale the device's own first tag stands alone.
 */
export function resolveRecognitionLocale(
  storedLocale: string | undefined,
  deviceTags: readonly string[],
): string {
  const languageOf = (tag: string): string => tag.toLowerCase().split('-')[0];
  if (!storedLocale) {
    return deviceTags[0] ?? FALLBACK_RECOGNITION_LOCALE;
  }
  const language = languageOf(storedLocale);
  return deviceTags.find((tag) => languageOf(tag) === language) ?? storedLocale;
}

export function useVoiceInput(): UseVoiceInputResult {
  const { user } = useAuth();
  const [state, setState] = useState<VoiceInputState>({
    isListening: false,
    transcript: '',
    partialTranscript: '',
    error: null,
    isAvailable: !isExpoGo, // Assume available if not in Expo Go; will verify on mount
  });

  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Clear timeout helper
  const clearTimeoutRef = useCallback(() => {
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  }, []);

  // Check availability on mount
  useEffect(() => {
    if (isExpoGo) {
      setState((prev) => ({
        ...prev,
        isAvailable: false,
      }));
      return;
    }

    // Check if speech recognition is available (synchronous call)
    const available = ExpoSpeechRecognitionModule?.isRecognitionAvailable() ?? false;
    setState((prev) => ({
      ...prev,
      isAvailable: available,
    }));
  }, []);

  // Handle speech start event
  safeUseSpeechEvent('start', () => {
    setState((prev) => ({ ...prev, isListening: true, error: null }));
  });

  // Handle speech end event
  safeUseSpeechEvent('end', () => {
    clearTimeoutRef();
    setState((prev) => {
      // Check if we got no transcript at all - that's a "no speech" error
      if (!prev.transcript && !prev.partialTranscript) {
        return {
          ...prev,
          isListening: false,
          error: { type: 'no_speech', messageKey: 'voice.noSpeech' },
        };
      }
      return { ...prev, isListening: false };
    });
  });

  // Handle speech results
  safeUseSpeechEvent('result', (event: ExpoSpeechRecognitionResultEvent) => {
    const results = event.results;
    if (results && results.length > 0) {
      const transcript = results[0].transcript;
      if (event.isFinal) {
        clearTimeoutRef();
        setState((prev) => ({
          ...prev,
          transcript,
          partialTranscript: '',
        }));
      } else {
        setState((prev) => ({ ...prev, partialTranscript: transcript }));
      }
    }
  });

  // Handle errors
  safeUseSpeechEvent('error', (event: ExpoSpeechRecognitionErrorEvent) => {
    clearTimeoutRef();
    const voiceError = mapErrorCode(event.error, event.message);
    setState((prev) => ({
      ...prev,
      isListening: false,
      error: voiceError,
    }));
  });

  const startListening = useCallback(async () => {
    // Check if running in Expo Go
    if (isExpoGo || !state.isAvailable) {
      setState((prev) => ({
        ...prev,
        error: { type: 'not_available', messageKey: 'voice.notAvailableOnDevice' },
      }));
      return;
    }

    try {
      clearTimeoutRef();
      setState((prev) => ({
        ...prev,
        transcript: '',
        partialTranscript: '',
        error: null,
      }));

      // Request permissions first
      const permissionResult = await ExpoSpeechRecognitionModule?.requestPermissionsAsync();
      if (!permissionResult?.granted) {
        setState((prev) => ({
          ...prev,
          error: { type: 'permission_denied', messageKey: 'voice.micPermissionDenied' },
        }));
        return;
      }

      const locale = resolveRecognitionLocale(
        user?.locale,
        getLocales().map((deviceLocale) => deviceLocale.languageTag),
      );

      // Start recognition with options
      ExpoSpeechRecognitionModule?.start({
        lang: locale,
        interimResults: true,
        maxAlternatives: 1,
        continuous: false, // Stop after first utterance
      });

      // Set up timeout to auto-stop after VOICE_TIMEOUT_MS
      timeoutRef.current = setTimeout(() => {
        ExpoSpeechRecognitionModule?.stop();
        setState((prev) => ({
          ...prev,
          isListening: false,
          error: { type: 'timeout', messageKey: 'voice.timedOut' },
        }));
      }, VOICE_TIMEOUT_MS);
    } catch (error) {
      // The platform's own words, matched for a permission refusal. They are
      // the recognizer's, not ours, so they classify the failure and then ride
      // along as `detail` rather than becoming the wording.
      const platformMessage = error instanceof Error ? error.message : '';
      const isPermissionError =
        platformMessage.toLowerCase().includes('permission') ||
        platformMessage.toLowerCase().includes('denied') ||
        platformMessage.toLowerCase().includes('not authorized');
      setState((prev) => ({
        ...prev,
        error: isPermissionError
          ? { type: 'permission_denied', messageKey: 'voice.micPermissionDenied' }
          : {
              type: 'unknown',
              messageKey: 'voice.startFailed',
              detail: platformMessage === '' ? undefined : platformMessage,
            },
      }));
    }
  }, [state.isAvailable, clearTimeoutRef, user?.locale]);

  const stopListening = useCallback(async () => {
    clearTimeoutRef();
    if (isExpoGo) return;
    try {
      ExpoSpeechRecognitionModule?.stop();
    } catch (error) {
      console.error('Failed to stop voice recognition:', error);
    }
  }, [clearTimeoutRef]);

  const cancelListening = useCallback(async () => {
    clearTimeoutRef();
    if (isExpoGo) return;
    try {
      ExpoSpeechRecognitionModule?.abort();
      setState((prev) => ({
        ...prev,
        isListening: false,
        partialTranscript: '',
      }));
    } catch (error) {
      console.error('Failed to cancel voice recognition:', error);
    }
  }, [clearTimeoutRef]);

  const clearTranscript = useCallback(() => {
    setState((prev) => ({
      ...prev,
      transcript: '',
      partialTranscript: '',
    }));
  }, []);

  const clearError = useCallback(() => {
    setState((prev) => ({
      ...prev,
      error: null,
    }));
  }, []);

  return {
    ...state,
    startListening,
    stopListening,
    cancelListening,
    clearTranscript,
    clearError,
  };
}
