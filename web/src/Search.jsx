import React, { useEffect, useRef, useState } from "react";
import { Search as SearchIcon } from "lucide-react";
import { board, rpc, href, navigate } from "./api";
import { useApp } from "./context";
import { ErrorMessage } from "./components";

function Highlight({ text, query }) {
  if (!text || !query) return text;
  const words = query
    .trim()
    .split(/\s+/)
    .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const parts = text.split(new RegExp("(" + words.join("|") + ")", "gi"));
  return parts.map((p, i) => (i % 2 ? <mark key={i}>{p}</mark> : p));
}
export default function Search({ params }) {
  const app = useApp();
  const query = params.get("q") || "";
  const [input, setInput] = useState(query);
  const scope = params.get("scope") || "all";
  const [refreshKey, setRefreshKey] = useState(0);
  const [results, setResults] = useState({});
  const [error, setError] = useState(null);
  const sequence = useRef(0);
  const busy = useRef(new Set());
  const workerKey = app.workers
    .map((w) => w.id + w.online + (w.capabilities || []).join("/"))
    .join(",");
  async function source(key, offset, seq) {
    if (busy.current.has(key)) return;
    busy.current.add(key);
    setResults((old) => ({
      ...old,
      [key]: { ...old[key], loading: true, error: null },
    }));
    try {
      const data =
        key === "board"
          ? await board("search", { query, offset })
          : await rpc(key, "manager/search", { query, offset });
      if (seq !== sequence.current) return;
      setResults((old) => {
        const next = {
          ...old,
          [key]: {
            ...data,
            data: offset
              ? [...(old[key]?.data || []), ...data.data]
              : data.data,
            loading: false,
          },
        };
        return next;
      });
    } catch (e) {
      if (seq === sequence.current)
        setResults((old) => ({
          ...old,
          [key]: { ...old[key], loading: false, error: e },
        }));
    } finally {
      if (seq === sequence.current) busy.current.delete(key);
    }
  }
  useEffect(() => {
    const seq = ++sequence.current;
    busy.current = new Set();
    setResults({});
    setError(null);
    setInput(query);
    if (query.trim().length < 2) return;
    if (scope !== "sessions") source("board", 0, seq);
    if (scope !== "board")
      for (const w of app.workers) {
        if (!w.online)
          setResults((old) => ({
            ...old,
            [w.id]: {
              data: [],
              notice:
                "Disconnected — session history is stored on this machine.",
            },
          }));
        else if (!w.capabilities?.includes("session-search"))
          setResults((old) => ({
            ...old,
            [w.id]: {
              data: [],
              notice: "Update this worker to enable session search.",
            },
          }));
        else source(w.id, 0, seq);
      }
    return () => {
      sequence.current++;
    };
  }, [query, scope, workerKey, refreshKey]);
  const sources = [
    ...(scope !== "sessions"
      ? [{ id: "board", name: "Topics, replies, attachments & your reminders" }]
      : []),
    ...(scope !== "board" ? app.workers : []),
  ];
  return (
    <section className="page search-page">
      <div className="page-heading">
        <div>
          <p className="eyebrow">FIND THE CONVERSATION</p>
          <h1>Search</h1>
          <p>
            Search shared board content and messages across connected machines,
            regardless of working folder.
          </p>
        </div>
      </div>
      <form
        className="search-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (input.trim().length < 2) {
            setError(new Error("Enter at least two characters"));
            return;
          }
          if (input.trim() === query) setRefreshKey((v) => v + 1);
          navigate("search", { q: input.trim(), scope });
        }}
      >
        <label className="search-query">
          <span>Search text</span>
          <input
            type="search"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            maxLength={200}
            required
            autoFocus
            placeholder="A phrase, project or message…"
          />
        </label>
        <label>
          Search in
          <select
            aria-label="Search scope"
            value={scope}
            onChange={(e) => {
              navigate("search", { q: input.trim(), scope: e.target.value });
            }}
          >
            <option value="all">Everything</option>
            <option value="board">Board & my reminders</option>
            <option value="sessions">Session history</option>
          </select>
        </label>
        <button className="primary">
          <SearchIcon size={16} />
          Search
        </button>
      </form>
      <ErrorMessage error={error} />
      {query.trim().length >= 2 ? (
        sources.map((sourceInfo) => {
          const r = results[sourceInfo.id];
          return (
            <section className="search-source" key={sourceInfo.id}>
              <h2>{sourceInfo.name}</h2>
              {r?.notice && <p className="muted">{r.notice}</p>}
              <ErrorMessage error={r?.error} />
              {r?.error && (
                <button
                  onClick={() => source(sourceInfo.id, 0, sequence.current)}
                >
                  Retry
                </button>
              )}
              {r?.unavailable?.length > 0 && (
                <p className="notice">
                  {r.unavailable.length} sessions could not be read. Other
                  results are shown.
                </p>
              )}
              {r?.incomplete && (
                <p className="notice">
                  The local index is still being built; these results are
                  partial.{" "}
                  <button
                    onClick={() => source(sourceInfo.id, 0, sequence.current)}
                  >
                    Continue indexing
                  </button>
                </p>
              )}
              {r?.loading && (
                <p role="status">
                  Searching
                  {sourceInfo.id === "board"
                    ? "…"
                    : " and updating the local message index…"}{" "}
                </p>
              )}
              {r?.data?.map((item, i) => (
                <a
                  className="search-result"
                  key={item.id + ":" + i}
                  href={
                    sourceInfo.id === "board"
                      ? item.kind === "reminder"
                        ? href("inbox", { reminder: item.id })
                        : href("topics", {
                            channel: item.channel,
                            topic: item.topic,
                            reply: item.kind === "reply" ? item.id : null,
                          })
                      : href("sessions", {
                          worker: sourceInfo.id,
                          thread: item.thread,
                          search: item.role === "session" ? null : query,
                        })
                  }
                >
                  <div className="card-meta">
                    <span>{item.kind || item.role}</span>
                    <small>
                      {item.channelName ? "#" + item.channelName : item.cwd}
                    </small>
                  </div>
                  <h3>
                    <Highlight text={item.title} query={query} />
                  </h3>
                  <p>
                    <Highlight text={item.body} query={query} />
                  </p>
                </a>
              ))}
              {r && !r.loading && !r.error && !r.notice && !r.data?.length && (
                <p className="muted">No matches</p>
              )}
              {r?.nextOffset != null && (
                <button
                  disabled={r.loading}
                  onClick={() =>
                    source(sourceInfo.id, r.nextOffset, sequence.current)
                  }
                >
                  Load more results
                </button>
              )}
            </section>
          );
        })
      ) : (
        <div className="empty">
          <SearchIcon size={32} />
          <p>Enter at least two characters to search.</p>
        </div>
      )}
    </section>
  );
}
