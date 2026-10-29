import React, { useEffect, useRef, useState } from "react";
import {
  Bot,
  Plus,
  Pause,
  Play,
  MessageSquare,
  Settings2,
  ExternalLink,
  Folder,
  ArrowUp,
} from "lucide-react";
import { board, rpc, href, navigate, authGuard } from "./api";
import { useApp } from "./context";
import { Modal, ErrorMessage, BusyButton } from "./components";
export function FolderPicker({ worker, value, onChange }) {
  const [open, setOpen] = useState(false);
  const [path, setPath] = useState(value || "~");
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const [newFolder, setNewFolder] = useState("");
  const [creating, setCreating] = useState(false);
  async function browse(p) {
    try {
      const result = await rpc(worker, "fs/list", { path: p });
      setData(result);
      setPath(result.path);
      setError(null);
    } catch (e) {
      setError(e);
    }
  }
  return (
    <>
      <div className="input-row">
        <input
          aria-label="Working folder"
          value={value}
          onInput={(e) => onChange(e.currentTarget.value)}
          onChange={(e) => onChange(e.target.value)}
          required
        />
        <button
          type="button"
          onClick={() => {
            setOpen(true);
            browse(value || "~");
          }}
          disabled={!worker}
        >
          <Folder size={16} />
          Browse
        </button>
      </div>
      {open && (
        <Modal title="Choose a working folder" onClose={() => setOpen(false)}>
          <form
            className="input-row"
            onSubmit={(e) => {
              e.preventDefault();
              browse(path);
            }}
          >
            <input
              aria-label="Folder path"
              value={path}
              onChange={(e) => setPath(e.target.value)}
            />
            <button>Go</button>
          </form>
          <div className="actions folder-actions">
            <button
              type="button"
              disabled={!data?.parent}
              onClick={() => browse(data.parent)}
            >
              <ArrowUp size={15} />
              Parent
            </button>
            <button type="button" onClick={() => setCreating((c) => !c)}>
              <Plus size={15} />
              New folder
            </button>
          </div>
          {creating && (
            <form
              className="input-row"
              onSubmit={async (e) => {
                e.preventDefault();
                try {
                  const result = await rpc(worker, "fs/mkdir", {
                    path:
                      newFolder.startsWith("/") || newFolder.startsWith("~/")
                        ? newFolder
                        : `${data?.path || path}/${newFolder}`,
                  });
                  setNewFolder("");
                  setCreating(false);
                  await browse(result.path);
                } catch (e) {
                  setError(e);
                }
              }}
            >
              <input
                aria-label="New folder name"
                value={newFolder}
                onChange={(e) => setNewFolder(e.target.value)}
                required
              />
              <button>Create</button>
            </form>
          )}
          <ErrorMessage error={error} />
          <div className="folder-list">
            {data?.entries.map((f) => (
              <button type="button" key={f.path} onClick={() => browse(f.path)}>
                <Folder size={15} />
                {f.name}
              </button>
            ))}
            {data?.truncated && (
              <p className="muted">
                Some folders are omitted. Enter a path to navigate directly.
              </p>
            )}
          </div>
          <button
            type="button"
            className="primary wide"
            disabled={!data}
            onClick={() => {
              onChange(data.path);
              setOpen(false);
            }}
          >
            Use this folder
          </button>
        </Modal>
      )}
    </>
  );
}
export function CodexFields({ config, setConfig, models = [] }) {
  return (
    <>
      <label>
        Model
        <input
          list="available-models"
          value={config.model || ""}
          placeholder="Worker default"
          onChange={(e) => setConfig((c) => ({ ...c, model: e.target.value }))}
        />
        <datalist id="available-models">
          {models.map((m) => (
            <option key={m.id || m.model} value={m.model || m.id}>
              {m.displayName || m.model}
            </option>
          ))}
        </datalist>
      </label>
      <div className="form-columns">
        <label>
          Reasoning effort
          <select
            value={config.effort || ""}
            onChange={(e) =>
              setConfig((c) => ({ ...c, effort: e.target.value }))
            }
          >
            <option value="">Default</option>
            {["low", "medium", "high", "xhigh"].map((e) => (
              <option key={e}>{e}</option>
            ))}
          </select>
        </label>
        <label>
          Approval policy
          <select
            value={config.approvalPolicy || "on-request"}
            onChange={(e) =>
              setConfig((c) => ({ ...c, approvalPolicy: e.target.value }))
            }
          >
            <option value="on-request">When requested</option>
            <option value="untrusted">Untrusted commands</option>
            <option value="never">Never ask</option>
          </select>
        </label>
      </div>
      <label>
        Execution permissions
        <select
          value={config.sandbox || "workspace-write"}
          onChange={(e) =>
            setConfig((c) => ({ ...c, sandbox: e.target.value }))
          }
        >
          <option value="workspace-write">Write in working folder</option>
          <option value="read-only">Read only</option>
          <option value="danger-full-access">Full access as worker user</option>
        </select>
      </label>
    </>
  );
}
export function codexParams(config) {
  return {
    ...(config.model?.trim() ? { model: config.model.trim() } : {}),
    approvalPolicy: config.approvalPolicy || "on-request",
    sandbox: config.sandbox || "workspace-write",
    ...(config.effort
      ? { config: { model_reasoning_effort: config.effort } }
      : {}),
  };
}
export function AgentDialog({ agent = null, source = null, onClose }) {
  const app = useApp();
  const [name, setName] = useState(agent?.name || source?.name || "");
  const [handle, setHandle] = useState(agent?.handle || "");
  const [worker, setWorker] = useState(
    source?.worker ||
      agent?.worker ||
      app.workers.find((w) => w.online)?.id ||
      "",
  );
  const [cwd, setCwd] = useState("~");
  const [persona, setPersona] = useState(
    agent?.persona || source?.persona || "",
  );
  const [config, setConfig] = useState({
    approvalPolicy: "on-request",
    sandbox: "workspace-write",
  });
  const [models, setModels] = useState([]);
  const [subscriptions, setSubscriptions] = useState({});
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(!!agent);
  const requestId = useRef(crypto.randomUUID());
  const created = useRef(agent);
  useEffect(() => {
    if (agent) {
      board("agent_subscriptions", { id: agent.id })
        .then((r) =>
          setSubscriptions(
            Object.fromEntries(r.data.map((s) => [s.target, s.subscribed])),
          ),
        )
        .catch(setError)
        .finally(() => setLoading(false));
    }
  }, []);
  useEffect(() => {
    let current = true;
    if (worker)
      rpc(worker, "model/list", {})
        .then((r) => {
          if (current) setModels(r.data || []);
        })
        .catch(() => {});
    return () => (current = false);
  }, [worker]);
  async function save(e) {
    e.preventDefault();
    if (busy || loading) return;
    const stillSignedIn = authGuard();
    setBusy(true);
    setError(null);
    try {
      let result = created.current;
      if (result) {
        const r = await board("update_agent", { id: result.id, name, persona });
        if (!r.synchronized)
          app.report(
            new Error(
              "Saved. Changes will reach the worker when it reconnects.",
            ),
          );
      } else {
        result = await board("create_agent", {
          requestId: requestId.current,
          name,
          handle,
          worker,
          persona,
          ...(source?.thread
            ? { thread: source.thread }
            : { cwd, settings: codexParams(config) }),
        });
        created.current = result;
      }
      for (const channel of app.channels) {
        stillSignedIn();
        await board("set_subscription", {
          actor: result.id,
          kind: "channel",
          target: channel.id,
          subscribed: !!subscriptions[channel.id],
        });
      }
      stillSignedIn();
      await app.refresh();
      onClose();
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      title={
        agent
          ? "Edit agent"
          : source
            ? "Register session as an agent"
            : "Create an agent"
      }
      onClose={onClose}
    >
      <form onSubmit={save}>
        <p className="muted">
          One persona, one persistent session across all channels.
        </p>
        <div className="form-columns">
          <label>
            Name
            <input
              required
              maxLength={100}
              value={name}
              autoFocus
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label>
            Handle
            <input
              required
              disabled={!!agent || !!created.current}
              pattern={"[a-z0-9_\\-]{2,40}"}
              minLength={2}
              maxLength={40}
              value={handle}
              onChange={(e) => setHandle(e.target.value.toLowerCase())}
              placeholder="sage"
            />
          </label>
        </div>
        {!agent && !source && (
          <>
            <label>
              Worker
              <select
                value={worker}
                required
                onChange={(e) => setWorker(e.target.value)}
              >
                <option value="">Choose a machine</option>
                {app.workers.map((w) => (
                  <option value={w.id} key={w.id} disabled={!w.online}>
                    {w.name}
                    {w.online ? "" : " · offline"}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Working folder
              <FolderPicker worker={worker} value={cwd} onChange={setCwd} />
            </label>
            <CodexFields
              config={config}
              setConfig={setConfig}
              models={models}
            />
          </>
        )}
        {(source || agent) && (
          <p className="info">
            {source
              ? `Keeping the existing session and folder: ${source.cwd}`
              : `Worker: ${app.workers.find((w) => w.id === agent.worker)?.name || agent.worker}`}
          </p>
        )}
        <label>
          Persona / custom session instructions
          <textarea
            rows={8}
            value={persona}
            maxLength={100000}
            onChange={(e) => setPersona(e.target.value)}
          />
        </label>
        <p className="field-hint">
          Developer instructions applied between turns. They aren’t repeated in
          notifications.
        </p>
        <fieldset>
          <legend>Channel subscriptions · new topics</legend>
          {loading ? (
            <p>Loading subscriptions…</p>
          ) : (
            app.channels.map((c) => (
              <label className="check" key={c.id}>
                <input
                  type="checkbox"
                  checked={!!subscriptions[c.id]}
                  onChange={(e) =>
                    setSubscriptions((s) => ({
                      ...s,
                      [c.id]: e.target.checked,
                    }))
                  }
                />
                #{c.name}
              </label>
            ))
          )}
        </fieldset>
        <ErrorMessage error={error} />
        <BusyButton
          className="primary wide"
          type="submit"
          busy={busy}
          disabled={loading}
        >
          {agent ? "Save agent" : "Create agent"}
        </BusyButton>
      </form>
    </Modal>
  );
}
export default function Agents({ params }) {
  const app = useApp();
  const [editing, setEditing] = useState(undefined);
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(null);
  const opened = useRef(null);
  useEffect(() => {
    const id = params?.get("id");
    if (id && opened.current !== id) {
      const found = app.agents.find((a) => a.id === id);
      if (found) {
        opened.current = id;
        setEditing(found);
      }
    }
  }, [params?.get("id"), app.agents]);
  async function pause(a) {
    if (busy) return;
    setBusy(a.id);
    try {
      const r = await board("update_agent", { id: a.id, paused: !a.paused });
      if (!r.synchronized)
        app.report(
          new Error(
            "Change saved. The worker is unreachable; it will be applied when it reconnects.",
          ),
        );
      await app.refresh();
    } catch (e) {
      setError(e);
    } finally {
      setBusy(null);
    }
  }
  return (
    <section className="page">
      <div className="page-heading">
        <div>
          <p className="eyebrow">YOUR TEAMMATES</p>
          <h1>Agents</h1>
          <p>Persistent personas, connected to sessions on your machines.</p>
        </div>
        <button className="primary" onClick={() => setEditing(null)}>
          <Plus size={16} />
          Create agent
        </button>
      </div>
      <ErrorMessage error={error} />
      <div className="agent-grid">
        {app.agents.map((a) => (
          <article className="agent-card" key={a.id}>
            <div className="agent-heading">
              <span className="avatar">
                <Bot size={24} />
              </span>
              <div>
                <h2>{a.name}</h2>
                <span className="muted">@{a.handle}</span>
              </div>
              <span className={`badge ${a.paused ? "" : "online"}`}>
                {a.paused ? "Paused" : a.online ? "Online" : "Offline"}
              </span>
            </div>
            <p className="worker-caption">
              {app.workers.find((w) => w.id === a.worker)?.name || a.worker}
            </p>
            <p className="persona-preview">
              {a.persona || "No custom persona instructions."}
            </p>
            <ErrorMessage error={a.error ? new Error(a.error) : null} />
            <div className="agent-actions">
              <button onClick={() => setEditing(a)}>
                <Settings2 size={15} />
                Edit persona
              </button>
              <button disabled={busy === a.id} onClick={() => pause(a)}>
                {a.paused ? <Play size={15} /> : <Pause size={15} />}{" "}
                {a.paused ? "Resume" : "Pause"}
              </button>
              <a
                className="button"
                href={href("sessions", { worker: a.worker, thread: a.thread })}
              >
                <MessageSquare size={15} />
                Session
                <ExternalLink size={13} />
              </a>
            </div>
          </article>
        ))}
      </div>
      {!app.agents.length && (
        <div className="empty">
          <Bot size={32} />
          <h2>Your agents will appear here</h2>
          <p>
            Create a new persona, or register an existing session without losing
            its history.
          </p>
        </div>
      )}
      {editing !== undefined && (
        <AgentDialog
          agent={editing}
          onClose={() => {
            setEditing(undefined);
            if (params?.get("id")) navigate("agents");
          }}
        />
      )}
    </section>
  );
}
