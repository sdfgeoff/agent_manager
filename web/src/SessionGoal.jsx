import React, { useEffect, useRef, useState } from "react";
import { rpc, events } from "./api";
import { ErrorMessage, BusyButton } from "./components";

const labels = {
  active: "Active",
  paused: "Paused",
  blocked: "Blocked",
  usageLimited: "Usage limit reached",
  budgetLimited: "Budget reached",
  complete: "Complete",
};
export default function SessionGoal({
  worker,
  threadId,
  agentPaused,
  onChanged,
}) {
  const [goal, setGoal] = useState(undefined);
  const [error, setError] = useState(null);
  const [editing, setEditing] = useState(false);
  const [objective, setObjective] = useState("");
  const [budget, setBudget] = useState("");
  const [busy, setBusy] = useState(false);
  const [unsupported, setUnsupported] = useState(false);
  const generation = useRef(0);
  const pending = useRef(false);
  const mutating = useRef(false);
  const revision = useRef(0);
  const supported = worker?.capabilities?.includes("session-goals");
  const readonly = !worker?.online || busy;
  async function load() {
    if (
      !worker?.online ||
      !supported ||
      pending.current ||
      mutating.current ||
      document.hidden
    )
      return;
    pending.current = true;
    const epoch = generation.current;
    const current = revision.current;
    try {
      const result = await rpc(worker.id, "thread/goal/get", { threadId });
      if (epoch !== generation.current || current !== revision.current) return;
      setGoal(result.goal ?? null);
      setUnsupported(false);
      setError(null);
    } catch (e) {
      if (epoch !== generation.current || current !== revision.current) return;
      if (
        /(-32601|unsupported method|not supported|disabled|not enabled)/i.test(
          e.message,
        )
      )
        setUnsupported(true);
      else setError(e);
    } finally {
      if (epoch === generation.current) pending.current = false;
    }
  }
  useEffect(() => {
    generation.current++;
    pending.current = false;
    setBusy(false);
    mutating.current = false;
    setError(null);
    setUnsupported(false);
    load();
    const listener = ({ detail }) => {
      if (detail?.type === "reconnected") load();
      if (
        detail?.type === "codex" &&
        detail.worker === worker?.id &&
        detail.message?.params?.threadId === threadId &&
        [
          "thread/goal/updated",
          "thread/goal/cleared",
          "thread/tokenUsage/updated",
          "turn/completed",
        ].includes(detail.message.method)
      )
        load();
    };
    const visible = () => {
      if (!document.hidden) load();
    };
    events.addEventListener("server", listener);
    document.addEventListener("visibilitychange", visible);
    const interval = setInterval(load, 5000);
    return () => {
      generation.current++;
      clearInterval(interval);
      events.removeEventListener("server", listener);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [worker?.id, worker?.online, supported, threadId]);
  async function mutate(method, params) {
    if (mutating.current) return;
    mutating.current = true;
    revision.current++;
    const epoch = generation.current;
    setBusy(true);
    setError(null);
    try {
      const result = await rpc(worker.id, method, {
        threadId,
        origin: "user",
        ...params,
      });
      if (epoch !== generation.current) return;
      setGoal(method === "thread/goal/clear" ? null : result.goal);
      setEditing(false);
      onChanged?.();
    } catch (e) {
      if (epoch === generation.current) setError(e);
    } finally {
      if (epoch === generation.current) {
        setBusy(false);
        mutating.current = false;
      }
    }
  }
  function edit() {
    setObjective(goal?.objective || "");
    setBudget(goal?.tokenBudget == null ? "" : String(goal.tokenBudget));
    setEditing(true);
  }
  if (!worker?.online && goal === undefined)
    return (
      <div className="goal-panel muted">
        Worker offline. Reconnect it to view the session goal.
      </div>
    );
  if (!supported && worker?.online)
    return (
      <div className="goal-panel muted">
        Goal controls need an updated worker on this machine.
      </div>
    );
  if (unsupported && worker?.online)
    return (
      <div className="goal-panel muted">
        Goals are unavailable in this Codex installation. Enable goals or update
        Codex to use these controls.
      </div>
    );
  return (
    <section className="goal-panel" aria-label="Session goal">
      <div className="goal-heading">
        <strong>Goal</strong>
        <span className="badge">
          {goal
            ? labels[goal.status] || goal.status
            : goal === null
              ? "None"
              : "Loading…"}
        </span>
        {!editing && (
          <button disabled={readonly || goal === undefined} onClick={edit}>
            {goal ? "Edit goal" : "Set goal"}
          </button>
        )}
        {goal && !editing && (
          <>
            <BusyButton
              busy={busy}
              disabled={
                !worker?.online || (agentPaused && goal.status !== "active")
              }
              onClick={() =>
                mutate("thread/goal/set", {
                  status: goal.status === "active" ? "paused" : "active",
                  ...(goal.status === "complete"
                    ? {
                        objective: goal.objective,
                        tokenBudget: goal.tokenBudget ?? null,
                      }
                    : {}),
                })
              }
            >
              {goal.status === "active"
                ? "Pause goal"
                : goal.status === "complete"
                  ? "Restart goal"
                  : "Resume goal"}
            </BusyButton>
            <button
              disabled={readonly}
              onClick={() => {
                if (
                  confirm(
                    "Clear this goal? Its current objective and usage accounting will be removed.",
                  )
                )
                  mutate("thread/goal/clear", {});
              }}
            >
              Clear goal
            </button>
          </>
        )}
      </div>
      <ErrorMessage error={error} />
      {!worker?.online && (
        <p className="muted">Worker offline. Showing the last known goal.</p>
      )}
      {goal && !editing && (
        <>
          <p className="goal-objective">{goal.objective}</p>
          <p className="muted">
            {Number(goal.tokensUsed || 0).toLocaleString()} tokens used
            {goal.tokenBudget != null
              ? ` / ${Number(goal.tokenBudget).toLocaleString()} budget`
              : " · no token budget"}{" "}
            · {Math.floor(Number(goal.timeUsedSeconds || 0) / 60)} min elapsed
          </p>
        </>
      )}
      {agentPaused && (
        <p className="muted">
          Agent paused. Resume the agent before activating its goal.
        </p>
      )}
      {editing && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            mutate("thread/goal/set", {
              objective: objective.trim(),
              tokenBudget: budget === "" ? null : Number(budget),
              status: agentPaused
                ? "paused"
                : goal?.status === "active"
                  ? "active"
                  : "paused",
            });
          }}
        >
          {!goal && (
            <p className="muted">
              New goals are saved paused. Use Resume goal to start work.
            </p>
          )}
          <label>
            Objective
            <textarea
              required
              maxLength={4000}
              value={objective}
              onChange={(e) => setObjective(e.target.value)}
              rows={3}
            />
          </label>
          <label>
            Token budget (optional)
            <input
              type="number"
              min="1"
              max="9007199254740991"
              step="1"
              value={budget}
              onChange={(e) => setBudget(e.target.value)}
            />
          </label>
          {goal && objective.trim() !== goal.objective && (
            <p className="muted">
              Changing the objective starts a new goal and resets its usage.
            </p>
          )}
          <div className="actions">
            <BusyButton
              busy={busy}
              disabled={!worker?.online || !objective.trim()}
              type="submit"
            >
              Save goal
            </BusyButton>
            <button
              type="button"
              disabled={busy}
              onClick={() => setEditing(false)}
            >
              Cancel
            </button>
          </div>
        </form>
      )}
    </section>
  );
}
