// ABOUTME: Mobility data seeding utility for Pierre MCP Server
// ABOUTME: Seeds stretching exercises, yoga poses, and activity-muscle mappings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Mobility data seeder for Pierre MCP Server.
//!
//! This binary creates the default stretching exercises, yoga poses, and
//! activity-muscle mappings for the mobility feature.
//!
//! Usage:
//! ```bash
//! # Seed mobility data (uses DATABASE_URL from environment)
//! cargo run --bin seed-mobility
//!
//! # Override database URL
//! cargo run --bin seed-mobility -- --database-url sqlite:./data/users.db
//!
//! # Verbose output
//! cargo run --bin seed-mobility -- -v
//!
//! # Force re-seed (replaces existing data)
//! cargo run --bin seed-mobility -- --force
//! ```

use std::collections::HashMap;

use chrono::Utc;
use clap::Parser;
use pierre_core::errors::AppResult;
use pierre_core::models::mobility::{
    ActivityMuscleMapping, DifficultyLevel, StretchingCategory, StretchingExercise, YogaCategory,
    YogaPose, YogaPoseType,
};
use pierre_database::plugins::factory::Database;
use pierre_database::repositories::{SeedTable, SeederRepository};
use std::env;
use tracing::info;
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "seed-mobility",
    about = "Pierre MCP Server Mobility Data Seeder",
    long_about = "Create stretching exercises, yoga poses, and activity-muscle mappings for the Pierre Fitness app"
)]
struct SeedArgs {
    /// Database URL override
    #[arg(long)]
    database_url: Option<String>,

    /// Force re-seed even if data already exists
    #[arg(long)]
    force: bool,

    /// Enable verbose logging
    #[arg(long, short = 'v')]
    verbose: bool,
}

// ============================================================================
// Stretching Exercise Data
// ============================================================================

struct StretchingData {
    name: &'static str,
    description: &'static str,
    category: &'static str,
    difficulty: &'static str,
    primary_muscles: &'static [&'static str],
    secondary_muscles: &'static [&'static str],
    duration_seconds: i64,
    repetitions: Option<i64>,
    sets: i64,
    recommended_for_activities: &'static [&'static str],
    contraindications: &'static [&'static str],
    instructions: &'static [&'static str],
    cues: &'static [&'static str],
}

const STRETCHING_EXERCISES: &[StretchingData] = &[
    // Running-focused stretches
    StretchingData {
        name: "Standing Quad Stretch",
        description: "A classic standing stretch targeting the quadriceps muscles. Essential for runners to maintain knee health and stride efficiency.",
        category: "static",
        difficulty: "beginner",
        primary_muscles: &["quadriceps"],
        secondary_muscles: &["hip_flexors"],
        duration_seconds: 30,
        repetitions: None,
        sets: 2,
        recommended_for_activities: &["running", "cycling", "hiking"],
        contraindications: &["knee_injury", "balance_issues"],
        instructions: &[
            "Stand on one leg, using a wall or chair for balance if needed",
            "Bend your other knee and grab your ankle behind you",
            "Pull your heel toward your glutes",
            "Keep your knees close together",
            "Hold for 30 seconds, then switch legs"
        ],
        cues: &["Keep standing knee slightly bent", "Engage your core", "Squeeze glutes to deepen the stretch"],
    },
    StretchingData {
        name: "Standing Calf Stretch",
        description: "Targets the gastrocnemius and soleus muscles. Critical for runners and anyone who spends time on their feet.",
        category: "static",
        difficulty: "beginner",
        primary_muscles: &["calves"],
        secondary_muscles: &["achilles"],
        duration_seconds: 30,
        repetitions: None,
        sets: 2,
        recommended_for_activities: &["running", "hiking", "walking"],
        contraindications: &["achilles_tendinitis"],
        instructions: &[
            "Face a wall and place your hands on it at shoulder height",
            "Step one leg back, keeping it straight with heel on the ground",
            "Bend your front knee and lean into the wall",
            "Feel the stretch in your back calf",
            "Hold for 30 seconds, then switch legs"
        ],
        cues: &["Keep back heel pressed down", "Back leg should be straight", "Don't bounce"],
    },
    StretchingData {
        name: "Hip Flexor Lunge Stretch",
        description: "Deep stretch for the iliopsoas and rectus femoris. Essential for athletes who sit frequently or run regularly.",
        category: "static",
        difficulty: "intermediate",
        primary_muscles: &["hip_flexors"],
        secondary_muscles: &["quadriceps", "psoas"],
        duration_seconds: 45,
        repetitions: None,
        sets: 2,
        recommended_for_activities: &["running", "cycling", "desk_work"],
        contraindications: &["knee_injury", "hip_replacement"],
        instructions: &[
            "Kneel on one knee with the other foot in front",
            "Keep your torso upright and core engaged",
            "Shift your weight forward, keeping back knee down",
            "You should feel a stretch in the front of your back hip",
            "For a deeper stretch, raise your arm on the same side as your back leg"
        ],
        cues: &["Tuck your tailbone slightly", "Don't arch your lower back", "Keep front knee over ankle"],
    },
    StretchingData {
        name: "Pigeon Pose Stretch",
        description: "Deep hip opener targeting the piriformis and glutes. Excellent for runners experiencing IT band issues.",
        category: "static",
        difficulty: "intermediate",
        primary_muscles: &["glutes", "piriformis"],
        secondary_muscles: &["hip_rotators"],
        duration_seconds: 60,
        repetitions: None,
        sets: 1,
        recommended_for_activities: &["running", "cycling", "sitting"],
        contraindications: &["knee_injury", "hip_injury"],
        instructions: &[
            "Start in a tabletop position on hands and knees",
            "Bring your right knee forward to your right wrist",
            "Slide your left leg back, straightening it behind you",
            "Square your hips to the floor as much as possible",
            "Hold and breathe deeply, then switch sides"
        ],
        cues: &["Use a pillow under hip for support if needed", "Keep hips level", "Relax into the stretch"],
    },
    StretchingData {
        name: "Hamstring Doorway Stretch",
        description: "Safe and effective hamstring stretch using a doorway for support. Great for those with tight hamstrings.",
        category: "static",
        difficulty: "beginner",
        primary_muscles: &["hamstrings"],
        secondary_muscles: &["calves"],
        duration_seconds: 45,
        repetitions: None,
        sets: 2,
        recommended_for_activities: &["running", "cycling", "desk_work"],
        contraindications: &["sciatica_acute"],
        instructions: &[
            "Lie on your back in a doorway",
            "Place one leg up against the door frame",
            "Keep your other leg flat on the floor",
            "Scoot closer to the door frame to increase the stretch",
            "Hold for 45 seconds, then switch legs"
        ],
        cues: &["Keep your back flat on the floor", "Straighten the raised leg as much as comfortable", "Breathe deeply"],
    },
    // Cycling-focused stretches
    StretchingData {
        name: "Figure-4 Glute Stretch",
        description: "Targets the glutes and piriformis while lying on your back. Perfect for post-cycling recovery.",
        category: "static",
        difficulty: "beginner",
        primary_muscles: &["glutes"],
        secondary_muscles: &["piriformis", "hip_rotators"],
        duration_seconds: 45,
        repetitions: None,
        sets: 2,
        recommended_for_activities: &["cycling", "running", "sitting"],
        contraindications: &["hip_replacement"],
        instructions: &[
            "Lie on your back with knees bent, feet flat",
            "Cross your right ankle over your left knee",
            "Pull your left thigh toward your chest",
            "Feel the stretch in your right glute",
            "Hold for 45 seconds, then switch sides"
        ],
        cues: &["Keep your head on the floor", "Push your knee away slightly to deepen", "Relax your shoulders"],
    },
    StretchingData {
        name: "Cat-Cow Spine Mobility",
        description: "Dynamic spinal mobility exercise that improves flexibility and relieves back tension.",
        category: "dynamic",
        difficulty: "beginner",
        primary_muscles: &["lower_back", "upper_back"],
        secondary_muscles: &["abdominals", "neck"],
        duration_seconds: 60,
        repetitions: Some(10),
        sets: 2,
        recommended_for_activities: &["cycling", "desk_work", "swimming"],
        contraindications: &["spinal_injury"],
        instructions: &[
            "Start on hands and knees in tabletop position",
            "Inhale: drop belly, lift chest and tailbone (Cow)",
            "Exhale: round spine, tuck chin and tailbone (Cat)",
            "Move slowly and smoothly between positions",
            "Repeat 10 times"
        ],
        cues: &["Move with your breath", "Initiate movement from your pelvis", "Keep movements controlled"],
    },
    // Swimming-focused stretches
    StretchingData {
        name: "Chest Doorway Stretch",
        description: "Opens up the chest and front shoulders. Essential for swimmers to counteract internal rotation.",
        category: "static",
        difficulty: "beginner",
        primary_muscles: &["chest", "pectorals"],
        secondary_muscles: &["anterior_deltoids"],
        duration_seconds: 30,
        repetitions: None,
        sets: 2,
        recommended_for_activities: &["swimming", "desk_work", "cycling"],
        contraindications: &["shoulder_injury"],
        instructions: &[
            "Stand in a doorway with arms at 90 degrees",
            "Place forearms on the door frame",
            "Step one foot forward through the doorway",
            "Lean forward until you feel a stretch in your chest",
            "Hold for 30 seconds"
        ],
        cues: &["Keep your core engaged", "Don't arch your lower back", "Adjust arm height to target different areas"],
    },
    StretchingData {
        name: "Lat Stretch on Wall",
        description: "Stretches the latissimus dorsi and improves overhead mobility. Key for swimmers and overhead athletes.",
        category: "static",
        difficulty: "beginner",
        primary_muscles: &["lats"],
        secondary_muscles: &["triceps", "obliques"],
        duration_seconds: 30,
        repetitions: None,
        sets: 2,
        recommended_for_activities: &["swimming", "climbing", "overhead_sports"],
        contraindications: &["shoulder_impingement"],
        instructions: &[
            "Stand facing a wall at arm's length",
            "Place both hands on the wall at hip height",
            "Step back and hinge at the hips",
            "Push your chest toward the floor",
            "Hold for 30 seconds"
        ],
        cues: &["Keep your back flat", "Let your head hang naturally", "Breathe into your side ribs"],
    },
    // Dynamic warm-up stretches
    StretchingData {
        name: "Leg Swings - Front to Back",
        description: "Dynamic stretch to warm up hip flexors and hamstrings before activity.",
        category: "dynamic",
        difficulty: "beginner",
        primary_muscles: &["hip_flexors", "hamstrings"],
        secondary_muscles: &["glutes"],
        duration_seconds: 30,
        repetitions: Some(15),
        sets: 1,
        recommended_for_activities: &["running", "soccer", "hiking"],
        contraindications: &["hip_injury"],
        instructions: &[
            "Hold onto a wall or post for balance",
            "Swing one leg forward and backward in a controlled motion",
            "Gradually increase the range of motion",
            "Perform 15 swings per leg",
            "Keep your core stable throughout"
        ],
        cues: &["Keep standing leg slightly bent", "Don't force the range", "Maintain upright posture"],
    },
    StretchingData {
        name: "Walking Lunges with Twist",
        description: "Dynamic full-body warm-up combining hip flexor stretch with thoracic rotation.",
        category: "dynamic",
        difficulty: "intermediate",
        primary_muscles: &["hip_flexors", "quadriceps"],
        secondary_muscles: &["core", "thoracic_spine"],
        duration_seconds: 60,
        repetitions: Some(10),
        sets: 1,
        recommended_for_activities: &["running", "tennis", "golf"],
        contraindications: &["knee_injury", "balance_issues"],
        instructions: &[
            "Take a large step forward into a lunge",
            "Lower your back knee toward the ground",
            "Twist your torso toward your front leg",
            "Return to standing and step forward with the other leg",
            "Repeat for 10 steps on each side"
        ],
        cues: &["Keep front knee over ankle", "Twist from your mid-back", "Take your time"],
    },
    StretchingData {
        name: "IT Band Foam Roll",
        description: "Self-myofascial release for the iliotibial band. Helps prevent runner's knee.",
        category: "static",
        difficulty: "intermediate",
        primary_muscles: &["it_band"],
        secondary_muscles: &["vastus_lateralis"],
        duration_seconds: 60,
        repetitions: None,
        sets: 1,
        recommended_for_activities: &["running", "cycling", "hiking"],
        contraindications: &["severe_it_band_syndrome"],
        instructions: &[
            "Lie on your side with a foam roller under your outer thigh",
            "Support yourself with your arms and top leg",
            "Slowly roll from hip to just above the knee",
            "Pause on tender spots for 20-30 seconds",
            "Roll for 60 seconds total per leg"
        ],
        cues: &["Don't roll directly on the knee", "Breathe through discomfort", "Control the pressure"],
    },
];

// ============================================================================
// Yoga Pose Data
// ============================================================================

struct YogaPoseData {
    english_name: &'static str,
    sanskrit_name: Option<&'static str>,
    description: &'static str,
    benefits: &'static [&'static str],
    category: &'static str,
    difficulty: &'static str,
    pose_type: &'static str,
    primary_muscles: &'static [&'static str],
    secondary_muscles: &'static [&'static str],
    hold_duration_seconds: i64,
    breath_guidance: Option<&'static str>,
    recommended_for_activities: &'static [&'static str],
    recommended_for_recovery: &'static [&'static str],
    contraindications: &'static [&'static str],
    instructions: &'static [&'static str],
    modifications: &'static [&'static str],
    cues: &'static [&'static str],
}

const YOGA_POSES: &[YogaPoseData] = &[
    // Standing poses
    YogaPoseData {
        english_name: "Downward Facing Dog",
        sanskrit_name: Some("Adho Mukha Svanasana"),
        description: "A foundational yoga pose that stretches the entire back body while building upper body strength.",
        benefits: &["Stretches hamstrings and calves", "Strengthens arms and shoulders", "Calms the mind", "Improves digestion"],
        category: "standing",
        difficulty: "beginner",
        pose_type: "stretch",
        primary_muscles: &["hamstrings", "calves", "shoulders"],
        secondary_muscles: &["lats", "spine"],
        hold_duration_seconds: 30,
        breath_guidance: Some("Breathe deeply, pushing heels toward the floor on each exhale"),
        recommended_for_activities: &["running", "cycling", "all_activities"],
        recommended_for_recovery: &["post_cardio", "morning", "rest_day"],
        contraindications: &["carpal_tunnel", "high_blood_pressure", "pregnancy_late"],
        instructions: &[
            "Start on hands and knees in tabletop",
            "Tuck toes and lift hips up and back",
            "Straighten legs as much as comfortable",
            "Press chest toward thighs",
            "Hold for 5-10 breaths"
        ],
        modifications: &["Bend knees slightly", "Use blocks under hands"],
        cues: &["Spread fingers wide", "Roll shoulders away from ears", "Lift sitting bones high"],
    },
    YogaPoseData {
        english_name: "Warrior I",
        sanskrit_name: Some("Virabhadrasana I"),
        description: "A powerful standing pose that builds leg strength and opens the hip flexors.",
        benefits: &["Strengthens legs", "Opens hip flexors", "Builds core stability", "Improves focus"],
        category: "standing",
        difficulty: "beginner",
        pose_type: "strength",
        primary_muscles: &["quadriceps", "hip_flexors"],
        secondary_muscles: &["glutes", "core"],
        hold_duration_seconds: 30,
        breath_guidance: Some("Inhale arms up, exhale sink deeper into the lunge"),
        recommended_for_activities: &["running", "hiking"],
        recommended_for_recovery: &["rest_day", "active_recovery"],
        contraindications: &["knee_injury", "hip_injury"],
        instructions: &[
            "Step one foot back about 3-4 feet",
            "Turn back foot out 45 degrees",
            "Bend front knee to 90 degrees",
            "Raise arms overhead, palms facing",
            "Square hips to the front"
        ],
        modifications: &["Shorten stance", "Keep hands on hips"],
        cues: &["Front knee over ankle", "Lift through the chest", "Ground through back heel"],
    },
    YogaPoseData {
        english_name: "Warrior II",
        sanskrit_name: Some("Virabhadrasana II"),
        description: "A strong standing pose that builds endurance and opens the hips laterally.",
        benefits: &["Builds leg endurance", "Opens hips", "Strengthens ankles", "Improves concentration"],
        category: "standing",
        difficulty: "beginner",
        pose_type: "strength",
        primary_muscles: &["quadriceps", "hip_abductors"],
        secondary_muscles: &["core", "shoulders"],
        hold_duration_seconds: 30,
        breath_guidance: Some("Breathe steadily, maintaining the pose with each breath"),
        recommended_for_activities: &["running", "cycling", "hiking"],
        recommended_for_recovery: &["rest_day", "active_recovery"],
        contraindications: &["knee_injury"],
        instructions: &[
            "Stand with feet wide apart (4-5 feet)",
            "Turn front foot out 90 degrees, back foot in slightly",
            "Bend front knee to 90 degrees",
            "Extend arms parallel to the floor",
            "Gaze over front hand"
        ],
        modifications: &["Shorten stance", "Don't bend as deep"],
        cues: &["Stack knee over ankle", "Keep torso centered", "Shoulders relaxed"],
    },
    YogaPoseData {
        english_name: "Triangle Pose",
        sanskrit_name: Some("Trikonasana"),
        description: "A standing pose that stretches the hamstrings, groins, and spine while strengthening the legs.",
        benefits: &["Stretches hamstrings", "Opens chest and shoulders", "Strengthens legs", "Improves balance"],
        category: "standing",
        difficulty: "beginner",
        pose_type: "stretch",
        primary_muscles: &["hamstrings", "obliques"],
        secondary_muscles: &["hip_abductors", "chest"],
        hold_duration_seconds: 30,
        breath_guidance: Some("Inhale to lengthen, exhale to deepen the side bend"),
        recommended_for_activities: &["running", "cycling"],
        recommended_for_recovery: &["post_cardio", "rest_day"],
        contraindications: &["low_blood_pressure", "neck_injury"],
        instructions: &[
            "Stand with feet wide apart",
            "Turn front foot out 90 degrees",
            "Extend arms to sides at shoulder height",
            "Reach forward then down, placing hand on shin or floor",
            "Extend top arm toward ceiling"
        ],
        modifications: &["Use a block under lower hand", "Look down instead of up"],
        cues: &["Keep both sides of torso long", "Open your chest", "Engage your legs"],
    },
    // Seated poses
    YogaPoseData {
        english_name: "Seated Forward Fold",
        sanskrit_name: Some("Paschimottanasana"),
        description: "A deep hamstring and back stretch that calms the nervous system.",
        benefits: &["Stretches entire back body", "Calms the mind", "Relieves stress", "Stimulates digestion"],
        category: "seated",
        difficulty: "beginner",
        pose_type: "stretch",
        primary_muscles: &["hamstrings", "lower_back"],
        secondary_muscles: &["calves", "spine"],
        hold_duration_seconds: 60,
        breath_guidance: Some("Inhale to lengthen spine, exhale to fold deeper"),
        recommended_for_activities: &["running", "cycling"],
        recommended_for_recovery: &["evening", "post_cardio", "rest_day"],
        contraindications: &["lower_back_injury", "sciatica"],
        instructions: &[
            "Sit with legs extended in front",
            "Flex feet, engage quadriceps",
            "Inhale and lengthen spine",
            "Exhale and hinge forward from hips",
            "Hold wherever you feel a stretch"
        ],
        modifications: &["Bend knees slightly", "Use a strap around feet", "Sit on a blanket"],
        cues: &["Lead with your chest, not your head", "Keep spine long", "Relax your shoulders"],
    },
    YogaPoseData {
        english_name: "Butterfly Pose",
        sanskrit_name: Some("Baddha Konasana"),
        description: "A hip opener that stretches the inner thighs and groin.",
        benefits: &["Opens hips", "Stretches inner thighs", "Improves circulation", "Calms the mind"],
        category: "seated",
        difficulty: "beginner",
        pose_type: "stretch",
        primary_muscles: &["hip_adductors", "groin"],
        secondary_muscles: &["lower_back"],
        hold_duration_seconds: 60,
        breath_guidance: Some("With each exhale, let the knees drop slightly lower"),
        recommended_for_activities: &["running", "cycling", "sitting"],
        recommended_for_recovery: &["rest_day", "evening", "post_cardio"],
        contraindications: &["groin_injury", "knee_injury"],
        instructions: &[
            "Sit and bring soles of feet together",
            "Let knees drop out to sides",
            "Hold feet with hands",
            "Sit tall or fold forward gently",
            "Hold for 1-2 minutes"
        ],
        modifications: &["Place blocks under knees", "Sit on a blanket"],
        cues: &["Relax your hips", "Don't force knees down", "Breathe into tight areas"],
    },
    YogaPoseData {
        english_name: "Half Lord of the Fishes",
        sanskrit_name: Some("Ardha Matsyendrasana"),
        description: "A seated twist that improves spinal mobility and aids digestion.",
        benefits: &["Increases spinal mobility", "Stretches hips and shoulders", "Aids digestion", "Energizes the spine"],
        category: "twist",
        difficulty: "intermediate",
        pose_type: "stretch",
        primary_muscles: &["spine", "obliques"],
        secondary_muscles: &["glutes", "chest"],
        hold_duration_seconds: 30,
        breath_guidance: Some("Inhale to lengthen, exhale to twist deeper"),
        recommended_for_activities: &["cycling", "desk_work"],
        recommended_for_recovery: &["rest_day", "morning"],
        contraindications: &["spinal_injury", "pregnancy"],
        instructions: &[
            "Sit with legs extended",
            "Bend right knee and place foot outside left thigh",
            "Twist torso to the right",
            "Use left elbow outside right knee for leverage",
            "Look over right shoulder"
        ],
        modifications: &["Keep bottom leg straight", "Twist less deeply"],
        cues: &["Lengthen spine before twisting", "Keep both sitting bones grounded", "Lead with your belly"],
    },
    // Supine poses
    YogaPoseData {
        english_name: "Reclined Pigeon",
        sanskrit_name: Some("Supta Kapotasana"),
        description: "A gentle hip opener done lying down, perfect for tight hips.",
        benefits: &["Opens hips gently", "Stretches glutes", "Relieves lower back tension", "Accessible for all levels"],
        category: "supine",
        difficulty: "beginner",
        pose_type: "stretch",
        primary_muscles: &["glutes", "piriformis"],
        secondary_muscles: &["hip_rotators"],
        hold_duration_seconds: 60,
        breath_guidance: Some("Breathe deeply, allowing the hip to open with each exhale"),
        recommended_for_activities: &["running", "cycling", "sitting"],
        recommended_for_recovery: &["evening", "post_cardio", "rest_day"],
        contraindications: &["knee_injury"],
        instructions: &[
            "Lie on your back with knees bent",
            "Cross right ankle over left knee",
            "Thread hands behind left thigh",
            "Pull left thigh toward chest",
            "Hold for 1-2 minutes each side"
        ],
        modifications: &["Keep head on floor", "Use a strap around thigh"],
        cues: &["Flex the crossed foot", "Relax your shoulders", "Let gravity do the work"],
    },
    YogaPoseData {
        english_name: "Happy Baby",
        sanskrit_name: Some("Ananda Balasana"),
        description: "A playful pose that opens the hips and releases the lower back.",
        benefits: &["Opens inner hips", "Releases lower back", "Calms the mind", "Stretches hamstrings"],
        category: "supine",
        difficulty: "beginner",
        pose_type: "relaxation",
        primary_muscles: &["hip_adductors", "lower_back"],
        secondary_muscles: &["hamstrings", "groin"],
        hold_duration_seconds: 60,
        breath_guidance: Some("Rock gently side to side with your breath"),
        recommended_for_activities: &["running", "cycling"],
        recommended_for_recovery: &["evening", "rest_day"],
        contraindications: &["pregnancy", "knee_injury"],
        instructions: &[
            "Lie on your back",
            "Bring knees toward armpits",
            "Hold outer edges of feet",
            "Keep lower back on the floor",
            "Gently rock side to side"
        ],
        modifications: &["Hold behind knees instead of feet", "Use a strap"],
        cues: &["Keep tailbone down", "Relax your neck", "Breathe into your hips"],
    },
    YogaPoseData {
        english_name: "Supine Spinal Twist",
        sanskrit_name: Some("Supta Matsyendrasana"),
        description: "A relaxing twist that releases tension in the spine and hips.",
        benefits: &["Releases spinal tension", "Stretches hips and chest", "Aids digestion", "Promotes relaxation"],
        category: "supine",
        difficulty: "beginner",
        pose_type: "relaxation",
        primary_muscles: &["spine", "obliques"],
        secondary_muscles: &["glutes", "chest"],
        hold_duration_seconds: 60,
        breath_guidance: Some("Let the twist deepen naturally with each exhale"),
        recommended_for_activities: &["all_activities"],
        recommended_for_recovery: &["evening", "rest_day", "post_cardio"],
        contraindications: &["spinal_injury"],
        instructions: &[
            "Lie on your back with arms extended",
            "Bring knees to chest",
            "Drop both knees to one side",
            "Turn head to opposite direction",
            "Hold for 1-2 minutes each side"
        ],
        modifications: &["Place pillow between knees", "Keep knees higher"],
        cues: &["Keep both shoulders on floor", "Relax completely", "Breathe into your belly"],
    },
    YogaPoseData {
        english_name: "Legs Up The Wall",
        sanskrit_name: Some("Viparita Karani"),
        description: "A restorative inversion that promotes circulation and relaxation.",
        benefits: &["Promotes blood flow to legs", "Reduces swelling", "Calms nervous system", "Relieves tired legs"],
        category: "inversion",
        difficulty: "beginner",
        pose_type: "relaxation",
        primary_muscles: &["hamstrings"],
        secondary_muscles: &["lower_back"],
        hold_duration_seconds: 300,
        breath_guidance: Some("Breathe slowly and deeply, allowing complete relaxation"),
        recommended_for_activities: &["running", "hiking", "standing_work"],
        recommended_for_recovery: &["evening", "rest_day", "post_long_run"],
        contraindications: &["glaucoma", "uncontrolled_high_blood_pressure"],
        instructions: &[
            "Sit sideways next to a wall",
            "Lie back and swing legs up the wall",
            "Scoot hips close to the wall",
            "Rest arms by your sides",
            "Stay for 5-15 minutes"
        ],
        modifications: &["Place blanket under hips", "Bend knees slightly"],
        cues: &["Close your eyes", "Let your body be heavy", "Release all effort"],
    },
    // Balance poses
    YogaPoseData {
        english_name: "Tree Pose",
        sanskrit_name: Some("Vrksasana"),
        description: "A standing balance pose that builds focus and leg strength.",
        benefits: &["Improves balance", "Strengthens legs and ankles", "Opens hips", "Builds focus"],
        category: "balance",
        difficulty: "beginner",
        pose_type: "balance",
        primary_muscles: &["quadriceps", "glutes"],
        secondary_muscles: &["core", "hip_abductors"],
        hold_duration_seconds: 30,
        breath_guidance: Some("Find a steady breath to maintain balance"),
        recommended_for_activities: &["running", "all_activities"],
        recommended_for_recovery: &["morning", "rest_day"],
        contraindications: &["ankle_injury"],
        instructions: &[
            "Stand on one leg",
            "Place other foot on inner thigh or calf (never knee)",
            "Press foot and leg together",
            "Bring hands to heart or overhead",
            "Focus on a fixed point"
        ],
        modifications: &["Keep toes on floor", "Use a wall for support"],
        cues: &["Root down through standing foot", "Lift through crown of head", "Soften your gaze"],
    },
    // Breathing/relaxation
    YogaPoseData {
        english_name: "Child's Pose",
        sanskrit_name: Some("Balasana"),
        description: "A restful pose that gently stretches the back and promotes relaxation.",
        benefits: &["Releases back tension", "Calms the mind", "Gentle hip stretch", "Promotes rest"],
        category: "supine",
        difficulty: "beginner",
        pose_type: "relaxation",
        primary_muscles: &["lower_back", "hips"],
        secondary_muscles: &["shoulders", "ankles"],
        hold_duration_seconds: 60,
        breath_guidance: Some("Breathe into your lower back, feeling it expand with each inhale"),
        recommended_for_activities: &["all_activities"],
        recommended_for_recovery: &["any_time", "rest_day", "stress_relief"],
        contraindications: &["knee_injury", "pregnancy"],
        instructions: &[
            "Kneel on the floor with big toes touching",
            "Sit back on your heels",
            "Fold forward, extending arms or resting them by sides",
            "Rest forehead on the floor",
            "Stay as long as needed"
        ],
        modifications: &["Widen knees", "Place blanket under knees", "Use pillow under chest"],
        cues: &["Let everything soften", "Surrender to gravity", "No effort needed"],
    },
    YogaPoseData {
        english_name: "Corpse Pose",
        sanskrit_name: Some("Savasana"),
        description: "The final relaxation pose that integrates the benefits of practice.",
        benefits: &["Deep relaxation", "Reduces stress", "Lowers blood pressure", "Integrates practice"],
        category: "supine",
        difficulty: "beginner",
        pose_type: "relaxation",
        primary_muscles: &[],
        secondary_muscles: &[],
        hold_duration_seconds: 300,
        breath_guidance: Some("Let breath be natural and effortless"),
        recommended_for_activities: &["all_activities"],
        recommended_for_recovery: &["any_time", "evening", "stress_relief"],
        contraindications: &["pregnancy_late"],
        instructions: &[
            "Lie flat on your back",
            "Let feet fall open",
            "Place arms by sides, palms up",
            "Close your eyes",
            "Release all muscular effort"
        ],
        modifications: &["Place bolster under knees", "Cover with blanket"],
        cues: &["Scan body and release tension", "Let go completely", "Simply be"],
    },
];

// ============================================================================
// Activity-Muscle Mapping Data
// ============================================================================

struct ActivityMappingData {
    activity_type: &'static str,
    primary_muscles: &'static [(&'static str, u8)],
    secondary_muscles: &'static [(&'static str, u8)],
    recommended_stretch_categories: &'static [&'static str],
    recommended_yoga_categories: &'static [&'static str],
}

const ACTIVITY_MAPPINGS: &[ActivityMappingData] = &[
    ActivityMappingData {
        activity_type: "running",
        primary_muscles: &[
            ("calves", 9),
            ("quadriceps", 8),
            ("hamstrings", 8),
            ("glutes", 7),
            ("hip_flexors", 7),
        ],
        secondary_muscles: &[("core", 5), ("lower_back", 4), ("it_band", 6)],
        recommended_stretch_categories: &["static", "dynamic"],
        recommended_yoga_categories: &["standing", "supine"],
    },
    ActivityMappingData {
        activity_type: "cycling",
        primary_muscles: &[
            ("quadriceps", 9),
            ("glutes", 8),
            ("hip_flexors", 8),
            ("calves", 6),
        ],
        secondary_muscles: &[
            ("hamstrings", 5),
            ("lower_back", 6),
            ("neck", 5),
            ("shoulders", 4),
        ],
        recommended_stretch_categories: &["static"],
        recommended_yoga_categories: &["standing", "supine", "twist"],
    },
    ActivityMappingData {
        activity_type: "swimming",
        primary_muscles: &[("lats", 9), ("shoulders", 9), ("chest", 7), ("triceps", 7)],
        secondary_muscles: &[("core", 6), ("hip_flexors", 5), ("calves", 4)],
        recommended_stretch_categories: &["static", "dynamic"],
        recommended_yoga_categories: &["standing", "supine"],
    },
    ActivityMappingData {
        activity_type: "hiking",
        primary_muscles: &[
            ("quadriceps", 8),
            ("glutes", 8),
            ("calves", 7),
            ("hamstrings", 6),
        ],
        secondary_muscles: &[("hip_flexors", 5), ("core", 5), ("lower_back", 5)],
        recommended_stretch_categories: &["static"],
        recommended_yoga_categories: &["standing", "supine"],
    },
    ActivityMappingData {
        activity_type: "strength_training",
        primary_muscles: &[
            ("chest", 7),
            ("back", 7),
            ("shoulders", 7),
            ("biceps", 6),
            ("triceps", 6),
        ],
        secondary_muscles: &[("core", 6), ("quadriceps", 5), ("glutes", 5)],
        recommended_stretch_categories: &["static", "pnf"],
        recommended_yoga_categories: &["standing", "supine", "twist"],
    },
    ActivityMappingData {
        activity_type: "desk_work",
        primary_muscles: &[
            ("hip_flexors", 8),
            ("lower_back", 7),
            ("neck", 7),
            ("shoulders", 6),
        ],
        secondary_muscles: &[("chest", 5), ("hamstrings", 5), ("wrists", 4)],
        recommended_stretch_categories: &["static", "dynamic"],
        recommended_yoga_categories: &["seated", "standing", "twist"],
    },
    ActivityMappingData {
        activity_type: "walking",
        primary_muscles: &[("calves", 6), ("glutes", 5), ("quadriceps", 5)],
        secondary_muscles: &[("hamstrings", 4), ("hip_flexors", 4), ("core", 3)],
        recommended_stretch_categories: &["static"],
        recommended_yoga_categories: &["standing", "supine"],
    },
];

#[tokio::main]
async fn main() -> AppResult<()> {
    let args = SeedArgs::parse();

    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt().with_env_filter(log_level).init();

    info!("=== Pierre MCP Server Mobility Data Seeder ===");

    let database_url = args
        .database_url
        .or_else(|| env::var("DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite:./data/users.db".into());

    info!("Connecting to database: {}", database_url);
    let db = Database::init_for_seeding(&database_url).await?;

    // Check if data already exists via SeederRepository
    let stretch_count = db
        .seed_count_table(SeedTable::StretchingExercises)
        .await
        .unwrap_or(0);
    let yoga_count = db.seed_count_table(SeedTable::YogaPoses).await.unwrap_or(0);
    let mapping_count = db
        .seed_count_table(SeedTable::ActivityMuscleMapping)
        .await
        .unwrap_or(0);

    if (stretch_count > 0 || yoga_count > 0 || mapping_count > 0) && !args.force {
        info!(
            "Mobility data already seeded ({} stretches, {} yoga poses, {} mappings). Use --force to re-seed.",
            stretch_count, yoga_count, mapping_count
        );
        return Ok(());
    }

    let now = Utc::now();

    info!(
        "Seeding {} stretching exercises...",
        STRETCHING_EXERCISES.len()
    );
    for data in STRETCHING_EXERCISES {
        let exercise = to_stretching_exercise(data, now);
        db.seed_upsert_stretching_exercise(&exercise).await?;
        info!("  + {}", data.name);
    }

    info!("Seeding {} yoga poses...", YOGA_POSES.len());
    for data in YOGA_POSES {
        let pose = to_yoga_pose(data, now);
        db.seed_upsert_yoga_pose(&pose).await?;
        info!("  + {}", data.english_name);
    }

    info!(
        "Seeding {} activity-muscle mappings...",
        ACTIVITY_MAPPINGS.len()
    );
    for data in ACTIVITY_MAPPINGS {
        let mapping = to_activity_mapping(data, now);
        db.seed_upsert_activity_mapping(&mapping).await?;
        info!("  + {}", data.activity_type);
    }

    info!("");
    info!("=== Seeding Complete ===");
    info!(
        "Created {} stretching exercises",
        STRETCHING_EXERCISES.len()
    );
    info!("Created {} yoga poses", YOGA_POSES.len());
    info!(
        "Created {} activity-muscle mappings",
        ACTIVITY_MAPPINGS.len()
    );

    Ok(())
}

// ============================================================================
// Data → Model Conversion Functions
// ============================================================================

fn to_str_vec(slice: &[&str]) -> Vec<String> {
    slice.iter().map(|s| (*s).to_owned()).collect()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_stretching_exercise(data: &StretchingData, now: chrono::DateTime<Utc>) -> StretchingExercise {
    StretchingExercise {
        id: Uuid::new_v4().to_string(),
        name: data.name.to_owned(),
        description: data.description.to_owned(),
        category: StretchingCategory::parse(data.category),
        difficulty: DifficultyLevel::parse(data.difficulty),
        primary_muscles: to_str_vec(data.primary_muscles),
        secondary_muscles: to_str_vec(data.secondary_muscles),
        duration_seconds: data.duration_seconds as u32,
        repetitions: data.repetitions.map(|r| r as u32),
        sets: data.sets as u32,
        recommended_for_activities: to_str_vec(data.recommended_for_activities),
        contraindications: to_str_vec(data.contraindications),
        instructions: to_str_vec(data.instructions),
        cues: to_str_vec(data.cues),
        image_url: None,
        video_url: None,
        created_at: now,
        updated_at: now,
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_yoga_pose(data: &YogaPoseData, now: chrono::DateTime<Utc>) -> YogaPose {
    YogaPose {
        id: Uuid::new_v4().to_string(),
        english_name: data.english_name.to_owned(),
        sanskrit_name: data.sanskrit_name.map(str::to_owned),
        description: data.description.to_owned(),
        benefits: to_str_vec(data.benefits),
        category: YogaCategory::parse(data.category),
        difficulty: DifficultyLevel::parse(data.difficulty),
        pose_type: YogaPoseType::parse(data.pose_type),
        primary_muscles: to_str_vec(data.primary_muscles),
        secondary_muscles: to_str_vec(data.secondary_muscles),
        chakras: Vec::new(),
        hold_duration_seconds: data.hold_duration_seconds as u32,
        breath_guidance: data.breath_guidance.map(str::to_owned),
        recommended_for_activities: to_str_vec(data.recommended_for_activities),
        recommended_for_recovery: to_str_vec(data.recommended_for_recovery),
        contraindications: to_str_vec(data.contraindications),
        instructions: to_str_vec(data.instructions),
        modifications: to_str_vec(data.modifications),
        progressions: Vec::new(),
        cues: to_str_vec(data.cues),
        warmup_poses: Vec::new(),
        followup_poses: Vec::new(),
        image_url: None,
        video_url: None,
        created_at: now,
        updated_at: now,
    }
}

fn to_activity_mapping(
    data: &ActivityMappingData,
    now: chrono::DateTime<Utc>,
) -> ActivityMuscleMapping {
    let primary_muscles: HashMap<String, u8> = data
        .primary_muscles
        .iter()
        .map(|(k, v)| ((*k).to_owned(), *v))
        .collect();
    let secondary_muscles: HashMap<String, u8> = data
        .secondary_muscles
        .iter()
        .map(|(k, v)| ((*k).to_owned(), *v))
        .collect();

    ActivityMuscleMapping {
        id: Uuid::new_v4().to_string(),
        activity_type: data.activity_type.to_owned(),
        primary_muscles,
        secondary_muscles,
        recommended_stretch_categories: to_str_vec(data.recommended_stretch_categories),
        recommended_yoga_categories: to_str_vec(data.recommended_yoga_categories),
        created_at: now,
        updated_at: now,
    }
}
