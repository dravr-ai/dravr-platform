// ABOUTME: `pierre-cli oauth2-client create`: the confidential client a resource server introspects tokens with
// ABOUTME: Stores it through the OAuth client registry and prints its client_id and secret once, to stdout only

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A satellite resource server asks this server whether a token it holds is
//! still active (RFC 7662, `POST /oauth2/introspect`), authenticating as a
//! confidential client the admin names in `OAUTH2_INTROSPECTION_CLIENTS`.
//! A dynamic registration is never such a client, so the admin creates one
//! here, against the database.

use std::sync::Arc;

use clap::Subcommand;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::RepositoryRegistry;
use secrecy::ExposeSecret;

/// `pierre-cli oauth2-client`: OAuth clients the admin provisions.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum Oauth2ClientCommand {
    /// Create the confidential client a resource server introspects tokens with
    ///
    /// It authenticates with HTTP Basic (`client_secret_basic`), may use only
    /// the `client_credentials` grant, has no redirect URI and no scope, and
    /// never expires. Its secret is printed once; to rotate it, delete the
    /// client and create another.
    Create {
        /// The resource server's name, shown wherever the client is named
        #[arg(long)]
        name: String,
    },
}

/// Run one `oauth2-client` verb.
pub(crate) async fn dispatch(
    repos: &Arc<RepositoryRegistry>,
    action: Oauth2ClientCommand,
) -> AppResult<()> {
    match action {
        Oauth2ClientCommand::Create { name } => create(repos, &name).await,
    }
}

/// Create the introspection client named `name` and print its credentials.
async fn create(repos: &Arc<RepositoryRegistry>, name: &str) -> AppResult<()> {
    let (client_id, client_secret) =
        ClientRegistrationManager::new(Arc::clone(&repos.oauth2_server))
            .register_introspection_client(name)
            .await
            .map_err(|e| {
                AppError::invalid_input(format!(
                    "Could not create the client: {}",
                    e.error_description.as_deref().unwrap_or(&e.error)
                ))
            })?;
    // Exposed here alone, to stdout, once: the server keeps only its hash,
    // and tracing never sees it.
    println!("Created confidential OAuth client '{name}'");
    println!("client_id:     {client_id}");
    println!("client_secret: {}", client_secret.expose_secret());
    println!();
    println!("The secret is shown once and cannot be read back.");
    println!("Add the client_id to OAUTH2_INTROSPECTION_CLIENTS (comma-separated) on the server,");
    println!("then authenticate to POST /oauth2/introspect with HTTP Basic.");
    Ok(())
}
