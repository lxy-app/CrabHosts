use crate::HostEntry;

/// Imports valid, active hosts mappings. Unrelated comments and malformed lines are ignored.
pub fn parse_hosts(input: &str) -> Vec<HostEntry> {
    let mut entries: Vec<HostEntry> = Vec::new();
    for line in input.lines() {
        let content = line.split('#').next().unwrap_or_default().trim();
        if content.is_empty() {
            continue;
        }
        let mut fields = content.split_whitespace();
        let Some(ip) = fields.next() else {
            continue;
        };
        let hostnames = fields.map(str::to_owned).collect::<Vec<_>>();
        if hostnames.is_empty() {
            continue;
        }
        let Ok(imported) = HostEntry::new(ip, hostnames, true, None) else {
            continue;
        };
        if let Some(existing) = entries.iter_mut().find(|entry| entry.ip == imported.ip) {
            for hostname in imported.hostnames {
                if !existing.hostnames.contains(&hostname) {
                    existing.hostnames.push(hostname);
                }
            }
        } else {
            entries.push(imported);
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::parse_hosts;

    #[test]
    fn imports_active_ipv4_and_ipv6_entries() {
        let entries = parse_hosts(
            "# comment\n127.0.0.1 localhost dev.local # inline\n::1 localhost\ninvalid line\n",
        );
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].hostnames, ["localhost", "dev.local"]);
        assert_eq!(entries[1].ip, "::1");
    }
}
