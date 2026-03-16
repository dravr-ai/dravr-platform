# ABOUTME: Creates VPC network infrastructure for Dravr MCP Server
# ABOUTME: Includes VPC, subnet, private service connect, and VPC connector

# -----------------------------------------------------------------------------
# VPC Network
# -----------------------------------------------------------------------------

resource "google_compute_network" "vpc" {
  name                    = var.vpc_name
  project                 = var.project_id
  auto_create_subnetworks = false
  routing_mode            = "REGIONAL"
}

# -----------------------------------------------------------------------------
# Subnet
# -----------------------------------------------------------------------------

resource "google_compute_subnetwork" "subnet" {
  name                     = "${var.vpc_name}-subnet"
  project                  = var.project_id
  region                   = var.region
  network                  = google_compute_network.vpc.id
  ip_cidr_range            = var.subnet_cidr
  private_ip_google_access = true
}

# -----------------------------------------------------------------------------
# Private Service Connection (for Cloud SQL)
# -----------------------------------------------------------------------------

resource "google_compute_global_address" "private_ip_range" {
  count = var.enable_database ? 1 : 0

  name          = "${var.vpc_name}-private-ip"
  project       = var.project_id
  purpose       = "VPC_PEERING"
  address_type  = "INTERNAL"
  prefix_length = 16
  network       = google_compute_network.vpc.id
}

resource "google_service_networking_connection" "private_vpc_connection" {
  count = var.enable_database ? 1 : 0

  network                 = google_compute_network.vpc.id
  service                 = "servicenetworking.googleapis.com"
  reserved_peering_ranges = [google_compute_global_address.private_ip_range[0].name]

  deletion_policy = "ABANDON"
}

# -----------------------------------------------------------------------------
# Serverless VPC Connector (for Cloud Run)
# -----------------------------------------------------------------------------

resource "google_vpc_access_connector" "connector" {
  name          = "${var.vpc_name}-connector"
  project       = var.project_id
  region        = var.region
  ip_cidr_range = var.vpc_connector_cidr
  network       = google_compute_network.vpc.name

  min_instances = 2
  max_instances = 3

  depends_on = [google_compute_network.vpc]
}

# -----------------------------------------------------------------------------
# Cloud NAT (enables VPC-connected Cloud Run services to reach external hosts)
# Without Cloud NAT, services with vpc_egress=ALL_TRAFFIC cannot make outbound
# connections to the public internet (e.g., firebaseapp.com for auth handler).
# -----------------------------------------------------------------------------

resource "google_compute_router" "router" {
  name    = "${var.vpc_name}-router"
  project = var.project_id
  region  = var.region
  network = google_compute_network.vpc.id
}

resource "google_compute_router_nat" "nat" {
  name                               = "${var.vpc_name}-nat"
  project                            = var.project_id
  region                             = var.region
  router                             = google_compute_router.router.name
  nat_ip_allocate_option             = "AUTO_ONLY"
  source_subnetwork_ip_ranges_to_nat = "ALL_SUBNETWORKS_ALL_IP_RANGES"

  log_config {
    enable = true
    filter = "ERRORS_ONLY"
  }
}

# -----------------------------------------------------------------------------
# Firewall Rules
# -----------------------------------------------------------------------------

# Allow internal traffic for known services within VPC
resource "google_compute_firewall" "allow_internal" {
  name    = "${var.vpc_name}-allow-internal"
  project = var.project_id
  network = google_compute_network.vpc.name

  allow {
    protocol = "tcp"
    ports    = ["5432", "6379"]
  }

  allow {
    protocol = "icmp"
  }

  source_ranges = [var.subnet_cidr, var.vpc_connector_cidr]
}
