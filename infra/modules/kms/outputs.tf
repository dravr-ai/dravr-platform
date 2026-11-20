# ABOUTME: Outputs for the KMS module: the KEK and the checkpoint signing key
# ABOUTME: Exposes PIERRE_KMS_KEY_RESOURCE and PIERRE_CHECKPOINT_SIGNING_KEY values

output "key_resource" {
  description = "Full KMS crypto-key resource name; set as PIERRE_KMS_KEY_RESOURCE for the GcpKmsKekProvider"
  value       = google_kms_crypto_key.dek_kek.id
}

output "key_ring_id" {
  description = "KMS key ring resource ID"
  value       = google_kms_key_ring.dek_kek.id
}

output "checkpoint_signing_key_version" {
  description = "The checkpoint signing key's version 1 resource; set as PIERRE_CHECKPOINT_SIGNING_KEY for the KMS Ed25519 signer"
  value       = "${google_kms_crypto_key.checkpoint_signer.id}/cryptoKeyVersions/1"
}
