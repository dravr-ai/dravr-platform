# Mobile surfaces — the screen-by-screen checklist

Every screen in `frontend-mobile`, its Expo Router route, the component behind it, the anchor you
wait on, and what "correct" looks like with the seeded `mobiletest@pierre.dev` (Garmin, 30
activities / 30 days).

Routing is **expo-router** file-based under `app/`; there is no `src/navigation/`. Screen
components live in `src/screens/`; the files under `app/` are thin route wrappers that re-export
them.

**Anchor column:** the `testID` to wait on. `—` means the screen has **no stable container
anchor**; wait on distinctive visible text instead, and if you write a regression flow for that
screen, add the `testID` as part of the fix rather than matching on body copy.

---

## Tab structure

Six tabs, rendered by `ExpandableTabBar` (`src/components/ui/ExpandableTabBar.tsx`), not the
default expo-router tab bar — `tabBarStyle` is `display: 'none'` and the floating glass bar is
drawn in its place.

| Tab | Route group | testID | Label |
|---|---|---|---|
| Chat | `(chat)` | `tab-chat` | Chat |
| Coaches | `(coaches)` | `tab-coaches` | Coaches |
| Discover | `(discover)` | `tab-discover` | Discover |
| Groups | `(groups)` | `tab-groups` | Groups |
| Insights | `(social)` | `tab-insights` | Insights |
| Settings | `(settings)` | `tab-settings` | Settings |

The bar expands: `expandable-tab-bar-plus` opens a menu of `tab-menu-item-(chat)` … plus two
quick actions, `quick-action-new-chat` and `quick-action-new-coach`. Re-tapping the **active**
Chat tab is not a no-op — it `router.replace`s chat with `conversationId: 'new'`, resetting to
coach selection. Test both the collapsed row and the expanded menu; they are different code
paths to the same routes.

`TAB_BAR_BOTTOM_OFFSET` (= collapsed height + 40) is the space screens must reserve. Content
clipped under the floating bar is a finding on every screen.

---

## Auth surfaces

Reached before login; `app/index.tsx` redirects `/` → `/(auth)/login`.

| Screen | Route | Component | Anchor | Expect |
|---|---|---|---|---|
| Login | `/(auth)/login` | `auth/LoginScreen` | `login-screen` | `email-input`, `password-input`, `login-button`, `forgot-password-link`, `google-signin-button`. Empty-email and invalid-email validation both surface a message. Wrong password surfaces an error, never a silent no-op. |
| Register | `/(auth)/register` | `auth/RegisterScreen` | — | Form submits; a new account lands on pending-approval or the app depending on tenant policy. No anchor testID exists. |
| Forgot password | `/(auth)/forgot-password` | `auth/ForgotPasswordScreen` | `forgot-password-screen` | `forgot-email-input`, `send-code-button`. |
| Reset password | `/(auth)/reset-password` | `auth/ResetPasswordScreen` | `reset-password-screen` | `reset-code-input`, `new-password-input`, `confirm-password-input`, `reset-password-button`. Mismatched confirmations are rejected. |
| Pending approval | `/(auth)/pending-approval` | `auth/PendingApprovalScreen` | — | Reached only for a non-active account. No anchor testID exists. |

iOS pops a native **Save Password?** sheet after login; it blocks the whole accessibility tree
until dismissed. That is the OS, not the app.

---

## Onboarding surfaces

Gated by `RootLayoutNav` in `app/_layout.tsx`: it reads `needs_provider_connection` from
`/api/me/onboarding-status`, asks the shared step registry for the current step, and maps it to a
route. Users cannot navigate out; they exit by completing the step.

| Step | Route | Component | Anchor | Expect |
|---|---|---|---|---|
| Profile type | `/(onboarding)/profile-type` | `OnboardingProfileTypeScreen` | `profile-type-screen` | Athlete vs coach choice. Completion is AsyncStorage: `dravr.profile_type_chosen.<userId>` = `'1'`. |
| Connect provider | `/(onboarding)/connect` | `OnboardingConnectScreen` | `onboarding-screen` | Provider cards from `/api/providers`. "Welcome" heading. A user with a live connection must pass through, not hard-gate. |
| Coach proposal | `/(onboarding)/coach-proposal` | `OnboardingCoachProposalScreen` | `coach-proposal-screen` | Proposed coaches, non-empty for a user with activities. Completion: `dravr.coach_proposal_done.<userId>` = `'1'`. |
| Messaging channel | `/(onboarding)/messaging-channel` | `OnboardingMessagingChannelScreen` | `messaging-channel-screen` | Channel list from `messagingApi.getAvailableChannels`. |
| Messaging configure | `/(onboarding)/messaging-configure` | `OnboardingMessagingConfigureScreen` | `messaging-configure-screen` | `messaging-qr` renders a real link token, not a placeholder. |

Both storage-backed hooks **fail open** (`useProfileTypeChosen.ts`, `useCoachProposalSeen.ts`): a
storage error is treated as done. So a user stuck on one of these steps is a genuine routing
defect, never a storage hiccup. To re-run onboarding, clear the two keys (or clear the keychain
and relaunch); to skip it on a re-run, set them to `'1'`.

`.maestro/onboarding/01-forced-onboarding.yaml` covers the zero-provider guard but is
deliberately **not** registered in `config.yaml` — it needs a user with no providers, which
`mobiletest` is not.

---

## Chat tab — `(chat)`

| Screen | Route | Component | Anchor | Expect |
|---|---|---|---|---|
| Chat | `/(app)/(tabs)/(chat)` | `chat/ChatScreen` | `chat-screen` | Coach selection on a fresh conversation; `message-input` + `send-button`; header `chat-title` / `chat-title-button` (rename → `rename-conversation-dialog`) and `history-button`. |
| Conversations | `/(app)/(tabs)/(chat)/conversations` | `conversations/ConversationsScreen` | — (`back-button`) | Reached via `history-button`. Lists prior conversations; rename via `rename-conversation-dialog`; delete/swipe actions. No container testID. |

Chat mechanics that bite:

- `send-button` is state-driven: the testID is `send-button-disabled` until the input has text,
  then flips to `send-button`. Wait for the enabled id, do not tap blind.
- `MessageList` starts as a `ScrollView` empty state and swaps to a FlashList once messages
  exist — `messages-list` only appears after the swap.
- `thinking-indicator` appears while the model runs and must disappear when the reply lands. An
  indicator that never clears is P1.
- Voice input is `voice-input-button` (`ChatInputBar`). Speech recognition needs the **native**
  build; in Expo Go, exercise the button's visibility and disabled states, not transcription.
- `ProviderModal` and `WorkoutPlanCard` render inside chat — a workout-plan card with all-zero
  or missing fields against a user with 30 seeded activities is the silent-stub smell.

---

## Coaches tab — `(coaches)`

| Screen | Route | Component | Anchor | Expect |
|---|---|---|---|---|
| Coach library | `/(app)/(tabs)/(coaches)` | `coaches/CoachLibraryScreen` | `coach-library-screen` | Seeded system coaches render. `coach-search-input` filters; `category-filter-scroll` filters; `favorites-toggle` and `show-hidden-toggle` change the set; `create-coach-button` opens the editor. |
| Coach detail | `/(app)/(tabs)/(coaches)/[coachId]` | `coaches/CoachDetailScreen` | `coach-detail-screen` | `coach-title`, `category-badge`, `use-count` show real values. `use-in-chat-button` lands in chat with that coach. `edit-button` / `hide-button` / `delete-button` gated correctly for a system coach. |
| Coach editor | `/(app)/(tabs)/(coaches)/editor` | `coaches/CoachEditorScreen` | `coach-editor-screen` | The multi-step wizard: `coach-title-input`, `coach-description-input`, `category-picker` → `selected-category`, `tag-input` + `add-tag-button` → `tags-container`, `system-prompt-input` (+ `expand-prompt-button` → `expanded-modal`), `startup-query-input`, `activity-count-input`, `time-frame-picker`, `prefetch-toggle`, `version-history-button`. Validation surfaces `title-error` / `description-error` / `prompt-error`; `token-counter` / `token-count-text` reflect real content. `forked-from-banner` appears when forking a system coach. |

Anything you create here persists in the dev DB — clean it up (see `.maestro/coaches/09-*` and
`.maestro/coach-wizard/10-*`).

---

## Discover tab — `(discover)`

| Screen | Route | Component | Anchor | Expect |
|---|---|---|---|---|
| Store | `/(app)/(tabs)/(discover)` | `store/StoreScreen` | `store-screen` | `coach-list` non-empty against seeded coaches; `search-input` filters; category + sort controls change the ordering; `loading-indicator` resolves; pull-to-refresh works. |
| Store coach detail | `/(app)/(tabs)/(discover)/[coachId]` | `store/StoreCoachDetailScreen` | `store-coach-detail-screen` | `coach-title`, `category-badge`, `install-count` show real values. Install then uninstall round-trips — an install that reports success but leaves the library unchanged is P1. |

---

## Groups tab — `(groups)`

| Screen | Route | Component | Anchor | Expect |
|---|---|---|---|---|
| Group list | `/(app)/(tabs)/(groups)` | `groups/GroupListScreen` | `group-list-screen` | `group-list`, or an empty state offering `create-group-empty-button` / `join-group-empty-button`. Header buttons `create-group-header-button` / `join-group-header-button` always present. |
| Create group | `/(app)/(tabs)/(groups)/create` | `groups/CreateGroupScreen` | `create-group-screen` | `group-name-input`, `group-description-input`, `create-group-button`. Empty name is rejected. |
| Join group | `/(app)/(tabs)/(groups)/join` | `groups/JoinGroupScreen` | `join-group-screen` | `invite-code-input`, `join-group-button`. A bad code surfaces an error, not a silent failure. |
| Group detail | `/(app)/(tabs)/(groups)/[groupId]` | `groups/GroupDetailScreen` | `group-detail-screen` | Members render. `chat-with-coach-button`, `share-invite-button`, `remove-coach-button`, `leave-group-button` — each does what it says; a "leave" that leaves the user in the group is the canonical silent stub. |

---

## Insights tab — `(social)`

| Screen | Route | Component | Anchor | Expect |
|---|---|---|---|---|
| Social feed | `/(app)/(tabs)/(social)` | `social/SocialFeedScreen` | `social-feed-screen` | `feed-list` renders seeded social data; `feed-search-input` filters; `suggestions-banner` with `share-suggestion-button` / `dismiss-suggestions` — dismissal is component state (`setShowSuggestionsBanner`), so it is *expected* to return after a remount; do not file that. |
| Friends | `/(app)/(tabs)/(social)/friends` | `social/FriendsScreen` | `friends-screen` | Friend list from seeded social data; `search-friends-button` and `friend-requests-button` navigate. |
| Search friends | `/(app)/(tabs)/(social)/search-friends` | `social/SearchFriendsScreen` | `search-friends-screen` | `user-search-input` → `search-results-list` returns real users; sending a request succeeds once and is reflected. |
| Friend requests | `/(app)/(tabs)/(social)/friend-requests` | `social/FriendRequestsScreen` | — | Accept / decline round-trip. No container testID. |
| Social settings | `/(app)/(tabs)/(social)/social-settings` | `social/SocialSettingsScreen` | `social-settings-screen` | `discoverable-switch` toggles and `save-button` persists — reload and confirm the value stuck. |
| Share insight | `/(app)/(tabs)/(social)/share-insight` (also `/(app)/share-insight` as a modal) | `social/ShareInsightScreen` | `share-insight-screen` | `insight-content-input`, `share-button`, `close-button`. |
| Adapted insight | `/(app)/(tabs)/(social)/adapted-insight` (also `/(app)/adapted-insight`) | `social/AdaptedInsightScreen` | `adapt-insight-screen` | Note the anchor is `adapt-insight-screen`, not `adapted-…`. |
| Adapted insights (list) | `/(app)/(tabs)/(social)/adapted-insights` | `social/AdaptedInsightsScreen` | — | No container testID. |
| Activity detail | `/(app)/(tabs)/(social)/activity/[activityId]` | `ActivityDetailScreen` | — (`back-button`) | Real metrics from a seeded Garmin activity — an all-zero card here is the silent-stub smell. `ask-pierre-button`, `share-with-friends-button`, `get-insights-button` each navigate. |

---

## Settings tab — `(settings)`

| Screen | Route | Component | Anchor | Expect |
|---|---|---|---|---|
| Settings | `/(app)/(tabs)/(settings)` | `settings/SettingsScreen` | `settings-screen` | A profile header (`settings-profile-section`, `settings-edit-profile-button`) above one row per named pane, inside `settings-pane-list`. The rows are DERIVED from `SETTINGS_PANES` in `@pierre/shared-constants`, so read the list from there rather than from a copy here: each row is `settings-pane-<id>` and pushes that pane's `mobile` route. `settings-pane-tokens` rides the `api_tokens` flag and `settings-pane-billing` the build-time billing toggle. `settings-safe-header` carries the status-bar inset and the scroll's bottom padding is `tabBarBottomOffset(insets.bottom)`. **Every row must navigate** — a row whose destination does not exist is the exact defect class the parity registry guards. |
| Profile | `/(app)/(tabs)/(settings)/profile` | `settings/ProfileScreen` | `profile-screen` | Reached by `settings-edit-profile-button` and `settings-pane-profile`. Display-name edit (`profile-display-name-input`) saves and survives a reload. Also holds appearance (`profile-appearance-section`, `appearance-option-system|dark|light`) and language (`profile-language-section`, `language-switcher`), as the web Profile pane does. |
| Messaging channels | `/(app)/(tabs)/(settings)/messaging` | `settings/MessagingChannelsScreen` | `messaging-channels-screen` | Reached by `settings-pane-messaging`. Channel link/unlink round-trips. |
| API Tokens | `/(app)/(tabs)/(settings)/tokens` | `settings/TokensScreen` | `tokens-screen` | Reached by `settings-pane-tokens`, behind the `api_tokens` flag. `mcp-token-list` lists active tokens; `new-token-button` mints one and `revoke-token-<id>` takes it back. **No token value is echoed back** into the rendered tree after the create sheet closes. |
| Account | `/(app)/(tabs)/(settings)/account` | `settings/AccountScreen` | `account-screen` | Reached by `settings-pane-account`. Sections, in the order `settingsPaneSections('account')` declares: `account-section-account-status`, `-usage`, `-security`, `-connected-mcp-apps`, `-sign-out`. Same five things web groups under Account. |
| About | `/(app)/(tabs)/(settings)/about` | `settings/AboutScreen` | `about-screen` | Reached by `settings-pane-about`. Rows from `settingsPaneSections('about')`: `about-section-version`, `-coach-model`, `-help`, `-legal`. `about-coach-model-value` states which model answers, **read-only** — there is no key field on any athlete surface. Both links open `dravr.ai/docs`. |
| Coaching style | `/(app)/(tabs)/(settings)/coaching-style` | `settings/CoachingStyleScreen` | `coaching-style-screen` | `persona-status` reflects the saved persona; a change persists across a reload. |
| Connected apps | `/(app)/(tabs)/(settings)/connected-apps` | `settings/ConnectedAppsScreen` | `connected-apps-screen` | OAuth clients the user has authorised; revoke actually revokes. |
| Connections | `/(app)/(tabs)/(settings)/connections` (also `/(app)/connections` as a modal) | `connections/ConnectionsScreen` | — (`back-button`, `connections-drag-indicator`) | Provider cards from `/api/providers`: `sciotte`, `sciotte_garmin`, `strava`, `whoop`, `intervals_icu`. `garmin` is **intentionally absent** — see below. Connect/disconnect round-trips; a disconnect that leaves the card connected is P1. |

The panes in `ADMIN_HIDDEN_PANES` — connections, messaging, about — are hidden for `admin` /
`super_admin` (grep `isAdminUser` in `SettingsScreen.tsx`); admins are pure operators. Both
directions of that gate are part of the sweep.

**`garmin` never appears in the provider list.** `compute_providers_status` in
`crates/pierre-routes-auth/src/oauth.rs` skips it explicitly, because Garmin's OAuth API is
uncredentialed and a "Garmin Connect" card would 500 on connect; `sciotte_garmin` is the
supported Garmin path. `mobiletest` is seeded as a Garmin athlete, so its activities exist while
no Garmin card does. Do not file this.

---

## Modal / stack screens over the tabs

Declared in `app/(app)/_layout.tsx`.

| Screen | Route | Component | Anchor | Notes |
|---|---|---|---|---|
| Notifications | `/(app)/notifications` | `notifications/NotificationCenterScreen` | — (`notifications-back`, `mark-all-read`) | Entered from `NotificationBellButton`. Mark-all-read must clear the badge and survive a reload. |
| Connections | `/(app)/connections` | `connections/ConnectionsScreen` | — | Modal presentation of the same screen as the settings route. |
| Share insight | `/(app)/share-insight` | `social/ShareInsightScreen` | `share-insight-screen` | Modal presentation. |
| Adapted insight | `/(app)/adapted-insight` | `social/AdaptedInsightScreen` | `adapt-insight-screen` | Modal presentation. |
| Memory | `/(app)/memory` | `memory/MemoryScreen` | `memory-screen` | Reached from **Settings** (`settings-pane-memory`) as well as by deep link. Renders the user-facing memory inspector. A Settings row that does not navigate is a finding, not a known gap. |
| Billing | `/(app)/billing` | `settings/BillingScreen` | — | `BILLING_ENABLED = false` in `src/constants/features.ts` gates **both** `settings-pane-billing` and the route, which redirects out. Unreachable is correct while the flag is off; flip it to sweep the screen. |

---

## Coverage bookkeeping

Count what you actually opened. Do **not** trust the number below on faith — a hardcoded count
is exactly what went stale here before. Re-derive it at the start of every sweep:

```bash
cd frontend-mobile
find src/screens -name '*.tsx' -not -path '*__tests__*' -not -name '*.test.tsx' | wc -l
find src/screens -name '*.tsx' -not -path '*__tests__*' -not -name '*.test.tsx' \
  | sed 's|src/screens/||; s|/.*||' | sort | uniq -c | sort -rn
```

As of 2026-09-02 that is **52 screen components**, plus the tab bar itself:

- 16 chat · 11 settings · 7 onboarding · 5 auth · 4 groups · 3 store · 2 conversations
- 1 each: notifications, memory, connections, coaches

If your count differs from 52, screens were added or removed since this doc was written — sweep
what is on disk and correct this file in the same change.

Three of those — Connections, Share Insight, Adapted Insight — have **two** routes each (a tabbed
route and a modal presentation). Both routes are worth opening; count the screen once.

State the visited/total per role in the report, and name every screen you did not open and why —
`BILLING_ENABLED = false`, "no emulator available for the Android pass", and "register not
exercised because the tenant auto-approves" are all acceptable reasons. Silence is not.
