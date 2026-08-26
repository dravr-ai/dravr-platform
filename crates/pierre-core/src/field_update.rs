// ABOUTME: Three-way update intent that tells an absent JSON key from an explicit null
// ABOUTME: Lets a PATCH-style request keep, clear, or replace one nullable column

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

/// What an update request asks of one nullable field.
///
/// A plain `Option<T>` on an update DTO collapses two different requests into
/// the same value: "the client never mentioned this field" and "the client
/// asked for this field to be empty" both arrive as `None`. A handler that
/// coalesces with `request.field.or(existing.field)` therefore preserves on
/// both — so a column that has been set once can never be cleared again.
///
/// `FieldUpdate` keeps the two apart. Declare the field as
/// `#[serde(default)] field: FieldUpdate<T>`: serde skips the deserializer for
/// an absent key and takes the [`Keep`](Self::Keep) default, while a present
/// key — `null` or a value — deserializes into [`Set`](Self::Set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldUpdate<T> {
    /// The key was absent from the request: leave the stored value alone.
    #[default]
    Keep,
    /// The key was present: `Some` writes that value, `None` clears the column.
    Set(Option<T>),
}

impl<T> FieldUpdate<T> {
    /// Resolve the request against what is currently stored.
    ///
    /// [`Keep`](Self::Keep) yields `existing` untouched; [`Set`](Self::Set)
    /// yields exactly what the request carried, `None` included, which is what
    /// makes a stored value clearable.
    pub fn resolve(self, existing: Option<T>) -> Option<T> {
        match self {
            Self::Keep => existing,
            Self::Set(value) => value,
        }
    }

    /// The concrete value this request assigns, if it assigns one.
    ///
    /// Both "left alone" and "cleared" answer `None`, so validation that only
    /// bounds-checks a supplied value reads the request through this.
    pub fn assigned(self) -> Option<T> {
        match self {
            Self::Keep => None,
            Self::Set(value) => value,
        }
    }

    /// Whether the request left this field alone.
    ///
    /// Used as a `skip_serializing_if` predicate so a serialized update keeps
    /// the key absent rather than emitting a `null` that would clear.
    pub const fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for FieldUpdate<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(deserializer).map(Self::Set)
    }
}

impl<T: Serialize> Serialize for FieldUpdate<T> {
    /// Writes the wire form of a *present* key: the value, or `null` to clear.
    ///
    /// [`Keep`](Self::Keep) has no wire form of its own — its meaning is the
    /// absence of the key — so pair the field with
    /// `skip_serializing_if = "FieldUpdate::is_keep"`, which drops it before
    /// this runs.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Keep | Self::Set(None) => serializer.serialize_none(),
            Self::Set(Some(value)) => serializer.serialize_some(value),
        }
    }
}
