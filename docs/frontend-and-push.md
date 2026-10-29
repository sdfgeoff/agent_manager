# Unified React frontend and Web Push

## Scope and layout

1. Keep one persistent application shell: Topics, Sessions, Agents, Inbox and Settings. Channel navigation is shared. Threads and Kanban use the same URL-backed topic selection, search, status and assignee filters. Returning from another section restores the topic context.
2. Keep sessions independent of board registration. Agent chips connect a registered session to its persona settings; agent cards link back to the underlying session. Retain working-folder browsing, model/permission controls, approvals, streaming, steering and earlier history.
3. Add keyboard-accessible mention completion to topic/reply composers. Enter selects a mention before it submits; Shift+Enter adds a line. Rendering uses React Markdown without raw HTML. Published names open person details or agent/session links.
4. Keep message drafts and staged attachments across navigation within a sign-in. Clear them on sign-out/account changes. Reject stale asynchronous authentication responses and stop subsequent mutation steps when the sign-in changes. Refetch session history on reconnect/visibility recovery while buffering live events.

## Build and deployment

1. React/Vite source is compiled during development or CI. A Cargo build script embeds all generated `web/dist` assets. The deployed Rust server serves the HTML, content-hashed JavaScript/CSS, manifest, icons and root-scoped service worker. No Node process is deployed.
2. Dependencies are locked with npm and Cargo. CI builds the frontend before Rust checks and before the musl release build. The existing signed release webhook, updater and database rollback remain in use. Worker protocol/session APIs remain compatible; workers need no upgrade for these UI and server notification features.

## Accounts

1. Human-cookie-authenticated, CSRF-protected account endpoints provide listing, create, remove and own-password change. Agent bearer credentials grant no access to them. User creation is atomic and never overwrites an existing password. Own-password change verifies the current password and guards against concurrent credential changes.
2. Password changes keep the requesting browser signed in through a rotated cookie, invalidate other sessions and delete old push registrations. Last-account removal remains prohibited. Account removal retains historical authorship, deactivates subscriptions, excludes the person from mention completion and prevents new assignments.

## Push lifecycle and privacy

1. An explicit user action requests permission and creates a standard browser PushSubscription. The server stores the subscription against the human account and originating browser session. A persistent server VAPID key authenticates encrypted Web Push requests.
2. The existing durable human inbox is the delivery source. Each browser has an independent notification cursor. A bounded background sender coalesces unread events, retries with backoff and removes expired endpoints. Generation checks prevent late responses from deleting or acknowledging a newly replaced subscription. Read state and agent-runtime delivery remain independent.
3. The service worker handles push and notificationclick, with no fetch cache. Push text is generic, and click destinations are restricted to the application origin. Account checks happen when the application opens. Sign-out disables the session's registrations; changing/resetting a password revokes the account's registrations. Providers can deliver an already accepted message after revocation, so private message content never enters the payload.
4. Subscription endpoints are restricted to known browser push-provider domains on HTTPS/443, with no URL credentials, fragments or redirects. Browser keys are validated before encryption. No arbitrary server-side URL fetch is accepted through the subscription API.
5. This is Web Push support, not a guarantee of delivery when an OS suspends a browser or a user force-quits it. iPhone/iPad users must open the installed Home Screen app. Actual background delivery requires user opt-in and a real push-service registration; automated fixtures do not establish device-specific delivery.

## Verification

1. Rust tests cover account permissions/CSRF, duplicate-user safety, current-password validation, session invalidation, last-user protection, removed-user behaviour, push encryption/decryption, persistent keys, cursors, retries, expired endpoints, registration-generation races, logout and password revocation. Existing session, board, protocol compatibility and updater tests remain enabled.
2. Node service-worker tests cover generic payload handling, safe destinations, focus/open behaviour and absence of private fetch caching. Frontend build, TypeScript parsing and Prettier checks run in CI.
3. Chrome DevTools checks use private fixture data and a fake Codex runtime. Notification API fixtures are confined to the isolated test context, restored afterward, and never grant OS permission. No production credentials or model inference are used for browser tests.

4. Chrome checks cover mention selection and posting, shared thread/Kanban selection, retained drafts, session streaming/approval/agent registration, attachment upload/download, password changes, account removal and a delayed old-account 401 after signing into another account. Reconnect/visibility recovery refetches history without losing the draft. Narrow layouts were inspected in a 386px same-origin iframe because the DevTools network-emulation command failed; this is not physical-phone testing.
