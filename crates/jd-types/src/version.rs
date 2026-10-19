//! Schema versioning for documents and API envelopes.

use serde::{Deserialize, Serialize};

/// A monotonically increasing document/API schema version.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct SchemaVersion(pub u32);

impl SchemaVersion {
    /// The version this build writes and the newest it can read.
    pub const CURRENT: SchemaVersion = SchemaVersion(1);

    /// True when a document at `self` can be opened by this build (possibly after migration).
    pub fn is_supported(self) -> bool {
        self.0 >= 1 && self <= Self::CURRENT
    }
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
