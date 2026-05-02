//! Host matching logic: match discovered Proxmox guests to Uptrakit hosts.
//!
//! Supports manual matching and semi-automatic suggestions based on
//! machine_id, hostname, IP address, and name similarity.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use uuid::Uuid;

use uptrakit_shared_db::entity::{host, proxmox_host_mapping};

use crate::error::{ProxmoxError, Result};

// ── Match method ────────────────────────────────────────────────────────────

/// Match method used to link a Proxmox guest to an Uptrakit host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMethod {
    /// Manually matched by a user via the "match" action.
    Manual,
    /// Suggested by machine_id match, approved by user.
    SuggestedMachineId,
    /// Suggested by hostname match, approved by user.
    SuggestedHostname,
    /// Suggested by IP address match, approved by user.
    SuggestedIp,
    /// Suggested by hostname + IP match, approved by user.
    SuggestedHostnameIp,
    /// Suggested by Proxmox name match, approved by user.
    SuggestedName,
}

impl MatchMethod {
    /// Returns the string representation stored in the database.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::SuggestedMachineId => "suggested_machine_id",
            Self::SuggestedHostname => "suggested_hostname",
            Self::SuggestedIp => "suggested_ip",
            Self::SuggestedHostnameIp => "suggested_hostname_ip",
            Self::SuggestedName => "suggested_name",
        }
    }

    /// All known variants for testing.
    #[cfg(test)]
    const KNOWN_VARIANTS: &'static [Self] = &[
        Self::Manual,
        Self::SuggestedMachineId,
        Self::SuggestedHostname,
        Self::SuggestedIp,
        Self::SuggestedHostnameIp,
        Self::SuggestedName,
    ];
}

impl std::str::FromStr for MatchMethod {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "manual" => Ok(Self::Manual),
            "suggested_machine_id" => Ok(Self::SuggestedMachineId),
            "suggested_hostname" => Ok(Self::SuggestedHostname),
            "suggested_ip" => Ok(Self::SuggestedIp),
            "suggested_hostname_ip" => Ok(Self::SuggestedHostnameIp),
            "suggested_name" => Ok(Self::SuggestedName),
            _ => Err(format!("unknown match method: {s}")),
        }
    }
}

impl std::fmt::Display for MatchMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Match confidence ────────────────────────────────────────────────────────

/// Confidence level for a match suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchConfidence {
    /// Proxmox name matches friendly_name only.
    Low,
    /// Hostname OR IP match.
    Medium,
    /// Hostname + IP, or machine_id match.
    High,
}

impl MatchConfidence {
    /// Returns the string representation for API responses.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

// ── Match suggestion ────────────────────────────────────────────────────────

/// A suggested match between a Proxmox guest mapping and an Uptrakit host.
#[derive(Debug, Clone)]
pub struct MatchSuggestion {
    /// The proxmox_host_mapping ID.
    pub mapping_id: Uuid,
    /// The suggested host ID.
    pub host_id: Uuid,
    /// The suggested host's friendly name.
    pub host_name: String,
    /// Confidence level.
    pub confidence: MatchConfidence,
    /// The match method that produced this suggestion.
    pub match_method: MatchMethod,
    /// Human-readable reason for the suggestion.
    pub reason: String,
}

// ── Suggestion computation ──────────────────────────────────────────────────

/// Compute match suggestions for unmatched Proxmox guest mappings.
///
/// For each unmatched mapping, finds the best matching host using these
/// signals (highest confidence first):
///
/// 1. **machine_id** (High): exact match of `mapping.machine_id` to `host.machine_id`
/// 2. **hostname + IP** (High): case-insensitive hostname AND IP in mapping's `ip_addresses`
/// 3. **hostname only** (Medium): case-insensitive hostname match
/// 4. **IP only** (Medium): host IP found in mapping's `ip_addresses` JSON array
/// 5. **name only** (Low): case-insensitive Proxmox name matches host name
///
/// Uses greedy assignment: highest confidence suggestions first, each host
/// can appear in at most one suggestion.
pub fn compute_suggestions(
    unmatched_mappings: &[proxmox_host_mapping::Model],
    unmatched_hosts: &[host::Model],
) -> Vec<MatchSuggestion> {
    if unmatched_mappings.is_empty() || unmatched_hosts.is_empty() {
        return Vec::new();
    }

    // Collect all candidate suggestions per mapping (best one only)
    let mut candidates: Vec<MatchSuggestion> = Vec::new();

    for mapping in unmatched_mappings {
        let ip_set = parse_ip_addresses(&mapping.ip_addresses);

        let mut best: Option<MatchSuggestion> = None;

        for host in unmatched_hosts {
            let suggestion = evaluate_match(mapping, host, &ip_set);
            if let Some(s) = suggestion
                && best.as_ref().is_none_or(|b| s.confidence > b.confidence)
            {
                best = Some(s);
            }
        }

        if let Some(s) = best {
            candidates.push(s);
        }
    }

    // Sort by confidence descending, then by mapping order (stable sort preserves insertion order)
    candidates.sort_by_key(|a| Reverse(a.confidence));

    // Greedy assignment: each host can appear at most once
    let mut used_hosts: HashSet<Uuid> = HashSet::new();
    let mut result: Vec<MatchSuggestion> = Vec::new();

    for suggestion in candidates {
        if used_hosts.contains(&suggestion.host_id) {
            continue;
        }
        used_hosts.insert(suggestion.host_id);
        result.push(suggestion);
    }

    result
}

/// Parse ip_addresses JSON string into a set of IP strings.
fn parse_ip_addresses(ip_json: &Option<String>) -> HashSet<String> {
    match ip_json {
        Some(json_str) => serde_json::from_str::<Vec<String>>(json_str)
            .unwrap_or_default()
            .into_iter()
            .collect(),
        None => HashSet::new(),
    }
}

/// Evaluate the best match between a mapping and a host.
fn evaluate_match(
    mapping: &proxmox_host_mapping::Model,
    host: &host::Model,
    mapping_ips: &HashSet<String>,
) -> Option<MatchSuggestion> {
    let mapping_id = mapping.id;
    let host_id = host.id;
    let host_name = host.friendly_name.clone();

    // 1. machine_id match (High)
    if let Some(ref mid) = mapping.machine_id
        && !mid.is_empty()
        && mid == &host.machine_id
    {
        return Some(MatchSuggestion {
            mapping_id,
            host_id,
            host_name,
            confidence: MatchConfidence::High,
            match_method: MatchMethod::SuggestedMachineId,
            reason: "Machine ID matches".to_string(),
        });
    }

    let hostname_matches = mapping.hostname.as_ref().is_some_and(|mh| {
        mh.eq_ignore_ascii_case(&host.hostname) || mh.eq_ignore_ascii_case(&host.friendly_name)
    });

    let ip_matches = host
        .ip_address
        .as_ref()
        .is_some_and(|hip| mapping_ips.contains(hip));

    // 2. hostname + IP (High)
    if hostname_matches && ip_matches {
        return Some(MatchSuggestion {
            mapping_id,
            host_id,
            host_name,
            confidence: MatchConfidence::High,
            match_method: MatchMethod::SuggestedHostnameIp,
            reason: "Hostname and IP address match".to_string(),
        });
    }

    // 3. hostname only (Medium)
    if hostname_matches {
        return Some(MatchSuggestion {
            mapping_id,
            host_id,
            host_name,
            confidence: MatchConfidence::Medium,
            match_method: MatchMethod::SuggestedHostname,
            reason: "Hostname matches".to_string(),
        });
    }

    // 4. IP only (Medium)
    if ip_matches {
        return Some(MatchSuggestion {
            mapping_id,
            host_id,
            host_name,
            confidence: MatchConfidence::Medium,
            match_method: MatchMethod::SuggestedIp,
            reason: "IP address matches".to_string(),
        });
    }

    // 5. name match (Low)
    if let Some(ref pname) = mapping.proxmox_name
        && (pname.eq_ignore_ascii_case(&host.friendly_name)
            || pname.eq_ignore_ascii_case(&host.hostname))
    {
        return Some(MatchSuggestion {
            mapping_id,
            host_id,
            host_name,
            confidence: MatchConfidence::Low,
            match_method: MatchMethod::SuggestedName,
            reason: "Proxmox name matches host name".to_string(),
        });
    }

    None
}

/// Build a lookup map from mapping_id to suggestion.
pub fn suggestions_by_mapping_id(
    suggestions: Vec<MatchSuggestion>,
) -> HashMap<Uuid, MatchSuggestion> {
    suggestions.into_iter().map(|s| (s.mapping_id, s)).collect()
}

// ── DB operations ───────────────────────────────────────────────────────────

/// Set or update a manual match between a mapping and a host.
pub async fn manual_match(db: &DatabaseConnection, mapping_id: Uuid, host_id: Uuid) -> Result<()> {
    apply_match(db, mapping_id, host_id, MatchMethod::Manual).await
}

/// Apply a suggested match that has been approved by the user.
pub async fn apply_suggested_match(
    db: &DatabaseConnection,
    mapping_id: Uuid,
    host_id: Uuid,
    method: MatchMethod,
) -> Result<()> {
    apply_match(db, mapping_id, host_id, method).await
}

/// Internal: apply a match with the given method.
async fn apply_match(
    db: &DatabaseConnection,
    mapping_id: Uuid,
    host_id: Uuid,
    method: MatchMethod,
) -> Result<()> {
    let tx = db.begin().await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to begin Proxmox match transaction: {e}"
        )))
    })?;

    tracing::debug!(
        %mapping_id,
        %host_id,
        method = method.as_str(),
        "applying Proxmox guest-to-host match"
    );

    let mapping = proxmox_host_mapping::Entity::find_by_id(mapping_id)
        .one(&tx)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to find mapping: {e}"
            )))
        })?
        .ok_or_else(|| {
            rootcause::report!(ProxmoxError::Database(format!(
                "mapping {mapping_id} not found"
            )))
        })?;

    // Preserve one-host-to-one-mapping invariant by clearing stale/conflicting
    // rows before assigning this host to the requested mapping.
    let conflicts = proxmox_host_mapping::Entity::find()
        .filter(proxmox_host_mapping::Column::HostId.eq(host_id))
        .filter(proxmox_host_mapping::Column::Id.ne(mapping_id))
        .all(&tx)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to query conflicting mappings: {e}"
            )))
        })?;

    for conflict in conflicts {
        tracing::warn!(
            conflict_mapping_id = %conflict.id,
            %host_id,
            "clearing conflicting Proxmox mapping to preserve host uniqueness"
        );

        let mut conflict_active: proxmox_host_mapping::ActiveModel = conflict.into();
        conflict_active.host_id = Set(None);
        conflict_active.match_method = Set(None);
        conflict_active.update(&tx).await.map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to clear conflicting mapping: {e}"
            )))
        })?;
    }

    let mut active: proxmox_host_mapping::ActiveModel = mapping.into();
    active.host_id = Set(Some(host_id));
    active.match_method = Set(Some(method.as_str().to_string()));
    active.update(&tx).await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to update mapping: {e}"
        )))
    })?;

    tx.commit().await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to commit Proxmox match transaction: {e}"
        )))
    })?;

    tracing::info!(
        %mapping_id,
        %host_id,
        method = method.as_str(),
        "Proxmox guest matched to host"
    );

    Ok(())
}

/// Remove a match from a mapping.
pub async fn unmatch(db: &DatabaseConnection, mapping_id: Uuid) -> Result<()> {
    tracing::debug!(%mapping_id, "removing Proxmox guest-to-host match");

    let mapping = proxmox_host_mapping::Entity::find_by_id(mapping_id)
        .one(db)
        .await
        .map_err(|e| {
            rootcause::report!(ProxmoxError::Database(format!(
                "failed to find mapping: {e}"
            )))
        })?
        .ok_or_else(|| {
            rootcause::report!(ProxmoxError::Database(format!(
                "mapping {mapping_id} not found"
            )))
        })?;

    let mut active: proxmox_host_mapping::ActiveModel = mapping.into();
    active.host_id = Set(None);
    active.match_method = Set(None);
    active.update(db).await.map_err(|e| {
        rootcause::report!(ProxmoxError::Database(format!(
            "failed to update mapping: {e}"
        )))
    })?;

    tracing::info!(%mapping_id, "Proxmox guest-to-host match removed");

    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;
    use sea_orm::{DbBackend, MockDatabase, MockExecResult};
    use time::OffsetDateTime;

    fn make_mapping(
        id: Uuid,
        hostname: Option<&str>,
        ip_addresses: Option<&str>,
        machine_id: Option<&str>,
        proxmox_name: Option<&str>,
    ) -> proxmox_host_mapping::Model {
        proxmox_host_mapping::Model {
            id,
            tenant_id: Uuid::nil(),
            plugin_config_id: Uuid::nil(),
            host_id: None,
            proxmox_node: "pve1".to_string(),
            proxmox_vmid: 100,
            proxmox_type: "qemu".to_string(),
            proxmox_name: proxmox_name.map(|s| s.to_string()),
            proxmox_status: "running".to_string(),
            hostname: hostname.map(|s| s.to_string()),
            ip_addresses: ip_addresses.map(|s| s.to_string()),
            machine_id: machine_id.map(|s| s.to_string()),
            match_method: None,
            discovered_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn make_host(
        id: Uuid,
        hostname: &str,
        friendly_name: &str,
        ip: Option<&str>,
        machine_id: &str,
    ) -> host::Model {
        host::Model {
            id,
            tenant_id: Uuid::nil(),
            machine_id: machine_id.to_string(),
            hostname: hostname.to_string(),
            friendly_name: friendly_name.to_string(),
            os_type: None,
            os_version: None,
            architecture: None,
            ip_address: ip.map(|s| s.to_string()),
            host_features: None,
            last_seen_at: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            deactivated_at: None,
        }
    }

    // ── MatchMethod tests ───────────────────────────────────────────────

    #[test]
    fn match_method_as_str() {
        assert_eq!(MatchMethod::Manual.as_str(), "manual");
        assert_eq!(
            MatchMethod::SuggestedMachineId.as_str(),
            "suggested_machine_id"
        );
        assert_eq!(
            MatchMethod::SuggestedHostname.as_str(),
            "suggested_hostname"
        );
        assert_eq!(MatchMethod::SuggestedIp.as_str(), "suggested_ip");
        assert_eq!(
            MatchMethod::SuggestedHostnameIp.as_str(),
            "suggested_hostname_ip"
        );
        assert_eq!(MatchMethod::SuggestedName.as_str(), "suggested_name");
    }

    #[test]
    fn match_method_roundtrip() {
        for variant in MatchMethod::KNOWN_VARIANTS {
            let s = variant.as_str();
            let parsed: MatchMethod = s.parse().unwrap();
            assert_eq!(&parsed, variant, "roundtrip failed for {s}");
        }
    }

    #[test]
    fn match_method_from_str_unknown() {
        assert!("bogus".parse::<MatchMethod>().is_err());
    }

    // ── MatchConfidence tests ───────────────────────────────────────────

    #[test]
    fn confidence_ordering() {
        assert!(MatchConfidence::Low < MatchConfidence::Medium);
        assert!(MatchConfidence::Medium < MatchConfidence::High);
    }

    // ── compute_suggestions tests ───────────────────────────────────────

    #[test]
    fn empty_inputs() {
        assert!(compute_suggestions(&[], &[]).is_empty());

        let m = make_mapping(Uuid::now_v7(), Some("web"), None, None, None);
        assert!(compute_suggestions(&[m], &[]).is_empty());

        let h = make_host(Uuid::now_v7(), "web", "web", None, "mid1");
        assert!(compute_suggestions(&[], &[h]).is_empty());
    }

    #[test]
    fn machine_id_exact_match() {
        let mid = Uuid::now_v7();
        let hid = Uuid::now_v7();
        let mappings = [make_mapping(mid, Some("web"), None, Some("abc123"), None)];
        let hosts = [make_host(hid, "web", "web", None, "abc123")];

        let suggestions = compute_suggestions(&mappings, &hosts);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].mapping_id, mid);
        assert_eq!(suggestions[0].host_id, hid);
        assert_eq!(suggestions[0].confidence, MatchConfidence::High);
        assert_eq!(suggestions[0].match_method, MatchMethod::SuggestedMachineId);
    }

    #[test]
    fn hostname_and_ip_match() {
        let mid = Uuid::now_v7();
        let hid = Uuid::now_v7();
        let ips = r#"["10.0.0.1","10.0.0.2"]"#;
        let mappings = [make_mapping(mid, Some("web-server"), Some(ips), None, None)];
        let hosts = [make_host(
            hid,
            "web-server",
            "web-server",
            Some("10.0.0.1"),
            "m1",
        )];

        let suggestions = compute_suggestions(&mappings, &hosts);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].confidence, MatchConfidence::High);
        assert_eq!(
            suggestions[0].match_method,
            MatchMethod::SuggestedHostnameIp
        );
    }

    #[test]
    fn hostname_only_match() {
        let mid = Uuid::now_v7();
        let hid = Uuid::now_v7();
        let mappings = [make_mapping(mid, Some("Web-Server"), None, None, None)];
        let hosts = [make_host(
            hid,
            "web-server",
            "web-server",
            Some("10.0.0.99"),
            "m1",
        )];

        let suggestions = compute_suggestions(&mappings, &hosts);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].confidence, MatchConfidence::Medium);
        assert_eq!(suggestions[0].match_method, MatchMethod::SuggestedHostname);
    }

    #[test]
    fn ip_only_match() {
        let mid = Uuid::now_v7();
        let hid = Uuid::now_v7();
        let ips = r#"["192.168.1.5"]"#;
        let mappings = [make_mapping(mid, Some("vm1"), Some(ips), None, None)];
        let hosts = [make_host(
            hid,
            "different-name",
            "different",
            Some("192.168.1.5"),
            "m1",
        )];

        let suggestions = compute_suggestions(&mappings, &hosts);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].confidence, MatchConfidence::Medium);
        assert_eq!(suggestions[0].match_method, MatchMethod::SuggestedIp);
    }

    #[test]
    fn name_only_match() {
        let mid = Uuid::now_v7();
        let hid = Uuid::now_v7();
        let mappings = [make_mapping(mid, None, None, None, Some("my-server"))];
        let hosts = [make_host(hid, "other", "my-server", None, "m1")];

        let suggestions = compute_suggestions(&mappings, &hosts);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].confidence, MatchConfidence::Low);
        assert_eq!(suggestions[0].match_method, MatchMethod::SuggestedName);
    }

    #[test]
    fn no_match() {
        let mappings = [make_mapping(Uuid::now_v7(), Some("web"), None, None, None)];
        let hosts = [make_host(Uuid::now_v7(), "db", "database", None, "m1")];

        let suggestions = compute_suggestions(&mappings, &hosts);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn multiple_mappings_highest_confidence_wins() {
        let m1 = Uuid::now_v7();
        let m2 = Uuid::now_v7();
        let hid = Uuid::now_v7();

        // m1 matches by name only (Low), m2 matches by machine_id (High)
        let mappings = [
            make_mapping(m1, None, None, None, Some("web")),
            make_mapping(m2, Some("other"), None, Some("exact-mid"), None),
        ];
        let hosts = [make_host(hid, "web", "web", None, "exact-mid")];

        let suggestions = compute_suggestions(&mappings, &hosts);
        // Host is used by m2 (High), m1 gets nothing
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].mapping_id, m2);
        assert_eq!(suggestions[0].confidence, MatchConfidence::High);
    }

    #[test]
    fn each_host_used_at_most_once() {
        let m1 = Uuid::now_v7();
        let m2 = Uuid::now_v7();
        let hid = Uuid::now_v7();

        // Both mappings match the same host by hostname
        let mappings = [
            make_mapping(m1, Some("web"), None, None, None),
            make_mapping(m2, Some("web"), None, None, None),
        ];
        let hosts = [make_host(hid, "web", "web", None, "m1")];

        let suggestions = compute_suggestions(&mappings, &hosts);
        assert_eq!(
            suggestions.len(),
            1,
            "each host should be used at most once"
        );
    }

    #[test]
    fn multiple_hosts_matched_to_multiple_mappings() {
        let m1 = Uuid::now_v7();
        let m2 = Uuid::now_v7();
        let h1 = Uuid::now_v7();
        let h2 = Uuid::now_v7();

        let mappings = [
            make_mapping(m1, Some("web"), Some(r#"["10.0.0.1"]"#), None, None),
            make_mapping(m2, Some("db"), Some(r#"["10.0.0.2"]"#), None, None),
        ];
        let hosts = [
            make_host(h1, "web", "web", Some("10.0.0.1"), "mid1"),
            make_host(h2, "db", "db", Some("10.0.0.2"), "mid2"),
        ];

        let suggestions = compute_suggestions(&mappings, &hosts);
        assert_eq!(suggestions.len(), 2);
    }

    #[tokio::test]
    async fn manual_match_preserves_single_host_mapping_under_conflict() {
        let tenant_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let mapping_id = Uuid::now_v7();
        let conflicting_mapping_id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();

        let mapping = proxmox_host_mapping::Model {
            id: mapping_id,
            tenant_id,
            plugin_config_id,
            host_id: None,
            proxmox_node: "pve1".to_string(),
            proxmox_vmid: 100,
            proxmox_type: "qemu".to_string(),
            proxmox_name: Some("vm-100".to_string()),
            proxmox_status: "running".to_string(),
            hostname: Some("vm-100".to_string()),
            ip_addresses: None,
            machine_id: None,
            match_method: None,
            discovered_at: now,
            updated_at: now,
        };
        let conflict = proxmox_host_mapping::Model {
            id: conflicting_mapping_id,
            tenant_id,
            plugin_config_id,
            host_id: Some(host_id),
            proxmox_node: "pve1".to_string(),
            proxmox_vmid: 101,
            proxmox_type: "qemu".to_string(),
            proxmox_name: Some("vm-101".to_string()),
            proxmox_status: "running".to_string(),
            hostname: Some("vm-101".to_string()),
            ip_addresses: None,
            machine_id: None,
            match_method: Some(MatchMethod::Manual.as_str().to_string()),
            discovered_at: now,
            updated_at: now,
        };

        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_results([vec![mapping.clone()]])
            .append_query_results([vec![conflict.clone()]])
            .append_query_results([vec![conflict]])
            .append_query_results([vec![mapping]])
            .append_exec_results([
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                },
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                },
            ])
            .into_connection();

        let result = manual_match(&db, mapping_id, host_id).await;
        assert!(result.is_ok(), "manual match should succeed: {result:?}");

        let logs = db.into_transaction_log();
        assert_eq!(
            logs.len(),
            1,
            "conflict clear + assignment should be coherent in one transaction"
        );
        let statements: Vec<String> = logs
            .iter()
            .flat_map(|tx| tx.statements().iter())
            .map(ToString::to_string)
            .collect();

        let update_statements: Vec<&String> = statements
            .iter()
            .filter(|sql| sql.contains("UPDATE `proxmox_host_mappings`"))
            .collect();
        assert_eq!(
            update_statements.len(),
            2,
            "expected one conflicting-row clear and one assignment update"
        );
        assert!(
            update_statements
                .iter()
                .any(|sql| sql.contains("`host_id` = NULL")),
            "one update should clear the conflicting mapping"
        );
        assert!(
            update_statements
                .iter()
                .any(|sql| sql.contains("`match_method` = 'manual'")),
            "one update should set the manual match method on target mapping"
        );
    }
}
