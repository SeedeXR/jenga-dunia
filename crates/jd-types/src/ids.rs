//! ULID-backed identifiers. Stored as strings in JSON; deserialisation validates the ULID so
//! ids entering through any adapter are well-formed.

use serde::{Deserialize, Deserializer, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, schemars::JsonSchema)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// A fresh, time-ordered identifier.
            pub fn new() -> Self {
                Self(ulid::Ulid::new().to_string())
            }
            /// The string form.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                ulid::Ulid::from_string(s).map(|u| Self(u.to_string()))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(|e| serde::de::Error::custom(format!("invalid ULID `{s}`: {e}")))
            }
        }
    };
}

id_type!(
    /// Identifies a project document.
    ProjectId
);
id_type!(
    /// Identifies a graph node.
    NodeId
);
id_type!(
    /// Identifies a background job.
    JobId
);
id_type!(
    /// Correlates a request across logs and adapters.
    RequestId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_through_json_and_reject_malformed_input() {
        let id = NodeId::new();
        let json = serde_json::to_string(&id).expect("json");
        assert_eq!(json, format!("\"{}\"", id.as_str()));
        let back: NodeId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, id);
        let parsed: NodeId = id.as_str().parse().expect("valid ulid");
        assert_eq!(parsed, id);
        assert!("not-a-ulid".parse::<NodeId>().is_err());
        assert!(
            serde_json::from_str::<NodeId>("\"not-a-ulid\"").is_err(),
            "JSON ids must be validated"
        );
        assert!(serde_json::from_str::<NodeId>("42").is_err());
    }
}
