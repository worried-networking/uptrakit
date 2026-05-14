use rootcause::prelude::*;
use serde::Deserialize;
use url::Url;

use crate::error::{Result, SkillsError};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillEntryDto {
    source: String,
    source_url: String,
    source_type: String,
    skill_path: String,
    skill_folder_hash: String,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "forward-declared for future plugin tasks; struct not yet constructed outside tests"
    )
)]
#[derive(Debug)]
pub(crate) struct SkillLockEntry {
    pub(crate) name: String,
    #[expect(
        dead_code,
        reason = "stored from lock file for completeness; not read at runtime"
    )]
    pub(crate) source: String,
    pub(crate) source_url: String,
    pub(crate) source_type: String,
    pub(crate) skill_path: String,
    pub(crate) skill_folder_hash: String,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "forward-declared for future plugin tasks; not called outside tests"
    )
)]
pub(crate) fn parse_skill_lock(json: &str) -> Result<Vec<SkillLockEntry>> {
    let raw: std::collections::HashMap<String, serde_json::Value> = serde_json::from_str(json)
        .map_err(|e| report!(SkillsError::LockFileMalformed(e.to_string())))?;

    let mut entries = Vec::with_capacity(raw.len());
    for (name, value) in raw {
        let dto: SkillEntryDto = match serde_json::from_value(value) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(skill = %name, error = %e, "skipping malformed skill lock entry");
                continue;
            }
        };
        entries.push(SkillLockEntry {
            name,
            source: dto.source,
            source_url: dto.source_url,
            source_type: dto.source_type,
            skill_path: dto.skill_path,
            skill_folder_hash: dto.skill_folder_hash,
        });
    }
    Ok(entries)
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "forward-declared for future plugin tasks; not called outside tests"
    )
)]
pub(crate) fn encode_skill_identifier(source_url: &str, skill_path: &str) -> String {
    format!("{source_url}#{skill_path}")
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "called from SkillsConfig::validate_identifier; SkillsConfig not yet externally constructed"
    )
)]
pub(crate) fn parse_skill_identifier(id: &str) -> Result<(Url, String)> {
    if id.len() > 1024 {
        return Err(report!(SkillsError::InvalidIdentifier(
            "identifier exceeds 1024 bytes".to_string()
        )));
    }

    let hash_pos = id.find('#').ok_or_else(|| {
        report!(SkillsError::InvalidIdentifier(
            "identifier must contain '#' separator between URL and skill path".to_string()
        ))
    })?;

    let url_part = &id[..hash_pos];
    let path_part = &id[hash_pos + 1..];

    if !url_part.starts_with("https://") && !url_part.starts_with("http://") {
        return Err(report!(SkillsError::InvalidIdentifier(
            "identifier URL must start with https:// or http://".to_string()
        )));
    }

    let url = Url::parse(url_part).map_err(|e| {
        report!(SkillsError::InvalidIdentifier(format!(
            "invalid URL in identifier: {e}"
        )))
    })?;

    if path_part.is_empty() || path_part.len() > 512 {
        return Err(report!(SkillsError::InvalidIdentifier(
            "skill path must be 1–512 bytes".to_string()
        )));
    }
    if path_part.starts_with('/') {
        return Err(report!(SkillsError::InvalidIdentifier(
            "skill path must not start with '/'".to_string()
        )));
    }
    for ch in path_part.chars() {
        if ch.is_control() {
            return Err(report!(SkillsError::InvalidIdentifier(
                "skill path must not contain control characters".to_string()
            )));
        }
    }
    for segment in path_part.split('/') {
        if segment == ".." {
            return Err(report!(SkillsError::InvalidIdentifier(
                "skill path must not contain '..' segments".to_string()
            )));
        }
    }

    Ok((url, path_part.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LOCK: &str = r#"{
      "brainstorming": {
        "source": "obra/superpowers",
        "sourceUrl": "https://github.com/obra/superpowers",
        "sourceType": "github",
        "skillPath": "skills/brainstorming/SKILL.md",
        "skillFolderHash": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        "installedAt": "2025-01-01T00:00:00Z",
        "updatedAt": "2025-01-02T00:00:00Z"
      },
      "spec": {
        "source": "obra/superpowers",
        "sourceUrl": "https://github.com/obra/superpowers",
        "sourceType": "github",
        "skillPath": "skills/spec/SKILL.md",
        "skillFolderHash": "cafecafecafecafecafecafecafecafecafecafe",
        "installedAt": "2025-01-01T00:00:00Z",
        "updatedAt": "2025-01-02T00:00:00Z"
      }
    }"#;

    const NON_GITHUB_LOCK: &str = r#"{
      "local-skill": {
        "source": "local/source",
        "sourceUrl": "https://gitlab.com/local/source",
        "sourceType": "gitlab",
        "skillPath": "skills/local-skill/SKILL.md",
        "skillFolderHash": "aabbccddaabbccddaabbccddaabbccddaabbccdd"
      }
    }"#;

    #[test]
    fn parse_valid_lock_returns_entries() {
        let entries = parse_skill_lock(SAMPLE_LOCK).expect("parse ok");
        assert_eq!(entries.len(), 2);
        let bs = entries
            .iter()
            .find(|e| e.name == "brainstorming")
            .expect("brainstorming");
        assert_eq!(bs.source_url, "https://github.com/obra/superpowers");
        assert_eq!(bs.source_type, "github");
        assert_eq!(bs.skill_path, "skills/brainstorming/SKILL.md");
        assert_eq!(
            bs.skill_folder_hash,
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        );
    }

    #[test]
    fn parse_non_github_entry_still_included() {
        let entries = parse_skill_lock(NON_GITHUB_LOCK).expect("parse ok");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source_type, "gitlab");
    }

    #[test]
    fn parse_malformed_json_fails() {
        let result = parse_skill_lock("not json");
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_object_returns_empty_vec() {
        let entries = parse_skill_lock("{}").expect("parse ok");
        assert!(entries.is_empty());
    }

    #[test]
    fn encode_roundtrips_through_parse() {
        let encoded = encode_skill_identifier(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        );
        let (url, path) = parse_skill_identifier(&encoded).expect("parse ok");
        assert_eq!(url.as_str(), "https://github.com/obra/superpowers");
        assert_eq!(path, "skills/brainstorming/SKILL.md");
    }

    #[test]
    fn parse_identifier_rejects_no_hash() {
        let result = parse_skill_identifier("https://github.com/owner/repo");
        assert!(result.is_err());
    }

    #[test]
    fn parse_identifier_rejects_path_traversal() {
        let result = parse_skill_identifier("https://github.com/owner/repo#skills/../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn parse_identifier_rejects_leading_slash_in_path() {
        let result =
            parse_skill_identifier("https://github.com/owner/repo#/skills/brainstorming/SKILL.md");
        assert!(result.is_err());
    }

    #[test]
    fn parse_identifier_rejects_empty_path() {
        let result = parse_skill_identifier("https://github.com/owner/repo#");
        assert!(result.is_err());
    }

    #[test]
    fn parse_identifier_rejects_total_length_over_1024() {
        let long_path = "a".repeat(1014);
        let id = format!("https://github.com/o/r#{long_path}");
        assert!(id.len() > 1024);
        let result = parse_skill_identifier(&id);
        assert!(result.is_err());
    }
}
