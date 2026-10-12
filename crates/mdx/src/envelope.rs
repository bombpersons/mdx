//! The JSON response envelope.
//!
//! This is the toolkit's contract with its callers. Every command emits exactly
//! one of these documents on stdout. The rules it encodes:
//!
//! * nothing is ever truncated without saying so in `coverage`;
//! * a partial result is distinguishable from a complete one;
//! * the identity of the analysed artefact travels with every response;
//! * the process exit code and the `exit` field always agree.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

/// Process exit codes. Kept in sync with `ExitCode` by construction.
pub mod exit {
    pub const OK: i32 = 0;
    pub const USAGE: i32 = 1;
    pub const PARTIAL: i32 = 2;
    pub const FAILED: i32 = 3;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Exit {
    Ok,
    Usage,
    Partial,
    Failed,
}

impl Exit {
    pub fn code(self) -> i32 {
        match self {
            Exit::Ok => exit::OK,
            Exit::Usage => exit::USAGE,
            Exit::Partial => exit::PARTIAL,
            Exit::Failed => exit::FAILED,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    /// Everything the command could observe was observed.
    Complete,
    /// Some of what could have been observed was not.
    Partial,
    /// Coverage could not be established at all.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Coverage {
    pub status: CoverageStatus,
    /// Conditions that hold for this response, e.g. `["frames_1..=600"]`.
    pub conditions: Vec<String>,
    /// Named slices that were cut, written as `name[from..to]`.
    pub truncated: Vec<String>,
    /// Hard limits that shaped the result, e.g. a byte budget.
    pub limits: Vec<String>,
    /// Anything an analyst must not conclude from this response.
    pub limitations: Vec<String>,
}

impl Coverage {
    pub fn complete(conditions: Vec<String>) -> Self {
        Self {
            status: CoverageStatus::Complete,
            conditions,
            truncated: Vec::new(),
            limits: Vec::new(),
            limitations: Vec::new(),
        }
    }

    /// A result that is complete as far as it goes but did not cover
    /// everything the command could have covered.
    pub fn partial(
        conditions: Vec<String>,
        truncated: Vec<String>,
        limitations: Vec<String>,
    ) -> Self {
        Self {
            status: CoverageStatus::Partial,
            conditions,
            truncated,
            limits: Vec::new(),
            limitations,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.status == CoverageStatus::Complete
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Subject {
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
    pub format: String,
    pub architecture: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Warning {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

impl Warning {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            location: None,
        }
    }

    pub fn at(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Failure {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Envelope<T> {
    /// Qualified command name, e.g. `mdx.header`.
    pub tool: String,
    pub schema_version: u32,
    pub ok: bool,
    pub exit: Exit,
    pub subject: Option<Subject>,
    pub coverage: Coverage,
    // Always serialized, even when empty. An agent consuming these
    // envelopes cannot tell "this command reported no warnings" from "this
    // tool does not report warnings", and the second reading is the one that
    // silently hides a truncated or partial result.
    pub warnings: Vec<Warning>,
    pub limitations: Vec<String>,
    pub data: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
}

impl<T: Serialize> Envelope<T> {
    pub fn ok(tool: &str, subject: Option<Subject>, coverage: Coverage, data: T) -> Self {
        let warnings = Vec::new();
        let limitations = coverage.limitations.clone();
        Self {
            tool: tool.to_string(),
            schema_version: SCHEMA_VERSION,
            ok: coverage.is_complete(),
            exit: if coverage.is_complete() {
                Exit::Ok
            } else {
                Exit::Partial
            },
            subject,
            coverage,
            warnings,
            limitations,
            data,
            error: None,
        }
    }
}

impl Envelope<serde_json::Value> {
    pub fn failure(tool: &str, failure: Failure) -> Self {
        Self {
            tool: tool.to_string(),
            schema_version: SCHEMA_VERSION,
            ok: false,
            exit: Exit::Failed,
            subject: None,
            coverage: Coverage {
                status: CoverageStatus::Unknown,
                conditions: vec![],
                truncated: vec![],
                limits: vec![],
                limitations: vec![],
            },
            warnings: vec![],
            limitations: vec![],
            data: serde_json::Value::Null,
            error: Some(failure),
        }
    }
}

/// Serialise an envelope as canonical JSON: keys sorted, compact, one line.
pub fn render<T: Serialize>(envelope: &Envelope<T>) -> String {
    let value = serde_json::to_value(envelope).expect("envelope is serialisable");
    serde_json::to_string(&value).expect("envelope is serialisable")
}

/// Print an envelope and return the process exit code.
pub fn emit<T: Serialize>(envelope: &Envelope<T>) -> i32 {
    println!("{}", render(envelope));
    envelope.exit.code()
}
