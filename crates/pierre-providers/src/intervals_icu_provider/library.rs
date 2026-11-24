// ABOUTME: Intervals.icu workout library save — find or create the named folder, then create the workout unless the folder already holds it
// ABOUTME: Inherent methods on IntervalsIcuProvider; always the connected account's own library, never a coached athlete's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The requests behind
//! [`FitnessProvider::save_library_workout`](crate::core::FitnessProvider::save_library_workout)
//! for Intervals.icu: `GET /folders`, which lists every folder with the
//! workouts in it; `POST /folders` when the named folder is missing; and
//! `POST /workouts` unless the folder already holds the same workout. All
//! three go to the credential's own athlete path. Nothing in the library is
//! updated or deleted.

use serde_json::Value;

use super::{send_traced, status_error, CallAuth, IntervalsIcuProvider, Target};
use crate::errors::{AppError, AppResult};
use crate::intervals_icu_library::{
    find_folder, find_same_workout, folder_body, library_workout_body, CreatedLibraryItem,
    LibraryFolder,
};
use crate::models::{LibraryWorkoutRef, PlannedSession};

/// Longest folder name a save creates or looks for.
const MAX_FOLDER_NAME_CHARS: usize = 100;

impl IntervalsIcuProvider {
    /// Keep `session` in the account's own library, in its plain folder
    /// called `folder`, creating the folder when the library has none.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error on a provider built for a coached
    /// athlete, whose library is not the connected account's, or for an
    /// empty or overlong folder name; otherwise the error of a request that
    /// fails or is refused.
    pub(super) async fn save_to_library(
        &self,
        session: &PlannedSession,
        folder: &str,
    ) -> AppResult<LibraryWorkoutRef> {
        if self.subject.is_some() {
            return Err(AppError::invalid_input(
                "An Intervals.icu workout library is the connected account's own: it is saved to \
                 through that account, never through a coached athlete's",
            ));
        }
        let folder = folder.trim();
        let length = folder.chars().count();
        if length == 0 || length > MAX_FOLDER_NAME_CHARS {
            return Err(AppError::invalid_input(format!(
                "folder must name a library folder in 1 to {MAX_FOLDER_NAME_CHARS} characters"
            )));
        }
        let folders: Vec<LibraryFolder> = self
            .get_athlete_json("list_folders", "/folders", &[], Target::Athlete)
            .await?;
        let auth = self.require_credentials().await?;
        let (folder_id, folder_name, folder_created) =
            if let Some(found) = find_folder(&folders, folder) {
                let body = library_workout_body(session, found.id);
                if let Some(same) = find_same_workout(found, &body) {
                    return Ok(LibraryWorkoutRef {
                        workout_id: same.id.to_string(),
                        folder_id: found.id.to_string(),
                        folder: found.name.clone(),
                        folder_created: false,
                        already_saved: true,
                    });
                }
                (found.id, found.name.clone(), false)
            } else {
                let created = self
                    .post_athlete_json(&auth, "create_folder", "/folders", &folder_body(folder))
                    .await?;
                (created.id, folder.to_owned(), true)
            };
        let created = self
            .post_athlete_json(
                &auth,
                "create_workout",
                "/workouts",
                &library_workout_body(session, folder_id),
            )
            .await?;
        Ok(LibraryWorkoutRef {
            workout_id: created.id.to_string(),
            folder_id: folder_id.to_string(),
            folder: folder_name,
            folder_created,
            already_saved: false,
        })
    }

    /// `POST` `body` to the credential's own athlete-scoped `suffix` and read
    /// back the id of what it created.
    ///
    /// # Errors
    ///
    /// Returns the transport failure, [`status_error`] for a refused call, or
    /// the decode failure.
    async fn post_athlete_json(
        &self,
        auth: &CallAuth,
        op: &'static str,
        suffix: &str,
        body: &Value,
    ) -> AppResult<CreatedLibraryItem> {
        let url = self.athlete_url(auth, suffix);
        let req = auth
            .authorize(self.http.post(&url))?
            .header("Accept", "application/json")
            .json(body);
        self.admit_request().await?;
        let response = send_traced(req, op, &url)
            .await
            .map_err(|e| AppError::external_service("intervals_icu", format!("{op}: {e}")))?;
        if !response.status().is_success() {
            return Err(status_error(op, response.status()));
        }
        response
            .json()
            .await
            .map_err(|e| AppError::external_service("intervals_icu", format!("{op} decode: {e}")))
    }
}
