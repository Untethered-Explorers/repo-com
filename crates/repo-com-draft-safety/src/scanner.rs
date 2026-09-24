use std::fmt;
use std::ops::Range;

use repo_com_draft_content::RenderedMessage;
use repo_com_draft_model::{DraftMetadata, DraftPreview, DraftRevision};
use serde::{Deserialize, Serialize};

use crate::patterns::{
    MatchSource, MetadataField, SecretFinding, SecretLocation, SecretReasonCode,
};

const AUTHORIZATION_NAMES: &[&[u8]] = &[b"proxy-authorization", b"authorization"];
const AUTHORIZATION_SCHEMES: &[&[u8]] = &[b"bearer", b"bot", b"basic", b"token", b"discord"];
const PRIVATE_KEY_MARKERS: &[&[u8]] = &[
    b"-----BEGIN PRIVATE KEY-----",
    b"-----BEGIN ENCRYPTED PRIVATE KEY-----",
    b"-----BEGIN RSA PRIVATE KEY-----",
    b"-----BEGIN EC PRIVATE KEY-----",
    b"-----BEGIN DSA PRIVATE KEY-----",
    b"-----BEGIN OPENSSH PRIVATE KEY-----",
    b"-----BEGIN PGP PRIVATE KEY BLOCK-----",
];
const SENSITIVE_QUERY_KEYS: &[&[u8]] = &[
    b"accesstoken",
    b"auth",
    b"authtoken",
    b"authorization",
    b"apikey",
    b"clientsecret",
    b"credential",
    b"idtoken",
    b"jwttoken",
    b"oauthtoken",
    b"password",
    b"passwd",
    b"privatekey",
    b"refreshtoken",
    b"secret",
    b"sessiontoken",
    b"token",
];
const WEAK_SENSITIVE_QUERY_KEYS: &[&[u8]] = &[
    b"key",
    b"sig",
    b"signature",
    b"xamzcredential",
    b"xamzsignature",
    b"xgoogcredential",
    b"xgoogsignature",
];
const ASSIGNMENT_KEYS: &[&[u8]] = &[
    b"accesskey",
    b"accesstoken",
    b"apikey",
    b"apisecret",
    b"authtoken",
    b"authorization",
    b"bottoken",
    b"clientsecret",
    b"connectionstring",
    b"cookie",
    b"credential",
    b"discordtoken",
    b"jwt",
    b"passphrase",
    b"passwd",
    b"password",
    b"privatekey",
    b"refreshtoken",
    b"secret",
    b"secretkey",
    b"secretaccesskey",
    b"sessiontoken",
    b"signingkey",
    b"token",
    b"webhooktoken",
    b"xapikey",
];
const PLACEHOLDER_WORDS: &[&[u8]] = &[
    b"changeme",
    b"example",
    b"placeholder",
    b"redacted",
    b"replace_me",
    b"your_token",
    b"your_password",
    b"dummy",
    b"none",
    b"null",
    b"undefined",
    b"true",
    b"false",
];

/// Pure, deterministic pre-send credential scanner.
///
/// The scanner owns no state and performs no file, process, terminal, or
/// network I/O. It returns only reason codes and numeric source locations;
/// matched values are never retained in a result.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SecretScanner;

impl SecretScanner {
    /// Creates the stateless scanner.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Scans exact rendered text and bounded draft metadata.
    ///
    /// The metadata is scanned field by field, so a finding can identify the
    /// safe metadata field without copying its value into the result.
    #[must_use]
    pub fn scan(&self, text: &str, metadata: &DraftMetadata) -> SecretScanResult {
        self.scan_input(SecretScanInput::with_metadata(text, metadata))
    }

    /// Compatibility alias for [`Self::scan`].
    #[must_use]
    pub fn scan_with_metadata(&self, text: &str, metadata: &DraftMetadata) -> SecretScanResult {
        self.scan(text, metadata)
    }

    /// Compatibility alias for [`Self::scan`].
    #[must_use]
    pub fn scan_text_and_metadata(&self, text: &str, metadata: &DraftMetadata) -> SecretScanResult {
        self.scan(text, metadata)
    }

    /// Scans text when a caller has no metadata projection.
    #[must_use]
    pub fn scan_text(&self, text: &str) -> SecretScanResult {
        self.scan_input(SecretScanInput::new(text))
    }

    /// Compatibility alias for [`Self::scan_text`].
    #[must_use]
    pub fn scan_text_only(&self, text: &str) -> SecretScanResult {
        self.scan_text(text)
    }

    /// Scans the exact text and metadata carried by a rendered revision.
    #[must_use]
    pub fn scan_rendered(&self, rendered: &RenderedMessage) -> SecretScanResult {
        self.scan_input(SecretScanInput::with_metadata(
            rendered.exact_text(),
            rendered.metadata(),
        ))
    }

    /// Compatibility alias for [`Self::scan_rendered`].
    #[must_use]
    pub fn scan_rendered_message(&self, rendered: &RenderedMessage) -> SecretScanResult {
        self.scan_rendered(rendered)
    }

    /// Scans a draft preview without changing the preview or its source draft.
    #[must_use]
    pub fn scan_preview(&self, preview: &DraftPreview) -> SecretScanResult {
        self.scan(preview.exact_text(), preview.metadata())
    }

    /// Scans an immutable draft revision's stored text and metadata.
    #[must_use]
    pub fn scan_revision(&self, revision: &DraftRevision) -> SecretScanResult {
        self.scan(revision.exact_text(), revision.metadata())
    }

    /// Scans an explicit optional-metadata input.
    #[must_use]
    pub fn scan_optional_metadata(
        &self,
        text: &str,
        metadata: Option<&DraftMetadata>,
    ) -> SecretScanResult {
        self.scan_input(SecretScanInput { text, metadata })
    }

    /// Scans an input object while keeping all source values borrowed only for
    /// the duration of this call.
    #[must_use]
    pub fn scan_input(&self, input: SecretScanInput<'_>) -> SecretScanResult {
        let mut findings = Vec::new();
        collect_findings(input.text, MatchSource::RenderedText, None, &mut findings);

        if let Some(metadata) = input.metadata {
            collect_metadata(metadata, &mut findings);
        }

        SecretScanResult::from_raw(findings)
    }

    /// Compatibility alias for [`Self::scan_input`].
    #[must_use]
    pub fn scan_request(&self, input: SecretScanInput<'_>) -> SecretScanResult {
        self.scan_input(input)
    }
}

/// A borrowed scanner input. The input text and metadata are never stored in a
/// result or retained by the scanner.
#[derive(Clone, Copy)]
pub struct SecretScanInput<'a> {
    text: &'a str,
    metadata: Option<&'a DraftMetadata>,
}

impl<'a> SecretScanInput<'a> {
    /// Creates an input with no metadata.
    #[must_use]
    pub const fn new(text: &'a str) -> Self {
        Self {
            text,
            metadata: None,
        }
    }

    /// Creates an input with bounded draft metadata.
    #[must_use]
    pub const fn with_metadata(text: &'a str, metadata: &'a DraftMetadata) -> Self {
        Self {
            text,
            metadata: Some(metadata),
        }
    }

    /// Creates an input from explicit text and metadata projections.
    #[must_use]
    pub const fn from_parts(text: &'a str, metadata: &'a DraftMetadata) -> Self {
        Self::with_metadata(text, metadata)
    }

    /// Returns the borrowed final text.
    #[must_use]
    pub const fn text(&self) -> &'a str {
        self.text
    }

    /// Compatibility alias for [`Self::text`].
    #[must_use]
    pub const fn rendered_text(&self) -> &'a str {
        self.text()
    }

    /// Returns the borrowed metadata, if supplied.
    #[must_use]
    pub const fn metadata(&self) -> Option<&'a DraftMetadata> {
        self.metadata
    }
}

impl fmt::Debug for SecretScanInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretScanInput")
            .field("text_length", &self.text.len())
            .field("has_metadata", &self.metadata.is_some())
            .finish()
    }
}

/// Compatibility name for the borrowed scanner input.
pub type ScanInput<'a> = SecretScanInput<'a>;
/// Compatibility name for a scanner request.
pub type SecretScanRequest<'a> = SecretScanInput<'a>;

/// The stable high-level state represented by a scan result.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretScanStatus {
    /// No high-confidence pattern was found.
    Clear,
    /// At least one high-confidence pattern was found and send is blocked by
    /// default pending the approval service's exact-revision TTY decision.
    Finding,
}

impl SecretScanStatus {
    /// Returns the stable machine-readable status code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Finding => "finding",
        }
    }
}

impl fmt::Display for SecretScanStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// The explicit, side-effect-free result consumed by eligibility and approval
/// services.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SecretScanResult {
    findings: Vec<SecretFinding>,
}

impl SecretScanResult {
    fn from_raw(mut raw: Vec<RawFinding>) -> Self {
        raw.sort_by_key(|finding| {
            (
                finding.priority,
                finding.location.start(),
                finding.location.end(),
            )
        });
        raw.dedup_by_key(|finding| {
            (
                finding.reason,
                finding.location.start(),
                finding.location.end(),
            )
        });

        let mut accepted: Vec<RawFinding> = Vec::with_capacity(raw.len());
        for finding in raw {
            if !accepted
                .iter()
                .any(|existing| ranges_overlap(&existing.location, &finding.location))
            {
                accepted.push(finding);
            }
        }
        accepted.sort_by_key(|finding| {
            (
                finding.location.start(),
                finding.location.end(),
                finding.priority,
                finding.reason.code(),
            )
        });

        Self {
            findings: accepted
                .into_iter()
                .map(|finding| SecretFinding::new(finding.reason, finding.location))
                .collect(),
        }
    }

    /// Returns all findings in deterministic source order.
    #[must_use]
    pub fn findings(&self) -> &[SecretFinding] {
        &self.findings
    }

    /// Returns whether at least one finding blocks send by default.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        !self.findings.is_empty()
    }

    /// Returns whether the default safety decision is blocked.
    #[must_use]
    pub fn blocked(&self) -> bool {
        self.is_blocked()
    }

    /// Returns whether send is blocked by default.
    #[must_use]
    pub fn blocks_send(&self) -> bool {
        self.is_blocked()
    }

    /// Returns whether no high-confidence pattern was found.
    #[must_use]
    pub fn is_clear(&self) -> bool {
        !self.is_blocked()
    }

    /// Returns whether the scan is clear of high-confidence findings.
    #[must_use]
    pub fn is_safe(&self) -> bool {
        self.is_clear()
    }

    /// Returns the stable high-level decision.
    #[must_use]
    pub fn decision(&self) -> SecretScanStatus {
        self.status()
    }

    /// Returns the safe finding iterator.
    pub fn iter(&self) -> std::slice::Iter<'_, SecretFinding> {
        self.findings.iter()
    }

    /// Compatibility alias for [`Self::is_clear`].
    #[must_use]
    pub fn allows_send(&self) -> bool {
        self.is_clear()
    }

    /// Returns whether any finding exists.
    #[must_use]
    pub fn has_findings(&self) -> bool {
        self.is_blocked()
    }

    /// Returns the finding count without exposing values.
    #[must_use]
    pub fn finding_count(&self) -> usize {
        self.findings.len()
    }

    /// Returns the high-level scan status.
    #[must_use]
    pub fn status(&self) -> SecretScanStatus {
        if self.is_blocked() {
            SecretScanStatus::Finding
        } else {
            SecretScanStatus::Clear
        }
    }

    /// Returns unique reason codes in finding order.
    #[must_use]
    pub fn reason_codes(&self) -> Vec<SecretReasonCode> {
        let mut codes = Vec::new();
        for finding in &self.findings {
            if !codes.contains(&finding.reason_code()) {
                codes.push(finding.reason_code());
            }
        }
        codes
    }

    /// Returns reason codes once per finding.
    #[must_use]
    pub fn all_reason_codes(&self) -> Vec<SecretReasonCode> {
        self.findings
            .iter()
            .map(SecretFinding::reason_code)
            .collect()
    }

    /// Returns whether a specific reason code was observed.
    #[must_use]
    pub fn has_reason(&self, reason: SecretReasonCode) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.reason_code() == reason)
    }

    /// Consumes the result and returns its redacted findings.
    #[must_use]
    pub fn into_findings(self) -> Vec<SecretFinding> {
        self.findings
    }
}

impl fmt::Display for SecretScanResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} ({} finding{})",
            self.status(),
            self.finding_count(),
            if self.finding_count() == 1 { "" } else { "s" }
        )
    }
}

/// Convenience free function for callers that do not need to retain a scanner.
#[must_use]
pub fn scan(text: &str, metadata: &DraftMetadata) -> SecretScanResult {
    SecretScanner::new().scan(text, metadata)
}

/// Convenience free function for a text-only scan.
#[must_use]
pub fn scan_text(text: &str) -> SecretScanResult {
    SecretScanner::new().scan_text(text)
}

/// Compatibility alias for the scanner result type.
pub type ScanResult = SecretScanResult;
/// Compatibility alias for the scanner result type.
pub type SecretScanOutcome = SecretScanResult;
/// Compatibility alias for the high-level scanner decision.
pub type SecretScanDecision = SecretScanStatus;

#[derive(Clone, Copy)]
struct RawFinding {
    reason: SecretReasonCode,
    location: SecretLocation,
    priority: u8,
}

fn collect_metadata(metadata: &DraftMetadata, findings: &mut Vec<RawFinding>) {
    for (field, value) in [
        (MetadataField::RepositoryLabel, metadata.repository_label()),
        (MetadataField::Branch, metadata.branch()),
        (MetadataField::Commit, metadata.commit()),
    ] {
        if let Some(value) = value {
            collect_findings(value, MatchSource::Metadata, Some(field), findings);
        }
    }
}

fn collect_findings(
    text: &str,
    source: MatchSource,
    field: Option<MetadataField>,
    findings: &mut Vec<RawFinding>,
) {
    let mut matches = Vec::new();
    matches.extend(
        find_private_key_markers(text)
            .into_iter()
            .map(|range| (SecretReasonCode::PrivateKeyMarker, range, 0)),
    );
    matches.extend(
        find_authorization_values(text)
            .into_iter()
            .map(|range| (SecretReasonCode::AuthorizationValue, range, 1)),
    );
    matches.extend(
        find_discord_tokens(text)
            .into_iter()
            .map(|range| (SecretReasonCode::DiscordBotToken, range, 2)),
    );
    matches.extend(
        find_credential_urls(text)
            .into_iter()
            .map(|range| (SecretReasonCode::CredentialUrl, range, 3)),
    );
    matches.extend(
        find_secret_assignments(text)
            .into_iter()
            .map(|range| (SecretReasonCode::SecretAssignment, range, 4)),
    );

    for (reason, range, priority) in matches {
        if range.start >= range.end {
            continue;
        }
        let location = match (source, field) {
            (MatchSource::RenderedText, _) => SecretLocation::rendered_text(range.start, range.end),
            (MatchSource::Metadata, Some(field)) => {
                SecretLocation::metadata(field, range.start, range.end)
            }
            (MatchSource::Metadata, None) => SecretLocation::rendered_text(range.start, range.end),
        };
        findings.push(RawFinding {
            reason,
            location,
            priority,
        });
    }
}

fn ranges_overlap(left: &SecretLocation, right: &SecretLocation) -> bool {
    left.source() == right.source()
        && left.field() == right.field()
        && left.start() < right.end()
        && right.start() < left.end()
}

fn find_private_key_markers(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    for marker in PRIVATE_KEY_MARKERS {
        let mut start = 0;
        while let Some(relative) = find_bytes_case_insensitive(&bytes[start..], marker) {
            let found = start + relative;
            ranges.push(found..found + marker.len());
            start = found + marker.len();
        }
    }
    ranges.sort_by_key(|range| (range.start, range.end));
    ranges.dedup();
    ranges
}

fn find_authorization_values(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    for name in AUTHORIZATION_NAMES {
        let mut start = 0;
        while let Some(relative) = find_bytes_case_insensitive(&bytes[start..], name) {
            let name_start = start + relative;
            let name_end = name_start + name.len();
            if identifier_boundary_before(bytes, name_start)
                && identifier_boundary_after(bytes, name_end)
                && (name_start == 0 || !matches!(bytes[name_start - 1], b'?' | b'&' | b';' | b'='))
                && let Some(range) = parse_authorization_value(bytes, name_end)
            {
                ranges.push(range);
            }
            start = name_end;
        }
    }
    ranges.sort_by_key(|range| (range.start, range.end));
    ranges.dedup();
    ranges
}

fn parse_authorization_value(bytes: &[u8], mut position: usize) -> Option<Range<usize>> {
    skip_spaces(bytes, &mut position);
    if position < bytes.len() && matches!(bytes[position], b'"' | b'\'' | b'`') {
        position += 1;
        skip_spaces(bytes, &mut position);
    }
    if position >= bytes.len() || !matches!(bytes[position], b':' | b'=') {
        return None;
    }
    position += 1;
    skip_spaces(bytes, &mut position);

    let (scheme_end, has_scheme) = parse_authorization_scheme(bytes, position);
    let value_start = if has_scheme { scheme_end } else { position };
    if value_start >= bytes.len() {
        return None;
    }

    let value_end = value_end(bytes, value_start);
    if value_end <= value_start || is_placeholder(&bytes[value_start..value_end]) {
        return None;
    }
    let value = &bytes[value_start..value_end];
    if !has_scheme && value.len() < 3 {
        return None;
    }
    Some(position..value_end)
}

fn parse_authorization_scheme(bytes: &[u8], position: usize) -> (usize, bool) {
    for scheme in AUTHORIZATION_SCHEMES {
        let end = position + scheme.len();
        if end < bytes.len()
            && bytes[position..end].eq_ignore_ascii_case(scheme)
            && (end == bytes.len() || bytes[end].is_ascii_whitespace())
        {
            let mut value_start = end;
            skip_spaces(bytes, &mut value_start);
            if value_start < bytes.len() {
                return (value_start, true);
            }
        }
    }
    (position, false)
}

fn find_discord_tokens(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        if !token_boundary_before(bytes, start) || !is_token_byte(bytes[start]) {
            start += 1;
            continue;
        }

        let first_end = match bytes[start..]
            .iter()
            .position(|byte| *byte == b'.')
            .map(|position| start + position)
        {
            Some(position) => position,
            None => {
                start += 1;
                continue;
            }
        };
        let second_start = first_end + 1;
        let second_end = match bytes[second_start..]
            .iter()
            .position(|byte| *byte == b'.')
            .map(|position| second_start + position)
        {
            Some(position) => position,
            None => {
                start += 1;
                continue;
            }
        };
        let first = &bytes[start..first_end];
        let second = &bytes[second_start..second_end];
        let third_start = second_end + 1;
        let third_end = bytes[third_start..]
            .iter()
            .position(|byte| !is_token_byte(*byte))
            .map(|position| third_start + position)
            .unwrap_or(bytes.len());
        let third = &bytes[third_start..third_end];

        let first_valid =
            (23..=32).contains(&first.len()) && first.iter().all(|byte| is_token_byte(*byte));
        let second_valid = second.len() == 6 && second.iter().all(|byte| is_token_byte(*byte));
        let third_valid =
            (27..=40).contains(&third.len()) && third.iter().all(|byte| is_token_byte(*byte));
        if first_valid && second_valid && third_valid && token_boundary_after(bytes, third_end) {
            ranges.push(start..third_end);
            start = third_end;
        } else {
            start += 1;
        }
    }
    ranges.sort_by_key(|range| (range.start, range.end));
    ranges.dedup();
    ranges
}

fn find_credential_urls(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let Some(scheme_length) = url_scheme_length(bytes, start) else {
            start += 1;
            continue;
        };
        if url_boundary_before(bytes, start) {
            let url_end = url_end(bytes, start + scheme_length);
            let authority_start = start + scheme_length;
            let authority_end = bytes[authority_start..url_end]
                .iter()
                .position(|byte| matches!(*byte, b'/' | b'?' | b'#'))
                .map(|position| authority_start + position)
                .unwrap_or(url_end);
            if let Some(range) = credential_in_authority(bytes, authority_start, authority_end) {
                ranges.push(range);
            }
            if let Some(query_start) = bytes[authority_end..url_end]
                .iter()
                .position(|byte| *byte == b'?')
                .map(|position| authority_end + position)
            {
                ranges.extend(credential_in_query(bytes, query_start, url_end));
            }
            if let Some(fragment_start) = bytes[authority_end..url_end]
                .iter()
                .position(|byte| *byte == b'#')
                .map(|position| authority_end + position)
            {
                ranges.extend(credential_in_query(bytes, fragment_start, url_end));
            }
            start = url_end.max(start + 1);
        } else {
            start += 1;
        }
    }
    ranges.sort_by_key(|range| (range.start, range.end));
    ranges.dedup();
    ranges
}

fn url_scheme_length(bytes: &[u8], start: usize) -> Option<usize> {
    if start > 0 && is_identifier_byte(bytes[start - 1]) {
        return None;
    }
    let suffix = bytes.get(start..)?;
    let separator = suffix.windows(3).position(|window| window == b"://")?;
    let scheme_length = separator;
    if !(2..=16).contains(&scheme_length)
        || !suffix[0].is_ascii_alphabetic()
        || !suffix[..scheme_length]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
    {
        return None;
    }
    Some(scheme_length + 3)
}

fn credential_in_authority(bytes: &[u8], start: usize, end: usize) -> Option<Range<usize>> {
    let at = bytes[start..end].iter().rposition(|byte| *byte == b'@')? + start;
    if at <= start {
        return None;
    }
    let userinfo = &bytes[start..at];
    if let Some(colon) = userinfo.iter().position(|byte| *byte == b':') {
        let password_start = start + colon + 1;
        if password_start >= at || is_placeholder(&bytes[password_start..at]) {
            return None;
        }
        return Some(start..at);
    }
    if userinfo.len() >= 20 && has_character_variety(userinfo) {
        return Some(start..at);
    }
    None
}

fn credential_in_query(bytes: &[u8], query_start: usize, url_end: usize) -> Vec<Range<usize>> {
    let query_end = bytes[query_start..url_end]
        .iter()
        .position(|byte| *byte == b'#')
        .map(|position| query_start + position)
        .unwrap_or(url_end);
    let mut ranges = Vec::new();
    let mut pair_start = query_start + 1;
    while pair_start < query_end {
        let pair_end = bytes[pair_start..query_end]
            .iter()
            .position(|byte| matches!(*byte, b'&' | b';'))
            .map(|position| pair_start + position)
            .unwrap_or(query_end);
        let Some(equal) = bytes[pair_start..pair_end]
            .iter()
            .position(|byte| *byte == b'=')
            .map(|position| pair_start + position)
        else {
            pair_start = pair_end + 1;
            continue;
        };
        let key_start = pair_start;
        let key_end = equal;
        let value_start = equal + 1;
        let mut value_end = pair_end;
        while value_end > value_start && bytes[value_end - 1].is_ascii_whitespace() {
            value_end -= 1;
        }
        let key = normalize_ascii_key(&bytes[key_start..key_end]);
        let strong = is_bytes_in_list(&key, SENSITIVE_QUERY_KEYS);
        let weak = is_bytes_in_list(&key, WEAK_SENSITIVE_QUERY_KEYS);
        let value = &bytes[value_start..value_end];
        if (strong || weak && has_character_variety(value) && value.len() >= 8)
            && !is_placeholder(value)
            && (strong && value.len() >= 3 || weak)
        {
            ranges.push(value_start..value_end);
        }
        pair_start = pair_end + 1;
    }
    ranges
}

fn find_secret_assignments(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        if !is_identifier_start(bytes[start]) {
            start += 1;
            continue;
        }
        let key_start = start;
        let mut key_end = start;
        while key_end < bytes.len() && is_identifier_byte(bytes[key_end]) {
            key_end += 1;
        }
        let key = normalize_ascii_key(&bytes[key_start..key_end]);
        if is_bytes_in_list(&key, ASSIGNMENT_KEYS) {
            let mut separator = key_end;
            skip_spaces(bytes, &mut separator);
            if separator < bytes.len() && matches!(bytes[separator], b'"' | b'\'') {
                separator += 1;
                skip_spaces(bytes, &mut separator);
            }
            if separator < bytes.len()
                && matches!(bytes[separator], b':' | b'=')
                && let Some((value_start, value_end, quoted)) =
                    assignment_value(bytes, separator + 1)
            {
                let value = &bytes[value_start..value_end];
                if !is_placeholder(value) && assignment_value_is_confident(&key, value, quoted) {
                    ranges.push(value_start..value_end);
                }
            }
        }
        start = key_end.max(start + 1);
    }
    ranges.sort_by_key(|range| (range.start, range.end));
    ranges.dedup();
    ranges
}

fn assignment_value(bytes: &[u8], mut position: usize) -> Option<(usize, usize, bool)> {
    skip_spaces(bytes, &mut position);
    if position >= bytes.len() {
        return None;
    }
    if matches!(bytes[position], b'"' | b'\'' | b'`') {
        let quote = bytes[position];
        let start = position + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != quote {
            end += 1;
        }
        if end == bytes.len() || end == start {
            return None;
        }
        return Some((start, end, true));
    }
    let start = position;
    while position < bytes.len()
        && !bytes[position].is_ascii_whitespace()
        && !matches!(bytes[position], b',' | b';' | b'}' | b']' | b')')
    {
        position += 1;
    }
    let mut end = position;
    while end > start && matches!(bytes[end - 1], b'.' | b'!' | b'?') {
        end -= 1;
    }
    (end > start).then_some((start, end, false))
}

fn assignment_value_is_confident(key: &[u8], value: &[u8], quoted: bool) -> bool {
    if matches!(
        key,
        b"password" | b"passwd" | b"passphrase" | b"secret" | b"credential"
    ) {
        return true;
    }
    if value.len() < 3 {
        return false;
    }
    if quoted || (value.len() >= 4 && value.iter().all(u8::is_ascii_digit)) {
        return true;
    }
    if is_strong_assignment_key(key) && value.len() >= 3 {
        return true;
    }
    value.len() >= 8 || has_character_variety(value)
}

fn is_strong_assignment_key(key: &[u8]) -> bool {
    matches!(
        key,
        b"accesskey"
            | b"accesstoken"
            | b"apikey"
            | b"apisecret"
            | b"authtoken"
            | b"bottoken"
            | b"clientsecret"
            | b"connectionstring"
            | b"cookie"
            | b"discordtoken"
            | b"jwt"
            | b"privatekey"
            | b"refreshtoken"
            | b"secretaccesskey"
            | b"secretkey"
            | b"sessiontoken"
            | b"signingkey"
            | b"webhooktoken"
            | b"xapikey"
    )
}

fn value_end(bytes: &[u8], start: usize) -> usize {
    if start >= bytes.len() {
        return start;
    }
    if matches!(bytes[start], b'"' | b'\'' | b'`') {
        let quote = bytes[start];
        let mut end = start + 1;
        while end < bytes.len() && bytes[end] != quote {
            end += 1;
        }
        return end;
    }
    let mut end = start;
    while end < bytes.len()
        && !bytes[end].is_ascii_whitespace()
        && !matches!(bytes[end], b',' | b';' | b')' | b']' | b'}' | b'<' | b'>')
    {
        end += 1;
    }
    while end > start && matches!(bytes[end - 1], b'.' | b',' | b';' | b')' | b']' | b'}') {
        end -= 1;
    }
    end
}

fn skip_spaces(bytes: &[u8], position: &mut usize) {
    while *position < bytes.len() && bytes[*position].is_ascii_whitespace() {
        *position += 1;
    }
}

fn find_bytes_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

fn normalize_ascii_key(value: &[u8]) -> Vec<u8> {
    value
        .iter()
        .filter(|byte| byte.is_ascii_alphanumeric())
        .map(|byte| byte.to_ascii_lowercase())
        .collect()
}

fn is_bytes_in_list(value: &[u8], list: &[&[u8]]) -> bool {
    list.contains(&value)
}

fn is_placeholder(value: &[u8]) -> bool {
    let lower = value.to_ascii_lowercase();
    PLACEHOLDER_WORDS
        .iter()
        .any(|word| lower.windows(word.len()).any(|window| window == *word))
        || (value.starts_with(b"<") && value.ends_with(b">"))
        || (value.starts_with(b"${") && value.ends_with(b"}"))
        || value.starts_with(b"$")
}

fn has_character_variety(value: &[u8]) -> bool {
    let has_lower = value.iter().any(|byte| byte.is_ascii_lowercase());
    let has_upper = value.iter().any(|byte| byte.is_ascii_uppercase());
    let has_digit = value.iter().any(|byte| byte.is_ascii_digit());
    let has_other = value.iter().any(|byte| !byte.is_ascii_alphanumeric());
    [has_lower, has_upper, has_digit, has_other]
        .into_iter()
        .filter(|present| *present)
        .count()
        >= 2
}

fn identifier_boundary_before(bytes: &[u8], position: usize) -> bool {
    position == 0 || !is_identifier_byte(bytes[position - 1])
}

fn identifier_boundary_after(bytes: &[u8], position: usize) -> bool {
    position == bytes.len() || !is_identifier_byte(bytes[position])
}

fn token_boundary_before(bytes: &[u8], position: usize) -> bool {
    position == 0 || (!is_token_byte(bytes[position - 1]) && bytes[position - 1] != b'.')
}

fn token_boundary_after(bytes: &[u8], position: usize) -> bool {
    position == bytes.len() || (!is_token_byte(bytes[position]) && bytes[position] != b'.')
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn url_boundary_before(bytes: &[u8], position: usize) -> bool {
    position == 0
        || (!is_identifier_byte(bytes[position - 1]) && !matches!(bytes[position - 1], b'/' | b'.'))
}

fn url_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len()
        && !bytes[end].is_ascii_whitespace()
        && !matches!(bytes[end], b'"' | b'\'' | b'`' | b'<' | b'>')
    {
        end += 1;
    }
    while end > start && matches!(bytes[end - 1], b'.' | b',' | b';' | b':') {
        end -= 1;
    }
    end
}
