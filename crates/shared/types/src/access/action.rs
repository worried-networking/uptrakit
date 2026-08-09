use std::str::FromStr;

use super::bounds;
use super::catalog::{CATALOG, ParseResourceError, Resource, VerbEntry};
use super::selector::SelectorSupport;
use super::verb::{ParseVerbError, Verb};

/// A concrete access action: `resource:verb`.
///
/// Fields are `pub(crate)` on purpose: a matrix-invalid built-in pair
/// (`hosts:approve`) must be unrepresentable, so construction goes through
/// [`Action::new`] / `FromStr` (or the in-crate catalog constants).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Action {
    pub(crate) resource: Resource,
    pub(crate) verb: Verb,
}

/// Error returned when parsing an invalid [`Action`] string.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseActionError {
    /// Longer than [`bounds::MAX_PATTERN_LEN`] bytes; checked before any
    /// parsing, never truncated.
    #[error("action string exceeds {max} bytes")]
    TooLong { max: usize },
    /// Not exactly `resource:verb` with non-empty sides.
    #[error("action must be `resource:verb`")]
    Structure,
    /// Verb outside the closed set (`clippy::map_err_ignore`-safe: the
    /// source rides along via `#[from]`, so `FromStr` uses plain `?`).
    #[error("unknown verb")]
    UnknownVerb(#[from] ParseVerbError),
    /// Resource-side failure.
    #[error(transparent)]
    Resource(#[from] ParseResourceError),
    /// Both sides valid but the validity matrix rejects the pair.
    #[error("verb `{verb}` is not valid for resource `{resource}`")]
    InvalidPair { resource: String, verb: Verb },
}

impl Action {
    /// Builds an action, enforcing the validity matrix for built-in
    /// resources (dynamic resources accept any closed-set verb; registry
    /// narrowing is decision-time).
    pub fn new(resource: Resource, verb: Verb) -> Result<Self, ParseActionError> {
        if resource.allowed_verbs().contains(&verb) {
            Ok(Self { resource, verb })
        } else {
            Err(ParseActionError::InvalidPair {
                resource: resource.as_str().to_string(),
                verb,
            })
        }
    }

    pub fn resource(&self) -> &Resource {
        &self.resource
    }

    pub fn verb(&self) -> Verb {
        self.verb
    }

    /// Per-action selector-support level from the catalog. Dynamic
    /// actions and (unreachable by construction) uncatalogued pairs are
    /// `SelectorSupport::None` — the most restrictive level, fail-closed.
    pub fn selector_support(&self) -> SelectorSupport {
        self.verb_entry()
            .map(|ve| ve.selector_support)
            .unwrap_or(SelectorSupport::None)
    }

    /// Catalog description for built-in actions; `None` for dynamic.
    pub fn description(&self) -> Option<&'static str> {
        self.verb_entry().map(|ve| ve.description)
    }

    fn verb_entry(&self) -> Option<&'static VerbEntry> {
        CATALOG
            .iter()
            .find(|e| e.resource == self.resource)
            .and_then(|e| e.verbs.iter().find(|ve| ve.verb == self.verb))
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.resource.as_str(), self.verb.as_str())
    }
}

impl FromStr for Action {
    type Err = ParseActionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > bounds::MAX_PATTERN_LEN {
            return Err(ParseActionError::TooLong {
                max: bounds::MAX_PATTERN_LEN,
            });
        }
        let Some((resource_s, verb_s)) = s.split_once(':') else {
            return Err(ParseActionError::Structure);
        };
        if resource_s.is_empty() || verb_s.is_empty() || verb_s.contains(':') {
            return Err(ParseActionError::Structure);
        }
        let verb: Verb = verb_s.parse()?;
        let resource: Resource = resource_s.parse()?;
        Self::new(resource, verb)
    }
}

impl serde::Serialize for Action {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Action {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// Open string schema: the action set is open (dynamic namespaces), so this
// is deliberately NOT an enum schema — the grammar is documented and the
// built-in catalog enumerated in the description (sourced from CATALOG,
// never a hand-maintained list). Divergence from the legacy permission
// enum's closed-schema treatment is mandated by the spec.
#[cfg(feature = "openapi")]
impl utoipa::PartialSchema for Action {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        let built_ins: Vec<&str> = CATALOG
            .iter()
            .flat_map(|e| e.verbs.iter().map(|ve| ve.action_str))
            .collect();
        let description = format!(
            "Access action `resource:verb`. Resources are dot-separated \
             kebab-case segments; verbs: read, create, update, delete, \
             trigger, approve, reject, manage, use. Dynamic namespaces \
             `plugin.<plugin_type>` and `surface.<surface_id>` are valid at \
             runtime. Built-in actions: {}.",
            built_ins.join(", ")
        );
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .description(Some(description))
            .into()
    }
}

#[cfg(feature = "openapi")]
impl utoipa::ToSchema for Action {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("Action")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::actions;

    #[test]
    fn a1_every_catalog_action_round_trips() {
        assert!(!CATALOG.is_empty());
        for entry in CATALOG {
            for ve in entry.verbs {
                let action: Action = ve.action_str.parse().expect("parses");
                assert_eq!(action.to_string(), ve.action_str);
                assert_eq!(action.resource().as_str(), entry.resource_str);
                assert_eq!(action.verb(), ve.verb);
                assert_eq!(action.selector_support(), ve.selector_support);
                assert_eq!(action.description(), Some(ve.description));
            }
        }
    }

    #[test]
    fn typed_constants_match_their_strings() {
        // Spot known members; full coverage is the A1 loop plus this
        // sample proving const/string pairing.
        let cases: &[(&Action, &str)] = &[
            (&actions::HOSTS_READ, actions::HOSTS_READ_STR),
            (&actions::UPDATES_TRIGGER, actions::UPDATES_TRIGGER_STR),
            (&actions::ACCESS_MANAGE, actions::ACCESS_MANAGE_STR),
            (&actions::MCP_USE, actions::MCP_USE_STR),
        ];
        for (action, s) in cases {
            assert_eq!(&action.to_string(), s);
            assert_eq!(&s.parse::<Action>().expect("parses"), *action);
        }
    }

    #[test]
    fn a2_a3_dynamic_and_edge_shapes_parse() {
        for good in [
            "plugin.package-manager.apt:manage",
            "surface.ssh-agent.hosts:use",
            "plugin.a:read",
            "surface.a.b1.c-d:trigger",
        ] {
            let action: Action = good.parse().expect("parses");
            assert_eq!(action.to_string(), good);
        }
    }

    #[test]
    fn dynamic_actions_fail_closed_on_catalog_metadata() {
        let action: Action = "plugin.foo:trigger".parse().expect("parses");
        assert_eq!(action.selector_support(), SelectorSupport::None);
        assert_eq!(action.description(), None);
    }

    #[test]
    fn a4_to_a9_rejections() {
        use ParseActionError as E;
        let too_long = format!("surface.{}:use", "a".repeat(bounds::MAX_PATTERN_LEN));
        type Check = fn(&E) -> bool;
        let cases: &[(&str, Check)] = &[
            ("frobnicate:read", |e| {
                matches!(e, E::Resource(ParseResourceError::UnknownResource))
            }),
            ("hosts:write", |e| matches!(e, E::UnknownVerb(_))),
            ("hosts:list", |e| matches!(e, E::UnknownVerb(_))),
            ("hosts:approve", |e| matches!(e, E::InvalidPair { .. })),
            ("settings:trigger", |e| matches!(e, E::InvalidPair { .. })),
            ("plugin.Apt:read", |e| {
                matches!(e, E::Resource(ParseResourceError::InvalidSegment))
            }),
            ("plugin.a..b:read", |e| {
                matches!(e, E::Resource(ParseResourceError::InvalidSegment))
            }),
            ("plugin.:read", |e| {
                matches!(e, E::Resource(ParseResourceError::EmptyDynamicRemainder))
            }),
            ("hosts", |e| matches!(e, E::Structure)),
            ("read", |e| matches!(e, E::Structure)),
            (":read", |e| matches!(e, E::Structure)),
            ("hosts:", |e| matches!(e, E::Structure)),
            ("a:b:c", |e| matches!(e, E::Structure)),
            (&too_long, |e| matches!(e, E::TooLong { .. })),
        ];
        for (input, check) in cases {
            let err = input.parse::<Action>().expect_err("must reject");
            assert!(check(&err), "wrong error {err:?} for {input:?}");
        }
    }

    #[test]
    fn a9_at_bound_accepted() {
        // Exactly MAX_PATTERN_LEN bytes must parse.
        let pad = bounds::MAX_PATTERN_LEN - "surface.:use".len();
        let s = format!("surface.{}:use", "a".repeat(pad));
        assert_eq!(s.len(), bounds::MAX_PATTERN_LEN);
        assert!(s.parse::<Action>().is_ok(), "at-bound length must parse");
    }

    #[test]
    fn constructor_seam_matrix_checked() {
        let err = Action::new(Resource::Hosts, Verb::Approve).expect_err("matrix rejects");
        assert!(matches!(err, ParseActionError::InvalidPair { .. }));
        let dynamic = Resource::plugin("foo").expect("builds");
        assert!(
            Action::new(dynamic, Verb::Approve).is_ok(),
            "dynamic resources accept any closed-set verb"
        );
    }

    #[test]
    fn serde_round_trip_and_deny() {
        let action = actions::HOSTS_READ;
        let json = serde_json::to_string(&action).expect("serialize");
        assert_eq!(json, r#""hosts:read""#);
        let back: Action = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, action);
        assert!(
            serde_json::from_str::<Action>(r#""hosts:approve""#).is_err(),
            "matrix-invalid action must fail deserialization"
        );
        assert!(
            serde_json::from_str::<Action>(r#""nonsense""#).is_err(),
            "malformed action must fail deserialization"
        );
    }

    #[cfg(feature = "openapi")]
    mod openapi_tests {
        use super::*;

        #[test]
        fn schema_is_open_string_with_catalog_docs() {
            let schema = <Action as utoipa::PartialSchema>::schema();
            let json = serde_json::to_value(&schema).expect("schema json");
            assert_eq!(json["type"], "string");
            assert!(json.get("enum").is_none(), "must be open, not an enum");
            let desc = json["description"].as_str().expect("description");
            assert!(desc.contains("hosts:read"));
            assert!(desc.contains("plugin.<plugin_type>"));
        }
    }
}
