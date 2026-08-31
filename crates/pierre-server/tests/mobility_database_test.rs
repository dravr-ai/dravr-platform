// ABOUTME: Unit tests for the mobility database module
// ABOUTME: Tests CRUD operations for stretching exercises and yoga poses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Mobility Database Unit Tests
//!
//! Tests the `MobilityRepository` operations on whichever backend the test factory opens:
//! - Stretching exercises: list, get, search, filter
//! - Yoga poses: list, get, search, filter by category/difficulty/recovery
//! - Activity-muscle mappings

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use pierre_core::models::mobility::{
    DifficultyLevel, ListStretchingFilter, ListYogaFilter, StretchingCategory, YogaCategory,
    YogaPoseType,
};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;

// ============================================================================
// Test Setup
// ============================================================================

/// Run one reference-data INSERT on whichever backend the test database is.
///
/// `stretching_exercises` and `yoga_poses` are catalogue tables with no
/// repository writer, so the fixtures are written with raw SQL. `$n`
/// placeholders parse on both engines and the timestamp is bound as a
/// `DateTime`, which `SQLite` stores as RFC 3339 text and `PostgreSQL` as
/// `TIMESTAMPTZ`.
macro_rules! execute_on {
    ($db:expr, $sql:expr, $($bind:expr),+ $(,)?) => {
        match $db {
            Database::SQLite(d) => {
                sqlx::query($sql)$(.bind($bind))+.execute(d.pool()).await.unwrap();
            }
            #[cfg(feature = "postgresql")]
            Database::PostgreSQL(d) => {
                sqlx::query($sql)$(.bind($bind))+.execute(d.pool()).await.unwrap();
            }
        }
    };
}

/// Insert a test stretching exercise
async fn insert_test_stretch(
    db: &Database,
    id: &str,
    name: &str,
    category: &str,
    difficulty: &str,
) {
    let now = Utc::now();
    execute_on!(
        db,
        r#"
        INSERT INTO stretching_exercises
        (id, name, description, category, difficulty, primary_muscles, secondary_muscles,
         duration_seconds, sets, recommended_for_activities, instructions, created_at, updated_at)
        VALUES ($1, $2, 'Test description', $3, $4, '["hamstrings"]', '["calves"]',
                30, 2, '["running"]', '["Step 1", "Step 2"]', $5, $5)
        "#,
        id,
        name,
        category,
        difficulty,
        now
    );
}

/// Insert a test yoga pose
async fn insert_test_pose(
    db: &Database,
    id: &str,
    name: &str,
    category: &str,
    difficulty: &str,
    recovery: &str,
) {
    let now = Utc::now();
    execute_on!(
        db,
        r#"
        INSERT INTO yoga_poses
        (id, english_name, sanskrit_name, description, benefits, category, difficulty, pose_type,
         primary_muscles, secondary_muscles, hold_duration_seconds, recommended_for_activities,
         recommended_for_recovery, instructions, created_at, updated_at)
        VALUES ($1, $2, NULL, 'Test description', '["benefit1"]', $3, $4, 'stretch',
                '["hamstrings"]', '["calves"]', 30, '["running"]', $5, '["Step 1"]', $6, $6)
        "#,
        id,
        name,
        category,
        difficulty,
        recovery,
        now
    );
}

// ============================================================================
// Enum Parsing Tests
// ============================================================================

#[test]
fn test_stretching_category_parsing() {
    assert_eq!(
        StretchingCategory::parse("static"),
        StretchingCategory::Static
    );
    assert_eq!(
        StretchingCategory::parse("dynamic"),
        StretchingCategory::Dynamic
    );
    assert_eq!(StretchingCategory::parse("pnf"), StretchingCategory::Pnf);
    assert_eq!(
        StretchingCategory::parse("ballistic"),
        StretchingCategory::Ballistic
    );

    // Case-insensitive
    assert_eq!(
        StretchingCategory::parse("Static"),
        StretchingCategory::Static
    );
    assert_eq!(
        StretchingCategory::parse("DYNAMIC"),
        StretchingCategory::Dynamic
    );

    // Unknown defaults to Static
    assert_eq!(
        StretchingCategory::parse("unknown"),
        StretchingCategory::Static
    );
}

#[test]
fn test_difficulty_level_parsing() {
    assert_eq!(
        DifficultyLevel::parse("beginner"),
        DifficultyLevel::Beginner
    );
    assert_eq!(
        DifficultyLevel::parse("intermediate"),
        DifficultyLevel::Intermediate
    );
    assert_eq!(
        DifficultyLevel::parse("advanced"),
        DifficultyLevel::Advanced
    );

    // Case-insensitive
    assert_eq!(
        DifficultyLevel::parse("Beginner"),
        DifficultyLevel::Beginner
    );
    assert_eq!(
        DifficultyLevel::parse("ADVANCED"),
        DifficultyLevel::Advanced
    );

    // Unknown defaults to Beginner
    assert_eq!(DifficultyLevel::parse("expert"), DifficultyLevel::Beginner);
}

#[test]
fn test_yoga_category_parsing() {
    assert_eq!(YogaCategory::parse("standing"), YogaCategory::Standing);
    assert_eq!(YogaCategory::parse("seated"), YogaCategory::Seated);
    assert_eq!(YogaCategory::parse("supine"), YogaCategory::Supine);
    assert_eq!(YogaCategory::parse("prone"), YogaCategory::Prone);
    assert_eq!(YogaCategory::parse("inversion"), YogaCategory::Inversion);
    assert_eq!(YogaCategory::parse("balance"), YogaCategory::Balance);
    assert_eq!(YogaCategory::parse("twist"), YogaCategory::Twist);

    // Unknown defaults to Standing
    assert_eq!(YogaCategory::parse("unknown"), YogaCategory::Standing);
}

#[test]
fn test_yoga_pose_type_parsing() {
    assert_eq!(YogaPoseType::parse("stretch"), YogaPoseType::Stretch);
    assert_eq!(YogaPoseType::parse("strength"), YogaPoseType::Strength);
    assert_eq!(YogaPoseType::parse("balance"), YogaPoseType::Balance);
    assert_eq!(YogaPoseType::parse("relaxation"), YogaPoseType::Relaxation);
    assert_eq!(YogaPoseType::parse("breathing"), YogaPoseType::Breathing);

    // Unknown defaults to Stretch
    assert_eq!(YogaPoseType::parse("cardio"), YogaPoseType::Stretch);
}

#[test]
fn test_stretching_category_as_str() {
    assert_eq!(StretchingCategory::Static.as_str(), "static");
    assert_eq!(StretchingCategory::Dynamic.as_str(), "dynamic");
    assert_eq!(StretchingCategory::Pnf.as_str(), "pnf");
    assert_eq!(StretchingCategory::Ballistic.as_str(), "ballistic");
}

#[test]
fn test_difficulty_level_as_str() {
    assert_eq!(DifficultyLevel::Beginner.as_str(), "beginner");
    assert_eq!(DifficultyLevel::Intermediate.as_str(), "intermediate");
    assert_eq!(DifficultyLevel::Advanced.as_str(), "advanced");
}

#[test]
fn test_yoga_category_as_str() {
    assert_eq!(YogaCategory::Standing.as_str(), "standing");
    assert_eq!(YogaCategory::Seated.as_str(), "seated");
    assert_eq!(YogaCategory::Supine.as_str(), "supine");
    assert_eq!(YogaCategory::Prone.as_str(), "prone");
    assert_eq!(YogaCategory::Inversion.as_str(), "inversion");
    assert_eq!(YogaCategory::Balance.as_str(), "balance");
    assert_eq!(YogaCategory::Twist.as_str(), "twist");
}

// ============================================================================
// Stretching Exercise Tests
// ============================================================================

#[tokio::test]
async fn test_list_stretching_empty() {
    let db = create_test_db().await.unwrap();
    let manager = db.repositories().mobility;

    let filter = ListStretchingFilter::default();
    let exercises = manager.list_stretching_exercises(&filter).await.unwrap();

    assert!(exercises.is_empty());
}

#[tokio::test]
async fn test_list_stretching_exercises() {
    let db = create_test_db().await.unwrap();

    // Insert test data
    insert_test_stretch(&db, "stretch-1", "Hamstring Stretch", "static", "beginner").await;
    insert_test_stretch(&db, "stretch-2", "Quad Stretch", "static", "intermediate").await;
    insert_test_stretch(&db, "stretch-3", "Leg Swings", "dynamic", "beginner").await;

    let manager = db.repositories().mobility;

    let filter = ListStretchingFilter::default();
    let exercises = manager.list_stretching_exercises(&filter).await.unwrap();

    assert_eq!(exercises.len(), 3);
}

#[tokio::test]
async fn test_list_stretching_by_category() {
    let db = create_test_db().await.unwrap();

    insert_test_stretch(&db, "stretch-1", "Hamstring Stretch", "static", "beginner").await;
    insert_test_stretch(&db, "stretch-2", "Leg Swings", "dynamic", "beginner").await;

    let manager = db.repositories().mobility;

    let filter = ListStretchingFilter {
        category: Some(StretchingCategory::Static),
        ..Default::default()
    };
    let exercises = manager.list_stretching_exercises(&filter).await.unwrap();

    assert_eq!(exercises.len(), 1);
    assert_eq!(exercises[0].name, "Hamstring Stretch");
    assert_eq!(exercises[0].category, StretchingCategory::Static);
}

#[tokio::test]
async fn test_list_stretching_by_difficulty() {
    let db = create_test_db().await.unwrap();

    insert_test_stretch(&db, "stretch-1", "Easy Stretch", "static", "beginner").await;
    insert_test_stretch(&db, "stretch-2", "Medium Stretch", "static", "intermediate").await;
    insert_test_stretch(&db, "stretch-3", "Hard Stretch", "static", "advanced").await;

    let manager = db.repositories().mobility;

    let filter = ListStretchingFilter {
        difficulty: Some(DifficultyLevel::Beginner),
        ..Default::default()
    };
    let exercises = manager.list_stretching_exercises(&filter).await.unwrap();

    assert_eq!(exercises.len(), 1);
    assert_eq!(exercises[0].difficulty, DifficultyLevel::Beginner);
}

#[tokio::test]
async fn test_get_stretching_exercise() {
    let db = create_test_db().await.unwrap();

    insert_test_stretch(&db, "stretch-123", "Test Stretch", "static", "beginner").await;

    let manager = db.repositories().mobility;

    let exercise = manager
        .get_stretching_exercise("stretch-123")
        .await
        .unwrap();

    assert!(exercise.is_some());
    let exercise = exercise.unwrap();
    assert_eq!(exercise.id, "stretch-123");
    assert_eq!(exercise.name, "Test Stretch");
}

#[tokio::test]
async fn test_get_stretching_exercise_not_found() {
    let db = create_test_db().await.unwrap();
    let manager = db.repositories().mobility;

    let result = manager
        .get_stretching_exercise("nonexistent")
        .await
        .unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn test_search_stretching_exercises() {
    let db = create_test_db().await.unwrap();

    insert_test_stretch(&db, "stretch-1", "Hamstring Stretch", "static", "beginner").await;
    insert_test_stretch(&db, "stretch-2", "Quad Stretch", "static", "beginner").await;
    insert_test_stretch(&db, "stretch-3", "Calf Raise", "dynamic", "beginner").await;

    let manager = db.repositories().mobility;

    let results = manager
        .search_stretching_exercises("stretch", None)
        .await
        .unwrap();

    assert_eq!(results.len(), 2); // Only matches "Hamstring Stretch" and "Quad Stretch"
}

#[tokio::test]
async fn test_list_stretching_with_pagination() {
    let db = create_test_db().await.unwrap();

    for i in 0..10 {
        insert_test_stretch(
            &db,
            &format!("stretch-{i}"),
            &format!("Stretch {i}"),
            "static",
            "beginner",
        )
        .await;
    }

    let manager = db.repositories().mobility;

    let filter = ListStretchingFilter {
        limit: Some(3),
        offset: Some(0),
        ..Default::default()
    };
    let page1 = manager.list_stretching_exercises(&filter).await.unwrap();
    assert_eq!(page1.len(), 3);

    let filter = ListStretchingFilter {
        limit: Some(3),
        offset: Some(3),
        ..Default::default()
    };
    let page2 = manager.list_stretching_exercises(&filter).await.unwrap();
    assert_eq!(page2.len(), 3);
}

// ============================================================================
// Yoga Pose Tests
// ============================================================================

#[tokio::test]
async fn test_list_yoga_empty() {
    let db = create_test_db().await.unwrap();
    let manager = db.repositories().mobility;

    let filter = ListYogaFilter::default();
    let poses = manager.list_yoga_poses(&filter).await.unwrap();

    assert!(poses.is_empty());
}

#[tokio::test]
async fn test_list_yoga_poses() {
    let db = create_test_db().await.unwrap();

    insert_test_pose(
        &db,
        "pose-1",
        "Warrior I",
        "standing",
        "beginner",
        r#"["post_cardio"]"#,
    )
    .await;
    insert_test_pose(
        &db,
        "pose-2",
        "Downward Dog",
        "inversion",
        "beginner",
        r#"["rest_day"]"#,
    )
    .await;
    insert_test_pose(
        &db,
        "pose-3",
        "Child's Pose",
        "seated",
        "beginner",
        r#"["post_cardio", "rest_day"]"#,
    )
    .await;

    let manager = db.repositories().mobility;

    let filter = ListYogaFilter::default();
    let poses = manager.list_yoga_poses(&filter).await.unwrap();

    assert_eq!(poses.len(), 3);
}

#[tokio::test]
async fn test_list_yoga_by_category() {
    let db = create_test_db().await.unwrap();

    insert_test_pose(
        &db,
        "pose-1",
        "Warrior I",
        "standing",
        "beginner",
        r#"["post_cardio"]"#,
    )
    .await;
    insert_test_pose(
        &db,
        "pose-2",
        "Tree Pose",
        "balance",
        "intermediate",
        r#"["rest_day"]"#,
    )
    .await;

    let manager = db.repositories().mobility;

    let filter = ListYogaFilter {
        category: Some(YogaCategory::Standing),
        ..Default::default()
    };
    let poses = manager.list_yoga_poses(&filter).await.unwrap();

    assert_eq!(poses.len(), 1);
    assert_eq!(poses[0].english_name, "Warrior I");
    assert_eq!(poses[0].category, YogaCategory::Standing);
}

#[tokio::test]
async fn test_list_yoga_by_difficulty() {
    let db = create_test_db().await.unwrap();

    insert_test_pose(&db, "pose-1", "Easy Pose", "seated", "beginner", r"[]").await;
    insert_test_pose(&db, "pose-2", "Crow Pose", "balance", "advanced", r"[]").await;

    let manager = db.repositories().mobility;

    let filter = ListYogaFilter {
        difficulty: Some(DifficultyLevel::Advanced),
        ..Default::default()
    };
    let poses = manager.list_yoga_poses(&filter).await.unwrap();

    assert_eq!(poses.len(), 1);
    assert_eq!(poses[0].difficulty, DifficultyLevel::Advanced);
}

#[tokio::test]
async fn test_get_yoga_pose() {
    let db = create_test_db().await.unwrap();

    insert_test_pose(&db, "pose-123", "Test Pose", "standing", "beginner", r"[]").await;

    let manager = db.repositories().mobility;

    let pose = manager.get_yoga_pose("pose-123").await.unwrap();

    assert!(pose.is_some());
    let pose = pose.unwrap();
    assert_eq!(pose.id, "pose-123");
    assert_eq!(pose.english_name, "Test Pose");
}

#[tokio::test]
async fn test_get_yoga_pose_not_found() {
    let db = create_test_db().await.unwrap();
    let manager = db.repositories().mobility;

    let result = manager.get_yoga_pose("nonexistent").await.unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn test_search_yoga_poses() {
    let db = create_test_db().await.unwrap();

    insert_test_pose(&db, "pose-1", "Warrior I", "standing", "beginner", r"[]").await;
    insert_test_pose(&db, "pose-2", "Warrior II", "standing", "beginner", r"[]").await;
    insert_test_pose(
        &db,
        "pose-3",
        "Downward Dog",
        "inversion",
        "beginner",
        r"[]",
    )
    .await;

    let manager = db.repositories().mobility;

    let results = manager.search_yoga_poses("warrior", None).await.unwrap();

    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn test_get_poses_for_recovery() {
    let db = create_test_db().await.unwrap();

    insert_test_pose(
        &db,
        "pose-1",
        "Recovery Pose 1",
        "supine",
        "beginner",
        r#"["post_cardio"]"#,
    )
    .await;
    insert_test_pose(
        &db,
        "pose-2",
        "Recovery Pose 2",
        "seated",
        "beginner",
        r#"["post_cardio", "rest_day"]"#,
    )
    .await;
    insert_test_pose(
        &db,
        "pose-3",
        "Other Pose",
        "standing",
        "beginner",
        r#"["morning"]"#,
    )
    .await;

    let manager = db.repositories().mobility;

    let poses = manager
        .get_poses_for_recovery("post_cardio", None)
        .await
        .unwrap();

    assert_eq!(poses.len(), 2);
}

#[tokio::test]
async fn test_list_yoga_with_pagination() {
    let db = create_test_db().await.unwrap();

    for i in 0..10 {
        insert_test_pose(
            &db,
            &format!("pose-{i}"),
            &format!("Pose {i}"),
            "standing",
            "beginner",
            r"[]",
        )
        .await;
    }

    let manager = db.repositories().mobility;

    let filter = ListYogaFilter {
        limit: Some(5),
        offset: Some(0),
        ..Default::default()
    };
    let page1 = manager.list_yoga_poses(&filter).await.unwrap();
    assert_eq!(page1.len(), 5);

    let filter = ListYogaFilter {
        limit: Some(5),
        offset: Some(5),
        ..Default::default()
    };
    let page2 = manager.list_yoga_poses(&filter).await.unwrap();
    assert_eq!(page2.len(), 5);
}

// ============================================================================
// Combined Filter Tests
// ============================================================================

#[tokio::test]
async fn test_list_stretching_multiple_filters() {
    let db = create_test_db().await.unwrap();

    insert_test_stretch(&db, "stretch-1", "Static Beginner", "static", "beginner").await;
    insert_test_stretch(&db, "stretch-2", "Static Advanced", "static", "advanced").await;
    insert_test_stretch(&db, "stretch-3", "Dynamic Beginner", "dynamic", "beginner").await;

    let manager = db.repositories().mobility;

    let filter = ListStretchingFilter {
        category: Some(StretchingCategory::Static),
        difficulty: Some(DifficultyLevel::Beginner),
        ..Default::default()
    };
    let exercises = manager.list_stretching_exercises(&filter).await.unwrap();

    assert_eq!(exercises.len(), 1);
    assert_eq!(exercises[0].name, "Static Beginner");
}

#[tokio::test]
async fn test_list_yoga_multiple_filters() {
    let db = create_test_db().await.unwrap();

    insert_test_pose(
        &db,
        "pose-1",
        "Standing Beginner",
        "standing",
        "beginner",
        r"[]",
    )
    .await;
    insert_test_pose(
        &db,
        "pose-2",
        "Standing Advanced",
        "standing",
        "advanced",
        r"[]",
    )
    .await;
    insert_test_pose(
        &db,
        "pose-3",
        "Seated Beginner",
        "seated",
        "beginner",
        r"[]",
    )
    .await;

    let manager = db.repositories().mobility;

    let filter = ListYogaFilter {
        category: Some(YogaCategory::Standing),
        difficulty: Some(DifficultyLevel::Beginner),
        ..Default::default()
    };
    let poses = manager.list_yoga_poses(&filter).await.unwrap();

    assert_eq!(poses.len(), 1);
    assert_eq!(poses[0].english_name, "Standing Beginner");
}

// ============================================================================
// Round-Trip Tests (as_str -> parse)
// ============================================================================

#[test]
fn test_stretching_category_round_trip() {
    let categories = [
        StretchingCategory::Static,
        StretchingCategory::Dynamic,
        StretchingCategory::Pnf,
        StretchingCategory::Ballistic,
    ];

    for category in categories {
        let serialized = category.as_str();
        let deserialized = StretchingCategory::parse(serialized);
        assert_eq!(category, deserialized, "Round-trip failed for {serialized}");
    }
}

#[test]
fn test_difficulty_level_round_trip() {
    let levels = [
        DifficultyLevel::Beginner,
        DifficultyLevel::Intermediate,
        DifficultyLevel::Advanced,
    ];

    for level in levels {
        let serialized = level.as_str();
        let deserialized = DifficultyLevel::parse(serialized);
        assert_eq!(level, deserialized, "Round-trip failed for {serialized}");
    }
}

#[test]
fn test_yoga_category_round_trip() {
    let categories = [
        YogaCategory::Standing,
        YogaCategory::Seated,
        YogaCategory::Supine,
        YogaCategory::Prone,
        YogaCategory::Inversion,
        YogaCategory::Balance,
        YogaCategory::Twist,
    ];

    for category in categories {
        let serialized = category.as_str();
        let deserialized = YogaCategory::parse(serialized);
        assert_eq!(category, deserialized, "Round-trip failed for {serialized}");
    }
}

#[test]
fn test_yoga_pose_type_round_trip() {
    let types = [
        YogaPoseType::Stretch,
        YogaPoseType::Strength,
        YogaPoseType::Balance,
        YogaPoseType::Relaxation,
        YogaPoseType::Breathing,
    ];

    for pose_type in types {
        let serialized = pose_type.as_str();
        let deserialized = YogaPoseType::parse(serialized);
        assert_eq!(
            pose_type, deserialized,
            "Round-trip failed for {serialized}"
        );
    }
}
