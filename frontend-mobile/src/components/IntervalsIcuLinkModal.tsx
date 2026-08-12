// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Intervals.icu link modal for mobile — collects athlete id + API key (non-OAuth)
// ABOUTME: Server validates the HTTP Basic (athlete_id:api_key) pair live before storing

import React, { useState, useEffect } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  Modal,
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
} from 'react-native';
import { Activity, X, CheckCircle2 } from 'lucide-react-native';
import { useThemeColors } from '../constants/theme';
import { oauthApi } from '../services/api';
import { Input } from './ui';

interface IntervalsIcuLinkModalProps {
  visible: boolean;
  onClose: () => void;
  onConnected: () => void;
}

function extractErrorMessage(err: unknown, prefix: string): string {
  const data = (err as { response?: { data?: { message?: string; error?: string } } })?.response
    ?.data;
  const detail = data?.message || data?.error;
  if (detail) return detail;
  return `${prefix}: ${err}`;
}

export function IntervalsIcuLinkModal({ visible, onClose, onConnected }: IntervalsIcuLinkModalProps) {
  const colors = useThemeColors();
  const [athleteId, setAthleteId] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  useEffect(() => {
    if (visible) {
      setAthleteId('');
      setApiKey('');
      setIsLoading(false);
      setError(null);
      setSuccess(null);
    }
  }, [visible]);

  const handleSubmit = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const result = await oauthApi.linkIntervalsIcu({
        athlete_id: athleteId.trim(),
        api_key: apiKey.trim(),
      });
      const name = result.athlete?.name || result.athlete?.id || 'your account';
      setSuccess(`Connected ${name}`);
      setTimeout(onConnected, 1200);
    } catch (err) {
      setError(extractErrorMessage(err, 'Could not link Intervals.icu'));
      setIsLoading(false);
    }
  };

  const canSubmit = athleteId.trim().length > 0 && apiKey.trim().length > 0 && !isLoading;

  return (
    <Modal visible={visible} animationType="slide" transparent onRequestClose={onClose}>
      <KeyboardAvoidingView
        behavior={Platform.OS === 'ios' ? 'padding' : undefined}
        className="flex-1 bg-black/60 justify-end"
      >
        <View
          className="bg-background-primary rounded-t-3xl pt-4 pb-10 px-4"
          onStartShouldSetResponder={() => true}
        >
          <View className="items-center mb-3">
            <View className="w-10 h-1 rounded-full bg-border-default" />
          </View>

          {/* Header */}
          <View className="flex-row items-center justify-between mb-4">
            <View className="flex-row items-center">
              <View
                className="w-9 h-9 rounded-xl items-center justify-center mr-3"
                style={{ backgroundColor: '#1273DE' }}
              >
                <Activity size={18} color="#FFFFFF" />
              </View>
              <View>
                <Text className="text-lg font-semibold text-text-primary">Connect Intervals.icu</Text>
                <Text className="text-xs text-text-tertiary">API key — no OAuth redirect</Text>
              </View>
            </View>
            <TouchableOpacity onPress={onClose} hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}>
              <X size={22} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>

          {success ? (
            <View className="items-center py-6">
              <CheckCircle2 size={40} color="#34D399" />
              <Text className="mt-3 text-base font-medium text-text-primary">{success}</Text>
            </View>
          ) : (
            <>
              <Text className="text-sm text-text-secondary mb-4 leading-5">
                Find your athlete id and API key under Settings → Developer on intervals.icu.
              </Text>

              {/* These were hand-rolled boxed fields, so the Input primitive's
                  editorial underline never reached them — the same bypass that
                  put a boxed textarea beside an underline on web. */}
              <Input
                label="Athlete ID"
                placeholder="i123456"
                value={athleteId}
                onChangeText={setAthleteId}
                autoCapitalize="none"
                autoCorrect={false}
                testID="intervals-athlete-id"
              />

              <Input
                label="API Key"
                placeholder="API key"
                value={apiKey}
                onChangeText={setApiKey}
                secureTextEntry
                showPasswordToggle
                autoCapitalize="none"
                autoCorrect={false}
                testID="intervals-api-key"
              />
              {error && (
                <Text className="text-sm text-error mb-3" testID="intervals-error">
                  {error}
                </Text>
              )}

              <TouchableOpacity
                // Intervals' brand blue belongs on the provider mark, not on the
                // submit action — see ConnectionsScreen for the same split.
                className={`py-3.5 rounded-xl items-center bg-primary ${canSubmit ? '' : 'opacity-50'}`}
                onPress={handleSubmit}
                disabled={!canSubmit}
                activeOpacity={0.8}
                testID="intervals-submit"
              >
                {isLoading ? (
                  <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
                ) : (
                  <Text className="text-base font-semibold text-on-primary">Connect</Text>
                )}
              </TouchableOpacity>
            </>
          )}
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}
