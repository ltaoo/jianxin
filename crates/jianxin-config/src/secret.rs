//! Secret references.
//!
//! Tokens and passwords must not live in the config file in plaintext. They are
//! resolved at use time from the environment, the OS keychain, an external
//! command, or a file. This mirrors "Token 不应该进入普通数据库明文".

use jianxin_domain::{MailError, MailErrorCode, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where a secret actually comes from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SecretRef {
    /// Literal value. Convenient for local experiments; avoid for real accounts.
    Plain(String),
    /// Read `$NAME` from the process environment.
    Env { env: String },
    /// Run a shell command and use its trimmed stdout.
    ///
    /// On macOS this is the keychain bridge:
    /// `security find-internet-password -s imap.gmail.com -w`
    Command { command: String },
    /// Look the secret up in the OS keychain by service/account name.
    Keychain { keychain: String },
    /// Read the secret from a file (trimmed).
    File { file: PathBuf },
}

impl SecretRef {
    /// Whether a value is present without resolving it.
    pub fn has_value(&self) -> bool {
        match self {
            SecretRef::Plain(s) => !s.is_empty(),
            _ => true,
        }
    }

    pub fn resolve(&self) -> Result<String> {
        match self {
            SecretRef::Plain(value) => Ok(value.clone()),
            SecretRef::Env { env } => std::env::var(env).map_err(|_| {
                MailError::new(
                    MailErrorCode::AuthenticationRequired,
                    format!("environment variable `{env}` is not set"),
                )
            }),
            SecretRef::Command { command } => run_command(command),
            SecretRef::Keychain { keychain } => read_keychain(keychain),
            SecretRef::File { file } => {
                let raw = std::fs::read_to_string(file).map_err(|e| {
                    MailError::new(
                        MailErrorCode::AuthenticationRequired,
                        format!("cannot read secret file {}: {e}", file.display()),
                    )
                })?;
                Ok(raw.trim_end().to_string())
            }
        }
    }
}

/// Run `sh -c <command>` and return trimmed stdout.
pub fn run_command(command: &str) -> Result<String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| {
            MailError::new(
                MailErrorCode::AuthenticationRequired,
                format!("failed to run secret command `{command}`: {e}"),
            )
        })?;

    if !output.status.success() {
        return Err(MailError::new(
            MailErrorCode::AuthenticationRequired,
            format!(
                "secret command `{command}` failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}

/// Platform keychain read.
///
/// * macOS  — `security find-generic-password -s <name> -w`
/// * Linux  — `secret-tool lookup service <name>` (Secret Service / keyring)
/// * Windows — `cmdkey` has no read API, so this is unsupported; use `command`.
fn read_keychain(service: &str) -> Result<String> {
    let command = if cfg!(target_os = "macos") {
        format!("security find-generic-password -s '{service}' -w")
    } else if cfg!(target_os = "linux") {
        format!("secret-tool lookup service '{service}'")
    } else {
        return Err(MailError::new(
            MailErrorCode::UnsupportedFeature,
            "keychain lookup is only supported on macOS and Linux; use `command` instead",
        ));
    };
    run_command(&command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_env() {
        std::env::set_var("JIANXIN_TEST_SECRET", "s3cret");
        let s = SecretRef::Env {
            env: "JIANXIN_TEST_SECRET".into(),
        };
        assert_eq!(s.resolve().unwrap(), "s3cret");
    }

    #[test]
    fn resolves_command() {
        let s = SecretRef::Command {
            command: "echo hello".into(),
        };
        assert_eq!(s.resolve().unwrap(), "hello");
    }

    #[test]
    fn plain_string_deserialises() {
        let s: SecretRef = serde_json::from_str("\"literal\"").unwrap();
        assert!(matches!(s, SecretRef::Plain(v) if v == "literal"));
    }

    #[test]
    fn env_object_deserialises() {
        let s: SecretRef = serde_json::from_str("{\"env\":\"FOO\"}").unwrap();
        assert!(matches!(s, SecretRef::Env { env } if env == "FOO"));
    }
}
