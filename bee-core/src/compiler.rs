//! Policy compiler: authoring [`Policy`] → backend-neutral [`CompiledPolicy`].
//!
//! Two responsibilities that the constitution keeps out of the kernel:
//! 1. **Glob lowering** — reduce each path pattern to a verifier-safe primitive
//!    (exact/subtree/postfix/segment/bounded-star), or fail closed on an irreducible pattern (R6).
//! 2. **Name resolution** — resolve `:project_root`/`~` tokens, executable names → paths, and
//!    domains → IPs, via an injected [`Resolver`] (no ambient I/O in the core logic — NFR-005).

use std::net::IpAddr;
use std::path::PathBuf;

use bee_common::AccessMode;

use crate::error::CompileError;
use crate::policy::{Mode, Policy};

/// Host-environment resolution injected into the compiler so it stays pure and testable.
pub trait Resolver {
    /// Absolute project root (replaces the `:project_root` token).
    fn project_root(&self) -> &str;
    /// Absolute home directory (replaces a leading `~`).
    fn home(&self) -> &str;
    /// Resolve an executable name or path to an absolute path (PATH search + existence check).
    fn resolve_exec(&self, name: &str) -> Result<PathBuf, CompileError>;
    /// Resolve a network host (domain/IP/CIDR) to concrete IP addresses.
    fn resolve_host(&self, host: &str) -> Result<Vec<IpAddr>, CompileError>;
}

/// A lowered filesystem rule region + access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsPrimitive {
    /// Exact or subtree match on a resolved absolute path (both go to the LPM prefix map;
    /// `subtree=true` means "and everything beneath").
    Prefix {
        path: Vec<u8>,
        subtree: bool,
        mode: AccessMode,
    },
    /// Literal suffix, e.g. `.log` from `*.log`.
    Postfix { suffix: Vec<u8>, mode: AccessMode },
    /// A path component equals a literal, e.g. `target` from `**/target`.
    Segment { name: Vec<u8>, mode: AccessMode },
    /// Single-segment `*` glob matched against the basename.
    BoundedStar { pattern: Vec<u8>, mode: AccessMode },
}

/// A compiled executable rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledExec {
    pub path: Vec<u8>,
    /// Pin the resolved inode (TOCTOU-hard) rather than match by path.
    pub pin_inode: bool,
}

/// A compiled network egress rule (one per resolved IP).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledNet {
    pub addr: IpAddr,
    pub port: u16,
}

/// Normalized, host-resolved policy intent.
///
/// This representation is backend-neutral: it may contain capabilities that a particular
/// enforcement backend cannot install. Backends must prepare and validate their own runnable plan
/// before touching enforcement state.
#[derive(Debug, Clone, Default)]
pub struct CompiledPolicy {
    pub mode: Mode,
    pub fs: Vec<FsPrimitive>,
    pub exec: Vec<CompiledExec>,
    pub net: Vec<CompiledNet>,
    /// Whether the author requested exfiltration detection.
    pub exfiltration_enabled: bool,
    /// Resolved sensitive paths for an exfiltration-capable backend.
    pub sensitive: Vec<FsPrimitive>,
}

impl Policy {
    /// Normalize and resolve this policy without choosing an enforcement backend. Fails closed on
    /// irreducible authoring patterns or unresolvable names. Target: < 50ms for ≤100 path + ≤50 net
    /// rules (SC-006).
    pub fn compile(&self, r: &dyn Resolver) -> Result<CompiledPolicy, CompileError> {
        let mut fs = Vec::new();

        // Inject FR-008 protected defaults first, then explicit rules; explicit rules that name the
        // same resolved path override (a later duplicate prefix with the same length wins at load).
        for (path, access) in protected_defaults(r) {
            fs.push(FsPrimitive::Prefix {
                path: path.into_bytes(),
                subtree: true,
                mode: access,
            });
        }

        for (raw, access) in &self.filesystem {
            let resolved = resolve_tokens(raw, r);
            let prim = lower_pattern(&resolved, access.to_bits())?;
            fs.push(prim);
        }

        let mut exec = Vec::new();
        for entry in &self.exec.allow {
            let (name, pin) = match entry.strip_prefix('!') {
                Some(rest) => (rest, true),
                None => (entry.as_str(), false),
            };
            let path = r.resolve_exec(name)?;
            exec.push(CompiledExec {
                path: path.into_os_string().into_encoded_bytes(),
                pin_inode: pin,
            });
        }

        let mut net = Vec::new();
        for entry in &self.network.allow {
            let (host, port) = split_host_port(entry)?;
            for addr in r.resolve_host(host)? {
                net.push(CompiledNet { addr, port });
            }
        }

        let mut sensitive = Vec::new();
        for raw in &self.exfiltration.sensitive_paths {
            let resolved = resolve_tokens(raw, r);
            sensitive.push(lower_pattern(&resolved, AccessMode::READ)?);
        }

        Ok(CompiledPolicy {
            mode: self.mode,
            fs,
            exec,
            net,
            exfiltration_enabled: self.exfiltration.enabled,
            sensitive,
        })
    }
}

/// FR-008: within any writable root, protect VCS/config/credential dirs unless explicitly overridden.
fn protected_defaults(r: &dyn Resolver) -> Vec<(String, AccessMode)> {
    let root = r.project_root();
    let home = r.home();
    vec![
        (format!("{root}/.git"), AccessMode::READ),
        (format!("{root}/.bee"), AccessMode::DENY),
        (format!("{home}/.ssh"), AccessMode::DENY),
        (format!("{home}/.aws"), AccessMode::DENY),
    ]
}

/// Resolve `:project_root` and a leading `~` to absolute paths. Other characters pass through.
pub fn resolve_tokens(raw: &str, r: &dyn Resolver) -> String {
    if let Some(rest) = raw.strip_prefix(":project_root") {
        return format!("{}{}", r.project_root(), rest);
    }
    if let Some(rest) = raw.strip_prefix('~') {
        return format!("{}{}", r.home(), rest);
    }
    raw.to_string()
}

/// Classify a resolved pattern into a verifier-safe primitive, or fail closed.
///
/// Lowering rules (research R6):
/// * no `*`                      → Prefix{subtree} (covers exact and subtree)
/// * `**/<name>` (name no `/`,`*`) → Segment(name)
/// * `*<suffix>` (suffix no `/`,`*`) → Postfix(suffix)   (e.g. `*.log`)
/// * single segment with `*` (no `/`) → BoundedStar
/// * anything else (mid-path `**`, dir-anchored glob) → UnsupportedGlob
pub fn lower_pattern(resolved: &str, mode: AccessMode) -> Result<FsPrimitive, CompileError> {
    let b = resolved.as_bytes();

    if !resolved.contains('*') {
        return Ok(FsPrimitive::Prefix {
            path: b.to_vec(),
            subtree: true,
            mode,
        });
    }

    // `**/<name>`
    if let Some(name) = resolved.strip_prefix("**/") {
        if !name.is_empty() && !name.contains('/') && !name.contains('*') {
            return Ok(FsPrimitive::Segment {
                name: name.as_bytes().to_vec(),
                mode,
            });
        }
    }

    // `*<suffix>` — leading star, single segment, no further star.
    if let Some(suffix) = resolved.strip_prefix('*') {
        if !suffix.is_empty() && !suffix.contains('/') && !suffix.contains('*') {
            return Ok(FsPrimitive::Postfix {
                suffix: suffix.as_bytes().to_vec(),
                mode,
            });
        }
    }

    // Single-segment glob (no `/`), any number of `*`.
    if !resolved.contains('/') {
        return Ok(FsPrimitive::BoundedStar {
            pattern: b.to_vec(),
            mode,
        });
    }

    Err(CompileError::UnsupportedGlob(
        resolved.to_string(),
        "dir-anchored or mid-path '**' globs are not verifier-safe; use a bare '*.ext'/'**/name' \
         pattern or a subtree rule"
            .into(),
    ))
}

/// Split `host:port`, handling bracketed IPv6 (`[::1]:443`).
fn split_host_port(entry: &str) -> Result<(&str, u16), CompileError> {
    let (host, port) = entry
        .rsplit_once(':')
        .ok_or_else(|| CompileError::BadNetRule(entry.to_string(), "expected host:port".into()))?;
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let port: u16 = port
        .parse()
        .map_err(|_| CompileError::BadNetRule(entry.to_string(), "invalid port".into()))?;
    if host.is_empty() {
        return Err(CompileError::BadNetRule(
            entry.to_string(),
            "empty host".into(),
        ));
    }
    Ok((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> AccessMode {
        AccessMode::READ
    }

    #[test]
    fn lower_plain_path_is_subtree() {
        let p = lower_pattern("/home/u/project", m()).unwrap();
        assert_eq!(
            p,
            FsPrimitive::Prefix {
                path: b"/home/u/project".to_vec(),
                subtree: true,
                mode: m()
            }
        );
    }

    #[test]
    fn lower_postfix() {
        assert_eq!(
            lower_pattern("*.log", m()).unwrap(),
            FsPrimitive::Postfix {
                suffix: b".log".to_vec(),
                mode: m()
            }
        );
    }

    #[test]
    fn lower_segment() {
        assert_eq!(
            lower_pattern("**/target", m()).unwrap(),
            FsPrimitive::Segment {
                name: b"target".to_vec(),
                mode: m()
            }
        );
    }

    #[test]
    fn lower_bounded_star() {
        assert_eq!(
            lower_pattern("foo*bar", m()).unwrap(),
            FsPrimitive::BoundedStar {
                pattern: b"foo*bar".to_vec(),
                mode: m()
            }
        );
    }

    #[test]
    fn lower_irreducible_is_error() {
        // mid-path glob
        assert!(matches!(
            lower_pattern("/var/*/secret", m()),
            Err(CompileError::UnsupportedGlob(..))
        ));
        // dir-anchored postfix
        assert!(matches!(
            lower_pattern("/proj/*.log", m()),
            Err(CompileError::UnsupportedGlob(..))
        ));
        // bare mid-path **
        assert!(matches!(
            lower_pattern("/var/**/x", m()),
            Err(CompileError::UnsupportedGlob(..))
        ));
    }

    #[test]
    fn host_port_split() {
        assert_eq!(
            split_host_port("crates.io:443").unwrap(),
            ("crates.io", 443)
        );
        assert_eq!(split_host_port("[::1]:8080").unwrap(), ("::1", 8080));
        assert!(split_host_port("noport").is_err());
        assert!(split_host_port("host:notaport").is_err());
    }
}
