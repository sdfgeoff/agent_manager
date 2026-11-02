import React, { useEffect, useRef, useState } from "react";
import { rpc, href } from "./api";
import { useApp } from "./context";
import { ErrorMessage } from "./components";

const count = (n) => (n || 0).toLocaleString();
export default function Usage({ params }) {
  const app = useApp();
  const worker = params.get("worker") || app.workers[0]?.id || "";
  const agent = params.get("agent") || "";
  const [data, setData] = useState(null);
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const selected = app.workers.find((w) => w.id === worker);
  async function load(more = false) {
    const seq = ++generation.current;
    setBusy(true);
    try {
      const result = await rpc(worker, "manager/usage", {
        ...(agent ? { agentId: agent } : {}),
        ...(more && data?.nextCursor ? { before: data.nextCursor } : {}),
      });
      if (seq !== generation.current) return;
      setData((old) => ({
        ...result,
        data: more ? [...(old?.data || []), ...result.data] : result.data,
      }));
      setError(null);
    } catch (e) {
      if (seq === generation.current) setError(e);
    } finally {
      if (seq === generation.current) setBusy(false);
    }
  }
  useEffect(() => {
    setData(null);
    setError(null);
    if (selected?.online) load();
    return () => {
      generation.current++;
    };
  }, [worker, agent, selected?.online]);
  const totals = data?.totals;
  return (
    <section className="page">
      <div className="page-heading">
        <div>
          <p className="eyebrow">OBSERVED TOKENS</p>
          <h1>Usage</h1>
          <p>
            Recorded per worker, agent and turn. Token counts are not billing
            charges.
          </p>
        </div>
        <button disabled={!selected?.online || busy} onClick={() => load()}>
          Refresh
        </button>
      </div>
      <div className="filters">
        <label>
          Worker{" "}
          <select
            aria-label="Usage worker"
            value={worker}
            onChange={(e) => {
              location.hash = href("usage", { worker: e.target.value });
            }}
          >
            {app.workers.map((w) => (
              <option key={w.id} value={w.id}>
                {w.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Agent{" "}
          <select
            aria-label="Usage agent"
            value={agent}
            onChange={(e) => {
              location.hash = href("usage", { worker, agent: e.target.value });
            }}
          >
            <option value="">All agents and other sessions</option>
            {app.agents
              .filter((a) => a.worker === worker)
              .map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))}
          </select>
        </label>
      </div>
      {!selected && <p>No workers registered.</p>}
      {selected && !selected.online && (
        <p>Connect this worker to read its saved usage.</p>
      )}
      <ErrorMessage error={error} />
      {error && (
        <p className="muted">
          Usage recording requires a worker with token-usage support. Older
          workers continue to work without it.
        </p>
      )}
      {totals && (
        <>
          <div className="worker-summary">
            {[
              ["Observed total", totals.totalTokens],
              ["Input", totals.inputTokens],
              ["Cached input (included in input)", totals.cachedInputTokens],
              ["Output", totals.outputTokens],
              ["Reasoning (included in output)", totals.reasoningOutputTokens],
              ["Turns with usage", totals.turns],
            ].map(([label, value]) => (
              <article className="summary-card" key={label}>
                <strong>{count(value)}</strong>
                <p>{label}</p>
              </article>
            ))}
          </div>
          <p className="muted">
            {totals.firstSeen
              ? "First usage observed " +
                new Date(totals.firstSeen * 1000).toLocaleString() +
                ". "
              : ""}
            {count(totals.partialTurns)} partial turns. Earlier usage is not
            backfilled. Records stay on the worker; back up its usage database.
            Refresh to update.
          </p>
          <h2>Turns</h2>
          {!data.data.length && (
            <p>
              No usage recorded yet. Counts appear when the worker receives
              Codex token updates.
            </p>
          )}
          <div className="activity-agents">
            {data.data.map((t) => (
              <article className="summary-card" key={t.id}>
                <h3>
                  {app.agents.find((a) => a.id === t.agentId)?.name ||
                    t.agentId ||
                    "Other session"}
                </h3>
                <p>
                  {count(t.tokens.totalTokens)} tokens · {t.status}
                  {t.partial ? " · Partial coverage" : ""}
                </p>
                <p className="muted">
                  Input {count(t.tokens.inputTokens)} · Cached{" "}
                  {count(t.tokens.cachedInputTokens)} · Output{" "}
                  {count(t.tokens.outputTokens)} · Reasoning{" "}
                  {count(t.tokens.reasoningOutputTokens)}
                </p>
                <p className="muted">
                  {new Date(t.updatedAt * 1000).toLocaleString()}
                  {t.configuredModel
                    ? " · Configured model: " + t.configuredModel
                    : ""}
                </p>
                <p className="muted">Turn {t.turnId}</p>
                <a href={href("sessions", { worker, thread: t.threadId })}>
                  Open session
                </a>
              </article>
            ))}
          </div>
          {data.nextCursor && (
            <button disabled={busy} onClick={() => load(true)}>
              Load older turns
            </button>
          )}
        </>
      )}
    </section>
  );
}
