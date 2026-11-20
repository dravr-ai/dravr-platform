# ABOUTME: Cloud KMS Key Encryption Key (KEK) for the database DEK, and the Ed25519 key that signs record_log checkpoints
# ABOUTME: The KEK never leaves KMS; the app wraps/unwraps the DEK via KMS encrypt/decrypt (ADR-017)

# -----------------------------------------------------------------------------
# Key ring + symmetric KEK
# -----------------------------------------------------------------------------

resource "google_kms_key_ring" "dek_kek" {
  project  = var.project_id
  name     = "${var.service_name}-kek-ring"
  location = var.location
}

resource "google_kms_crypto_key" "dek_kek" {
  name     = "${var.service_name}-dek-kek"
  key_ring = google_kms_key_ring.dek_kek.id
  purpose  = "ENCRYPT_DECRYPT"

  # Automatic rotation. KMS retains prior key versions, so DEKs wrapped under an
  # older KEK version still decrypt after rotation — no app-side re-wrap needed.
  rotation_period = var.rotation_period

  labels = var.labels

  # A KEK must never be deleted: destroying it makes every wrapped DEK — and thus
  # all data-at-rest — permanently unrecoverable.
  lifecycle {
    prevent_destroy = true
  }
}

# -----------------------------------------------------------------------------
# IAM: grant the Cloud Run runtime SA wrap/unwrap (encrypt/decrypt) on the KEK
# -----------------------------------------------------------------------------

resource "google_kms_crypto_key_iam_member" "runtime_encrypter_decrypter" {
  crypto_key_id = google_kms_crypto_key.dek_kek.id
  role          = "roles/cloudkms.cryptoKeyEncrypterDecrypter"
  member        = "serviceAccount:${var.runtime_service_account_email}"
}

# -----------------------------------------------------------------------------
# Ed25519 checkpoint signing key (carnet#1542, design Q11)
# -----------------------------------------------------------------------------
# Signs barrage's record_log checkpoints: one C2SP signed note per hour over
# every athlete's log head and the deletion receipt chain's head. The private
# key never leaves KMS; every signature is a KMS call the audit log records.
# An asymmetric key does not rotate: its public key is published as did:web
# and pinned in every athlete's export, so version 1 stays the signing key.

resource "google_kms_crypto_key" "checkpoint_signer" {
  name     = "${var.service_name}-checkpoint-signer"
  key_ring = google_kms_key_ring.dek_kek.id
  purpose  = "ASYMMETRIC_SIGN"

  version_template {
    algorithm        = "EC_SIGN_ED25519"
    protection_level = "SOFTWARE"
  }

  labels = var.labels

  # Destroying it would leave every signed checkpoint, and the proofs in every
  # athlete's export, unverifiable against a published key.
  lifecycle {
    prevent_destroy = true
  }
}

# The runtime SA signs with it and reads its public key; nothing else is
# granted on it.
resource "google_kms_crypto_key_iam_member" "runtime_checkpoint_signer" {
  crypto_key_id = google_kms_crypto_key.checkpoint_signer.id
  role          = "roles/cloudkms.signerVerifier"
  member        = "serviceAccount:${var.runtime_service_account_email}"
}
