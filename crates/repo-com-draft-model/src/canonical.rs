use std::fmt::Write as _;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::{
    AuthorizedReplyReference, DestinationAlias, DraftBody, DraftMetadata, DraftRevision, EventType,
    Severity,
};

pub const HASH_ALGORITHM: &str = "sha256";
pub const HASH_HEX_LENGTH: usize = 64;
const CANONICAL_VERSION: u8 = 1;

#[derive(Serialize)]
struct CanonicalRevision<'a> {
    canonical_version: u8,
    text: &'a DraftBody,
    metadata: &'a DraftMetadata,
    event_type: &'a EventType,
    severity: &'a Severity,
    destination_alias: &'a DestinationAlias,
    resolved_destination: &'a repo_com_config::ResolvedDestination,
    repository_id: &'a str,
    expires_at_unix_seconds: u64,
    reply_reference: Option<&'a AuthorizedReplyReference>,
}

impl<'a> From<&'a DraftRevision> for CanonicalRevision<'a> {
    fn from(revision: &'a DraftRevision) -> Self {
        Self {
            canonical_version: CANONICAL_VERSION,
            text: &revision.text,
            metadata: &revision.metadata,
            event_type: &revision.event_type,
            severity: &revision.severity,
            destination_alias: &revision.destination_alias,
            resolved_destination: &revision.resolved_destination,
            repository_id: &revision.repository_id,
            expires_at_unix_seconds: revision.expiry.expires_at_unix_seconds(),
            reply_reference: revision.reply_reference.as_ref(),
        }
    }
}

impl DraftRevision {
    #[must_use]
    pub fn canonical_json(&self) -> String {
        serde_json::to_string(&CanonicalRevision::from(self))
            .expect("draft revision canonical fields contain only serializable values")
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&CanonicalRevision::from(self))
            .expect("draft revision canonical fields contain only serializable values")
    }

    #[must_use]
    pub fn canonical_hash(&self) -> String {
        sha256_hex(&self.canonical_bytes())
    }

    #[must_use]
    pub fn canonical_hash_bytes(&self) -> [u8; 32] {
        Sha256::digest(self.canonical_bytes()).into()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(HASH_HEX_LENGTH);
    for byte in digest {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}
