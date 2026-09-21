# ABOUTME: Outputs of the frontend_domain module — the load balancer's addresses and the DNS records each hostname needs
# ABOUTME: The DNS authorization CNAMEs are what validate the certificates; the A/AAAA records are the cutover itself

output "ipv4_address" {
  description = "Static IPv4 address of the load balancer (the A record for every hostname)"
  value       = google_compute_global_address.ipv4.address
}

output "ipv6_address" {
  description = "Static IPv6 address of the load balancer (the AAAA record for every hostname)"
  value       = google_compute_global_address.ipv6.address
}

output "dns_authorization_records" {
  description = "Per hostname, the CNAME record to publish (DNS only, unproxied) so Certificate Manager can validate its certificate before the hostname resolves to the load balancer"
  value = {
    for d, auth in google_certificate_manager_dns_authorization.domain : d => {
      name = auth.dns_resource_record[0].name
      type = auth.dns_resource_record[0].type
      data = auth.dns_resource_record[0].data
    }
  }
}

output "certificate_map_id" {
  description = "Certificate map attached to the HTTPS proxy"
  value       = google_certificate_manager_certificate_map.frontend.id
}
