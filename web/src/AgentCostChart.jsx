import React, { useEffect, useState } from "react";
import { api, rpc, href } from "./api";
import { useApp } from "./context";
import { ErrorMessage } from "./components";
import { costSeries } from "./usage-cost";
const usd = (value) =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 6,
  }).format(value);
export default function AgentCostChart({ agent }) {
  const app = useApp();
  const online = app.workers.find((w) => w.id === agent.worker)?.online;
  const [rows, setRows] = useState([]);
  const [rates, setRates] = useState(null);
  const [fields, setFields] = useState({
    input: "",
    cachedInput: "",
    output: "",
  });
  const [period, setPeriod] = useState("daily");
  const [error, setError] = useState(null);
  const [usageError, setUsageError] = useState(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let current = true;
    setLoading(true);
    api("/api/settings/usage-pricing/" + encodeURIComponent(agent.id))
      .then((r) => {
        if (current) {
          setRates(r);
          if (r)
            setFields({
              input: String(r.input),
              cachedInput: String(r.cachedInput),
              output: String(r.output),
            });
        }
      })
      .catch((e) => {
        if (current) setError(e);
      });
    if (online) {
      rpc(agent.worker, "manager/usage/daily", { agentId: agent.id })
        .then((r) => {
          if (current) {
            setRows(r.data);
            setUsageError(null);
          }
        })
        .catch((e) => {
          if (current) setUsageError(e);
        })
        .finally(() => {
          if (current) setLoading(false);
        });
    } else setLoading(false);
    return () => {
      current = false;
    };
  }, [agent.id, agent.worker, online]);
  async function save(e) {
    e.preventDefault();
    setBusy(true);
    try {
      const result = await api(
        "/api/settings/usage-pricing/" + encodeURIComponent(agent.id),
        Object.fromEntries(
          Object.entries(fields).map(([k, v]) => [k, Number(v)]),
        ),
      );
      setRates(result);
      setError(null);
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  const series = costSeries(rows, rates, period);
  const known = series.filter((s) => s.value !== null);
  const max = Math.max(...known.map((s) => s.value), 0.000001);
  const x = (i) => 54 + (i * 490) / (series.length - 1);
  const y = (value) => 155 - (value * 120) / max;
  return (
    <section className="agent-cost-chart">
      <h2>Estimated cost</h2>
      <p className="muted">
        API-equivalent USD for observed tokens, using the rates below for all of
        this agent's models. This is not a subscription charge.
      </p>
      <div className="view-switch" aria-label="Cost chart period">
        <button
          type="button"
          className={period === "daily" ? "active" : ""}
          onClick={() => setPeriod("daily")}
        >
          Daily
        </button>
        <button
          type="button"
          className={period === "monthly" ? "active" : ""}
          onClick={() => setPeriod("monthly")}
        >
          Monthly
        </button>
      </div>
      {loading ? (
        <p>Loading usage…</p>
      ) : !online ? (
        <p>Connect the worker to load usage.</p>
      ) : usageError ? (
        <p className="muted">
          Daily usage is unavailable: {usageError.message}. Collection requires
          a worker with daily usage support.
        </p>
      ) : !rows.length ? (
        <p>
          No daily usage recorded yet. Collection begins after the worker is
          updated.
        </p>
      ) : !rates ? (
        <p>Enter rates below to estimate costs.</p>
      ) : (
        <>
          {known.length ? (
            <>
              <p>
                <strong>
                  {usd(known.reduce((total, s) => total + s.value, 0))}
                </strong>{" "}
                observed over the{" "}
                {period === "daily" ? "last 30 days" : "last 12 months"}
              </p>
              <svg
                viewBox="0 0 580 195"
                role="img"
                aria-label={
                  period === "daily"
                    ? "Daily estimated cost in USD"
                    : "Monthly estimated cost in USD"
                }
                style={{ width: "100%", display: "block" }}
              >
                <title>
                  Estimated cost. UTC dates; gaps mean unknown usage.
                </title>
                <path
                  d="M54 30V155H554"
                  fill="none"
                  stroke="currentColor"
                  opacity=".35"
                />
                <text x="2" y="35" fill="currentColor" fontSize="11">
                  {usd(max)}
                </text>
                <text x="12" y="158" fill="currentColor" fontSize="11">
                  $0
                </text>
                {series.map((point, i) =>
                  point.value === null ? null : (
                    <g key={point.date}>
                      {i > 0 && series[i - 1].value !== null && (
                        <line
                          x1={x(i - 1)}
                          y1={y(series[i - 1].value)}
                          x2={x(i)}
                          y2={y(point.value)}
                          stroke="currentColor"
                          strokeWidth="2"
                        />
                      )}
                      <circle
                        cx={x(i)}
                        cy={y(point.value)}
                        r="4"
                        fill="currentColor"
                      >
                        <title>
                          {point.date}: {usd(point.value)}
                          {point.partial ? " (partial)" : ""}
                        </title>
                      </circle>
                    </g>
                  ),
                )}
                <text x="54" y="183" fill="currentColor" fontSize="11">
                  {series[0].date}
                </text>
                <text
                  x="554"
                  y="183"
                  textAnchor="end"
                  fill="currentColor"
                  fontSize="11"
                >
                  {series.at(-1).date}
                </text>
              </svg>
              <details>
                <summary>View chart values</summary>
                <table>
                  <thead>
                    <tr>
                      <th>UTC {period === "daily" ? "day" : "month"}</th>
                      <th>Estimated USD</th>
                    </tr>
                  </thead>
                  <tbody>
                    {series.map((s) => (
                      <tr key={s.date}>
                        <td>{s.date}</td>
                        <td>
                          {s.value === null
                            ? "Unknown"
                            : usd(s.value) + (s.partial ? " · partial" : "")}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </details>
            </>
          ) : (
            <p>No priced observations in this period.</p>
          )}
          <p className="muted">
            UTC dates. Gaps are unknown, not zero. Monthly points sum observed
            days only; partial coverage can understate cost.
          </p>
        </>
      )}
      <details open={!rates}>
        <summary>Estimate rates · USD per million tokens</summary>
        <form onSubmit={save}>
          {Object.entries({
            input: "Input",
            cachedInput: "Cached input",
            output: "Output",
          }).map(([key, label]) => (
            <label key={key}>
              {label}
              <input
                aria-label={label + " USD per million tokens"}
                type="number"
                min="0"
                max="1000000"
                step="any"
                required
                value={fields[key]}
                onChange={(e) =>
                  setFields((old) => ({ ...old, [key]: e.target.value }))
                }
              />
            </label>
          ))}
          <button type="submit" disabled={busy}>
            {busy ? "Saving…" : "Save estimate rates"}
          </button>
        </form>
        <p className="muted">
          Cached input is charged once at its own rate. Reasoning is included in
          output. Changing rates recalculates the chart; it does not change your
          model or billing.
        </p>
      </details>
      <ErrorMessage error={error} />
      <p>
        <a href={href("usage", { worker: agent.worker, agent: agent.id })}>
          View token usage and turns
        </a>
      </p>
    </section>
  );
}
