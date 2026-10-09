// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The bottom sheet behind a reply's verdict chip — every claim verdict on that reply, one section each
// ABOUTME: Studies open as inline links and the source reply expands in place; support's ids sit behind the card's menu

import React, { useState } from 'react';
import { View, Text, TouchableOpacity, ScrollView, ActivityIndicator, StyleSheet } from 'react-native';
import { Feather } from '@expo/vector-icons';
import type { ClaimVerdict } from '@pierre/shared-types';
import { VERDICT_STATUS_TONE } from '@pierre/shared-types';
import {
  EVIDENCE_STRENGTH_LABEL_KEY,
  VERDICT_STATUS_LABEL_KEY,
  verdictCategoryLabelKey,
} from '@pierre/shared-constants';
import { claimInContext, formatDateTime, parseEvidenceRefs } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';
import { Sheet } from '../../components/ui';
import { openExternal } from '../../utils/openExternal';
import { presentMenu } from '../../utils/presentMenu';
import { verdictChipPalette, type ThemeColors } from './MessageList';
import { useTranslation } from '@pierre/i18n';

/** The reply the verdicts were drawn from, as the thread holds it. */
export interface VerdictSource {
  /** What the conversation is called — the name its header shows. */
  title: string;
  /** The reply's stored content. */
  content: string;
  /** RFC3339 instant the reply was written. */
  createdAt: string;
}

export interface VerdictSheetProps {
  visible: boolean;
  /** Every verdict on the reply whose chip opened the sheet. */
  verdicts: ClaimVerdict[];
  /** The rows are still on their way: the chip landed before the read did. */
  loading: boolean;
  onClose: () => void;
  /** Send a claim back to the coach as a follow-up question. */
  onAskAboutClaim: (verdict: ClaimVerdict) => void;
  /**
   * The reply the verdicts were drawn from. A card names its conversation
   * and expands the passage the claim sits in; without it the section is
   * not drawn.
   */
  source?: VerdictSource;
  /**
   * Hand support the ids that locate a verdict. The host owns the clipboard
   * and its confirmation; without it the card has no actions menu.
   */
  onCopyReference?: (verdict: ClaimVerdict) => void;
}

const SECTION_HEADING = 'text-sm font-semibold text-text-secondary mt-4 mb-1';
const INLINE_LINK = 'text-sm text-primary font-medium';
// A link that stands on its own line is a 44pt row (DESIGN.md §8) that still
// reads as an inline link: no fill, no border, the words left-aligned.
const LINK_ROW = 'min-h-11 justify-center self-start';

/**
 * The conversation a verdict came from, as an inline link that expands the
 * passage of the reply the claim sits in. A phone has no hover, so a press
 * toggles it; "See it in the conversation" closes the sheet onto the reply
 * whose chip opened it.
 */
function SourceSection({
  source,
  claim,
  language,
  colors,
  onShowReply,
}: {
  source: VerdictSource;
  claim: string;
  language: string;
  colors: ThemeColors;
  onShowReply: () => void;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const passage = claimInContext(source.content, claim);

  return (
    <>
      <Text className={SECTION_HEADING}>{t('chat.provenanceConversation')}</Text>
      <TouchableOpacity
        onPress={() => setExpanded((open) => !open)}
        accessibilityRole="button"
        accessibilityState={{ expanded }}
        testID="verdict-source"
        className="flex-row items-center gap-2 min-h-11"
      >
        <Feather name="message-circle" size={16} color={colors.tokens.primary} />
        <Text className={`flex-1 ${INLINE_LINK}`} numberOfLines={1}>
          {source.title}
        </Text>
        <Feather name={expanded ? 'chevron-up' : 'chevron-down'} size={16} color={colors.text.secondary} />
      </TouchableOpacity>
      {expanded ? (
        <View
          testID="verdict-source-preview"
          className="pl-3"
          style={{ borderLeftWidth: StyleSheet.hairlineWidth, borderLeftColor: colors.border.default }}
        >
          <Text className="text-xs text-text-tertiary">{formatDateTime(source.createdAt, language)}</Text>
          <Text className="text-sm text-text-primary mt-1">
            {passage.before}
            {passage.match ? (
              <Text className="bg-primary-container text-on-primary-container">{passage.match}</Text>
            ) : null}
            {passage.after}
          </Text>
          <TouchableOpacity
            onPress={onShowReply}
            accessibilityRole="button"
            testID="verdict-see-in-conversation"
            className={LINK_ROW}
          >
            <Text className={INLINE_LINK}>{t('chat.verdictSeeInConversation')}</Text>
          </TouchableOpacity>
        </View>
      ) : null}
    </>
  );
}

/**
 * Everything the sheet says about one verdict, as one section under a faint
 * hairline: the status word in the verdict's ink beside the actions menu, the
 * evidence and confidence on one tertiary line, the claim quoted behind a
 * hairline rule, then the findings, the studies as inline links, the
 * conversation the reply came from, the timestamp and the inline ask link.
 */
function VerdictCard({
  verdict,
  language,
  colors,
  source,
  onClose,
  onAskAboutClaim,
  onCopyReference,
}: {
  verdict: ClaimVerdict;
  language: string;
  colors: ThemeColors;
  source?: VerdictSource;
  onClose: () => void;
  onAskAboutClaim: (verdict: ClaimVerdict) => void;
  onCopyReference?: (verdict: ClaimVerdict) => void;
}) {
  const { t } = useTranslation();
  const { ink } = verdictChipPalette(VERDICT_STATUS_TONE[verdict.status], colors);
  const references = parseEvidenceRefs(verdict.evidence_refs, verdict.evidence);
  const emittedLabel = formatDateTime(verdict.created_at, language);
  const meta = [
    t(verdictCategoryLabelKey(verdict.category)),
    t('chat.evidenceLabel', { strength: t(EVIDENCE_STRENGTH_LABEL_KEY[verdict.evidence_strength]) }),
    t('chat.confidenceLabel', { confidence: (verdict.confidence * 100).toFixed(0) }),
  ].join(' · ');

  return (
    <View testID="verdict-card" className="py-4 border-b border-border-faint">
      <View className="flex-row items-center justify-between">
        <Text className="text-sm font-semibold" style={{ color: ink }}>
          {t(VERDICT_STATUS_LABEL_KEY[verdict.status])}
        </Text>
        {onCopyReference ? (
          <TouchableOpacity
            onPress={() =>
              presentMenu([{ label: t('chat.verdictCopyReference'), onPress: () => onCopyReference(verdict) }], {
                title: t('chat.verdictActions'),
                cancelLabel: t('common.cancel'),
              })
            }
            accessibilityRole="button"
            accessibilityLabel={t('chat.verdictActions')}
            hitSlop={12}
            testID="verdict-actions"
          >
            <Feather name="more-horizontal" size={20} color={colors.text.secondary} />
          </TouchableOpacity>
        ) : null}
      </View>
      <Text className="text-sm text-text-tertiary mt-0.5">{meta}</Text>

      <Text className={SECTION_HEADING}>{t('chat.theClaim')}</Text>
      <Text
        className="text-base italic text-text-secondary pl-3"
        style={{ borderLeftWidth: StyleSheet.hairlineWidth, borderLeftColor: colors.border.default }}
      >
        {verdict.claim_text}
      </Text>

      {verdict.explanation ? (
        <>
          <Text className={SECTION_HEADING}>{t('chat.detectorFindings')}</Text>
          <Text className="text-sm text-text-primary">{verdict.explanation}</Text>
        </>
      ) : null}

      {references.length > 0 ? (
        <>
          <Text className={SECTION_HEADING}>{t('chat.evidenceReferences')}</Text>
          {references.map((reference, index) => {
            const href = reference.href;
            if (!href) {
              return (
                <Text key={reference.id} className="text-xs text-text-primary">
                  {reference.id}
                </Text>
              );
            }
            // Named by author and year when the corpus names the study;
            // otherwise "Read the study", or numbered when there are several.
            const label =
              reference.label ??
              (references.length === 1 ? t('chat.verdictReadStudy') : t('chat.verdictStudyN', { n: index + 1 }));
            return (
              <TouchableOpacity
                key={reference.id}
                onPress={() => void openExternal(href, t)}
                accessibilityRole="link"
                // The arrow is decoration, as the web's icon is: the link
                // reads as its words alone.
                accessibilityLabel={label}
                accessibilityHint={reference.title ?? reference.id}
                testID="verdict-study-link"
                className={LINK_ROW}
              >
                <Text className={INLINE_LINK}>{label} ↗</Text>
              </TouchableOpacity>
            );
          })}
        </>
      ) : null}

      {source ? (
        <SourceSection
          source={source}
          claim={verdict.claim_text}
          language={language}
          colors={colors}
          onShowReply={onClose}
        />
      ) : null}

      <Text className="text-xs text-text-tertiary mt-4">
        {t('frag.verdictEmitted')} {emittedLabel}
      </Text>

      <Text
        className={`mt-3 ${INLINE_LINK}`}
        onPress={() => onAskAboutClaim(verdict)}
        accessibilityRole="button"
        testID="verdict-ask"
      >
        {t('chat.askAboutClaim')}
      </Text>
    </View>
  );
}

/**
 * Every verdict on one reply, in one sheet.
 *
 * The chip opens it with all the rows of its message — a reply that drew two
 * chips shows two sections, not the first one twice. A chip pressed before the
 * rows landed opens it on the loading line while the host re-reads them.
 * Its study links, source preview and copy action are driven on a device by
 * the Maestro flow `.maestro/chat/11-verdict-sheet.yaml`.
 */
export function VerdictSheet({
  visible,
  verdicts,
  loading,
  onClose,
  onAskAboutClaim,
  source,
  onCopyReference,
}: VerdictSheetProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();

  return (
    <Sheet visible={visible} onClose={onClose} testID="verdict-sheet" backdropTestID="verdict-sheet-backdrop">
      <View className="flex-row items-center justify-between mb-1">
        <Text className="text-lg font-semibold text-text-primary" testID="verdict-sheet-title">
          {verdicts.length === 1 ? t('chat.aboutThisClaim') : t('chat.verdictsTitle')}
        </Text>
        <TouchableOpacity onPress={onClose} accessibilityLabel={t('chat.close')} testID="verdict-sheet-close">
          <Feather name="x" size={22} color={colors.text.secondary} />
        </TouchableOpacity>
      </View>

      {loading && verdicts.length === 0 ? (
        <View className="flex-row items-center gap-2 py-6">
          <ActivityIndicator size="small" color={colors.text.secondary} />
          <Text className="text-sm text-text-secondary">{t('chat.verdictsLoading')}</Text>
        </View>
      ) : null}

      <ScrollView keyboardShouldPersistTaps="handled">
        {verdicts.map((verdict) => (
          <VerdictCard
            key={verdict.id}
            verdict={verdict}
            language={language}
            colors={colors}
            source={source}
            onClose={onClose}
            onAskAboutClaim={onAskAboutClaim}
            onCopyReference={onCopyReference}
          />
        ))}
      </ScrollView>
    </Sheet>
  );
}
