# Roundtable CLI

Connect a Mac or Linux machine to Roundtable and supervise Codex and Claude Code sessions.
The runtime is Rust. Node is only an npm launcher; background services run the native binary.
Version 0.2.0 is prepared in source but not yet published. The login-only server is live;
session monitoring/control requires deploying the matching server changes first.

```sh
npm install -g --install-links --allow-scripts=@roundtable/cli github:DevStrategist/roundtable-cli#v0.2.2
roundtable login
roundtable setup
```

Open https://roundtable.bot/cli/sessions to view machines, sessions, activity and usage.
Installation alone does not read or upload conversations. `setup` enables metadata monitoring
at user login. Enable visible prompt/response collection for selected projects and remote control:

```sh
roundtable monitor --content /absolute/project/path --allow-control
```

Grant each assistant access from the Coding sessions page. Monitor, conversation read, prompting
and interruption are separate permissions. Grants expire after 24 hours in the UI. Machine-wide
grants explicitly cover future sessions. Memory-bank permissions do not grant machine control.

```sh
roundtable sessions list --json
roundtable sessions start codex --cwd /absolute/project/path
roundtable sessions start claude --cwd /absolute/project/path
roundtable sessions prompt codex:SESSION_ID "Run the focused test suite"
roundtable sessions events codex:SESSION_ID --follow
roundtable sessions interrupt codex:SESSION_ID
roundtable commands list
roundtable commands resolve COMMAND_ID --outcome delivered
```

Managed sessions default to read-only Codex sandboxing or Claude's Read/Grep/Glob tools with
`dontAsk` permissions. Add `--workspace-write` when starting a session to allow workspace edits.
Existing Codex sessions on a shared app-server are discoverable and controllable. Standalone
Codex and existing Claude terminal sessions are observed; Roundtable never opens a second writer
against a live Claude conversation. Use `sessions start claude` for remotely promptable Claude.
Provider CLIs must already be installed and signed in. Their credentials stay on the machine.
Local prompts require an idle session. Remote queued guidance waits for the next idle turn and
expires after 60 seconds. After an uncertain delivery, inspect the provider before resolving
the command as `delivered` or `not-delivered`; resolution never sends the prompt again.

```sh
roundtable service status
roundtable service stop
roundtable service start
roundtable service uninstall
roundtable monitor --metadata-only --deny-control
roundtable logout
```

On a headless Linux machine without Secret Service, `roundtable login --insecure-storage` stores
the machine credential in a private 0600 file. On Linux without systemd, run `roundtable daemon`
under your own supervisor. macOS uses a user LaunchAgent; Linux uses a systemd user service.
Services start at login, not before login. Stopping the service ends Roundtable-managed provider
processes. Existing external terminal sessions are unaffected. Stop deliberately before installing
an upgraded service binary with `roundtable service install`.

Telemetry syncs every two seconds, with five-second inventory reconciliation and bounded retry
backoff. Offline activity is buffered locally. Overflow is reported as a coverage gap. Usage fields
are cumulative provider reports; unavailable fields are null. No token-price estimate is invented.
Claude input totals include uncached and cached tokens; cached input is a subset of input.
The dashboard shows stale/offline data explicitly and excludes it from current session counts.

macOS arm64/x64 and Linux glibc arm64/x64 packages are built by the release workflow. Linux x64
requires glibc 2.35+, Linux arm64 requires 2.39+. Musl/Alpine is not packaged. To build from source:

```sh
cargo install --path cli --locked
```

The npm scope must be configured by a maintainer before the first public release. CI produces
tarballs for review; publishing is a separate, manually enabled workflow step.
Configure each package's [npm trusted publisher](https://docs.npmjs.com/trusted-publishers/)
for `DevStrategist/Roundtable`, workflow `cli-release.yml`, environment `npm-publish`.
The workflow uses the [GitHub-hosted native runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
for all four targets. Those builds must pass before enabling publication.
