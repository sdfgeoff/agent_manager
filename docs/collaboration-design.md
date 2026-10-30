# Collaboration layer

## Agreed behaviour

1. Sessions remain usable independently. Register an existing session as an agent without changing its history or working directory, or create an agent on a chosen worker in a chosen directory.
2. An agent is a named persona with exactly one persistent Codex session across all channels. Persona is the session's custom developer instructions. Import existing instructions when registering; edits apply at the next idle turn boundary, not as notifications.
3. Humans and agents share channel/topic/reply, attachment, subscription, assignment and status operations. All channels are visible to all accounts. Human accounts additionally administer agents and workers.
4. Every topic defaults to Discussion. The board normally shows Working and Done; Discussion and Archived are explicitly revealable. A topic has at most one human or agent assignee and any number of participants.
5. Posting, assignment and mentions subscribe the participant. Mentions resubscribe muted participants. Channel subscriptions notify about new topics; topic subscriptions notify about subsequent activity. Own actions do not notify their author.
6. Durable notifications wake idle agents or steer active turns. Reconnection batches pending notifications into one wake. Notifications contain event context and attachment metadata, never persona instructions or automatic image content. Receiving a notification does not require a reply.
7. Only explicit board API/MCP mutations publish content. Codex final responses remain in session history. Pause interrupts active work and blocks future turns; Resume processes queued notifications in the same session.
8. Store attachment files on disk and metadata in SQLite. Keep standalone binary deployment. Repository management, automatic work pickup, Docker, PostgreSQL and S3 are outside this release.

## Implementation

1. Add relational board tables to the existing database using additive migrations. Board mutations, subscription updates, events and recipient notification rows share a transaction. Human read state is separate from worker delivery acceptance.
2. Expose authenticated board operations through `/api/board/{operation}`. Browser accounts use existing cookies and CSRF checks. Agents have distinct hashed bearer credentials restricted to board operations. The stdio MCP adapter translates tools to those same API operations.
3. Extend worker communication with optional agent capability advertisement and new `manager/agent/*` methods. Older workers continue to run standalone sessions and report an explicit upgrade requirement for agents.
4. Store agent identity/session binding and delivery journal in persistent private worker state. Never create a replacement session when a resume fails. Journal accepted notification IDs before replying to the server. Delivery is at least once at the small crash boundary between Codex acceptance and journal persistence; stable IDs let agents identify repeated events.
5. Serialize delivery, configuration and pause per agent. On idle delivery, resume the same session with current developer instructions and board MCP configuration. While busy, steer it; if completion races with steering, retry as a fresh turn. Persist pause before interrupting. Manual session controls respect pause and retain the agent's instructions/MCP.
6. Add Board, Agents and Inbox alongside existing Sessions using embedded HTML/CSS/JavaScript. Keep application state separated by view; preserve drafts when navigating.
7. Attachments are private drafts until linked by their owner to a post. Published attachments are readable by every board actor. Validate size/count, generate storage names, serve downloads with nosniff and safe content disposition, and allow inline previews only for selected image types.

## Implementation and verification plan

1. Schema and transactional domain API: actor identities, channels, topics/replies, subscriptions, events, inbox, attachments, agents.
2. Worker agent lifecycle and MCP adapter: import instructions, persist binding, configure/start/resume, deliver/steer, pause, idempotent delivery.
3. Server orchestration and authenticated API: capability detection, durable delivery pump, runtime error reporting, attachment serving.
4. Browser views: channel conversations and board, topic details/replies, assignments/status, subscriptions, attachments, agents/persona/session linking, pause and inbox.
5. Regression coverage: permissions, notification fan-out/resubscription, rollback/idempotency, attachment privacy, migration, delivery restart/pause/steer races, preserving instructions and standalone controls. Run Rust tests, fmt, Clippy and JS syntax checks. Exercise browser flows with Chrome DevTools and isolated fake Codex state; no production login or inference in automated tests.
6. Publish only after the combined checks pass. Retain previous server/database backups through the existing updater. Workers require an explicit binary upgrade for the new optional capability; existing sessions remain compatible.

## Verification completed for 0.3.0

1. Rust regression suites cover transactional subscription/mention/assignment delivery, private-to-published attachments, idempotent posts, pagination, worker revocation, browser/agent authentication separation, actual worker WebSocket delivery, actual stdio MCP posting, pause/steer/restart acceptance journals, persona retention and missing-rollout errors. Existing account, worker compatibility, session discovery and updater rollback suites remain enabled.
2. The installed Codex 0.165.0 smoke test uses a fresh Codex home without a developer login. It imports custom developer text from a real rollout and discovers all twelve board MCP tools after releasing/resuming the same session. No inference turn is requested; Codex may attempt an unauthenticated connection prewarm. This does not test a model's willingness or judgement when collaborating.
3. Chrome DevTools exercised login, topic creation with Enter, Shift+Enter line breaks, the hidden Discussion board phase, status/assignment changes, text attachment upload/posting, agent creation with persona/channel subscription, session registration importing existing instructions, pause state, and original session Enter-to-send without a page navigation. Fixtures used private isolated data and a fake Codex process. File-input tooling could not access the local workspace, so the attachment input was exercised with a browser-created test File instead.
4. Frontend JavaScript syntax, rustfmt and Clippy are checked locally and in CI. Browser drafts and upload references are cleared at logout; live topic refreshes retain the mounted reply/new-topic composers. This release keeps a small safe Markdown renderer and the embedded asset deployment; React/Vite remain an available future choice.

The v0.4 frontend replaces the original DOM implementation with React/Vite and adds account settings and Web Push; see [frontend-and-push.md](frontend-and-push.md).

Activity, cross-conversation search and private recurring reminders are added in v0.5; see [activity-search-reminders.md](activity-search-reminders.md).
