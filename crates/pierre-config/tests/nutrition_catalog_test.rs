// ABOUTME: Pins the admin nutrition catalogue to the evidence it cites
// ABOUTME: The athlete protein knob spans Thomas 2016's 1.2-2.0 g/kg/day, names that source, defaults to the kernel's value
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An admin moving `nutrition.protein_athlete_g_per_kg` is bounded by the
//! catalogue's range, and reads its source to decide. The range was 1.4-2.5
//! under "Phillips 2011, ISSN Position Stand" — a paper that is not an ISSN
//! position stand, and a ceiling above anything the joint ACSM / AND / DC
//! statement supports for athletes in general.

use std::collections::HashMap;

use dravr_cageux::config::intelligence::IntelligenceConfig;
use pierre_config::admin_definitions::ParameterDefinition;
use pierre_config::nutrition_params::{
    athlete_protein_g_per_kg, register_nutrition, PROTEIN_ATHLETE_G_PER_KG_KEY,
};
use serde_json::json;

fn protein_athlete() -> ParameterDefinition {
    let mut defs = HashMap::new();
    register_nutrition(&mut defs);
    defs.remove(PROTEIN_ATHLETE_G_PER_KG_KEY)
        .expect("the athlete protein knob is registered")
}

#[test]
fn athlete_protein_spans_the_joint_position_statement_range() {
    let def = protein_athlete();
    let range = def.valid_range.expect("a bounded knob");
    assert_eq!(range.min.as_f64(), Some(1.2));
    assert_eq!(range.max.as_f64(), Some(2.0));

    let default = def.default_value.as_f64().expect("a numeric default");
    assert!(
        (1.2..=2.0).contains(&default),
        "the default must sit inside the range it is bounded by: {default}"
    );
    assert_eq!(def.units.as_deref(), Some("g/kg/day"));
}

#[test]
fn athlete_protein_names_the_source_of_its_range() {
    let basis = protein_athlete()
        .scientific_basis
        .expect("a sourced knob states its source");
    assert!(basis.contains("Thomas, Erdman & Burke 2016"), "{basis}");
    assert!(basis.contains("1.2-2.0 g/kg/day"), "{basis}");
    assert!(
        !basis.contains("Phillips 2011"),
        "the old citation named a paper that set neither this range nor a position stand: {basis}"
    );
}

/// The catalogue shows the value the calculator uses when no override is set:
/// the kernel's compiled one, read rather than retyped.
#[test]
fn athlete_protein_default_is_the_kernels_compiled_value() {
    let kernel = IntelligenceConfig::<true>::default()
        .nutrition
        .macronutrients
        .protein_athlete_g_per_kg;
    let def = protein_athlete();
    assert_eq!(def.key, "nutrition.protein_athlete_g_per_kg");
    assert_eq!(def.category, "nutrition");
    assert!(def.is_runtime_configurable);
    assert_eq!(def.default_value.as_f64(), Some(kernel));
    assert!((kernel - 1.8).abs() < f64::EPSILON, "{kernel}");
}

/// A resolved override is read back only when it is a target inside the
/// range; anything else names the key instead of prescribing it.
#[test]
fn a_resolved_athlete_protein_override_is_read_only_inside_the_range() {
    for inside in [1.2, 1.5, 2.0] {
        let read = athlete_protein_g_per_kg(&json!(inside)).unwrap();
        assert!(
            (read - inside).abs() < f64::EPSILON,
            "{inside} read as {read}"
        );
    }
    for outside in [
        json!(1.19),
        json!(2.01),
        json!(0.0),
        json!("1.5"),
        json!(null),
    ] {
        let err = athlete_protein_g_per_kg(&outside)
            .expect_err("a value no write path accepts is refused");
        assert!(
            err.to_string().contains(PROTEIN_ATHLETE_G_PER_KG_KEY),
            "the refusal names the key: {err}"
        );
    }
}
