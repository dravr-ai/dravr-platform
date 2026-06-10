# ABOUTME: Outputs for the KMS module
# ABOUTME: Exposes the crypto-key resource name for PIERRE_KMS_KEY_RESOURCE

output "key_resource" {
  description = "Full KMS crypto-key resource name; set as PIERRE_KMS_KEY_RESOURCE for the GcpKmsKekProvider"
  value       = google_kms_crypto_key.dek_kek.id
}

output "key_ring_id" {
  description = "KMS key ring resource ID"
  value       = google_kms_key_ring.dek_kek.id
}
