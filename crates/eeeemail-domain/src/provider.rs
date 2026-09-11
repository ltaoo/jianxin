use serde::{Deserialize, Serialize};

use crate::{MailError, MailErrorCode, Result};

/// Known mail provider identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderId {
    GenericImap,
    Google,
    Microsoft,
}

impl ProviderId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GenericImap => "generic-imap",
            Self::Google => "google",
            Self::Microsoft => "microsoft",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "generic-imap" => Ok(Self::GenericImap),
            "google" => Ok(Self::Google),
            "microsoft" => Ok(Self::Microsoft),
            other => Err(MailError::new(
                MailErrorCode::InvalidConfig,
                format!("unknown provider `{other}`"),
            )),
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_wire_names() {
        for id in [
            ProviderId::GenericImap,
            ProviderId::Google,
            ProviderId::Microsoft,
        ] {
            assert_eq!(ProviderId::parse(id.as_str()).unwrap(), id);
        }
        assert!(ProviderId::parse("yahoo").is_err());
    }
}
