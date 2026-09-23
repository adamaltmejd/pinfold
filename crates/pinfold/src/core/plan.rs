//! The box spec: the JSON a caller gives `box up`, parsed into a [`Plan`].

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io::Read;
use std::path::PathBuf;

use serde::Deserialize;

/// A parsed box spec.
#[derive(Debug, Clone, Deserialize)]
pub struct Plan {
    /// The box's name; also its state directory name.
    pub name: String,
    /// The image to run.
    pub image: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    /// The uid:gid for PID 1 and all work. The host uid:gid when absent.
    #[serde(default)]
    pub user: Option<User>,
    /// Exact environment. `{ "from": "NAME" }` takes the caller's value.
    #[serde(default)]
    pub env: BTreeMap<String, Env>,
    /// Present means the box gets a proxy; absent means no way out at all.
    #[serde(default)]
    pub egress: Option<Egress>,
    #[serde(default)]
    pub cpus: Option<f64>,
    #[serde(default)]
    pub memory: Option<String>,
}

/// The box's egress allowlist.
#[derive(Debug, Clone, Deserialize)]
pub struct Egress {
    /// Exact host names, or `.suffix` for a name and its subdomains.
    #[serde(default)]
    pub allow: Vec<String>,
}

/// One host directory and where it appears in the box.
#[derive(Debug, Clone, Deserialize)]
pub struct Mount {
    pub host: PathBuf,
    pub guest: PathBuf,
    #[serde(default)]
    pub readonly: bool,
}

/// The box user.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
}

/// One environment entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Env {
    /// A literal value from the spec.
    Exact(String),
    /// `{ "from": "NAME" }`: the caller's value, passed by name only.
    From { from: String },
}

impl Plan {
    /// Parse and validate a box spec.
    pub fn parse(json: &str) -> Result<Plan, PlanError> {
        let plan: Plan = serde_json::from_str(json)?;
        plan.validate()?;
        Ok(plan)
    }

    /// Parse and validate one box spec from a reader, leaving any data after
    /// the JSON value unread so `box up` can watch the same stdin for EOF.
    pub fn from_reader(reader: impl Read) -> Result<Plan, PlanError> {
        let plan: Plan = serde_json::Deserializer::from_reader(reader)
            .into_iter::<Plan>()
            .next()
            .ok_or_else(|| PlanError::Invalid("no box spec on stdin".into()))??;
        plan.validate()?;
        Ok(plan)
    }

    fn validate(&self) -> Result<(), PlanError> {
        if !valid_name(&self.name) {
            return Err(PlanError::Invalid(format!(
                "name {:?} must start alphanumeric and hold only [A-Za-z0-9._-]",
                self.name
            )));
        }
        if self.image.is_empty() {
            return Err(PlanError::Invalid("image must not be empty".into()));
        }
        for mount in &self.mounts {
            if !mount.host.is_absolute() || !mount.guest.is_absolute() {
                return Err(PlanError::Invalid(format!(
                    "mount {}:{} must be absolute",
                    mount.host.display(),
                    mount.guest.display()
                )));
            }
        }
        for (name, value) in &self.env {
            if name.is_empty() || name.contains('=') {
                return Err(PlanError::Invalid(format!("env name {name:?} is invalid")));
            }
            if let Env::From { from } = value
                && std::env::var_os(from).is_none()
            {
                return Err(PlanError::MissingEnv(from.clone()));
            }
        }
        Ok(())
    }
}

fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.')
}

/// Why a box spec was refused.
#[derive(Debug)]
pub enum PlanError {
    Json(serde_json::Error),
    Invalid(String),
    MissingEnv(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::Json(error) => write!(formatter, "invalid box spec: {error}"),
            PlanError::Invalid(message) => write!(formatter, "invalid box spec: {message}"),
            PlanError::MissingEnv(name) => {
                write!(formatter, "environment variable {name} is not set")
            }
        }
    }
}

impl Error for PlanError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            PlanError::Json(error) => Some(error),
            PlanError::Invalid(_) | PlanError::MissingEnv(_) => None,
        }
    }
}

impl From<serde_json::Error> for PlanError {
    fn from(error: serde_json::Error) -> PlanError {
        PlanError::Json(error)
    }
}
