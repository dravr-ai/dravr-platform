// ABOUTME: Catalogue keys of the hosted pages a chat link opens: connect picker, Intervals.icu form, hosted login
// ABOUTME: One constant per string, plus the slot-less set a page template names directly as `{{t:<key>}}`
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Keys of the server-rendered hosted pages.
//!
//! The pages `/connect` opens from a chat — the provider picker, the
//! Intervals.icu API-key form, the hosted provider login, and the success and
//! link-error pages they end on — are rendered by `pierre-routes-auth` in the
//! athlete's own locale. Every string they show is a catalogue key declared
//! here, read through
//! [`MessagingStringsRegistry`](crate::MessagingStringsRegistry).
//!
//! A key whose text carries a positional slot (`{0}`) is filled by the
//! renderer that knows the value. A key with no slot is named by the template
//! itself, as `{{t:<key>}}`; [`TEMPLATE_KEYS`] is the set a renderer
//! substitutes, so a template can only name a key this module declares.

// ── Shared by the picker, the form and the hosted login ──

/// Key: "Connect your fitness account" No format placeholders.
pub const KEY_HOSTED_COMMON_SUBTITLE: &str = "hosted.common.subtitle";
/// Key: "Connect your `{0}` account"
pub const KEY_HOSTED_COMMON_SUBTITLE_NAMED: &str = "hosted.common.subtitleNamed";
/// Key: "Linked from `{0}`"
pub const KEY_HOSTED_COMMON_LINKED_FROM: &str = "hosted.common.linkedFrom";
/// Key: "your chat app" No format placeholders.
pub const KEY_HOSTED_COMMON_YOUR_CHAT_APP: &str = "hosted.common.yourChatApp";
/// Key: "Back" No format placeholders.
pub const KEY_HOSTED_COMMON_BACK: &str = "hosted.common.back";
/// Key: "`{0}` Account"
pub const KEY_HOSTED_COMMON_ACCOUNT_TITLE: &str = "hosted.common.accountTitle";
/// Key: "Email" No format placeholders.
pub const KEY_HOSTED_COMMON_EMAIL_LABEL: &str = "hosted.common.emailLabel";
/// Key: "Username" No format placeholders.
pub const KEY_HOSTED_COMMON_USERNAME_LABEL: &str = "hosted.common.usernameLabel";
/// Key: "Password" No format placeholders.
pub const KEY_HOSTED_COMMON_PASSWORD_LABEL: &str = "hosted.common.passwordLabel";
/// Key: "Log In" No format placeholders.
pub const KEY_HOSTED_COMMON_LOG_IN: &str = "hosted.common.logIn";
/// Key: "Credentials go directly to `{0}` — Dravr never stores your password."
pub const KEY_HOSTED_COMMON_CREDENTIALS_NOTE: &str = "hosted.common.credentialsNote";
/// Key: "Logging in" No format placeholders.
pub const KEY_HOSTED_COMMON_LOGGING_IN_ARIA: &str = "hosted.common.loggingInAria";
/// Key: "Connecting to `{0}`..."
pub const KEY_HOSTED_COMMON_CONNECTING_TO: &str = "hosted.common.connectingTo";
/// Key: "This may take up to 30 seconds." No format placeholders.
pub const KEY_HOSTED_COMMON_MAY_TAKE_TIME: &str = "hosted.common.mayTakeTime";
/// Key: "Loading..." No format placeholders.
pub const KEY_HOSTED_COMMON_LOADING: &str = "hosted.common.loading";
/// Key: "Verifying code..." No format placeholders.
pub const KEY_HOSTED_COMMON_VERIFYING_CODE: &str = "hosted.common.verifyingCode";
/// Key: "2-Step Verification" No format placeholders.
pub const KEY_HOSTED_COMMON_TWO_STEP_TITLE: &str = "hosted.common.twoStepTitle";
/// Key: "Waiting for approval" No format placeholders.
pub const KEY_HOSTED_COMMON_WAITING_APPROVAL_ARIA: &str = "hosted.common.waitingApprovalAria";
/// Key: "Check your phone" No format placeholders.
pub const KEY_HOSTED_COMMON_CHECK_PHONE: &str = "hosted.common.checkPhone";
/// Key: "Tap Yes on the notification" No format placeholders.
pub const KEY_HOSTED_COMMON_TAP_YES: &str = "hosted.common.tapYes";
/// Key: "Tap this number on your phone" No format placeholders.
pub const KEY_HOSTED_COMMON_TAP_NUMBER: &str = "hosted.common.tapNumber";
/// Key: "Check the notification on your device" No format placeholders.
pub const KEY_HOSTED_COMMON_CHECK_NOTIFICATION: &str = "hosted.common.checkNotification";
/// Key: "Waiting" No format placeholders.
pub const KEY_HOSTED_COMMON_WAITING_ARIA: &str = "hosted.common.waitingAria";
/// Key: "Verification Code" No format placeholders.
pub const KEY_HOSTED_COMMON_OTP_TITLE: &str = "hosted.common.otpTitle";
/// Key: "Enter the code sent to your device" No format placeholders.
pub const KEY_HOSTED_COMMON_OTP_LABEL: &str = "hosted.common.otpLabel";
/// Key: "Verify" No format placeholders.
pub const KEY_HOSTED_COMMON_VERIFY: &str = "hosted.common.verify";
/// Key: "Connected!" No format placeholders.
pub const KEY_HOSTED_COMMON_CONNECTED: &str = "hosted.common.connected";
/// Key: "Login failed" No format placeholders.
pub const KEY_HOSTED_COMMON_LOGIN_FAILED: &str = "hosted.common.loginFailed";
/// Key: "`{0}` didn't accept that sign-in. Check your details and try again."
pub const KEY_HOSTED_COMMON_SIGN_IN_REJECTED: &str = "hosted.common.signInRejected";
/// Key: "We couldn't sign you in to `{0}` right now. This is usually temporary — please try again in a few minutes."
pub const KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE: &str = "hosted.common.signInUnavailable";
/// Key: "This sign-in is no longer active. Please start the sign-in again." No format placeholders.
pub const KEY_HOSTED_COMMON_SIGN_IN_EXPIRED: &str = "hosted.common.signInExpired";
/// Key: "Try Again" No format placeholders.
pub const KEY_HOSTED_COMMON_TRY_AGAIN: &str = "hosted.common.tryAgain";

// ── Provider picker ──

/// Key: "Connect a provider - Dravr" No format placeholders.
pub const KEY_HOSTED_PICKER_PAGE_TITLE: &str = "hosted.picker.pageTitle";
/// Key: "Choose a provider" No format placeholders.
pub const KEY_HOSTED_PICKER_HEADING: &str = "hosted.picker.heading";
/// Key: "You'll connect securely here — your password is never shared with this chat." No format placeholders.
pub const KEY_HOSTED_PICKER_SECURITY_NOTE: &str = "hosted.picker.securityNote";
/// Key: "Continue" No format placeholders.
pub const KEY_HOSTED_PICKER_CONTINUE: &str = "hosted.picker.continue";
/// Key: "Your data is now available. You can return to `{0}`."
pub const KEY_HOSTED_PICKER_DATA_AVAILABLE: &str = "hosted.picker.dataAvailable";
/// Key: "Something went wrong" No format placeholders.
pub const KEY_HOSTED_PICKER_ERROR_TITLE: &str = "hosted.picker.errorTitle";
/// Key: "Back to providers" No format placeholders.
pub const KEY_HOSTED_PICKER_BACK_TO_PROVIDERS: &str = "hosted.picker.backToProviders";
/// Key: "Connected" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_CONNECTED: &str = "hosted.picker.tagConnected";
/// Key: "Authorize" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_AUTHORIZE: &str = "hosted.picker.tagAuthorize";
/// Key: "Athlete ID / API key" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_API_KEY: &str = "hosted.picker.tagApiKey";
/// Key: "Username / password" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_USERNAME_PASSWORD: &str = "hosted.picker.tagUsernamePassword";
/// Key: "Email / password" No format placeholders.
pub const KEY_HOSTED_PICKER_TAG_EMAIL_PASSWORD: &str = "hosted.picker.tagEmailPassword";
/// Key: "Strava sign-in didn't complete — you can connect with your Strava email and password instead." No format placeholders.
pub const KEY_HOSTED_PICKER_STRAVA_FALLBACK: &str = "hosted.picker.stravaFallback";
/// Key: "That sign-in didn't complete. Please go back and try again." No format placeholders.
pub const KEY_HOSTED_PICKER_SIGN_IN_INCOMPLETE: &str = "hosted.picker.signInIncomplete";

// ── Hosted provider login ──

/// Key: "Connect your account - Dravr" No format placeholders.
pub const KEY_HOSTED_LOGIN_PAGE_TITLE: &str = "hosted.login.pageTitle";
/// Key: "Your `{0}` data is now available. You can return to `{1}`."
pub const KEY_HOSTED_LOGIN_DATA_AVAILABLE: &str = "hosted.login.dataAvailable";

// ── Intervals.icu API-key form ──

/// Key: "Connect Intervals.icu - Dravr" No format placeholders.
pub const KEY_HOSTED_INTERVALS_PAGE_TITLE: &str = "hosted.intervals.pageTitle";
/// Key: "Dravr checks them with Intervals.icu before saving, and stores the key encrypted." No format placeholders.
pub const KEY_HOSTED_INTERVALS_STORAGE_NOTE: &str = "hosted.intervals.storageNote";
/// Key: "Enter your Athlete ID." No format placeholders.
pub const KEY_HOSTED_INTERVALS_ATHLETE_ID_REQUIRED: &str = "hosted.intervals.athleteIdRequired";
/// Key: "Enter your API key." No format placeholders.
pub const KEY_HOSTED_INTERVALS_API_KEY_REQUIRED: &str = "hosted.intervals.apiKeyRequired";
/// Key: "Intervals.icu rejected those credentials. Check your Athlete ID and API key, then try again." No format placeholders.
pub const KEY_HOSTED_INTERVALS_REJECTED: &str = "hosted.intervals.rejected";
/// Key: "Intervals.icu can't be connected right now. Please try again later." No format placeholders.
pub const KEY_HOSTED_INTERVALS_UNAVAILABLE: &str = "hosted.intervals.unavailable";
/// Key: "We could not save your Intervals.icu connection. Please try again in a moment." No format placeholders.
pub const KEY_HOSTED_INTERVALS_SAVE_FAILED: &str = "hosted.intervals.saveFailed";

// ── Success page ──

/// Key: "Connected - Dravr" No format placeholders.
pub const KEY_HOSTED_SUCCESS_PAGE_TITLE: &str = "hosted.success.pageTitle";
/// Key: "`{0}` connected"
pub const KEY_HOSTED_SUCCESS_HEADING: &str = "hosted.success.heading";
/// Key: "Account connected" No format placeholders.
pub const KEY_HOSTED_SUCCESS_HEADING_GENERIC: &str = "hosted.success.headingGeneric";
/// Key: "Your `{0}` data is now available in Dravr."
pub const KEY_HOSTED_SUCCESS_DATA_AVAILABLE: &str = "hosted.success.dataAvailable";
/// Key: "Your data is now available in Dravr." No format placeholders.
pub const KEY_HOSTED_SUCCESS_DATA_AVAILABLE_GENERIC: &str = "hosted.success.dataAvailableGeneric";
/// Key: "You can return to `{0}` to continue chatting with your agent."
pub const KEY_HOSTED_SUCCESS_RETURN_TO_CHAT: &str = "hosted.success.returnToChat";
/// Key: "This window will close automatically." No format placeholders.
pub const KEY_HOSTED_SUCCESS_AUTO_CLOSE: &str = "hosted.success.autoClose";

// ── Link-error page ──

/// Key: "Link error - Dravr" No format placeholders.
pub const KEY_HOSTED_ERROR_PAGE_TITLE: &str = "hosted.error.pageTitle";
/// Key: "Can't connect your account" No format placeholders.
pub const KEY_HOSTED_ERROR_HEADING: &str = "hosted.error.heading";
/// Key: "This link is incomplete. Please request a fresh link from your chat." No format placeholders.
pub const KEY_HOSTED_ERROR_MISSING_TOKEN: &str = "hosted.error.missingToken";
/// Key: "This link is invalid or has expired. Please request a fresh link from your chat." No format placeholders.
pub const KEY_HOSTED_ERROR_INVALID_LINK: &str = "hosted.error.invalidLink";
/// Key: "This link is malformed. Please request a fresh link from your chat." No format placeholders.
pub const KEY_HOSTED_ERROR_MALFORMED_LINK: &str = "hosted.error.malformedLink";
/// Key: "This link has already been opened. For your security, each link can only be opened once. Please request a fresh link from your chat." No format placeholders.
pub const KEY_HOSTED_ERROR_LINK_ALREADY_OPENED: &str = "hosted.error.linkAlreadyOpened";
/// Key: "We could not load your connections. Please try again in a moment." No format placeholders.
pub const KEY_HOSTED_ERROR_LOAD_FAILED: &str = "hosted.error.loadFailed";
/// Key: "Connecting this provider needs its notice accepted first. Please go back, tick the notice and try again." No format placeholders.
pub const KEY_HOSTED_ERROR_NOTICE_REQUIRED: &str = "hosted.error.noticeRequired";
/// Key: "We couldn't start the connection for this provider. Please go back and try again." No format placeholders.
pub const KEY_HOSTED_ERROR_OAUTH_START_FAILED: &str = "hosted.error.oauthStartFailed";
/// Key: "Something went wrong connecting your account. Please request a fresh link from your chat." No format placeholders.
pub const KEY_HOSTED_ERROR_GENERIC: &str = "hosted.error.generic";

// ── Intervals.icu strings the app dialog shares ──
//
// The web and mobile Intervals.icu dialog reads these same keys, so the
// hosted form and the app name the two fields and where to find them in one
// wording.

/// Key: "Athlete ID" No format placeholders.
pub const KEY_INTERVALS_ATHLETE_ID: &str = "shell.intervalsAthleteId";
/// Key: "API Key" No format placeholders.
pub const KEY_INTERVALS_API_KEY_LABEL: &str = "shell.intervalsApiKeyLabel";
/// Key: "Connect" No format placeholders.
pub const KEY_INTERVALS_CONNECT_ACTION: &str = "shell.intervalsConnectAction";
/// Key: "Find your Athlete ID and API key in Intervals.icu: open Settings and scroll down to Developer Settings, near the bottom of the page." No format placeholders.
pub const KEY_INTERVALS_CREDENTIALS_HELP: &str = "shell.intervalsCredentialsHelp";

/// Every slot-less key above: the set a hosted template may name as
/// `{{t:<key>}}` and a renderer substitutes.
pub const TEMPLATE_KEYS: [&str; 64] = [
    KEY_HOSTED_COMMON_SUBTITLE,
    KEY_HOSTED_COMMON_YOUR_CHAT_APP,
    KEY_HOSTED_COMMON_BACK,
    KEY_HOSTED_COMMON_EMAIL_LABEL,
    KEY_HOSTED_COMMON_USERNAME_LABEL,
    KEY_HOSTED_COMMON_PASSWORD_LABEL,
    KEY_HOSTED_COMMON_LOG_IN,
    KEY_HOSTED_COMMON_LOGGING_IN_ARIA,
    KEY_HOSTED_COMMON_MAY_TAKE_TIME,
    KEY_HOSTED_COMMON_LOADING,
    KEY_HOSTED_COMMON_VERIFYING_CODE,
    KEY_HOSTED_COMMON_TWO_STEP_TITLE,
    KEY_HOSTED_COMMON_WAITING_APPROVAL_ARIA,
    KEY_HOSTED_COMMON_CHECK_PHONE,
    KEY_HOSTED_COMMON_TAP_YES,
    KEY_HOSTED_COMMON_TAP_NUMBER,
    KEY_HOSTED_COMMON_CHECK_NOTIFICATION,
    KEY_HOSTED_COMMON_WAITING_ARIA,
    KEY_HOSTED_COMMON_OTP_TITLE,
    KEY_HOSTED_COMMON_OTP_LABEL,
    KEY_HOSTED_COMMON_VERIFY,
    KEY_HOSTED_COMMON_CONNECTED,
    KEY_HOSTED_COMMON_LOGIN_FAILED,
    KEY_HOSTED_COMMON_SIGN_IN_EXPIRED,
    KEY_HOSTED_COMMON_TRY_AGAIN,
    KEY_HOSTED_PICKER_PAGE_TITLE,
    KEY_HOSTED_PICKER_HEADING,
    KEY_HOSTED_PICKER_SECURITY_NOTE,
    KEY_HOSTED_PICKER_CONTINUE,
    KEY_HOSTED_PICKER_ERROR_TITLE,
    KEY_HOSTED_PICKER_BACK_TO_PROVIDERS,
    KEY_HOSTED_PICKER_TAG_CONNECTED,
    KEY_HOSTED_PICKER_TAG_AUTHORIZE,
    KEY_HOSTED_PICKER_TAG_API_KEY,
    KEY_HOSTED_PICKER_TAG_USERNAME_PASSWORD,
    KEY_HOSTED_PICKER_TAG_EMAIL_PASSWORD,
    KEY_HOSTED_PICKER_STRAVA_FALLBACK,
    KEY_HOSTED_PICKER_SIGN_IN_INCOMPLETE,
    KEY_HOSTED_LOGIN_PAGE_TITLE,
    KEY_HOSTED_INTERVALS_PAGE_TITLE,
    KEY_HOSTED_INTERVALS_STORAGE_NOTE,
    KEY_HOSTED_INTERVALS_ATHLETE_ID_REQUIRED,
    KEY_HOSTED_INTERVALS_API_KEY_REQUIRED,
    KEY_HOSTED_INTERVALS_REJECTED,
    KEY_HOSTED_INTERVALS_UNAVAILABLE,
    KEY_HOSTED_INTERVALS_SAVE_FAILED,
    KEY_HOSTED_SUCCESS_PAGE_TITLE,
    KEY_HOSTED_SUCCESS_HEADING_GENERIC,
    KEY_HOSTED_SUCCESS_DATA_AVAILABLE_GENERIC,
    KEY_HOSTED_SUCCESS_AUTO_CLOSE,
    KEY_HOSTED_ERROR_PAGE_TITLE,
    KEY_HOSTED_ERROR_HEADING,
    KEY_HOSTED_ERROR_MISSING_TOKEN,
    KEY_HOSTED_ERROR_INVALID_LINK,
    KEY_HOSTED_ERROR_MALFORMED_LINK,
    KEY_HOSTED_ERROR_LINK_ALREADY_OPENED,
    KEY_HOSTED_ERROR_LOAD_FAILED,
    KEY_HOSTED_ERROR_NOTICE_REQUIRED,
    KEY_HOSTED_ERROR_OAUTH_START_FAILED,
    KEY_HOSTED_ERROR_GENERIC,
    KEY_INTERVALS_ATHLETE_ID,
    KEY_INTERVALS_API_KEY_LABEL,
    KEY_INTERVALS_CONNECT_ACTION,
    KEY_INTERVALS_CREDENTIALS_HELP,
];
