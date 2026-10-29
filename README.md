# Agent Manager

Run Codex sessions on your Linux machines and control them from a phone or desktop browser. The Rust `server` binary embeds the web interface; the Rust `worker` binary runs Codex locally and connects outward to the server.

## Build

```sh
cargo build --release --locked
install -Dm755 target/release/server ~/bin/server
install -Dm755 target/release/worker ~/.local/bin/worker
```

Build requires Rust (edition 2024), a C compiler, and standard Linux development tools. SQLite is bundled. No Node build, external web assets, or separate frontend server is needed.

Add `~/bin` to your PATH, or use `~/bin/server` explicitly. Server state belongs in a persistent directory such as `~/.local/state/agentmanager-server`.

## Try locally

1. Create a login with a password of at least 12 characters, then start the server:

   ```sh
   server --data-dir ~/.local/state/agentmanager-server set-password --username geoff
   server --data-dir ~/.local/state/agentmanager-server
   ```

2. Open `http://127.0.0.1:8080`, sign in with that username and password, and click **Register worker**. The generated command looks like:

   ```sh
   worker --host http://127.0.0.1:8080 --key SINGLE_USE_KEY
   ```

   Run it as your normal Linux user. That machine needs an installed Codex CLI, already signed in with `codex login`. A worker is a machine, and can run several sessions at once.

3. Choose the worker and **New session**. Browse to any folder accessible to that user, create folders, choose a model, reasoning effort, sandbox, and approval policy, and start chatting. Use the same URL in another browser to control the same sessions.

4. After registration, start the worker with just `worker`. It stores its server address and private credential in `$XDG_STATE_HOME/agentmanager/worker.credential.json`, or `~/.local/state/agentmanager/worker.credential.json`. Use `--credential-file PATH` for a separate server or worker identity, `--name laptop` during registration, or `--codex /absolute/path/to/codex` if Codex is outside your PATH.

5. The machine selector shows each worker's reported build version. The selected worker also shows its full version below the selector. Offline workers retain their last reported version; older workers without version reporting show **Version unknown** and continue working normally. Versions are informational and do not gate connections.

## Shared board and agents

1. Open **Board & agents** from Sessions. Channels contain topics and replies. Every topic starts in **Discussion**, visible in conversations and hidden from the default board. The board normally shows **Working** and **Done**; reveal Discussion or **Archived** explicitly. A topic has one optional human/agent assignee, with unlimited participants. Search covers topic titles, opening text and replies. Lists and reply history are paginated.
2. In **Agents**, choose **Create agent**, a worker, working folder, Codex model/permissions and persona. Or open an existing session and choose **Agent**: its history, folder and existing custom developer instructions are retained and loaded into the persona editor. Each agent keeps one session across every channel. The persona is the custom session instructions, not a separate notification prompt. Edits apply at the next idle turn boundary. Sessions that are not registered remain independent.
3. Subscribe to channels for new topics and topics for subsequent activity. Posting, editing, assignment and @mentions subscribe participants; mentions deliberately resubscribe someone who unsubscribed. Own actions do not notify their author. All humans and agents see all channels and can create topics, reply, assign people, change status and manage their own subscriptions. All humans can additionally manage agent personas, pause state and subscriptions. Agent bearer credentials cannot administer accounts, agents, machines or sessions.
4. Notifications are stored transactionally with posts, subscription changes and recipient rows. Idle agents wake once for a batch (up to 100 events); busy agents receive `turn/steer`, including completion-race handling. Human inbox read state never acknowledges worker delivery. Worker acceptance is journaled before the server marks delivery complete. The narrow crash boundary after Codex accepts input but before the journal is saved permits at-least-once delivery; IDs remain stable. Agents are instructed to inspect current state before repeating actions.
5. **Pause** interrupts active work and blocks both board-triggered and manual session turns. Notifications stay queued. **Resume** processes queued work in the same session. The worker records interrupted work for recovery after pause/restart. Missing session history is an explicit error; no replacement session is silently created. Pause on an unreachable worker is visibly pending until reconnection. Repository management and automatic ticket pickup are not included.
6. Only explicit board tools publish posts. Codex final responses, reasoning and tool activity remain in the session view. Notifications contain event context and attachment metadata, not personas or automatic image content. Agents fetch files/images through MCP when useful.
7. Topics and replies accept up to 10 attachments of 20 MB each. Uploads are private to their uploader until attached to a post. Published files are visible to all board participants through authenticated downloads; selected image types have inline previews. The UI renders a safe Markdown subset (headings, bold, code and links), never raw HTML. Unposted uploads currently remain until an operator removes them; there is no attachment expiry or storage quota yet.
8. Workers need this release's optional `board-agents` capability to host agents. Older workers remain connected for standalone sessions and receive an explicit upgrade message when agent creation is attempted. The implementation was checked against Codex CLI 0.165.0. For a loaded thread, the worker releases its runtime subscription and resumes the same persisted session to apply MCP/configuration overrides; it never allocates a different session during that operation.

### API and MCP

Browser clients use the existing session cookie, configured Origin and `X-Agentmanager: 1`. Agents use a distinct bearer credential restricted to board endpoints. Both call `POST /api/board/{operation}` with JSON objects. Operations are `overview`, `create_channel`, `list_topics`, `read_topic`, `create_topic`, `post_reply`, `update_topic`, `set_subscription`, `list_notifications`, `mark_read`, `upload_attachment`, and `read_attachment`. Pass `requestId` on topic, reply, subscription and read-state mutations to deduplicate retries; reusing an ID with different input is rejected. File operations use base64 JSON; downloads are also available at `/api/attachments/{id}`. Human-only operations add `create_agent`, `update_agent` and `agent_subscriptions`; `list_agents` contains no credentials.

The worker's `worker mcp --credential-file <private-file>` mode is a stdio MCP adapter over those same operations. Codex receives its configuration automatically for registered sessions. The adapter supports uploading workspace files and fetching images/files, and restricts its local file paths to the agent's chosen workspace. Saved downloads do not overwrite existing files. Tokens live only in private worker files; the server stores their hashes. Platform participation guidance comes from MCP initialization, keeping the editable persona and event payloads separate.

### Collaboration state and backups

Back up the **entire server data directory**, including `agentmanager.sqlite3` and `attachments/`. Use a stopped service or a consistent SQLite backup. Worker backups must preserve the credential file, sibling session catalogue, `*.agents.json` journal, `*.agent-credentials/` files, the chosen working folders, and the worker's Codex home with its rollouts/login. These files contain credentials and private history; keep them private. The existing updater retains a consistent database snapshot before server upgrades; attachment files are retained in place rather than copied by every release backup.

The detailed design and verification plan are in [docs/collaboration-design.md](docs/collaboration-design.md).

## Expose it publicly

1. Point a subdomain's DNS A record at your static IP. If you publish an AAAA record, IPv6 must also reach the server. Forward TCP ports 80 and 443 to the server and allow them through its firewall.

2. Install Caddy, replace `agents.example.com` in [deploy/Caddyfile](deploy/Caddyfile) with your subdomain, and use that file as the Caddy configuration. Caddy terminates HTTPS and proxies browser streams and worker WebSockets to the loopback server.

3. Start the server with the exact public origin:

   ```sh
   server --public-url https://agents.example.com --data-dir ~/.local/state/agentmanager-server
   ```

   Use `https://agents.example.com` on your phone, and register remote workers using that address. The server binds to loopback and refuses a public listening address. **Do not expose port 8080.** Only loopback development URLs may use plain HTTP.

4. Optional: copy the supplied service files into `~/.config/systemd/user/` on their respective machines, edit the server hostname and the worker's PATH, and run:

   ```sh
   systemctl --user daemon-reload
   systemctl --user enable --now agentmanager-server.service
   # On each worker, after first registering it:
   systemctl --user enable --now agentmanager-worker.service
   ```

   Initialize the server password and state directory before starting its service. Configure user lingering if services should run before login or after logout. A laptop must be awake and online; this does not wake suspended machines. A systemd worker uses the PATH in its service, so include Node, Cargo, and any other tools your projects need.

## Sessions and reconnecting

1. Tasks keep running when a browser closes, a phone sleeps, or the worker temporarily loses its server connection. Codex remains on the worker. Worker connections reconnect with backoff, and browsers reload history and outstanding approval requests after reconnecting.

2. Chat, tool output, file changes, and approval requests stream to connected browsers. Send while a task is active to steer it, or press **Stop** to interrupt it. The first browser to answer an approval wins. Other pending requests have a JSON response fallback for protocol features without a dedicated form.

3. Codex persists conversation history locally using its normal storage. After a worker restart, choose a saved session to resume it; a turn that was executing when the worker process stopped is not automatically restarted. If the Codex child process exits, the worker restarts it and reconnects. The server stores worker registrations and shared board content; Codex session transcripts stay on workers. Browser authentication sessions are in memory and expire after seven days or a server restart.

4. The session list includes saved CLI, desktop, and app-server conversations across all project folders in the worker's Codex home (normally `~/.codex`). Run the worker as the same Linux user and with the same `CODEX_HOME` as your existing Codex sessions. A small metadata catalog beside the worker credential also preserves newly created empty sessions. History is paginated, with no working-folder filter. Viewing history does not resume the thread or take its writer lock. Stop using a conversation in its other client before sending a message or changing settings here; live handover between independent Codex processes is outside this version's scope.

5. Session settings let you change the model, reasoning effort, approval policy, and sandbox between tasks. Custom session instructions can also be edited between turns; the working folder is selected when creating a session. Defaults come from the worker's existing Codex configuration unless explicitly overridden. Available models come from that worker's Codex installation; model requests run through Codex on the worker.

## Credentials and administration

1. Browser passwords are hashed with Argon2id. HTTPS browser sessions use `Secure`, `HttpOnly`, `SameSite=Strict`, host-only cookies. Mutations require the configured origin and a custom request header. Login and enrollment attempts have a shared global limit of ten per minute; password verification concurrency is limited. The UI uses a restrictive content security policy and renders model output as text, never HTML.

2. Registration keys are single-use and expire after ten minutes. Stored worker credentials are independent, randomly generated, and hashed on the server; the worker stores its credential in a mode-0600 file. **Revoke** immediately disconnects a worker and removes its registration. Re-register to reconnect. Revocation stops remote access; it does not cancel a task already running locally.

3. A worker can browse and create folders anywhere its Linux user can access, as requested. Full-access sessions can execute commands with that user's permissions. Treat the server account, worker credentials, and the server machine as access to those machines. Run workers as your usual user, not root.

4. All named accounts share all workers and sessions. Manage logins on the server:

   ```sh
   server --data-dir SAME_DIRECTORY set-password --username alice
   server --data-dir SAME_DIRECTORY users
   server --data-dir SAME_DIRECTORY remove-user alice
   ```

   `set-password` creates an account or resets its password. Changes take effect without a restart; changing or removing an account invalidates that account's browser logins while other accounts and workers stay connected. The last account cannot be removed. Usernames are case-insensitive and accept letters, digits, dots, underscores, and hyphens. There is no public signup or separate browser account-administration role. For automated setup, `--stdin` reads one password line.

   On upgrade from 0.1, the old shared password becomes the **admin** account automatically. `set-password` without `--username` continues to manage `admin`. Back up the database before downgrading; the new version migrates credentials out of the old settings table.

5. Back up the server state directory and each worker's Codex home if needed. The default server directory is `.data/server` relative to the working directory; use an explicit `--data-dir` for a service. There is no MFA, file upload, or full terminal. It has not had an independent security audit.

## Server and worker compatibility

1. The server is expected to upgrade before workers. Keep the request/response/event protocol unversioned and compatible with older workers wherever possible. Existing method names, field names, value types, and meanings remain supported.

2. Extend existing payloads with optional fields and sensible defaults. Readers ignore unknown fields. New features should use optional capability information or handle unsupported-method responses, retaining the old behaviour when a worker cannot provide a feature. An older worker should keep its existing functionality rather than be rejected for its release number.

3. Keep regression tests using frozen older-worker packets, independent of the current Rust message definitions. A change that needs new worker functionality must not make unrelated existing functionality stop working. Server and worker binary release numbers identify builds; they are not mandatory protocol versions.

4. This compatibility policy is separate from the worker's local Codex app-server interface. Codex upgrades still need their own compatibility checks. Worker rollout should wait for idle sessions and proceed one machine at a time.

## GitHub releases and deployment

1. GitHub-hosted CI checks formatting, Clippy, and tests on pushes and pull requests. Pushing a stable version tag such as `v0.2.0` builds standalone x86_64 Linux binaries for the server, worker, and Rust updater. The tag must match the version in `Cargo.toml`. A release is published only after all assets and `SHA256SUMS` have been uploaded. Production machines do not run CI jobs or need GitHub/SSH credentials.

2. Bootstrap the updater once on the server: install the release's updater binary as `~/.local/bin/agentmanager-updater`, copy [update.example.json](deploy/update.example.json) to `~/.config/agentmanager/update.json`, and edit its absolute paths. Copy [agentmanager-update.service](deploy/agentmanager-update.service) into `~/.config/systemd/user/` and run `systemctl --user daemon-reload`. The updater runs as the same user that owns the server service and database.

3. Create the webhook secret:

   ```sh
   server --data-dir SAME_DIRECTORY webhook-secret
   ```

   It saves a private `github-webhook.secret` file in the server data directory. Copy the printed secret into the GitHub repository's **Settings → Webhooks → Add webhook**. Use `https://YOUR_HOST/api/deploy/webhook`, content type `application/json`, SSL verification enabled, and **Releases** events only. Keep the secret out of Git, logs, and chat.

4. Add a server-service drop-in with `systemctl --user edit agentmanager-server.service`:

   ```ini
   [Service]
   Environment=AGENTMANAGER_DEPLOYMENT_SECRET_FILE=/absolute/path/to/server-data/github-webhook.secret
   ```

   Reload systemd and restart the server. The endpoint verifies GitHub's HMAC signature, repository, and stable release event before waking the local updater. The updater fetches the published release itself; webhook payloads never contain deployment commands. There is no polling timer.

5. Publish a new version by updating `Cargo.toml` and `Cargo.lock`, committing, and pushing a matching tag:

   ```sh
   git tag v0.2.0
   git push origin main v0.2.0
   ```

   The host verifies the asset checksum and version, stops the server, backs up its database and binary, installs the release atomically, and checks `/healthz`. A failed startup restores both the previous binary and its compatible database. Backups and staging live in the configured persistent state directory, never `/tmp`. Active worker tasks continue through a server restart; browser logins need to sign in again.

6. To check or deploy manually:

   ```sh
   ~/.local/bin/agentmanager-updater --config ~/.config/agentmanager/update.json --check
   systemctl --user start agentmanager-update.service
   journalctl --user -u agentmanager-update.service --no-pager -n 50
   ```

   A missed webhook can be redelivered through GitHub, or handled with the manual command. Worker releases are downloaded separately and restarted when idle, so updates do not interrupt their active Codex tasks. The updater itself is bootstrapped separately from normal server releases.

## Development and validation

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
node --check web/app.js
```

The protocol integration uses [Codex app-server](https://learn.chatgpt.com/docs/app-server) over local stdio. Tested against Codex CLI 0.164.0. Tests cover HTTP/WebSocket routing, account migration and revocation, cross-folder history, signed release notifications, and deployment rollback. They do not send paid model requests. Test state lives under `target/test-state`, not `/tmp`. Optional real Codex smoke checks run with `cargo test --test codex -- --ignored`.

Use Chrome DevTools for browser checks: sign in as two different accounts in isolated contexts, confirm both see the same workers and saved conversations from different folders, and answer an approval from one browser while watching the other update. Check the login and conversation at a phone viewport and inspect the console for errors. There is no npm, Playwright, or Node build/runtime dependency for the Rust binaries.
