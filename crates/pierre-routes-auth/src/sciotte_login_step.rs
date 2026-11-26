// ABOUTME: Which step of a credential sign-in produced the outcome the platform is answering
// ABOUTME: A login names its method and account; a continuation names its held flow; both name the window's attempt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_sciotte::wire::LoginMethod;

use crate::sciotte_cancel::StepAttempt;

/// Which step of a sign-in produced the outcome being answered.
#[derive(Clone, Copy)]
pub(crate) enum LoginStep<'a> {
    /// The credential login, which starts a flow.
    Login {
        /// Backend provider name the login targets
        provider: &'a str,
        /// The window's attempt, when it named one: the only kind of login
        /// that asks for a takeover, since closing the takeover cancels it.
        attempt: Option<&'a StepAttempt>,
        /// How the athlete signed in: the provider's form, Google or Apple
        method: LoginMethod,
        /// The account the athlete typed at the provider, for the ops log
        /// only, and only ever masked there
        account: &'a str,
    },
    /// A 2FA pick or a code, continuing the flow the platform holds.
    Continuation {
        /// Backend provider name the held flow was started for
        provider: &'a str,
        /// The window's attempt, when the step named one: the ops log tags
        /// the step's outcome with it
        attempt: Option<&'a StepAttempt>,
        /// The held flow this continuation named to the service
        flow_id: &'a str,
    },
}

impl LoginStep<'_> {
    /// Backend provider name the step belongs to.
    pub(crate) const fn provider(&self) -> &str {
        match self {
            Self::Login { provider, .. } | Self::Continuation { provider, .. } => provider,
        }
    }

    /// The attempt id the window named for this step, `""` when it named
    /// none: the ops log joins a window's steps on it.
    pub(crate) fn attempt_id(&self) -> &str {
        match self {
            Self::Login { attempt, .. } | Self::Continuation { attempt, .. } => {
                attempt.map_or("", StepAttempt::attempt_id)
            }
        }
    }
}
