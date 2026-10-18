//! `mdx setup mcp`: register this mdx binary as an MCP server with the agent
//! clients installed here (Claude Code, Codex, OpenCode).
//!
//! Claude Code and Codex are configured through their own CLIs (`claude mcp
//! add`, `codex mcp add`). OpenCode has no non-interactive command, so its
//! JSON config gets an `mcp.mdx` entry; other settings, their order and an
//! existing entry's extra fields are kept, and the old file is backed up.
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use serde::Serialize as DeriveSerialize;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const CLIENTS: [&str; 3] = ["claude", "codex", "opencode"];
const NAME: &str = "mdx";

#[derive(Debug, DeriveSerialize)]
pub struct Outcome {
    pub client: String,
    /// registered, unchanged, not_found, failed, or planned (dry run).
    pub status: String,
    pub detail: String,
    /// What was run or would be run, for doing it by hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// Registers `binary` with each of `clients` (all three when empty: missing
/// clients are reported as not_found).
pub fn mcp(clients: &[String], binary: &Path, dry_run: bool) -> Vec<Outcome> {
    let clients: Vec<String> = if clients.is_empty() {
        CLIENTS.iter().map(|c| c.to_string()).collect()
    } else {
        clients.to_vec()
    };
    clients
        .iter()
        .map(|client| match client.as_str() {
            "claude" => cli_client("claude", &["--scope", "user"], binary, dry_run),
            "codex" => cli_client("codex", &[], binary, dry_run),
            "opencode" => opencode(binary, dry_run),
            other => Outcome {
                client: other.into(),
                status: "failed".into(),
                detail: format!("unknown client; choose from {}", CLIENTS.join(", ")),
                command: None,
            },
        })
        .collect()
}

fn outcome(client: &str, status: &str, detail: impl Into<String>, command: Option<String>) -> Outcome {
    Outcome { client: client.into(), status: status.into(), detail: detail.into(), command }
}

/// `<client> mcp remove [scope] mdx`, then `<client> mcp add [scope] mdx -- BINARY mcp`,
/// unless the client already has exactly that entry.
fn cli_client(client: &str, scope: &[&str], binary: &Path, dry_run: bool) -> Outcome {
    let binary = binary.to_string_lossy().into_owned();
    let mut add: Vec<String> = vec!["mcp".into(), "add".into()];
    add.extend(scope.iter().map(|s| s.to_string()));
    add.extend([NAME.into(), "--".into(), binary.clone(), "mcp".into()]);
    let shown = format!("{client} {}", add.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "));
    let Some((program, found)) = find_client(client) else {
        let place = if client == "claude" { "on PATH or where Claude Code and the desktop app install it" } else { "on PATH" };
        return outcome(client, "not_found", format!("{client} is not {place}; run the command once it is installed"), Some(shown));
    };
    // Say which CLI did the work when it wasn't the one on PATH.
    let using = match found {
        Found::Path => String::new(),
        Found::Fallback => format!(" (using {})", program.display()),
    };
    if current_entry(client, &program).is_some_and(|(command, args)| command == binary && args == ["mcp"]) {
        return outcome(client, "unchanged", format!("{NAME} -> {binary} mcp{using}"), Some(shown));
    }
    if dry_run {
        return outcome(client, "planned", format!("would replace any existing {NAME} entry using {}", program.display()), Some(shown));
    }
    // `add` refuses an existing name, so an older entry (e.g. a checkout
    // build) is removed first; a missing one makes remove fail harmlessly.
    let mut remove: Vec<String> = vec!["mcp".into(), "remove".into()];
    remove.extend(scope.iter().map(|s| s.to_string()));
    remove.push(NAME.into());
    let _ = run(&program, &remove);
    match run(&program, &add) {
        Ok(_) => outcome(client, "registered", format!("{NAME} -> {binary} mcp{using}"), Some(shown)),
        Err(error) => outcome(client, "failed", error, Some(shown)),
    }
}

/// The command and arguments of the client's existing `mdx` entry, if any
/// (for Claude Code, only a user-scope one, which is what `add` writes).
fn current_entry(client: &str, program: &Path) -> Option<(String, Vec<String>)> {
    match client {
        "claude" => parse_claude_get(&run(program, &["mcp".into(), "get".into(), NAME.into()]).ok()?),
        "codex" => parse_codex_get(&run(program, &["mcp".into(), "get".into(), NAME.into(), "--json".into()]).ok()?),
        _ => None,
    }
}

/// `claude mcp get mdx` prints `Scope:`, `Command:` and `Args:` lines.
fn parse_claude_get(text: &str) -> Option<(String, Vec<String>)> {
    let field = |name: &str| {
        text.lines().find_map(|line| line.trim().strip_prefix(name).map(|rest| rest.trim().to_string()))
    };
    if !field("Scope:")?.starts_with("User") {
        return None;
    }
    let args = field("Args:").unwrap_or_default();
    Some((field("Command:")?, args.split_whitespace().map(String::from).collect()))
}

/// `codex mcp get mdx --json` has `transport.command` and `transport.args`.
fn parse_codex_get(text: &str) -> Option<(String, Vec<String>)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let transport = &value["transport"];
    let args = match &transport["args"] {
        serde_json::Value::Array(items) => items.iter().map(|a| a.as_str().map(String::from)).collect::<Option<_>>()?,
        serde_json::Value::Null => Vec::new(),
        _ => return None,
    };
    Some((transport["command"].as_str()?.to_string(), args))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Found {
    Path,
    Fallback,
}

/// The client's CLI: on PATH, or for Claude Code in the places its installer
/// and the Claude desktop app put it without touching PATH.
fn find_client(client: &str) -> Option<(PathBuf, Found)> {
    if let Some(program) = which(client) {
        return Some((program, Found::Path));
    }
    if client != "claude" {
        return None;
    }
    let config_home = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map(PathBuf::from);
    let app_data = std::env::var_os("APPDATA").map(PathBuf::from);
    claude_candidates(home().as_deref(), config_home.as_deref(), app_data.as_deref())
        .into_iter()
        .find(|p| is_executable(p))
        .map(|p| (p, Found::Fallback))
}

/// Where Claude Code may be installed off PATH, best first: the native
/// installer's `~/.claude/local` and `~/.local/bin`, then the newest CLI the
/// Claude desktop app bundles under `claude-code/<version>/<build>/`.
fn claude_candidates(home: Option<&Path>, config_home: Option<&Path>, app_data: Option<&Path>) -> Vec<PathBuf> {
    let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
    let mut candidates = Vec::new();
    let mut bundles = Vec::new();
    if let Some(home) = home {
        candidates.push(home.join(".claude/local").join(exe));
        candidates.push(home.join(".local/bin").join(exe));
        bundles.push(config_home.map_or_else(|| home.join(".config"), Path::to_path_buf).join("Claude"));
        bundles.push(home.join("Library/Application Support/Claude"));
    }
    bundles.extend(app_data.map(|d| d.join("Claude")));
    for bundle in bundles {
        candidates.extend(newest_bundled(&bundle.join("claude-code"), exe));
    }
    candidates
}

/// `<root>/<version>/<build>/<exe>` with the highest version.
fn newest_bundled(root: &Path, exe: &str) -> Option<PathBuf> {
    let subdirs = |dir: &Path| -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        dirs
    };
    let version = |dir: &Path| -> Option<Vec<u64>> {
        dir.file_name()?.to_str()?.split('.').map(|part| part.parse().ok()).collect()
    };
    let mut versions: Vec<(Vec<u64>, PathBuf)> =
        subdirs(root).into_iter().filter_map(|dir| Some((version(&dir)?, dir))).collect();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .find_map(|(_, dir)| subdirs(&dir).into_iter().map(|build| build.join(exe)).find(|p| p.is_file()))
}

fn run(program: &Path, args: &[String]) -> Result<String, String> {
    let is_script = program
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    let mut command = if is_script {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(program);
        c
    } else {
        Command::new(program)
    };
    let output = command.args(args).output().map_err(|e| format!("{}: {e}", program.display()))?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if output.status.success() {
        Ok(text(&output.stdout))
    } else {
        Err(format!("{} exited with {}: {}", program.display(), output.status, text(&output.stderr)))
    }
}

fn quote(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:\\".contains(c)) {
        arg.into()
    } else {
        format!("\"{}\"", arg.replace('"', "\\\""))
    }
}

/// The first `name` (with Windows executable extensions) on PATH.
pub fn which(name: &str) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) { &["exe", "cmd", "bat"] } else { &[""] };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        extensions.iter().find_map(|ext| {
            let candidate = if ext.is_empty() { dir.join(name) } else { dir.join(format!("{name}.{ext}")) };
            is_executable(&candidate).then_some(candidate)
        })
    })
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// OpenCode's global config: `$XDG_CONFIG_HOME/opencode` (default
/// `~/.config/opencode`, on every platform), `opencode.jsonc` if present,
/// else `opencode.json`.
pub fn opencode_config() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home()?.join(".config"),
    }
    .join("opencode");
    let jsonc = base.join("opencode.jsonc");
    Some(if jsonc.is_file() { jsonc } else { base.join("opencode.json") })
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

fn opencode(binary: &Path, dry_run: bool) -> Outcome {
    const CLIENT: &str = "opencode";
    let Some(config) = opencode_config() else {
        return outcome(CLIENT, "failed", "no home directory to find the OpenCode config in", None);
    };
    if !config.exists() && which(CLIENT).is_none() && !config.parent().is_some_and(Path::is_dir) {
        return outcome(CLIENT, "not_found", format!("OpenCode is not on PATH and {} does not exist", config.display()), None);
    }
    match edit_opencode(&config, binary, dry_run) {
        Ok(status) => outcome(CLIENT, status, format!("{} mcp.{NAME} -> {} mcp", config.display(), binary.display()), None),
        Err(error) => outcome(CLIENT, "failed", error, None),
    }
}

/// Sets `mcp.mdx` in the config, returning registered, unchanged or planned.
fn edit_opencode(config: &Path, binary: &Path, dry_run: bool) -> Result<&'static str, String> {
    let original = match std::fs::read_to_string(config) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", config.display())),
    };
    let mut root = match &original {
        Some(text) => serde_json::from_str::<Json>(text).map_err(|e| {
            format!("{}: {e}; it may contain comments, which this edit would lose: add mcp.{NAME} by hand", config.display())
        })?,
        None => Json::Object(Vec::new()),
    };
    let Json::Object(top) = &mut root else {
        return Err(format!("{}: the config is not a JSON object", config.display()));
    };
    let mcp = entry(top, "mcp", Json::Object(Vec::new()));
    let Json::Object(servers) = mcp else {
        return Err(format!("{}: mcp is not an object", config.display()));
    };
    let server = entry(servers, NAME, Json::Object(Vec::new()));
    let Json::Object(fields) = server else {
        return Err(format!("{}: mcp.{NAME} is not an object", config.display()));
    };
    if fields.iter().any(|(k, v)| k == "type" && *v != Json::String("local".into())) {
        return Err(format!("{}: mcp.{NAME} is a remote server; not replacing it", config.display()));
    }
    let before = fields.clone();
    set(fields, "type", Json::String("local".into()));
    set(fields, "command", Json::Array(vec![Json::String(binary.to_string_lossy().into_owned()), Json::String("mcp".into())]));
    if !fields.iter().any(|(k, _)| k == "enabled") {
        set(fields, "enabled", Json::Bool(true));
    }
    if original.is_some() && *fields == before {
        return Ok("unchanged");
    }
    if dry_run {
        return Ok("planned");
    }
    let mut text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    text.push('\n');
    if let Some(parent) = config.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    if original.is_some() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let backup = config.with_file_name(format!(
            "{}.mdx-backup-{stamp}",
            config.file_name().unwrap().to_string_lossy()
        ));
        std::fs::copy(config, &backup).map_err(|e| format!("{}: {e}", backup.display()))?;
    }
    // Write beside the config, then rename over it, so a failure never
    // leaves a half-written file.
    let temporary = config.with_file_name(".mdx-setup.tmp");
    std::fs::write(&temporary, text).map_err(|e| format!("{}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, config).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {e}", config.display())
    })?;
    Ok("registered")
}

fn entry<'a>(object: &'a mut Vec<(String, Json)>, key: &str, default: Json) -> &'a mut Json {
    let at = match object.iter().position(|(k, _)| k == key) {
        Some(at) => at,
        None => {
            object.push((key.into(), default));
            object.len() - 1
        }
    };
    &mut object[at].1
}

fn set(object: &mut Vec<(String, Json)>, key: &str, value: Json) {
    *entry(object, key, Json::Null) = value;
}

/// JSON that keeps object keys in file order, so rewriting a config leaves
/// everything but the edited entry where it was.
#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Json::Null => s.serialize_unit(),
            Json::Bool(b) => s.serialize_bool(*b),
            Json::Number(n) => n.serialize(s),
            Json::String(v) => s.serialize_str(v),
            Json::Array(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Json::Object(fields) => {
                let mut map = s.serialize_map(Some(fields.len()))?;
                for (k, v) in fields {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Json;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON")
            }
            fn visit_unit<E>(self) -> Result<Json, E> {
                Ok(Json::Null)
            }
            fn visit_bool<E>(self, v: bool) -> Result<Json, E> {
                Ok(Json::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Json, E> {
                Ok(Json::Number(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Json, E> {
                Ok(Json::Number(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Json, E> {
                serde_json::Number::from_f64(v).map(Json::Number).ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E>(self, v: &str) -> Result<Json, E> {
                Ok(Json::String(v.into()))
            }
            fn visit_string<E>(self, v: String) -> Result<Json, E> {
                Ok(Json::String(v))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Json::Array(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
                let mut fields = Vec::new();
                while let Some((k, v)) = map.next_entry()? {
                    fields.push((k, v));
                }
                Ok(Json::Object(fields))
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_entry_is_added_keeping_order_and_other_servers() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("opencode.json");
        std::fs::write(
            &config,
            r#"{"theme":"x","mcp":{"psx":{"type":"local","command":["/bin/psx","mcp"],"enabled":true}},"model":"m"}"#,
        )
        .unwrap();
        let binary = Path::new("/opt/mdx");
        assert_eq!(edit_opencode(&config, binary, true).unwrap(), "planned");
        assert!(!std::fs::read_to_string(&config).unwrap().contains("mdx"), "dry run writes nothing");
        assert_eq!(edit_opencode(&config, binary, false).unwrap(), "registered");
        let text = std::fs::read_to_string(&config).unwrap();
        let (theme, mcp, model) = (text.find("theme").unwrap(), text.find("\"mcp\"").unwrap(), text.find("model").unwrap());
        assert!(theme < mcp && mcp < model, "{text}");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["mcp"]["psx"]["command"][0], "/bin/psx");
        assert_eq!(value["mcp"]["mdx"], serde_json::json!({"type":"local","command":["/opt/mdx","mcp"],"enabled":true}));
        assert_eq!(edit_opencode(&config, binary, false).unwrap(), "unchanged");
        assert!(std::fs::read_dir(dir.path()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("mdx-backup")));
    }

    #[test]
    fn opencode_config_with_comments_or_a_remote_entry_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("opencode.jsonc");
        std::fs::write(&config, "{\n  // mine\n  \"mcp\": {}\n}\n").unwrap();
        assert!(edit_opencode(&config, Path::new("/opt/mdx"), false).unwrap_err().contains("comments"));
        std::fs::write(&config, r#"{"mcp":{"mdx":{"type":"remote","url":"http://x"}}}"#).unwrap();
        assert!(edit_opencode(&config, Path::new("/opt/mdx"), false).unwrap_err().contains("remote"));
        let fresh = dir.path().join("new/opencode.json");
        assert_eq!(edit_opencode(&fresh, Path::new("/opt/mdx"), false).unwrap(), "registered");
    }

    #[test]
    fn existing_claude_and_codex_entries_are_read() {
        let claude = "mdx:\n  Scope: User config (available in all your projects)\n  Status: \u{2714} Connected\n  Type: stdio\n  Command: /opt/mdx\n  Args: mcp\n  Environment:\n";
        assert_eq!(parse_claude_get(claude), Some(("/opt/mdx".into(), vec!["mcp".into()])));
        assert_eq!(parse_claude_get(&claude.replace("User config", "Project config")), None, "only the user entry counts");
        let codex = r#"{"name":"mdx","enabled":true,"transport":{"type":"stdio","command":"/opt/mdx","args":["mcp"],"env":null}}"#;
        assert_eq!(parse_codex_get(codex), Some(("/opt/mdx".into(), vec!["mcp".into()])));
        assert_eq!(parse_codex_get(r#"{"transport":{"type":"streamable_http","url":"http://x"}}"#), None);
    }

    #[test]
    fn bundled_claude_is_found_newest_version_first() {
        let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".config/Claude/claude-code");
        for (version, build) in [("2.1.9", "aaa"), ("2.1.295", "bbb"), ("notes", "ccc")] {
            std::fs::create_dir_all(root.join(version).join(build)).unwrap();
            std::fs::write(root.join(version).join(build).join(exe), "").unwrap();
        }
        std::fs::create_dir_all(root.join("2.2.0/empty")).unwrap();
        let candidates = claude_candidates(Some(home.path()), None, None);
        assert_eq!(candidates[0], home.path().join(".claude/local").join(exe));
        assert_eq!(candidates.last(), Some(&root.join("2.1.295/bbb").join(exe)));
        let app_data = tempfile::tempdir().unwrap();
        let windows = app_data.path().join("Claude/claude-code/1.0.0/x");
        std::fs::create_dir_all(&windows).unwrap();
        std::fs::write(windows.join(exe), "").unwrap();
        let candidates = claude_candidates(None, None, Some(app_data.path()));
        assert_eq!(candidates, vec![windows.join(exe)]);
    }
}
