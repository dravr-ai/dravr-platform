# ABOUTME: Variables for the KMS module (KEK for envelope encryption of the DEK)
# ABOUTME: Configures key ring location, rotation period, and the runtime SA granted encrypt/decrypt

variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "service_name" {
  description = "Name of the service (used as prefix for the key ring and key IDs)"
  type        = string
}

variable "location" {
  description = "Location for the KMS key ring (region or 'global')"
  type        = string
}

variable "runtime_service_account_email" {
  description = "Cloud Run runtime service account granted cryptoKeyEncrypterDecrypter on the KEK"
  type        = string
}

variable "rotation_period" {
  description = "Automatic KEK rotation period (e.g. '7776000s' for 90 days). KMS retains prior versions so existing wrapped DEKs keep decrypting."
  type        = string
  default     = "7776000s"
}

variable "labels" {
  description = "Labels to apply to the key ring"
  type        = map(string)
  default     = {}
}
