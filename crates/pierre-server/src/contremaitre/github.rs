// ABOUTME: GitHub Contents API client for reading and writing prompt files
// ABOUTME: Supports read, commit (create/update), and delete operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use pierre_core::http_client::api_client;

use super::errors::ContremaitreError;
use super::manifest::{parse_manifest, Manifest};

/// GitHub API base URL.
const GITHUB_API_BASE: &str = "https://api.github.com";

/// A file fetched from the GitHub Contents API.
#[derive(Debug, Clone)]
pub struct GitHubFile {
    /// Decoded file content
    pub content: String,
    /// Git blob SHA (required for update/delete operations)
    pub sha: String,
    /// File path in the repository
    pub path: String,
}

/// Client for the GitHub Contents API (read, write, delete files).
///
/// Uses the shared `reqwest::Client` from `pierre_core::http_client` to avoid
/// creating a new HTTP connection pool per request.
pub struct GitHubContentsClient {
    repo: String,
    branch: String,
    pat: String,
}

/// Response from GitHub GET /repos/{owner}/{repo}/contents/{path}
#[derive(Deserialize)]
struct ContentsResponse {
    content: Option<String>,
    sha: String,
    path: String,
}

/// Request body for PUT /repos/{owner}/{repo}/contents/{path}
#[derive(Serialize)]
struct PutContentsRequest {
    message: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha: Option<String>,
    branch: String,
}

/// Request body for DELETE /repos/{owner}/{repo}/contents/{path}
#[derive(Serialize)]
struct DeleteContentsRequest {
    message: String,
    sha: String,
    branch: String,
}

/// Error response from GitHub API
#[derive(Deserialize)]
struct GitHubErrorResponse {
    message: String,
}

impl GitHubContentsClient {
    /// Create a new client for the given repository.
    #[must_use]
    pub fn new(repo: String, branch: String, pat: String) -> Self {
        Self { repo, branch, pat }
    }

    /// Read a file from the repository.
    ///
    /// Returns the decoded content and the Git blob SHA (needed for updates).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be fetched from GitHub (network failure,
    /// 404, base64 decoding failure, or UTF-8 decoding failure).
    pub async fn read_file(&self, path: &str) -> Result<GitHubFile, ContremaitreError> {
        let url = format!(
            "{GITHUB_API_BASE}/repos/{}/contents/{}?ref={}",
            self.repo, path, self.branch
        );

        let client = api_client();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.pat))
            .header("Accept", "application/vnd.github.v3+json")
            .header("User-Agent", "pierre-mcp-server")
            .send()
            .await?;

        let status = response.status().as_u16();
        if status != 200 {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<GitHubErrorResponse>(&body)
                .map(|e| e.message)
                .unwrap_or(body);
            return Err(ContremaitreError::GitHubApi { status, message });
        }

        let resp: ContentsResponse =
            response
                .json()
                .await
                .map_err(|e| ContremaitreError::GitHubApi {
                    status: 200,
                    message: format!("failed to parse response: {e}"),
                })?;

        let content_b64 = resp.content.unwrap_or_default().replace('\n', "");
        let content_bytes =
            STANDARD
                .decode(&content_b64)
                .map_err(|e| ContremaitreError::GitHubApi {
                    status: 200,
                    message: format!("base64 decode error: {e}"),
                })?;

        let content =
            String::from_utf8(content_bytes).map_err(|e| ContremaitreError::GitHubApi {
                status: 200,
                message: format!("UTF-8 decode error: {e}"),
            })?;

        Ok(GitHubFile {
            content,
            sha: resp.sha,
            path: resp.path,
        })
    }

    /// Read and parse the manifest.json from the repository root.
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest cannot be read or parsed.
    pub async fn read_manifest(&self) -> Result<Manifest, ContremaitreError> {
        let file = self.read_file("manifest.json").await?;
        parse_manifest(&file.content)
    }

    /// Commit a file to the repository (create or update).
    ///
    /// If `existing_sha` is provided, this is an update to an existing file.
    /// If `None`, this creates a new file. Returns the commit SHA on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be committed to GitHub.
    pub async fn commit_file(
        &self,
        path: &str,
        content: &str,
        message: &str,
        existing_sha: Option<&str>,
    ) -> Result<String, ContremaitreError> {
        let url = format!("{GITHUB_API_BASE}/repos/{}/contents/{}", self.repo, path);

        let encoded = STANDARD.encode(content.as_bytes());

        let body = PutContentsRequest {
            message: message.to_owned(),
            content: encoded,
            sha: existing_sha.map(str::to_owned),
            branch: self.branch.clone(),
        };

        let client = api_client();
        let response = client
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.pat))
            .header("Accept", "application/vnd.github.v3+json")
            .header("User-Agent", "pierre-mcp-server")
            .json(&body)
            .send()
            .await?;

        let status = response.status().as_u16();
        if status != 200 && status != 201 {
            let resp_body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<GitHubErrorResponse>(&resp_body)
                .map(|e| e.message)
                .unwrap_or(resp_body);
            return Err(ContremaitreError::GitHubApi { status, message });
        }

        // Extract commit SHA from response
        let resp: serde_json::Value =
            response
                .json()
                .await
                .map_err(|e| ContremaitreError::GitHubApi {
                    status,
                    message: format!("failed to parse commit response: {e}"),
                })?;

        let commit_sha = resp["commit"]["sha"]
            .as_str()
            .unwrap_or("unknown")
            .to_owned();

        Ok(commit_sha)
    }

    /// Delete a file from the repository.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be deleted from GitHub.
    pub async fn delete_file(
        &self,
        path: &str,
        message: &str,
        sha: &str,
    ) -> Result<(), ContremaitreError> {
        let url = format!("{GITHUB_API_BASE}/repos/{}/contents/{}", self.repo, path);

        let body = DeleteContentsRequest {
            message: message.to_owned(),
            sha: sha.to_owned(),
            branch: self.branch.clone(),
        };

        let client = api_client();
        let response = client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.pat))
            .header("Accept", "application/vnd.github.v3+json")
            .header("User-Agent", "pierre-mcp-server")
            .json(&body)
            .send()
            .await?;

        let status = response.status().as_u16();
        if status != 200 {
            let resp_body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<GitHubErrorResponse>(&resp_body)
                .map(|e| e.message)
                .unwrap_or(resp_body);
            return Err(ContremaitreError::GitHubApi { status, message });
        }

        Ok(())
    }
}
