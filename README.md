# Agent Manager

Run Codex sessions on your Linux machines and control them from a phone or desktop browser. The Rust `server` binary embeds the web interface; the Rust `worker` binary runs Codex locally and connects outward to the server.

## Build

```sh
cargo build --release --locked
install -Dm755 target/release/server ~/.local/bin/server
install -Dm755 target/release/worker ~/.local/bin/worker
```

Build requires Rust (edition 2024), a C compiler, and standard Linux development tools. SQLite is bundled. No Node build, external web assets, or separate frontend server is needed.

## Try locally

1. Set a server password of at least 12 characters, then start the server:

   ```sh
   server --data-dir ~/.local/state/agentmanager-server set-password
   server --data-dir ~/.local/state/agentmanager-server
   ```

2. Open `http://127.0.0.1:8080`, sign in, and click **Register worker**. The generated command looks like:

   ```sh
   worker --host http://127.0.0.1:8080 --key SINGLE_USE_KEY
   ```

   Run it as your normal Linux user. That machine needs an installed Codex CLI, already signed in with `codex login`. A worker is a machine, and can run several sessions at once.

3. Choose the worker and **New session**. Browse to any folder accessible to that user, create folders, choose a model, reasoning effort, sandbox, and approval policy, and start chatting. Use the same URL in another browser to control the same sessions.

4. After registration, start the worker with just `worker`. It stores its server address and private credential in `$XDG_STATE_HOME/agentmanager/worker.credential.json`, or `~/.local/state/agentmanager/worker.credential.json`. Use `--credential-file PATH` for a separate server or worker identity, `--name laptop` during registration, or `--codex /absolute/path/to/codex` if Codex is outside your PATH.

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

3. Codex persists conversation history locally using its normal storage. After a worker restart, choose a saved session to resume it; a turn that was executing when the worker process stopped is not automatically restarted. If the Codex child process exits, the worker restarts it and reconnects. The server stores worker registrations, not transcripts. Browser authentication sessions are in memory and expire after seven days or a server restart.

4. The session list contains sessions managed by this worker. A small metadata catalog beside the worker credential retains their ids, including empty sessions that Codex does not return from its history listing. Existing terminal and desktop sessions are not automatically imported. Live handover from another Codex process is outside this version's scope.

5. Session settings let you change the model, reasoning effort, approval policy, and sandbox between tasks. Additional instructions and the working folder are selected when creating a session. Defaults come from the worker's existing Codex configuration unless explicitly overridden. Available models come from that worker's Codex installation; this application makes no model requests of its own.

## Credentials and administration

1. Browser passwords are hashed with Argon2id. HTTPS browser sessions use `Secure`, `HttpOnly`, `SameSite=Strict`, host-only cookies. Mutations require the configured origin and a custom request header. Login and enrollment attempts have a shared global limit of ten per minute; password verification concurrency is limited. The UI uses a restrictive content security policy and renders model output as text, never HTML.

2. Registration keys are single-use and expire after ten minutes. Stored worker credentials are independent, randomly generated, and hashed on the server; the worker stores its credential in a mode-0600 file. **Revoke** immediately disconnects a worker and removes its registration. Re-register to reconnect. Revocation stops remote access; it does not cancel a task already running locally.

3. A worker can browse and create folders anywhere its Linux user can access, as requested. Full-access sessions can execute commands with that user's permissions. Treat the server account, worker credentials, and the server machine as access to those machines. Run workers as your usual user, not root.

4. To reset the password, run `server --data-dir SAME_DIRECTORY set-password`, then restart the server. To generate a registration key without the UI, use `server --data-dir SAME_DIRECTORY registration-key`. For automated password setup, `set-password --stdin` reads one line; avoid putting passwords in command arguments.

5. Back up the server state directory and each worker's Codex home if needed. The default server directory is `.data/server` relative to the working directory; use an explicit `--data-dir` for a service. This is a single-user first version, without MFA, file uploads, or a full terminal. It has not had an independent security audit.

## Development and validation

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
node --check web/app.js
```

The protocol integration uses [Codex app-server](https://learn.chatgpt.com/docs/app-server) over local stdio. Tested against Codex CLI 0.164.0. Tests exercise real HTTP/WebSocket routing, authorization, registration reuse/expiry, revocation, and a simulated Codex process. They do not send paid model requests. Optional ignored smoke tests check the installed Codex handshake, model catalog, and session lifecycle in a temporary Codex home.

To run the optional browser test, install Playwright somewhere outside the project and use an installed Chromium:

```sh
npm install --prefix /tmp/agentmanager-browser playwright
PLAYWRIGHT_MODULE=/tmp/agentmanager-browser/node_modules/playwright/index.mjs node tests/browser.mjs
```

It starts isolated test processes, checks two browsers including a phone viewport, and saves screenshots in a temporary directory. Set `CHROMIUM` if Chromium is not at `/usr/bin/chromium`, or `AGENTMANAGER_BIN_DIR=target/release` to exercise release binaries. To run the real Codex smoke checks: `cargo test --test codex -- --ignored`.
