# ABOUTME: Variables for the networking module
# ABOUTME: Configures VPC, subnet, and connector settings

variable "project_id" {
  description = "GCP project ID"
  type        = string
}

variable "region" {
  description = "GCP region"
  type        = string
}

variable "vpc_name" {
  description = "Name of the VPC network"
  type        = string
}

variable "subnet_cidr" {
  description = "CIDR range for the VPC subnet"
  type        = string
}

variable "vpc_connector_cidr" {
  description = "CIDR range for the serverless VPC connector"
  type        = string
}

variable "enable_database" {
  description = "Enable private service connect resources for Cloud SQL"
  type        = bool
  default     = false
}
