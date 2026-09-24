#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "../tests/config_contract.rs"]
mod config_contract;

pub mod model;
pub mod resolve;
pub mod validation;

pub use model::{
    AutoSend, AutoSendConfig, AutoSendEntry, Config, Destination, DestinationConfig, DiscordConfig,
    DiscordWorkspace, Inbound, InboundConfig, Mention, MentionConfig, MentionKind, MentionTarget,
    RepositoryConfig, RetentionConfig, RetentionPolicy, SCHEMA_VERSION, WorkspaceConfig,
};
pub use resolve::{
    CONFIG_FILE_NAME, ConfigDocument, ConfigResolver, LoadedConfig, ResolvedConfig,
    ResolvedDestination, ResolvedInbound, ResolvedMention, detect_repository_root,
    discover_config_path, discover_from_current, find_config_path, find_repository_root,
    load_config, load_config_file, normalize_explicit_path, normalize_path, resolve_config,
    resolve_destination, resolve_inbound, resolve_loaded, resolve_model, resolve_path,
};
pub use validation::{
    ConfigError, ConfigurationError, IoOperation, TomlErrorKind, WorkspaceCatalog,
    WorkspaceReferenceIndex, is_secret_like_field, parse_config, parse_config_str, validate_config,
    validate_workspace_references,
};
