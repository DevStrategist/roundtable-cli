use crate::state::{Session, State, Usage, now};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use walkdir::WalkDir;

pub fn home(provider: &str) -> PathBuf {
    let key = if provider == "codex" {
        "CODEX_HOME"
    } else {
        "CLAUDE_CONFIG_DIR"
    };
    std::env::var_os(key).map(PathBuf::from).unwrap_or_else(|| {
        directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .join(format!(".{provider}"))
    })
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 140
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}

pub struct Process {
    pub pid: u32,
    pub start: u64,
    pub provider: String,
    pub files: HashSet<PathBuf>,
}

pub fn processes(system: &mut System) -> Vec<Process> {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always)
            .with_user(UpdateKind::Always),
    );
    let uid = unsafe { libc::geteuid() };
    let mut result = vec![];
    for (pid, p) in system.processes() {
        if p.user_id().is_none_or(|u| **u != uid) {
            continue;
        }
        let name = p
            .exe()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy();
        let args = p.cmd();
        let provider = if name == "codex" {
            Some("codex")
        } else if name == "claude" || name == "claude.exe" {
            Some("claude")
        } else if name == "node" {
            args.get(1).and_then(|s| {
                let s = s.to_string_lossy();
                if s.contains("/@openai/codex/") && s.ends_with("/codex.js") {
                    Some("codex")
                } else if s.contains("/@anthropic-ai/claude-code/") && s.ends_with("/cli.js") {
                    Some("claude")
                } else {
                    None
                }
            })
        } else {
            None
        };
        let Some(provider) = provider else { continue };
        if args
            .iter()
            .any(|s| s == "proxy" || s == "agents" || s == "queue")
        {
            continue;
        }
        let mut files = HashSet::new();
        #[cfg(target_os = "linux")]
        if let Ok(entries) = fs::read_dir(format!("/proc/{pid}/fd")) {
            for entry in entries.flatten().take(4096) {
                if let Ok(path) = fs::read_link(entry.path())
                    && path.extension().is_some_and(|e| e == "jsonl")
                {
                    files.insert(path);
                }
            }
        }
        #[cfg(target_os = "macos")]
        if let Ok(output) = crate::providers::bounded_output(
            std::process::Command::new("lsof").args(["-a", "-p", &pid.to_string(), "-Fn"]),
            5,
        ) {
            for line in String::from_utf8_lossy(&output).lines() {
                if let Some(path) = line.strip_prefix('n') {
                    let path = PathBuf::from(path);
                    if path.extension().is_some_and(|e| e == "jsonl") {
                        files.insert(path);
                    }
                }
            }
        }
        result.push(Process {
            pid: pid.as_u32(),
            start: p.start_time(),
            provider: provider.into(),
            files,
        });
    }
    result
}

pub fn text_content(value: &Value) -> String {
    if let Some(s) = value.as_str() {
        return s.chars().take(16000).collect();
    }
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| match v["type"].as_str()? {
                    "text" | "input_text" | "output_text" => v["text"].as_str(),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(16000)
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse(state: &State, session: &mut Session, value: &Value) -> Result<()> {
    let mut event: Option<(&str, String)> = None;
    if session.provider == "codex" {
        let p = &value["payload"];
        match value["type"].as_str().unwrap_or("") {
            "session_meta" => {
                if let Some(cwd) = p["cwd"].as_str() {
                    session.project = cwd.into();
                }
                session.parent_id = p
                    .pointer("/source/subagent/thread_spawn/parent_thread_id")
                    .and_then(Value::as_str)
                    .map(|s| format!("codex:{s}"));
            }
            "event_msg" => match p["type"].as_str().unwrap_or("") {
                "user_message" => {
                    session.state = "working".into();
                    event = Some(("prompt", p["message"].as_str().unwrap_or("").into()));
                }
                "agent_message" => {
                    event = Some(("response", p["message"].as_str().unwrap_or("").into()))
                }
                "task_started" => {
                    session.state = "working".into();
                    session.turn_id = p["turn_id"].as_str().map(str::to_owned);
                }
                "task_complete" | "task_completed" | "turn_aborted" => {
                    session.state = "idle".into();
                    session.turn_id = None;
                }
                "token_count" => {
                    let u = &p["info"]["total_token_usage"];
                    if let Some(input) = u["input_tokens"].as_u64() {
                        session.usage.input_tokens = Some(input);
                        session.usage.output_tokens = u["output_tokens"].as_u64();
                        session.usage.cached_input_tokens = u["cached_input_tokens"].as_u64();
                    }
                }
                _ => {}
            },
            "response_item" => {
                // Newer Codex versions persist visible messages only as response items.
                // Never collect developer/system messages or reasoning records.
                if p["type"] == "message" && p["channel"] != "analysis" {
                    let kind = match p["role"].as_str() {
                        Some("user") => Some("prompt"),
                        Some("assistant") => Some("response"),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        let text = text_content(&p["content"]);
                        if !text.is_empty() {
                            if kind == "prompt" {
                                session.state = "working".into();
                            }
                            event = Some((kind, text));
                        }
                    }
                }
                if p["type"] == "function_call" || p["type"] == "custom_tool_call" {
                    event = Some((
                        "tool",
                        format!("Tool: {}", p["name"].as_str().unwrap_or("unknown")),
                    ));
                }
                if p["type"] == "function_call_output" || p["type"] == "custom_tool_call_output" {
                    event = Some(("tool", text_content(&p["output"])));
                }
            }
            _ => {}
        }
    } else {
        if let Some(cwd) = value["cwd"].as_str() {
            session.project = cwd.into();
        }
        match value["type"].as_str().unwrap_or("") {
            "user" => {
                let text = text_content(&value["message"]["content"]);
                if !text.is_empty() {
                    session.state = "working".into();
                    event = Some(("prompt", text));
                }
            }
            "assistant" => {
                let m = &value["message"];
                let text = text_content(&m["content"]);
                if !text.is_empty() {
                    event = Some(("response", text));
                }
                if m["stop_reason"] == "end_turn" {
                    session.state = "idle".into();
                    session.turn_id = None;
                }
                if let Some(id) = m["id"].as_str()
                    && m["usage"].is_object()
                {
                    state.db.execute("INSERT INTO usage_items VALUES (?,?,?) ON CONFLICT(session,item) DO UPDATE SET data=excluded.data",params![session.id,id,serde_json::to_string(&m["usage"])?])?;
                    let usage=state.db.query_row("SELECT sum(coalesce(json_extract(data,'$.input_tokens'),0)+coalesce(json_extract(data,'$.cache_read_input_tokens'),0)+coalesce(json_extract(data,'$.cache_creation_input_tokens'),0)),sum(json_extract(data,'$.cache_read_input_tokens')),sum(json_extract(data,'$.output_tokens')) FROM usage_items WHERE session=?",[&session.id],|r|Ok(Usage{input_tokens:r.get::<_,Option<i64>>(0)?.map(|n|n.max(0) as u64),cached_input_tokens:r.get::<_,Option<i64>>(1)?.map(|n|n.max(0) as u64),output_tokens:r.get::<_,Option<i64>>(2)?.map(|n|n.max(0) as u64),cost_usd:session.usage.cost_usd}))?;
                    session.usage = usage;
                }
            }
            "result" => {
                session.state = "idle".into();
                session.turn_id = None;
                session.usage.cost_usd = value["total_cost_usd"].as_f64();
            }
            _ => {}
        }
    }
    session.collection = if state.policy()?.content(&session.project) {
        "content"
    } else {
        "metadata"
    }
    .into();
    if let Some((kind, text)) = event {
        if session.provider == "codex" && matches!(kind, "prompt" | "response") {
            // Some versions emit both representations. Pair them without retaining
            // unshared text, while preserving repeated messages within one format.
            let source = value["type"].as_str().unwrap_or("");
            let other = if source == "event_msg" {
                "response_item"
            } else {
                "event_msg"
            };
            let hash = format!(
                "{:x}",
                Sha256::digest(format!("{:?}:{text}", session.turn_id))
            );
            let key = format!("visible-message:{}:{kind}", session.id);
            if state.get(&key)?.as_deref() == Some(&format!("{other}:{hash}")) {
                state.set(&key, "")?;
                return Ok(());
            }
            state.set(&key, &format!("{source}:{hash}"))?;
        }
        let mut value = json!({"kind":kind,"session_id":session.id,"session":session});
        if session.collection == "content" {
            value["text"] = json!(text);
        }
        state.event(value)?;
    }
    Ok(())
}

fn identify(path: &Path, provider: &str) -> Result<Option<Session>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut first = String::new();
    for _ in 0..5 {
        first.clear();
        if reader.by_ref().take(1_000_000).read_line(&mut first)? == 0 {
            break;
        }
        if let Ok(v) = serde_json::from_str::<Value>(&first) {
            let id = if provider == "codex" {
                v["payload"]["id"].as_str()
            } else {
                v["sessionId"].as_str()
            };
            if let Some(id) = id.filter(|s| valid_id(s)) {
                let project = if provider == "codex" {
                    v["payload"]["cwd"].as_str()
                } else {
                    v["cwd"].as_str()
                }
                .unwrap_or("");
                return Ok(Some(Session::new(provider, id, project, "transcript")));
            }
        }
    }
    Ok(None)
}

pub fn tail(state: &State, path: &Path, provider: &str, processes: &[Process]) -> Result<()> {
    let Some(mut session) = identify(path, provider)? else {
        return Ok(());
    };
    if let Ok(saved) = state.session(&session.id) {
        session = saved;
    }
    if session.source == "managed"
        || (session.source == "app-server"
            && state.get(&format!("history:{}", session.id))?.as_deref() == Some("true"))
    {
        return Ok(());
    }
    let meta = fs::metadata(path)?;
    let inode = format!("{}:{}", meta.dev(), meta.ino());
    let key = path.to_string_lossy();
    let saved: Option<(String, u64)> = state
        .db
        .query_row(
            "SELECT inode,offset FROM cursors WHERE path=?",
            [key.as_ref()],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)),
        )
        .optional()?;
    let mut offset = saved
        .filter(|(i, o)| i == &inode && *o <= meta.len())
        .map(|(_, o)| o)
        .unwrap_or(0);
    // Opting a project into collection makes its existing visible transcript available.
    if session.collection != "content" && state.policy()?.content(&session.project) {
        offset = 0;
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::new(file);
    let transaction = state.db.unchecked_transaction()?;
    let begin = offset;
    loop {
        let mut line = Vec::new();
        let size = reader
            .by_ref()
            .take(1_000_001)
            .read_until(b'\n', &mut line)?;
        if size == 0 {
            break;
        }
        if !line.ends_with(b"\n") && size <= 1_000_000 {
            break;
        } // Partial writes retry from the same offset.
        offset += size as u64;
        if size > 1_000_000 {
            state.event(json!({"kind":"status","session_id":session.id,"text":"Coverage gap: oversized provider record"}))?;
        } else if session.source != "managed"
            && let Ok(value) = serde_json::from_slice::<Value>(&line)
        {
            parse(state, &mut session, &value)?;
        }
        if offset - begin >= 2_000_000 {
            break;
        }
    }
    let mapped = processes
        .iter()
        .find(|p| p.provider == provider && p.files.contains(path));
    if session.source != "managed" && session.source != "app-server" {
        session.live = mapped.is_some();
        session.pid = mapped.map(|p| p.pid);
        session.process_start = mapped.map(|p| p.start);
        session.source = if mapped.is_some() {
            "process"
        } else {
            "transcript"
        }
        .into();
        session.capabilities.clear();
        if !session.live {
            session.state = "unknown".into();
        }
    }
    session.observed_at = now();
    state.save_session(&session)?;
    state.db.execute("INSERT INTO cursors VALUES (?,?,?) ON CONFLICT(path) DO UPDATE SET inode=excluded.inode,offset=excluded.offset",params![key,inode,offset as i64])?;
    transaction.commit()?;
    Ok(())
}

pub fn scan(state: &State, system: &mut System) -> Result<(usize, bool)> {
    let processes = processes(system);
    let mut ok = true;
    for provider in ["codex", "claude"] {
        let root = home(provider).join(if provider == "codex" {
            "sessions"
        } else {
            "projects"
        });
        if !root.exists() {
            continue;
        }
        let mut files = vec![];
        for entry in WalkDir::new(&root).follow_links(false).max_depth(8) {
            match entry {
                Ok(e)
                    if e.file_type().is_file()
                        && e.path().extension().is_some_and(|x| x == "jsonl") =>
                {
                    if let Ok(m) = e.metadata() {
                        files.push((m.modified().ok(), e.into_path()));
                    }
                }
                Ok(_) => {}
                Err(_) => ok = false,
            }
            if files.len() > 10000 {
                ok = false;
                break;
            }
        }
        files.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
        // Bound work per scan. Explicit open descriptors are always included, even for old sessions.
        let mut paths: HashSet<_> = files.into_iter().take(200).map(|(_, p)| p).collect();
        for p in &processes {
            if p.provider == provider {
                paths.extend(
                    p.files
                        .iter()
                        .filter(|path| path.starts_with(&root))
                        .cloned(),
                );
            }
        }
        for path in paths {
            if tail(state, &path, provider, &processes).is_err() {
                ok = false;
            }
        }
    }
    // Claude's supported inventory maps interactive/background sessions without guessing by cwd.
    if processes.iter().any(|p| p.provider == "claude")
        && let Ok(output) = crate::providers::bounded_output(
            std::process::Command::new("claude").args(["agents", "--json"]),
            5,
        )
        && let Ok(rows) = serde_json::from_slice::<Vec<Value>>(&output)
    {
        for row in rows {
            let Some(id) = row["sessionId"].as_str().filter(|s| valid_id(s)) else {
                continue;
            };
            let Some(p) = processes
                .iter()
                .find(|p| Some(p.pid as u64) == row["pid"].as_u64())
            else {
                continue;
            };
            let mut session = state.session(&format!("claude:{id}")).unwrap_or_else(|_| {
                Session::new("claude", id, row["cwd"].as_str().unwrap_or(""), "process")
            });
            if session.source == "managed" {
                continue;
            }
            session.live = true;
            session.pid = Some(p.pid);
            session.process_start = Some(p.start);
            session.observed_at = now();
            session.state = match row["status"].as_str() {
                Some("idle") => "idle",
                Some("running" | "working" | "busy") => "working",
                Some("waiting") => "waiting",
                _ => "unknown",
            }
            .into();
            state.save_session(&session)?;
        }
    }
    let identities: HashMap<_, _> = processes.iter().map(|p| (p.pid, p.start)).collect();
    for mut session in state.sessions()? {
        if matches!(session.source.as_str(), "managed" | "app-server") {
            continue;
        }
        if session
            .pid
            .is_some_and(|pid| identities.get(&pid).copied() != session.process_start)
        {
            session.live = false;
            session.state = "exited".into();
            session.capabilities.clear();
            state.save_session(&session)?;
        }
    }
    Ok((processes.len(), ok))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn discovers_claudes_linux_native_executable_name() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let binary = tmp.path().join("claude.exe");
        fs::copy("/bin/sleep", &binary)?;
        let mut child = std::process::Command::new(binary).arg("5").spawn()?;
        std::thread::sleep(std::time::Duration::from_millis(50));
        let found = processes(&mut System::new())
            .into_iter()
            .any(|p| p.pid == child.id() && p.provider == "claude");
        let _ = child.kill();
        let _ = child.wait();
        assert!(found);
        Ok(())
    }
    #[test]
    fn codex_visible_message_formats_deduplicate_without_collecting_reasoning() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let state = State::open(tmp.path().join("state"))?;
        state.set(
            "policy",
            &json!({"content_roots":[fs::canonicalize(tmp.path())?],"remote_control":false})
                .to_string(),
        )?;
        let mut session = Session::new(
            "codex",
            "formats",
            fs::canonicalize(tmp.path())?.to_str().unwrap(),
            "transcript",
        );
        let prompt = json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"visible prompt"}]}});
        let legacy = json!({"type":"event_msg","payload":{"type":"user_message","message":"visible prompt"}});
        parse(&state, &mut session, &prompt)?;
        parse(&state, &mut session, &legacy)?;
        parse(&state, &mut session, &legacy)?;
        parse(&state, &mut session, &prompt)?;
        parse(
            &state,
            &mut session,
            &json!({"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"text","text":"private reasoning"}]}}),
        )?;
        parse(
            &state,
            &mut session,
            &json!({"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"private instructions"}]}}),
        )?;
        parse(
            &state,
            &mut session,
            &json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"visible answer"}]}}),
        )?;
        let events = state.events(&session.id, 0)?;
        let rows = events["events"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["text"], "visible prompt");
        assert_eq!(rows[1]["text"], "visible prompt");
        assert_eq!(rows[2]["text"], "visible answer");
        Ok(())
    }
    #[test]
    fn partial_records_and_cumulative_usage_are_not_counted_twice() -> Result<()> {
        use std::io::Write;
        let tmp = tempfile::tempdir()?;
        let state = State::open(tmp.path().join("state"))?;
        let path = tmp.path().join("session.jsonl");
        fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"sample\",\"cwd\":\"/tmp\"}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"secret",
        )?;
        tail(&state, &path, "codex", &[])?;
        assert_eq!(
            state.events("codex:sample", 0)?["events"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        let mut f = fs::OpenOptions::new().append(true).open(&path)?;
        f.write_all(b"\"}}\n")?;
        tail(&state, &path, "codex", &[])?;
        tail(&state, &path, "codex", &[])?;
        let events = state.events("codex:sample", 0)?;
        assert_eq!(events["events"].as_array().unwrap().len(), 1);
        assert!(events["events"][0].get("text").is_none());
        assert!(!state.session("codex:sample")?.live);
        state.set(
            "policy",
            &json!({"content_roots":[fs::canonicalize("/tmp")?],"remote_control":false})
                .to_string(),
        )?;
        tail(&state, &path, "codex", &[])?;
        assert_eq!(
            state.events("codex:sample", 0)?["events"][1]["text"],
            "secret"
        );
        Ok(())
    }
    #[test]
    fn claude_partial_messages_replace_usage_instead_of_adding() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let state = State::open(tmp.path().join("state"))?;
        let mut session = Session::new("claude", "one", "/tmp", "transcript");
        for n in [1, 3, 3] {
            parse(
                &state,
                &mut session,
                &json!({"type":"assistant","message":{"id":"msg-1","usage":{"input_tokens":10,"cache_read_input_tokens":100,"cache_creation_input_tokens":20,"output_tokens":n}}}),
            )?;
        }
        assert_eq!(session.usage.input_tokens, Some(130));
        assert_eq!(session.usage.cached_input_tokens, Some(100));
        assert_eq!(session.usage.output_tokens, Some(3));
        Ok(())
    }
}
