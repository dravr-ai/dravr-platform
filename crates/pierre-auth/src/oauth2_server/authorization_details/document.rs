// ABOUTME: The one typed reader of a grant's stored authorization_details: a data-access document or an agent-write one
// ABOUTME: Every reader goes through it, so each must handle both kinds and none reads a write grant as a read ceiling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use serde::ser::Error as _;
use serde::{Serialize, Serializer};
use serde_json::Value;

use super::agent_write::{ResolvedAgentWrite, AGENT_WRITE_TYPE};
use super::ResolvedDetails;

/// What a grant's stored `authorization_details` grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantDocument {
    /// A preset read grant, narrowed by its constraints
    DataAccess(ResolvedDetails),
    /// An agent-write grant: records a client may propose, never read
    AgentWrite(ResolvedAgentWrite),
}

impl GrantDocument {
    /// The document a grant stored, read by its first detail's `type`.
    ///
    /// # Errors
    /// An internal error when `document` is neither kind this server wrote.
    pub fn from_document(document: &str) -> AppResult<Self> {
        let is_agent_write = serde_json::from_str::<Value>(document)
            .ok()
            .and_then(|value| {
                value
                    .get(0)
                    .and_then(|detail| detail.get("type"))
                    .and_then(Value::as_str)
                    .map(|kind| kind == AGENT_WRITE_TYPE)
            })
            .unwrap_or(false);
        if is_agent_write {
            ResolvedAgentWrite::from_document(document).map(Self::AgentWrite)
        } else {
            ResolvedDetails::from_document(document).map(Self::DataAccess)
        }
    }

    /// The read grant's details; `None` for an agent-write grant.
    #[must_use]
    pub fn data_access(self) -> Option<ResolvedDetails> {
        match self {
            Self::DataAccess(details) => Some(details),
            Self::AgentWrite(_) => None,
        }
    }

    /// The agent-write detail; `None` for a read grant.
    #[must_use]
    pub fn agent_write(self) -> Option<ResolvedAgentWrite> {
        match self {
            Self::AgentWrite(write) => Some(write),
            Self::DataAccess(_) => None,
        }
    }
}

/// The document as RFC 9396 §7 echoes it: the array the grant stored.
impl Serialize for GrantDocument {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::DataAccess(details) => details.serialize(serializer),
            Self::AgentWrite(write) => {
                let document = write.to_document().map_err(S::Error::custom)?;
                serde_json::from_str::<Value>(&document)
                    .map_err(S::Error::custom)?
                    .serialize(serializer)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth2_server::authorization_details::{preset_defaults, Preset};

    #[test]
    fn each_kind_reads_as_itself() {
        let read = preset_defaults(Preset::TrainingCoach, "https://mcp.example.test/mcp");
        let document = read.to_document().unwrap();
        assert_eq!(
            GrantDocument::from_document(&document).unwrap(),
            GrantDocument::DataAccess(read)
        );
        let write =
            ResolvedAgentWrite::for_write_scope(&[dravr_barrage::Stream::RecoveryDaily]).unwrap();
        let document = write.to_document().unwrap();
        assert_eq!(
            GrantDocument::from_document(&document).unwrap(),
            GrantDocument::AgentWrite(write)
        );
        assert!(GrantDocument::from_document("not json").is_err());
        assert!(GrantDocument::from_document("[]").is_err());
    }
}
