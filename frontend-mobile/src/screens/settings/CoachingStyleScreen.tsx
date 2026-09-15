// ABOUTME: Coaching-style picker screen — the personas the server renders from the live contract registry, one row each
// ABOUTME: Persona is orthogonal to the chosen coach — it shapes how every coach speaks

import React, { useEffect, useState } from 'react';
import { View, Text, ActivityIndicator, StyleSheet } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { PaneScrollView, Row, Section } from '../../components/ui';
import { useQuery } from '@tanstack/react-query';
import type { CoachingPersona, PersonaCard } from '@pierre/shared-types';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';
import { personasApi, userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useTranslation } from '@pierre/i18n';

/**
 * The persona picker.
 *
 * Every word on a row is the server's. This screen used to hold four
 * hand-written options — a tagline, a blurb and up to two bullets each, in
 * five locales — describing contracts it could not see, while
 * `GET /api/personas` rendered the same cards from the live contract registry
 * and no client read it. Its confirmation line also said the raw slug where
 * web said the brand name; both read `display_name` now.
 *
 * One `Section` holds a `Row` per persona, the selected one marked by a check
 * and followed by its rules and enforcement word — the athlete reads the
 * contract they are on, not every contract at once (DESIGN.md §10).
 */
export function CoachingStyleScreen() {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
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

  /** The row's own brand name, for a message about a persona. */
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
    <View className="flex-1 bg-background-primary" testID="coaching-style-screen">
      <PaneScrollView
        contentContainerStyle={{ paddingTop: spacing.lg, paddingBottom: spacing.xl }}
        showsVerticalScrollIndicator={false}
      >
        <Section
          title={t('settingsTabs.coaching')}
          description={t('app.coachingStyleIntro')}
          testID="coaching-style-section"
        >
          {/* The outcome of the last change, one line in the feedback ink. The
              slot keeps one id whichever way the write went: the test that
              pins the rollback waits on it, and the ink says which. A plain
              line, so it pays the pane's inset itself. */}
          {message && (
            <Text
              className={`text-sm mb-3 px-4 ${message.type === 'success' ? 'text-success' : 'text-error'}`}
              testID="persona-status"
            >
              {message.text}
            </Text>
          )}

          {isLoading && (
            <ActivityIndicator size="small" color={colors.tokens.primary} testID="persona-loading" />
          )}
          {isError && (
            <Text className="text-sm text-error px-4" testID="persona-error">
              {t('common.error')}
            </Text>
          )}

          <View>
            {personas.map((persona, index) => {
              const isSelected = selected === persona.slug;
              const isLast = index === personas.length - 1;
              const isVerified = persona.enforcement === 'verified';
              return (
                <View key={persona.slug}>
                  <Row
                    title={persona.display_name}
                    subtitle={persona.summary}
                    trailing={
                      isSelected ? (
                        isPending ? (
                          <ActivityIndicator size="small" color={colors.tokens.primary} />
                        ) : (
                          <Feather name="check" size={18} color={colors.tokens.primary} />
                        )
                      ) : undefined
                    }
                    showChevron={false}
                    onPress={() => handleSelect(persona.slug as CoachingPersona)}
                    accessibilityRole="radio"
                    accessibilityState={{ selected: isSelected }}
                    // The selected row's hairline moves under its rules, so the
                    // rules read as part of the row rather than the next one.
                    last={isLast || isSelected}
                    testID={`persona-card-${persona.slug}`}
                  />
                  {isSelected && (
                    <View className="px-4">
                      <View
                        className={`pb-3 gap-1.5 ${isLast ? '' : 'border-b border-border-faint'}`}
                        style={isLast ? undefined : { borderBottomWidth: StyleSheet.hairlineWidth }}
                        testID={`persona-details-${persona.slug}`}
                      >
                        {persona.rules.map((rule) => (
                          <View key={rule.key} className="flex-row">
                            <Text className="text-sm text-text-secondary mr-1.5">›</Text>
                            <Text className="flex-1 text-sm text-text-secondary">{rule.text}</Text>
                          </View>
                        ))}
                        {/* Whether the contract is enforced on every reply or
                            only logged — the one thing about a persona the
                            athlete cannot infer from how it reads. */}
                        <Text
                          className={`text-sm font-medium ${isVerified ? 'text-success' : 'text-text-tertiary'}`}
                          testID={`persona-enforcement-${persona.enforcement}`}
                        >
                          {persona.enforcement_label}
                        </Text>
                      </View>
                    </View>
                  )}
                </View>
              );
            })}
          </View>
        </Section>
      </PaneScrollView>
    </View>
  );
}
