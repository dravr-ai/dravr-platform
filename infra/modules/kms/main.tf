# ABOUTME: Cloud KMS Key Encryption Key (KEK) for envelope encryption of the database DEK
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
