import React, { useEffect, useRef, useState } from "react";
import { Activity as Pulse, RefreshCw, Terminal, Bot } from "lucide-react";
import { board, rpc, href, events } from "./api";
import { useApp } from "./context";
import { ErrorMessage } from "./components";

export default function Activity() {
  const app = useApp();
  const [data, setData] = useState({ agents: [], topics: [] });
  const [workers, setWorkers] = useState({});
  const [error, setError] = useState(null);
  const sequence = useRef(0);
  const loading = useRef(false);
  async function load() {
    if (loading.current || document.hidden) return;
    loading.current = true;
    const seq = sequence.current;
    try {
      const details = await board("agent_activity");
      if (seq !== sequence.current) return;
      setData(details);
      await Promise.all(
        app.workers.map(async (worker) => {
          let value;
          if (!worker.online) value = { offline: true };
          else
            try {
              value = await rpc(worker.id, "manager/status");
            } catch (e) {
              value = { error: e.message };
            }
          if (seq === sequence.current)
            setWorkers((old) => ({ ...old, [worker.id]: value }));
        }),
      );
      if (seq === sequence.current) setError(null);
    } catch (e) {
      if (seq === sequence.current) setError(e);
    } finally {
      loading.current = false;
    }
  }
  useEffect(() => {
    load();
    const timer = setInterval(load, 5000);
    const listener = (event) => {
      const data = event.detail;
      if (
        !data ||
        data.type === "reconnected" ||
        (data.type === "codex" &&
          [
            "turn/started",
            "turn/completed",
            "manager/requestsChanged",
            "manager/codexExited",
          ].includes(data.message?.method))
      )
        load();
    };
    events.addEventListener("server", listener);
    document.addEventListener("visibilitychange", listener);
    return () => {
      sequence.current++;
      clearInterval(timer);
      events.removeEventListener("server", listener);
      document.removeEventListener("visibilitychange", listener);
    };
  }, [app.workers.map((w) => w.id + w.online).join(","), app.revision]);
  function state(a) {
    const w = app.workers.find((w) => w.id === a.worker);
    const status = workers[a.worker];
    if (a.paused) return "Paused";
    if (!w?.online) return "Disconnected";
    if (a.error) return "Needs attention";
    if (!status || status.error || status.alive === false)
      return "Status unavailable";
    if (status.requests?.some((r) => r.params?.threadId === a.thread))
      return "Waiting for input";
    return status.active?.[a.thread] ? "Working" : "Ready";
  }
  return (
    <section className="page activity-page">
      <div className="page-heading">
        <div>
          <p className="eyebrow">ACROSS YOUR MACHINES</p>
          <h1>Activity</h1>
          <p>
            Agent status and assigned topics. Updates every five seconds while
            visible.
          </p>
        </div>
        <button onClick={load}>
          <RefreshCw size={16} />
          Refresh
        </button>
      </div>
      <ErrorMessage error={error} />
      <div className="worker-summary">
        {app.workers.map((w) => {
          const s = workers[w.id];
          const standalone = Object.keys(s?.active || {}).filter(
            (t) => !app.agents.some((a) => a.worker === w.id && a.thread === t),
          );
          return (
            <article className="summary-card" key={w.id}>
              <div className="card-meta">
                <Terminal size={18} />
                <strong>{w.name}</strong>
                <span className={w.online ? "online-text" : "muted"}>
                  {w.online ? "Connected" : "Disconnected"}
                </span>
              </div>
              <small>Worker {w.workerVersion || "version unknown"}</small>
              {s?.error && <p className="error">{s.error}</p>}
              {w.online && s && !s.error && (
                <p>
                  {Object.keys(s.active || {}).length} active sessions ·{" "}
                  {s.requests?.length || 0} waiting requests
                </p>
              )}
              <a href={href("sessions", { worker: w.id })}>Browse sessions</a>
              {standalone.map((t) => (
                <p key={t}>
                  <a href={href("sessions", { worker: w.id, thread: t })}>
                    Working session {t.slice(0, 12)}
                  </a>
                </p>
              ))}
              {(s?.requests || [])
                .filter(
                  (r) =>
                    !s.active?.[r.params?.threadId] &&
                    !app.agents.some(
                      (a) =>
                        a.worker === w.id && a.thread === r.params?.threadId,
                    ),
                )
                .map((r, i) => (
                  <p key={i}>
                    <a
                      href={href("sessions", {
                        worker: w.id,
                        thread: r.params?.threadId,
                      })}
                    >
                      Session waiting for input
                    </a>
                  </p>
                ))}
            </article>
          );
        })}
      </div>
      <h2>Agents</h2>
      <div className="activity-agents">
        {data.agents.map((a) => {
          const agent = app.agents.find((g) => g.id === a.id);
          const status = state(a);
          const topics = data.topics.filter((t) => t.assignee === a.id);
          return (
            <article className="summary-card" key={a.id}>
              <div className="card-meta">
                <Bot size={20} />
                <h3>{agent?.name || a.id}</h3>
                <span
                  className={
                    "activity-state state-" +
                    status.toLowerCase().replaceAll(" ", "-")
                  }
                >
                  {status}
                </span>
              </div>
              <p className="muted">
                {app.workers.find((w) => w.id === a.worker)?.name} · {a.queued}{" "}
                queued notifications
              </p>
              {a.error && <p className="error">{a.error}</p>}
              <div className="row-actions">
                <a
                  className="button"
                  href={href("sessions", {
                    worker: a.worker,
                    thread: a.thread,
                  })}
                >
                  Open session
                </a>
                <a className="button" href={href("agents", { id: a.id })}>
                  Persona & controls
                </a>
              </div>
              <h4>Assigned topics</h4>
              {topics.length ? (
                <ol className="assigned-topics">
                  {topics.map((t) => (
                    <li key={t.id}>
                      <a
                        href={href("topics", {
                          channel: t.channel,
                          topic: t.id,
                        })}
                      >
                        {t.title}
                      </a>
                      <small>{t.status}</small>
                    </li>
                  ))}
                </ol>
              ) : (
                <p className="muted">No open assigned topics</p>
              )}
            </article>
          );
        })}
      </div>
      {!data.agents.length && (
        <div className="empty">
          <Pulse size={32} />
          <h2>No agents yet</h2>
          <p>Register a session as an agent to see its activity here.</p>
          <a href={href("agents")}>Manage agents</a>
        </div>
      )}
    </section>
  );
}
