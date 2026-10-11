# Release verification

Verified October 11, 2026 (UTC), against the live Roundtable account and website.
The final installed package is **0.2.7**. Provider versions were Codex 0.162.1 and
Claude Code 2.1.296. Live hosts were macOS ARM64 and Ubuntu Linux x64.

| Flow | macOS ARM64 | Linux x64 |
| --- | --- | --- |
| Install the GitHub npm tarball and run the native CLI | Passed | Passed |
| Browser approval, computer naming and persistent machine login | Passed | Passed |
| Background service installation and restart after host restart | Passed | Passed |
| Local prompt → managed Codex/Claude response | Passed | Passed |
| Assistant MCP prompt → managed Codex/Claude response and receipt | Passed | Passed |
| Existing Codex/Claude discovery, project, live status and visible prompts | Passed | Passed |
| Existing shared Codex daemon: discover, read and send another prompt | Passed | Passed |
| Provider usage, activity streaming and live dashboard counts | Passed | Passed |
| Codex steering and guidance queued until idle | Passed | Passed |
| Codex/Claude interruption | Passed | Passed |
| Separate assistant access grants, denied access and revocation | Passed | Passed |

The release workflow builds and tests all four native targets: macOS ARM64/x64
and Linux ARM64/x64. Intel macOS and ARM64 Linux have CI coverage, not live cloud
account acceptance. [Release build](https://github.com/DevStrategist/roundtable-cli/actions/runs/38106885278).

The Rust tests cover transcript parsing and deduplication, usage accumulation,
durable buffering and overflow gaps, credential storage, service definitions,
private socket symlinks, WebSocket framing, and fragmented local IPC requests.
Node tests cover checksum verification and launcher argument/signal forwarding.
The server additionally exercises real PostgreSQL authorization and the compiled
Rust CLI over HTTP, including login, telemetry, commands and permission changes.

Cloud login used the explicit `--insecure-storage` private-file fallback because
headless credential stores were unavailable or locked. An unlocked interactive
macOS Keychain login was not part of this cloud acceptance.

## Capability limits

- Ordinary existing Claude and standalone Codex terminals are observation-only.
  Control requires a Roundtable-managed session or a supported shared Codex daemon.
- Claude interruption ends its managed process. Start another session to continue.
- Collection is metadata-only until project content is enabled. Assistant control
  also requires a local opt-in and an explicit assistant grant in Roundtable.
- Services start at user login. Linux requires a systemd user manager, or an external
  supervisor for `roundtable daemon`.
- Usage reflects provider reports. Missing prices, token fields and account quotas
  are not estimated.
