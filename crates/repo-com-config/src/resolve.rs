use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{MentionTarget, RepositoryConfig};
pub use crate::validation::{ConfigError, WorkspaceCatalog, WorkspaceReferenceIndex};
use crate::validation::{
    IoOperation, parse_config, validate_config, validate_workspace_references,
};

/// The committed configuration filename searched during ancestor discovery.
pub const CONFIG_FILE_NAME: &str = ".repo-com.toml";

/// A parsed configuration together with the normalized file it came from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LoadedConfig {
    /// Normalized absolute path to the committed file.
    pub path: PathBuf,
    /// Validated schema-version-1 configuration.
    pub config: RepositoryConfig,
}

/// Compatibility name for [`LoadedConfig`].
pub type ConfigDocument = LoadedConfig;

/// A named mention after alias lookup and target parsing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedMention {
    /// Repository-local mention alias.
    pub alias: String,
    /// Parsed role/user target.
    pub target: MentionTarget,
}

/// A destination after alias resolution. It contains exactly one channel and
/// only named mentions from the destination allowlist.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedDestination {
    /// Repository-local destination alias.
    pub alias: String,
    /// The one configured workspace.
    pub workspace_id: String,
    /// The resolved channel identifier.
    pub channel_id: String,
    /// Resolved named mentions, sorted by alias.
    pub allowed_mentions: Vec<ResolvedMention>,
}

/// An inbound alias resolved to the matching configured destination channel.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedInbound {
    /// Repository-local inbound alias.
    pub alias: String,
    /// The one configured workspace.
    pub workspace_id: String,
    /// The channel inherited from the matching destination alias.
    pub channel_id: String,
    /// Whether inbound retrieval is enabled.
    pub enabled: bool,
}

/// All validated and resolved configuration aliases.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedConfig {
    /// Normalized source path, or a caller-supplied logical path for in-memory
    /// validation.
    pub path: PathBuf,
    /// The original validated model, retained for retention and policy inputs.
    pub config: RepositoryConfig,
    /// Resolved outbound destinations.
    pub destinations: BTreeMap<String, ResolvedDestination>,
    /// Resolved inbound aliases.
    pub inbound: BTreeMap<String, ResolvedInbound>,
}

impl ResolvedConfig {
    /// Looks up one resolved destination alias.
    #[must_use]
    pub fn destination(&self, alias: &str) -> Option<&ResolvedDestination> {
        self.destinations.get(alias)
    }

    /// Looks up one resolved inbound alias.
    #[must_use]
    pub fn inbound(&self, alias: &str) -> Option<&ResolvedInbound> {
        self.inbound.get(alias)
    }

    /// Returns the canonical hash of the underlying configuration.
    #[must_use]
    pub fn canonical_hash(&self) -> String {
        self.config.canonical_hash()
    }
}

/// A side-effect-free configuration resolver.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConfigResolver;

impl ConfigResolver {
    /// Creates a resolver. The resolver has no mutable state.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Discovers an explicit normalized path or searches the bounded ancestor
    /// range. The repository root is inclusive and is never crossed.
    pub fn discover(
        explicit_path: Option<&Path>,
        current_dir: &Path,
        repository_root: &Path,
    ) -> Result<PathBuf, ConfigError> {
        discover_config_path(explicit_path, current_dir, repository_root)
    }

    /// Discovers a configuration by searching from `current_dir` to an
    /// explicitly supplied repository root.
    pub fn discover_at(current_dir: &Path, repository_root: &Path) -> Result<PathBuf, ConfigError> {
        discover_config_path(None, current_dir, repository_root)
    }

    /// Detects the nearest repository root and then discovers a configuration.
    pub fn discover_current(current_dir: &Path) -> Result<PathBuf, ConfigError> {
        let current = normalize_directory(current_dir)?;
        let root = find_repository_root(&current)?;
        discover_config_path(None, &current, &root)
    }

    /// Resolves a configuration after detecting the repository root from the
    /// supplied working directory.
    pub fn resolve_current(current_dir: &Path) -> Result<ResolvedConfig, ConfigError> {
        let path = Self::discover_current(current_dir)?;
        Self::resolve_path(&path)
    }

    /// Resolves one explicit file without ancestor discovery.
    pub fn resolve_explicit(path: &Path) -> Result<ResolvedConfig, ConfigError> {
        Self::resolve_path(path)
    }

    /// Loads an explicit file without searching any ancestors.
    pub fn load_explicit(path: &Path) -> Result<LoadedConfig, ConfigError> {
        load_config_file(path)
    }

    /// Discovers and loads a configuration.
    pub fn load(
        explicit_path: Option<&Path>,
        current_dir: &Path,
        repository_root: &Path,
    ) -> Result<LoadedConfig, ConfigError> {
        let path = discover_config_path(explicit_path, current_dir, repository_root)?;
        load_config_file(&path)
    }

    /// Parses and validates one file without discovery.
    pub fn parse_file(path: &Path) -> Result<LoadedConfig, ConfigError> {
        load_config_file(path)
    }

    /// Resolves all aliases in a discovered configuration.
    pub fn resolve(
        explicit_path: Option<&Path>,
        current_dir: &Path,
        repository_root: &Path,
    ) -> Result<ResolvedConfig, ConfigError> {
        let loaded = Self::load(explicit_path, current_dir, repository_root)?;
        resolve_loaded(&loaded, None)
    }

    /// Resolves a file directly, without discovery.
    pub fn resolve_path(path: &Path) -> Result<ResolvedConfig, ConfigError> {
        let loaded = load_config_file(path)?;
        resolve_loaded(&loaded, None)
    }

    /// Resolves a discovered configuration and verifies any supplied local
    /// workspace reference index.
    pub fn resolve_with_references(
        explicit_path: Option<&Path>,
        current_dir: &Path,
        repository_root: &Path,
        references: &WorkspaceReferenceIndex,
    ) -> Result<ResolvedConfig, ConfigError> {
        let loaded = Self::load(explicit_path, current_dir, repository_root)?;
        resolve_loaded(&loaded, Some(references))
    }
}

/// Normalizes an existing explicit file relative to `base` and returns its
/// canonical absolute path. No directory or file is created.
pub fn normalize_explicit_path(path: &Path, base: &Path) -> Result<PathBuf, ConfigError> {
    let base = normalize_directory(base)?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    normalize_file(&candidate)
}

/// Normalizes an explicit path relative to the process working directory.
pub fn normalize_path(path: &Path) -> Result<PathBuf, ConfigError> {
    let current = std::env::current_dir().map_err(|_| ConfigError::Io {
        path: path.to_path_buf(),
        operation: IoOperation::CurrentDirectory,
    })?;
    normalize_explicit_path(path, &current)
}

/// Finds the nearest ancestor containing a `.git` file or directory.
pub fn find_repository_root(start: &Path) -> Result<PathBuf, ConfigError> {
    let normalized_start = normalize_directory(start)?;
    let mut current = normalized_start.clone();
    loop {
        if fs::symlink_metadata(current.join(".git")).is_ok() {
            return Ok(current);
        }
        let Some(parent) = current.parent().map(Path::to_path_buf) else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent;
    }
    Err(ConfigError::RepositoryRootNotFound {
        start: normalized_start,
    })
}

/// Compatibility alias for [`find_repository_root`].
pub fn detect_repository_root(start: &Path) -> Result<PathBuf, ConfigError> {
    find_repository_root(start)
}

/// Detects the repository root and discovers a configuration from `start`.
pub fn discover_from_current(start: &Path) -> Result<PathBuf, ConfigError> {
    let current = normalize_directory(start)?;
    let root = find_repository_root(&current)?;
    discover_config_path(None, &current, &root)
}

/// Discovers one configuration path within the inclusive repository-root bound.
pub fn discover_config_path(
    explicit_path: Option<&Path>,
    current_dir: &Path,
    repository_root: &Path,
) -> Result<PathBuf, ConfigError> {
    let root = normalize_directory(repository_root)?;
    let current = normalize_directory(current_dir)?;

    if let Some(explicit) = explicit_path {
        let candidate = if explicit.is_absolute() {
            normalize_file(explicit)?
        } else {
            normalize_file(&current.join(explicit))?
        };
        if !is_within(&candidate, &root) {
            return Err(ConfigError::PathOutsideRepository {
                path: candidate,
                repository_root: root,
            });
        }
        return Ok(candidate);
    }

    let search_start = current.clone();
    let search_root = root.clone();
    if !is_within(&current, &root) {
        return Err(ConfigError::PathOutsideRepository {
            path: current,
            repository_root: root,
        });
    }

    let mut candidates = BTreeSet::new();
    let mut current = current;
    loop {
        let candidate = current.join(CONFIG_FILE_NAME);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => {
                let normalized = normalize_file(&candidate)?;
                if !is_within(&normalized, &root) {
                    return Err(ConfigError::PathOutsideRepository {
                        path: normalized,
                        repository_root: root,
                    });
                }
                candidates.insert(normalized);
            }
            Ok(_) => {
                return Err(ConfigError::Io {
                    path: candidate,
                    operation: IoOperation::Metadata,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(ConfigError::Io {
                    path: candidate,
                    operation: IoOperation::Metadata,
                });
            }
        }
        if current == root {
            break;
        }
        let Some(parent) = current.parent().map(Path::to_path_buf) else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent;
    }

    match candidates.len() {
        0 => Err(ConfigError::NoCandidates {
            start: search_start,
            repository_root: search_root,
        }),
        1 => Ok(candidates.into_iter().next().expect("one candidate exists")),
        _ => Err(ConfigError::MultipleCandidates {
            start: search_start,
            repository_root: search_root,
            candidates: candidates.into_iter().collect(),
        }),
    }
}

/// Compatibility alias for [`discover_config_path`].
pub fn find_config_path(
    explicit_path: Option<&Path>,
    current_dir: &Path,
    repository_root: &Path,
) -> Result<PathBuf, ConfigError> {
    discover_config_path(explicit_path, current_dir, repository_root)
}

/// Parses and validates one explicit file.
pub fn load_config_file(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let normalized = normalize_file(path)?;
    let source = fs::read_to_string(&normalized).map_err(|_| ConfigError::Io {
        path: normalized.clone(),
        operation: IoOperation::Read,
    })?;
    let config = parse_config(&normalized, &source)?;
    Ok(LoadedConfig {
        path: normalized,
        config,
    })
}

/// Compatibility alias for [`load_config_file`].
pub fn load_config(path: &Path) -> Result<LoadedConfig, ConfigError> {
    load_config_file(path)
}

/// Resolves one explicit file without discovery.
pub fn resolve_path(path: &Path) -> Result<ResolvedConfig, ConfigError> {
    let loaded = load_config_file(path)?;
    resolve_loaded(&loaded, None)
}

/// Resolves one explicit file without discovery.
pub fn resolve_config(path: &Path) -> Result<ResolvedConfig, ConfigError> {
    resolve_path(path)
}

/// Resolves all aliases in a loaded configuration, optionally checking local
/// workspace-reference evidence.
pub fn resolve_loaded(
    loaded: &LoadedConfig,
    references: Option<&WorkspaceReferenceIndex>,
) -> Result<ResolvedConfig, ConfigError> {
    validate_config(&loaded.config, &loaded.path)?;
    if let Some(references) = references {
        validate_workspace_references(&loaded.config, &loaded.path, references)?;
    }
    resolve_model(&loaded.config, &loaded.path)
}

/// Resolves a model without reading or writing any files.
pub fn resolve_model(
    config: &RepositoryConfig,
    path: &Path,
) -> Result<ResolvedConfig, ConfigError> {
    validate_config(config, path)?;
    let mut destinations = BTreeMap::new();
    for alias in config.destinations.keys() {
        destinations.insert(
            alias.clone(),
            resolve_destination_with_validated(config, alias, path)?,
        );
    }
    let mut inbound = BTreeMap::new();
    for alias in config.inbound.keys() {
        inbound.insert(
            alias.clone(),
            resolve_inbound_with_validated(config, alias, path)?,
        );
    }
    Ok(ResolvedConfig {
        path: path.to_path_buf(),
        config: config.clone(),
        destinations,
        inbound,
    })
}

/// Resolves one destination alias after validating the whole model.
pub fn resolve_destination(
    config: &RepositoryConfig,
    alias: &str,
    path: &Path,
) -> Result<ResolvedDestination, ConfigError> {
    validate_config(config, path)?;
    resolve_destination_with_validated(config, alias, path)
}

/// Resolves one inbound alias after validating the whole model.
pub fn resolve_inbound(
    config: &RepositoryConfig,
    alias: &str,
    path: &Path,
) -> Result<ResolvedInbound, ConfigError> {
    validate_config(config, path)?;
    resolve_inbound_with_validated(config, alias, path)
}

fn resolve_destination_with_validated(
    config: &RepositoryConfig,
    alias: &str,
    path: &Path,
) -> Result<ResolvedDestination, ConfigError> {
    let Some(destination) = config.destinations.get(alias) else {
        return Err(ConfigError::UnknownDestinationAlias {
            path: path.to_path_buf(),
            field: format!("destinations.{alias}"),
        });
    };
    let mut allowed_mentions = destination
        .allowed_mentions
        .iter()
        .map(|mention_alias| {
            let mention = config.mentions.get(mention_alias).ok_or_else(|| {
                ConfigError::MissingMentionAlias {
                    path: path.to_path_buf(),
                    field: format!("destinations.{alias}.allowed_mentions"),
                }
            })?;
            let target = MentionTarget::parse(&mention.target).ok_or_else(|| {
                ConfigError::InvalidMentionTarget {
                    path: path.to_path_buf(),
                    field: format!("mentions.{mention_alias}.target"),
                }
            })?;
            Ok(ResolvedMention {
                alias: mention_alias.clone(),
                target,
            })
        })
        .collect::<Result<Vec<_>, ConfigError>>()?;
    allowed_mentions.sort_by(|left, right| left.alias.cmp(&right.alias));
    Ok(ResolvedDestination {
        alias: alias.to_owned(),
        workspace_id: config.discord.workspace_id.clone(),
        channel_id: destination.channel_id.clone(),
        allowed_mentions,
    })
}

fn resolve_inbound_with_validated(
    config: &RepositoryConfig,
    alias: &str,
    path: &Path,
) -> Result<ResolvedInbound, ConfigError> {
    let Some(inbound) = config.inbound.get(alias) else {
        return Err(ConfigError::UnconfiguredInboundAlias {
            path: path.to_path_buf(),
            field: format!("inbound.{alias}"),
        });
    };
    let Some(destination) = config.destinations.get(alias) else {
        return Err(ConfigError::UnconfiguredInboundAlias {
            path: path.to_path_buf(),
            field: format!("inbound.{alias}"),
        });
    };
    Ok(ResolvedInbound {
        alias: alias.to_owned(),
        workspace_id: config.discord.workspace_id.clone(),
        channel_id: destination.channel_id.clone(),
        enabled: inbound.enabled,
    })
}

fn normalize_directory(path: &Path) -> Result<PathBuf, ConfigError> {
    let normalized = fs::canonicalize(path).map_err(|_| ConfigError::Io {
        path: path.to_path_buf(),
        operation: IoOperation::Normalize,
    })?;
    if !normalized.is_dir() {
        return Err(ConfigError::Io {
            path: normalized,
            operation: IoOperation::Metadata,
        });
    }
    Ok(normalized)
}

fn normalize_file(path: &Path) -> Result<PathBuf, ConfigError> {
    let normalized = fs::canonicalize(path).map_err(|_| ConfigError::Io {
        path: path.to_path_buf(),
        operation: IoOperation::Normalize,
    })?;
    if !normalized.is_file() {
        return Err(ConfigError::Io {
            path: normalized,
            operation: IoOperation::Metadata,
        });
    }
    Ok(normalized)
}

fn is_within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}
