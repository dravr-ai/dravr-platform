// ABOUTME: Coaching-style picker screen — the cards the server renders from the live contract registry
// ABOUTME: Persona is orthogonal to the chosen coach — it shapes how every coach speaks

import React, { useEffect, useState } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { useQuery } from '@tanstack/react-query';
import type { CoachingPersona, PersonaCard } from '@pierre/shared-types';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { colors, spacing, glassCard } from '../../constants/theme';
import { personasApi, userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useTranslation } from '@pierre/i18n';

const cardStyle: ViewStyle = {
  borderRadius: 16,
  padding: spacing.md,
  marginBottom: spacing.md,
  ...glassCard,
};

const cardSelectedStyle: ViewStyle = {
  borderColor: colors.pierre.violet,
  borderWidth: 1,
};

/**
 * The persona picker.
 *
 * Every word on a card is the server's. This screen used to hold four
 * hand-written options — a tagline, a blurb and up to two bullets each, in
 * five locales — describing contracts it could not see, while
 * `GET /api/personas` rendered the same cards from the live contract registry
 * and no client read it. Its confirmation line also said the raw slug where
 * web said the brand name; both read `display_name` now.
 */
export function CoachingStyleScreen() {
  const { t, language } = useTranslation();
  const router = useRouter();
  const { user, updateUser } = useAuth();
  const [selected, setSelected] = useState<CoachingPersona>('casual');
  const [isPending, setIsPending] = useState(false);
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

  useEffect(() => {
    if (user?.coaching_persona) {
      setSelected(user.coaching_persona);
    }
  }, [user?.coaching_persona]);

  const { data, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.personas.list(language),
    queryFn: () => personasApi.list(language),
  });
  const personas: PersonaCard[] = data?.personas ?? [];

  /** The card's own brand name, for a message about a persona. */
  const nameOf = (slug: string) =>
    personas.find((persona) => persona.slug === slug)?.display_name ?? slug;

  const handleSelect = async (persona: CoachingPersona) => {
    if (persona === selected || isPending) {
      return;
    }
    const previous = selected;
    setSelected(persona);
    setIsPending(true);
    try {
      const result = await userApi.setCoachingPersona(persona);
      setMessage({
        type: 'success',
        text: t('app.coachingStyleUpdated', { style: nameOf(result.persona) }),
      });
      // Sync the AuthContext user so other screens see the new persona.
      await updateUser({ coaching_persona: result.persona });
    } catch {
      setSelected(previous);
      setMessage({
        type: 'error',
        text: t('app.coachingStyleUpdateFailed', { style: nameOf(persona) }),
      });
    } finally {
      setIsPending(false);
      setTimeout(() => setMessage(null), 3000);
    }
  };

  return (
    <SafeAreaView
      className="flex-1 bg-background-primary"
      testID="coaching-style-screen"
    >
      {/* Header */}
      <View className="flex-row items-center px-4 py-3 border-b border-white/10">
        <TouchableOpacity
          className="p-2 mr-2"
          onPress={() => router.back()}
          testID="back-button"
        >
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-bold text-text-primary">{t('app.coachingStyleLower')}</Text>
      </View>

      <ScrollView
        className="flex-1 px-4"
        contentContainerStyle={{ paddingTop: spacing.md, paddingBottom: spacing.xl }}
        showsVerticalScrollIndicator={false}
      >
        <Text className="text-text-secondary text-sm leading-relaxed mb-4">
          {t('app.coachingStyleIntro')}
        </Text>

        {message && (
          <View
            className={`mb-4 p-3 rounded-lg border ${
              message.type === 'success'
                ? 'bg-emerald-500/10 border-emerald-500/30'
                : 'bg-red-500/10 border-red-500/30'
            }`}
            testID="persona-status"
          >
            <Text
              className={`text-sm ${
                message.type === 'success' ? 'text-emerald-400' : 'text-red-400'
              }`}
            >
              {message.text}
            </Text>
          </View>
        )}

        {isLoading && (
          <Text className="text-sm text-text-secondary" testID="persona-loading">
            {t('common.loading')}
          </Text>
        )}
        {isError && (
          <Text className="text-sm text-error" testID="persona-error">
            {t('common.error')}
          </Text>
        )}

        {personas.map((persona) => {
          const isSelected = selected === persona.slug;
          return (
            <TouchableOpacity
              key={persona.slug}
              accessibilityRole="radio"
              accessibilityState={{ selected: isSelected }}
              testID={`persona-card-${persona.slug}`}
              onPress={() => handleSelect(persona.slug as CoachingPersona)}
              disabled={isPending}
              activeOpacity={0.85}
              style={[cardStyle, isSelected ? cardSelectedStyle : null]}
            >
              <View className="flex-row items-center justify-between mb-1.5">
                <Text className="text-base font-semibold text-text-primary">
                  {persona.display_name}
                </Text>
                {isSelected && (
                  <View className="flex-row items-center">
                    {isPending ? (
                      <ActivityIndicator size="small" color={colors.pierre.violet} />
                    ) : (
                      <Text
                        className="text-xs font-semibold uppercase tracking-wide"
                        style={{ color: colors.pierre.violet }}
                      >
                        {t('app.active')}
                      </Text>
                    )}
                  </View>
                )}
              </View>
              <Text className="text-sm text-text-secondary leading-relaxed mb-3">
                {persona.summary}
              </Text>
              <View className="mb-3">
                {persona.rules.map((rule) => (
                  <View key={rule.key} className="flex-row mb-1.5">
                    <Text
                      className="text-sm mr-2 mt-0.5"
                      style={{ color: colors.pierre.violet }}
                    >
                      ›
                    </Text>
                    <Text className="text-xs text-text-tertiary flex-1 leading-relaxed">
                      {rule.text}
                    </Text>
                  </View>
                ))}
              </View>
              {/* Whether the contract is enforced on every reply or only
                  logged — the one thing about a persona the athlete cannot
                  infer from how it reads. */}
              <View
                className={`self-start rounded-full px-2 py-0.5 ${
                  persona.enforcement === 'verified' ? 'bg-success/15' : 'bg-background-tertiary'
                }`}
                testID={`persona-enforcement-${persona.enforcement}`}
              >
                <Text
                  className={`text-[11px] font-medium ${
                    persona.enforcement === 'verified' ? 'text-success' : 'text-text-tertiary'
                  }`}
                >
                  {persona.enforcement_label}
                </Text>
              </View>
            </TouchableOpacity>
          );
        })}
      </ScrollView>
    </SafeAreaView>
  );
}
