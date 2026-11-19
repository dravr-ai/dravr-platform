// ABOUTME: Catalogue keys of the refusals name_place answers with when it cannot name, rename or clear a place
// ABOUTME: One key per refusal; the texts live in dravr-contremaitre strings/, in every locale
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Keys of the `name_place` refusals (carnet#1484).
//!
//! An athlete names, renames or clears one of their places, either by the
//! place's id or by an activity that starts there. Each refusal says what
//! was missing, never an id, a name or a location.

/// Key: the athlete holds no such place, or the activity has no place to
/// clear. No format placeholders.
pub const KEY_NAME_PLACE_PLACE_NOT_FOUND: &str = "messaging.name_place.place_not_found";

/// Key: the athlete holds no activity with that id. No format placeholders.
pub const KEY_NAME_PLACE_ACTIVITY_NOT_FOUND: &str = "messaging.name_place.activity_not_found";

/// Key: the activity has no place and no recorded route to place it by. No
/// format placeholders.
pub const KEY_NAME_PLACE_ACTIVITY_HAS_NO_ROUTE: &str = "messaging.name_place.activity_has_no_route";

/// Key: the name is empty or too long once defanged. `{0}` is the longest
/// name accepted, in characters.
pub const KEY_NAME_PLACE_INVALID_NAME: &str = "messaging.name_place.invalid_name";
