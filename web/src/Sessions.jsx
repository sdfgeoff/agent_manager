import React, { useEffect, useRef, useState } from "react";
import {
  Plus,
  RefreshCw,
  Settings2,
  Send,
  Square,
  Bot,
  Terminal,
  Link2,
  Copy,
  Folder,
  MoreHorizontal,
  ArrowLeft,
} from "lucide-react";
import { api, rpc, href, navigate, events, authGuard } from "./api";
import { useApp, useDraft } from "./context";
import { Modal, ErrorMessage, RichText, BusyButton } from "./components";
import { AgentDialog, FolderPicker, CodexFields, codexParams } from "./Agents";
export default function Sessions({ params }) {
  const app = useApp();
  const worker =
    params.get("worker") ||
    app.workers.find((w) => w.online)?.id ||
    app.workers[0]?.id;
  const threadId = params.get("thread");
  const searchText = params.get("search") || "";
  const [searchPages, setSearchPages] = useState(0);
  const [searchingHistory, setSearchingHistory] = useState(false);
  const matched = itemsMatch;
  function itemsMatch(item) {
    if (!searchText || !["userMessage", "agentMessage"].includes(item.type))
      return false;
    const text = (
      item.text || (item.content || []).map((c) => c.text || "").join(" ")
    ).toLowerCase();
    return searchText
      .toLowerCase()
      .split(/\s+/)
      .every((word) => text.includes(word));
  }
  const selectedWorker = app.workers.find((w) => w.id === worker);
  const agent = app.agents.find(
    (a) => a.worker === worker && a.thread === threadId,
  );
  const [threads, setThreads] = useState([]);
  const [cursor, setCursor] = useState(null);
  const [thread, setThread] = useState(null);
  const [items, setItems] = useState([]);
  const [older, setOlder] = useState(null);
  const [status, setStatus] = useState({ active: {}, requests: [] });
  const [error, setError] = useState(null);
  const [loading, setLoading] = useState(false);
  const [sessionForm, setSessionForm] = useState(undefined);
  const [agentForm, setAgentForm] = useState(undefined);
  const [enrollment, setEnrollment] = useState(null);
  const [text, setText] = useDraft(`session:${worker}/${threadId}`);
  const [busy, setBusy] = useState(false);
  const sending = useRef(false);
  const epoch = useRef(0);
  const listEpoch = useRef(0);
  const scroller = useRef(null);
  const bottom = useRef(null);
  const syncing = useRef(false);
  const buffer = useRef([]);
  const active = status.active?.[threadId];
  async function list(more = false) {
    if (!worker || !selectedWorker?.online) {
      setThreads([]);
      return;
    }
    const version = ++listEpoch.current;
    try {
      const result = await rpc(worker, "manager/list", {
        limit: 50,
        ...(more && cursor ? { cursor } : {}),
      });
      if (version !== listEpoch.current) return;
      setThreads((old) =>
        [
          ...new Map(
            (more ? [...old, ...result.data] : result.data).map((t) => [
              t.id,
              t,
            ]),
          ).values(),
        ].sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0)),
      );
      setCursor(result.nextCursor);
    } catch (e) {
      if (version === listEpoch.current) setError(e);
    }
  }
  async function loadStatus() {
    if (!worker || !selectedWorker?.online) return;
    const current = epoch.current;
    try {
      const result = await rpc(worker, "manager/status");
      if (current === epoch.current) setStatus(result);
    } catch (e) {
      if (current === epoch.current) setError(e);
    }
  }
  function scroll(force = false) {
    const node = scroller.current;
    if (
      node &&
      (force || node.scrollHeight - node.scrollTop - node.clientHeight < 240)
    )
      requestAnimationFrame(() =>
        bottom.current?.scrollIntoView({ block: "end" }),
      );
  }
  function incoming(message) {
    const p = message.params || {};
    if (p.threadId && p.threadId !== threadId) return;
    if (
      message.method === "item/completed" ||
      message.method === "item/started"
    ) {
      if (p.item?.id) {
        setItems((old) => {
          const i = old.findIndex((x) => x.id === p.item.id);
          return i < 0
            ? [...old, p.item]
            : old.map((x, j) => (j === i ? p.item : x));
        });
        scroll();
      }
    }
    if (message.method === "item/agentMessage/delta") {
      setItems((old) => {
        const i = old.findIndex((x) => x.id === p.itemId);
        if (i < 0)
          return [
            ...old,
            { id: p.itemId, type: "agentMessage", text: p.delta || "" },
          ];
        return old.map((x, j) =>
          j === i ? { ...x, text: (x.text || "") + (p.delta || "") } : x,
        );
      });
      scroll();
    }
    if (
      ["turn/started", "turn/completed", "manager/requestsChanged"].includes(
        message.method,
      ) ||
      message.id !== undefined
    )
      loadStatus();
  }
  useEffect(() => {
    list();
    loadStatus();
    return () => listEpoch.current++;
  }, [worker, selectedWorker?.online]);
  async function refreshHistory(initial = false) {
    if (!threadId || !selectedWorker?.online || (!initial && syncing.current))
      return;
    const current = epoch.current;
    syncing.current = true;
    buffer.current = [];
    if (initial) setLoading(true);
    try {
      const [read, history] = await Promise.all([
        rpc(worker, "thread/read", { threadId }),
        rpc(worker, "thread/turns/list", {
          threadId,
          itemsView: "full",
          sortDirection: "desc",
          limit: 20,
        }),
      ]);
      if (epoch.current !== current) return;
      setThread(read.thread);
      const loaded = [...history.data].reverse().flatMap((t) => t.items || []);
      setItems((old) =>
        initial
          ? loaded
          : [
              ...new Map(
                [...old, ...loaded].map((item) => [item.id, item]),
              ).values(),
            ],
      );
      if (initial) setOlder(history.nextCursor);
      syncing.current = false;
      for (const message of buffer.current) incoming(message);
      buffer.current = [];
      await loadStatus();
      if (!searchText) scroll(initial);
    } catch (e) {
      if (epoch.current === current) {
        syncing.current = false;
        setError(e);
      }
    } finally {
      if (epoch.current === current) setLoading(false);
    }
  }
  useEffect(() => {
    epoch.current++;
    setSearchPages(0);
    setSearchingHistory(false);
    setThread(null);
    setItems([]);
    setOlder(null);
    setError(null);
    syncing.current = false;
    buffer.current = [];
    refreshHistory(true);
    return () => epoch.current++;
  }, [worker, threadId, selectedWorker?.online]);
  useEffect(() => {
    const handler = (event) => {
      const data = event.detail;
      if (data.type === "codex" && data.worker === worker) {
        const message = data.message;
        if (
          [
            "manager/sessionsChanged",
            "manager/resync",
            "manager/codexExited",
          ].includes(message.method)
        ) {
          list();
          loadStatus();
          if (message.method === "manager/resync") refreshHistory();
        } else if (syncing.current) buffer.current.push(message);
        else incoming(message);
      }
      if (data.type === "reconnected") {
        list();
        loadStatus();
        refreshHistory();
      }
    };
    events.addEventListener("server", handler);
    return () => events.removeEventListener("server", handler);
  }, [worker, threadId, selectedWorker?.online]);
  async function send() {
    if (sending.current || !text.trim() || !threadId) return;
    sending.current = true;
    setBusy(true);
    setError(null);
    const sent = text;
    const stillSignedIn = authGuard();
    try {
      const input = [{ type: "text", text: sent.trim() }];
      if (active)
        await rpc(worker, "turn/steer", {
          threadId,
          expectedTurnId: active,
          input,
        });
      else {
        await rpc(worker, "thread/resume", { threadId, excludeTurns: true });
        stillSignedIn();
        const result = await rpc(worker, "turn/start", { threadId, input });
        setStatus((s) => ({
          ...s,
          active: { ...s.active, [threadId]: result.turn.id },
        }));
      }
      setText((old) => (old === sent ? "" : old));
      await loadStatus();
      scroll(true);
    } catch (e) {
      setError(e);
    } finally {
      sending.current = false;
      setBusy(false);
    }
  }
  async function stop() {
    try {
      await rpc(worker, "turn/interrupt", { threadId, turnId: active });
      await loadStatus();
    } catch (e) {
      setError(e);
    }
  }
  async function settings() {
    try {
      if (active)
        throw new Error(
          "Stop or wait for the active turn before editing settings.",
        );
      const result = await rpc(worker, "thread/resume", {
        threadId,
        excludeTurns: true,
      });
      let instructions = null;
      try {
        instructions = (await rpc(worker, "manager/instructions", { threadId }))
          .persona;
      } catch (e) {
        if (!e.message.includes("Unsupported method")) throw e;
      }
      setSessionForm({ thread: result.thread, ...result, instructions });
    } catch (e) {
      setError(e);
    }
  }
  async function registerAgent() {
    try {
      if (agent) {
        setAgentForm({ agent });
        return;
      }
      if (active)
        throw new Error(
          "Stop or wait for the active turn before registering this session.",
        );
      const instructions = await rpc(worker, "manager/instructions", {
        threadId,
      });
      setAgentForm({
        source: {
          thread: threadId,
          worker,
          name: thread?.name || "",
          cwd: thread?.cwd,
          persona: instructions.persona,
        },
      });
    } catch (e) {
      setError(e);
    }
  }
  const searchHit = items.find(matched);
  useEffect(() => {
    if (!searchText || loading || searchingHistory || !thread) return;
    if (searchHit) {
      document
        .getElementById("session-item-" + searchHit.id)
        ?.scrollIntoView({ block: "center" });
    } else if (older && searchPages < 10) {
      setSearchPages((n) => n + 1);
      oldMessages(true);
    }
  }, [
    searchText,
    loading,
    thread?.id,
    older,
    searchHit?.id,
    searchingHistory,
    searchPages,
  ]);
  async function oldMessages(searching = false) {
    if (searching) setSearchingHistory(true);
    const current = epoch.current;
    const node = scroller.current;
    const height = node?.scrollHeight;
    try {
      const page = await rpc(worker, "thread/turns/list", {
        threadId,
        cursor: older,
        itemsView: "full",
        sortDirection: "desc",
        limit: 20,
      });
      if (current !== epoch.current) return;
      setItems((old) => [
        ...new Map(
          [...page.data]
            .reverse()
            .flatMap((t) => t.items || [])
            .concat(old)
            .map((x) => [x.id, x]),
        ).values(),
      ]);
      setOlder(page.nextCursor);
      requestAnimationFrame(() => {
        if (!searching && node && height)
          node.scrollTop += node.scrollHeight - height;
      });
    } catch (e) {
      setError(e);
    } finally {
      if (current === epoch.current && searching) setSearchingHistory(false);
    }
  }
  return (
    <section className="sessions-workspace">
      <div className="session-browser">
        <div className="session-browser-heading">
          <h2>Sessions</h2>
          <button
            className="icon"
            aria-label="Refresh sessions"
            onClick={() => {
              list();
              loadStatus();
              app.refresh();
            }}
          >
            <RefreshCw size={16} />
          </button>
        </div>
        <label className="small-label">
          MACHINE
          <select
            aria-label="Worker machine"
            value={worker || ""}
            onChange={(e) => navigate("sessions", { worker: e.target.value })}
          >
            {app.workers.map((w) => (
              <option key={w.id} value={w.id}>
                {w.online ? "●" : "○"} {w.name} ·{" "}
                {w.workerVersion ? "v" + w.workerVersion : "Version unknown"}
              </option>
            ))}
          </select>
        </label>
        <div className="worker-tools">
          <button
            className="text-button"
            onClick={async () => {
              try {
                setEnrollment(await api("/api/enrollment", {}));
              } catch (e) {
                setError(e);
              }
            }}
          >
            <Link2 size={13} />
            Register worker
          </button>
          {selectedWorker && (
            <button
              className="text-button danger"
              onClick={async () => {
                if (
                  !confirm(
                    `Revoke ${selectedWorker.name}? It will need a new registration key.`,
                  )
                )
                  return;
                try {
                  await api(`/api/workers/${worker}`, undefined, "DELETE");
                  app.refresh();
                } catch (e) {
                  setError(e);
                }
              }}
            >
              Revoke
            </button>
          )}
        </div>
        <button
          className="primary wide"
          disabled={!selectedWorker?.online}
          onClick={() => setSessionForm(null)}
        >
          <Plus size={15} />
          New session
        </button>
        <p className="eyebrow session-list-label">ALL FOLDERS</p>
        <div className="session-list">
          {threads.map((t) => {
            const bound = app.agents.find(
              (a) => a.worker === worker && a.thread === t.id,
            );
            return (
              <a
                href={href("sessions", { worker, thread: t.id })}
                key={t.id}
                className={t.id === threadId ? "selected" : ""}
              >
                <span>
                  {bound && <Bot size={13} />}{" "}
                  {t.name || t.preview || "Untitled session"}
                </span>
                <small>
                  {bound ? `@${bound.handle} · ` : ""}
                  {status.active?.[t.id] ? "Working · " : ""}
                  {t.cwd}
                </small>
              </a>
            );
          })}
          {!threads.length && (
            <p className="muted">
              {selectedWorker?.online
                ? "No saved sessions yet."
                : "Choose an online worker."}
            </p>
          )}
        </div>
        {cursor && (
          <button onClick={() => list(true)}>Load more sessions</button>
        )}
      </div>
      <div className="session-chat">
        <div className="session-heading-bar">
          {threadId && (
            <a
              className="icon button"
              aria-label="Back to session list"
              href={href("sessions", { worker })}
            >
              <ArrowLeft size={18} />
            </a>
          )}
          <div>
            <h1>{thread?.name || thread?.preview || "Your workspace"}</h1>
            <p>{thread?.cwd || selectedWorker?.name || "Choose a worker"}</p>
          </div>
          {agent && (
            <a className="agent-chip" href={href("agents", { id: agent.id })}>
              <Bot size={14} />@{agent.handle}
            </a>
          )}
          {threadId && (
            <>
              <span className="badge">
                {agent?.paused ? "Paused" : active ? "Working" : "Ready"}
              </span>
              <button
                className="icon"
                title={
                  agent
                    ? "Edit agent persona"
                    : "Register this session as an agent"
                }
                aria-label="Agent persona"
                onClick={registerAgent}
              >
                <Bot size={19} />
              </button>
              <button
                className="icon"
                aria-label="Session settings"
                onClick={settings}
              >
                <Settings2 size={18} />
              </button>
            </>
          )}
        </div>
        <ErrorMessage error={error} />
        {!threadId ? (
          <div className="empty grow">
            <Terminal size={40} />
            <h2>A session on any machine.</h2>
            <p>
              Work directly here, or give a session a persona and bring it into
              the shared board.
            </p>
            <button
              className="primary"
              disabled={!selectedWorker?.online}
              onClick={() => setSessionForm(null)}
            >
              <Plus size={16} />
              Create a session
            </button>
          </div>
        ) : (
          <>
            {searchText && (
              <div className="session-search-notice">
                Search: <strong>{searchText}</strong> ·{" "}
                {searchHit ? (
                  "Matching message highlighted"
                ) : searchingHistory ? (
                  "Searching earlier messages…"
                ) : older ? (
                  <button onClick={() => setSearchPages(0)}>
                    Search further back
                  </button>
                ) : (
                  "No matching message in available history"
                )}
              </div>
            )}
            <div className="chat-history" ref={scroller}>
              {loading && <p className="muted">Loading session…</p>}
              {older && (
                <button className="load-more" onClick={() => oldMessages()}>
                  Load earlier messages
                </button>
              )}
              {items.map((item) => (
                <div
                  id={"session-item-" + item.id}
                  className={matched(item) ? "search-focus" : ""}
                  key={item.id}
                >
                  <SessionItem item={item} />
                </div>
              ))}
              {status.requests
                ?.filter((r) => r.params?.threadId === threadId)
                .map((request) => (
                  <Request
                    key={String(request.id)}
                    request={request}
                    worker={worker}
                    onAnswered={loadStatus}
                  />
                ))}
              <div ref={bottom} />
            </div>
            <form
              className="session-composer"
              onSubmit={(e) => {
                e.preventDefault();
                send();
              }}
            >
              <textarea
                aria-label="Message"
                value={text}
                placeholder={
                  agent?.paused
                    ? "Agent paused · resume it from Agents to continue"
                    : "What would you like to work on?"
                }
                rows={3}
                maxLength={200000}
                onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => {
                  if (
                    e.key === "Enter" &&
                    !e.shiftKey &&
                    !e.nativeEvent.isComposing &&
                    e.keyCode !== 229
                  ) {
                    e.preventDefault();
                    send();
                  }
                }}
              />
              <div className="composer-toolbar">
                <span className="muted">
                  Enter to send · Shift+Enter for a new line
                </span>
                <div>
                  {active && (
                    <button type="button" className="danger" onClick={stop}>
                      <Square size={13} />
                      Stop
                    </button>
                  )}
                  <BusyButton
                    className="primary"
                    type="submit"
                    busy={busy}
                    disabled={!selectedWorker?.online || agent?.paused}
                  >
                    <Send size={15} />
                    {active ? "Steer" : "Send"}
                  </BusyButton>
                </div>
              </div>
            </form>
          </>
        )}
      </div>
      {sessionForm !== undefined && (
        <SessionDialog
          worker={worker}
          existing={sessionForm}
          onClose={() => setSessionForm(undefined)}
          onSaved={(t) => {
            setSessionForm(undefined);
            list();
            app.refresh();
            navigate("sessions", { worker, thread: t.id });
          }}
        />
      )}
      {agentForm !== undefined && (
        <AgentDialog
          {...agentForm}
          onClose={() => {
            setAgentForm(undefined);
            app.refresh();
          }}
        />
      )}
      {enrollment && (
        <Modal title="Register a worker" onClose={() => setEnrollment(null)}>
          <p>
            Run this on the machine you want to connect. Codex must be installed
            and signed in.
          </p>
          <pre>{`worker --host ${enrollment.host} --key ${enrollment.key}`}</pre>
          <button
            onClick={() =>
              navigator.clipboard.writeText(
                `worker --host ${enrollment.host} --key ${enrollment.key}`,
              )
            }
          >
            <Copy size={15} />
            Copy command
          </button>
          <p className="muted">
            Single use; expires in ten minutes. Worker credentials survive
            restarts.
          </p>
        </Modal>
      )}
    </section>
  );
}
function SessionItem({ item }) {
  if (["userMessage", "agentMessage"].includes(item.type)) {
    const text =
      item.type === "agentMessage"
        ? item.text || ""
        : (item.content || []).map((c) => c.text || c.type).join("\n");
    return (
      <article
        className={`chat-message ${item.type === "userMessage" ? "user" : "assistant"}`}
      >
        <div className="message-label">
          {item.type === "userMessage" ? "You" : "Codex"}
        </div>
        <RichText text={text} />
      </article>
    );
  }
  const title =
    item.type === "commandExecution"
      ? `$ ${item.command || "Command"} · ${item.status || ""}`
      : item.type === "fileChange"
        ? "File changes"
        : item.type === "reasoning"
          ? "Reasoning"
          : item.type;
  return (
    <details className="tool-output">
      <summary>{title}</summary>
      <pre>
        {item.aggregatedOutput ||
          item.output ||
          (Array.isArray(item.summary)
            ? item.summary
                .map((s) =>
                  typeof s === "string" ? s : s.text || JSON.stringify(s),
                )
                .join("\n")
            : JSON.stringify(item, null, 2))}
      </pre>
    </details>
  );
}
function Request({ request, worker, onAnswered }) {
  const p = request.params || {};
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [answers, setAnswers] = useState({});
  const [raw, setRaw] = useState("{}");
  async function answer(result) {
    if (busy) return;
    setBusy(true);
    try {
      await rpc(worker, "manager/respond", { requestId: request.id, result });
      onAnswered();
    } catch (e) {
      setError(e);
      setBusy(false);
    }
  }
  let actions;
  if (
    request.method === "item/commandExecution/requestApproval" ||
    request.method === "item/fileChange/requestApproval"
  )
    actions = (
      <div className="actions">
        {[
          ["Approve once", "accept"],
          ["Approve for session", "acceptForSession"],
          ["Decline", "decline"],
          ["Cancel", "cancel"],
        ].map(([text, decision]) => (
          <button
            key={decision}
            disabled={busy}
            onClick={() => answer({ decision })}
          >
            {text}
          </button>
        ))}
      </div>
    );
  else if (request.method === "item/tool/requestUserInput")
    actions = (
      <form
        onSubmit={(e) => {
          e.preventDefault();
          answer({
            answers: Object.fromEntries(
              (p.questions || []).map((q) => [
                q.id,
                { answers: [answers[q.id] || ""] },
              ]),
            ),
          });
        }}
      >
        {(p.questions || []).map((q) => (
          <label key={q.id}>
            {q.question}
            {q.options?.length ? (
              <select
                value={answers[q.id] || ""}
                onChange={(e) =>
                  setAnswers((a) => ({ ...a, [q.id]: e.target.value }))
                }
                required
              >
                <option value="">Choose an answer</option>
                {q.options.map((o) => (
                  <option key={o.label} value={o.label}>
                    {o.label} · {o.description}
                  </option>
                ))}
              </select>
            ) : (
              <input
                value={answers[q.id] || ""}
                onChange={(e) =>
                  setAnswers((a) => ({ ...a, [q.id]: e.target.value }))
                }
                required
              />
            )}
          </label>
        ))}
        <button disabled={busy}>Answer</button>
      </form>
    );
  else
    actions = (
      <form
        onSubmit={(e) => {
          e.preventDefault();
          try {
            answer(JSON.parse(raw));
          } catch {
            setError(new Error("Enter a valid JSON response."));
          }
        }}
      >
        <label>
          JSON response
          <textarea value={raw} onChange={(e) => setRaw(e.target.value)} />
        </label>
        <button disabled={busy}>Send response</button>
      </form>
    );
  return (
    <section className="approval">
      <h3>Your input is needed</h3>
      <pre>{p.command || p.reason || JSON.stringify(p, null, 2)}</pre>
      <ErrorMessage error={error} />
      {actions}
    </section>
  );
}
function SessionDialog({ worker, existing, onClose, onSaved }) {
  const [name, setName] = useState(existing?.thread?.name || "");
  const [cwd, setCwd] = useState(existing?.thread?.cwd || "~");
  const [instructions, setInstructions] = useState(
    existing?.instructions || "",
  );
  const [config, setConfig] = useState({
    model: existing?.model || "",
    effort: existing?.reasoningEffort || "",
    approvalPolicy: existing?.approvalPolicy || "on-request",
    sandbox:
      existing?.sandbox?.type === "readOnly"
        ? "read-only"
        : existing?.sandbox?.type === "dangerFullAccess"
          ? "danger-full-access"
          : "workspace-write",
  });
  const [models, setModels] = useState([]);
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    rpc(worker, "model/list")
      .then((r) => setModels(r.data || []))
      .catch(() => {});
  }, [worker]);
  return (
    <Modal
      title={existing ? "Session settings" : "New session"}
      onClose={onClose}
    >
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          if (busy) return;
          setBusy(true);
          try {
            const settings = codexParams(config);
            let result;
            if (existing) {
              result = await rpc(worker, "thread/resume", {
                ...settings,
                threadId: existing.thread.id,
                excludeTurns: true,
                ...(existing.instructions !== null
                  ? { developerInstructions: instructions }
                  : {}),
              });
            } else {
              const folder = await rpc(worker, "fs/list", { path: cwd });
              result = await rpc(worker, "thread/start", {
                ...settings,
                cwd: folder.path,
                developerInstructions: instructions,
              });
            }
            if (name.trim()) {
              await rpc(worker, "thread/name/set", {
                threadId: result.thread.id,
                name: name.trim(),
              });
              result.thread.name = name.trim();
            }
            onSaved(result.thread);
          } catch (e) {
            setError(e);
          } finally {
            setBusy(false);
          }
        }}
      >
        <label>
          Name
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={200}
            autoFocus
          />
        </label>
        <label>
          Working folder
          {existing ? (
            <input value={cwd} disabled />
          ) : (
            <FolderPicker worker={worker} value={cwd} onChange={setCwd} />
          )}
        </label>
        <CodexFields config={config} setConfig={setConfig} models={models} />
        <label>
          Persona / custom session instructions
          <textarea
            value={instructions}
            rows={6}
            maxLength={100000}
            onChange={(e) => setInstructions(e.target.value)}
            disabled={existing?.instructions === null}
          />
        </label>
        <ErrorMessage error={error} />
        <BusyButton className="primary wide" busy={busy} type="submit">
          {existing ? "Save settings" : "Create session"}
        </BusyButton>
      </form>
    </Modal>
  );
}
