use super::{ConnectionInfo, ServiceRegistration};
use std::collections::HashMap;
use uuid::Uuid;

pub fn match_host_to_service<'a>(
    host: &str,
    registrations: &'a [ServiceRegistration],
) -> Option<&'a ServiceRegistration> {
    // First try exact match
    if let Some(reg) = registrations.iter().find(|r| r.host == host) {
        return Some(reg);
    }

    // Then try wildcard matching
    registrations.iter().find(|r| {
        if r.host.starts_with('*') {
            let pattern = &r.host[1..];
            host.ends_with(pattern)
        } else {
            false
        }
    })
}

pub fn select_healthy_instance<'a>(
    registrations: &'a [ServiceRegistration],
    connections: &HashMap<Uuid, ConnectionInfo>,
) -> Option<&'a ServiceRegistration> {
    registrations.iter().find(|reg| {
        if let Some(conn) = connections.get(&reg.id) {
            // Check if connection is healthy (heartbeat within last 30 seconds)
            if let Ok(elapsed) = conn.last_heartbeat.elapsed() {
                elapsed.as_secs() < 60
            } else {
                false
            }
        } else {
            false
        }
    })
}
