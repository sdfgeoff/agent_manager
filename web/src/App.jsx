import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  MessagesSquare,
  Terminal,
  Bot,
  Bell,
  Settings,
  LogOut,
  Plus,
  Hash,
  Menu,
  X,
  Users,
  ExternalLink,
} from "lucide-react";
import {
  api,
  board,
  events,
  route,
  href,
  navigate,
  friendly,
  newAuthScope,
} from "./api";
import { AppContext } from "./context";
import { Modal, ErrorMessage } from "./components";
import Topics, { NewChannel } from "./Topics";
import Sessions from "./Sessions";
import Agents from "./Agents";
import SettingsPage, { revokeLocalPush } from "./Settings";
import Inbox from "./Inbox";
export default function App() {
  const [me, setMe] = useState(undefined);
  const [overview, setOverview] = useState({ people: [], channels: [] });
  const [agents, setAgents] = useState([]);
  const [workers, setWorkers] = useState([]);
  const [current, setRoute] = useState(route);
  const [revision, setRevision] = useState(0);
  const [connection, setConnection] = useState("Connecting…");
  const [connectionEpoch, setConnectionEpoch] = useState(0);
  const [error, setError] = useState(null);
  const [menu, setMenu] = useState(false);
  const [creatingChannel, setCreatingChannel] = useState(false);
  const [person, setPerson] = useState(null);
  const [authEpoch, setAuthEpoch] = useState(0);
  const drafts = useRef(new Map());
  const lastTopics = useRef({});
  const menuTrigger = useRef(null);
  const auth = useRef(0);
  const metadata = useRef(0);
  const refreshTimer = useRef(null);
  const signedOut = useCallback(() => {
    newAuthScope();
    clearTimeout(refreshTimer.current);
    auth.current++;
    metadata.current++;
    drafts.current.clear();
    lastTopics.current = {};
    setMenu(false);
    setMe(null);
    setOverview({ people: [], channels: [] });
    setAgents([]);
    setWorkers([]);
    setPerson(null);
    setCreatingChannel(false);
    setError(null);
    setAuthEpoch((v) => v + 1);
    setConnection("Signed out");
  }, []);
  const refresh = useCallback(async () => {
    const version = ++metadata.current;
    const epoch = auth.current;
    const [overview, agents, workers] = await Promise.all([
      board("overview"),
      board("list_agents"),
      api("/api/workers"),
    ]);
    if (version !== metadata.current || epoch !== auth.current) return;
    setOverview(overview);
    setAgents(agents);
    setWorkers(workers);
  }, []);
  const changed = useCallback(() => {
    clearTimeout(refreshTimer.current);
    refreshTimer.current = setTimeout(() => {
      setRevision((v) => v + 1);
      refresh().catch((e) => {
        if (e.status !== 401) setError(e);
      });
    }, 150);
  }, [refresh]);
  useEffect(() => {
    if (!menu) return;
    document.querySelector(".shell-sidebar a")?.focus();
    const keyboard = (e) => {
      if (document.querySelector("dialog[open]")) return;
      if (e.key === "Escape") {
        e.preventDefault();
        setMenu(false);
      }
      if (e.key === "Tab") {
        const nodes = [
          ...document.querySelectorAll(
            ".shell-sidebar a[href],.shell-sidebar button:not([disabled])",
          ),
        ];
        const first = nodes[0],
          last = nodes.at(-1);
        if (
          (e.shiftKey && document.activeElement === first) ||
          (!e.shiftKey && document.activeElement === last)
        ) {
          e.preventDefault();
          (e.shiftKey ? last : first)?.focus();
        }
      }
    };
    document.addEventListener("keydown", keyboard);
    return () => {
      document.removeEventListener("keydown", keyboard);
      menuTrigger.current?.focus();
    };
  }, [menu]);
  useEffect(() => {
    const listener = () => {
      setRoute(route());
      setMenu(false);
    };
    window.addEventListener("hashchange", listener);
    return () => window.removeEventListener("hashchange", listener);
  }, []);
  useEffect(() => {
    window.addEventListener("auth-lost", signedOut);
    const initialAuth = auth.current;
    api("/api/me")
      .then(async (data) => {
        if (auth.current !== initialAuth) return;
        setMe(data);
        await refresh();
      })
      .catch((e) => {
        if (auth.current !== initialAuth) return;
        if (e.status !== 401) setError(e);
        setMe(null);
      });
    return () => {
      window.removeEventListener("auth-lost", signedOut);
      clearTimeout(refreshTimer.current);
    };
  }, []);
  useEffect(() => {
    if (!me) return;
    const epoch = auth.current;
    const stream = new EventSource("/api/events");
    stream.onopen = () => {
      if (auth.current !== epoch) return;
      setConnection("Connected");
      changed();
      events.dispatchEvent(
        new CustomEvent("server", { detail: { type: "reconnected" } }),
      );
    };
    stream.onerror = () => {
      if (auth.current === epoch) setConnection("Reconnecting…");
    };
    stream.onmessage = (event) => {
      if (auth.current !== epoch) return;
      try {
        const data = JSON.parse(event.data);
        if (data.type === "board_changed") changed();
        if (data.type === "workers_changed") refresh().catch(setError);
        events.dispatchEvent(new CustomEvent("server", { detail: data }));
      } catch {}
    };
    const visible = () => {
      if (!document.hidden) {
        changed();
        events.dispatchEvent(
          new CustomEvent("server", { detail: { type: "reconnected" } }),
        );
      }
    };
    document.addEventListener("visibilitychange", visible);
    const fallback = setInterval(() => {
      if (!document.hidden) changed();
    }, 20000);
    return () => {
      stream.close();
      clearInterval(fallback);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [me?.username, connectionEpoch]);
  async function login(username, password) {
    newAuthScope();
    const epoch = ++auth.current;
    await api("/api/login", { username, password });
    const data = await api("/api/me");
    if (auth.current !== epoch) return;
    setMe(data);
    setError(null);
    setAuthEpoch((v) => v + 1);
    await refresh();
  }
  async function logout() {
    try {
      await api("/api/logout", {});
      await revokeLocalPush();
      signedOut();
    } catch (e) {
      setError(e);
    }
  }
  if (me === undefined)
    return <div className="boot">Connecting to your workspace…</div>;
  if (!me) return <Login onSubmit={login} outerError={error} />;
  const context = {
    me,
    people: overview.people,
    channels: overview.channels,
    agents,
    workers,
    revision,
    refresh,
    changed,
    report: setError,
    openPerson: setPerson,
    drafts,
    reconnect: () => {
      newAuthScope();
      auth.current++;
      metadata.current++;
      setConnectionEpoch((v) => v + 1);
    },
    signedOut,
  };
  if (current.page === "topics")
    lastTopics.current = Object.fromEntries(current.params);
  const activeChannel =
    Number(current.params.get("channel")) || overview.channels[0]?.id;
  const pages = [
    ["topics", "Topics", MessagesSquare],
    ["sessions", "Sessions", Terminal],
    ["agents", "Agents", Bot],
    ["inbox", "Inbox", Bell],
    ["settings", "Settings", Settings],
  ];
  const page = pages.some((p) => p[0] === current.page)
    ? current.page
    : "topics";
  const viewingPerson = overview.people.find((p) => p.id === person);
  const personAgent = agents.find((a) => a.id === person);
  return (
    <AppContext.Provider key={`${me.username}:${authEpoch}`} value={context}>
      <div className="app-shell">
        <aside className={`shell-sidebar ${menu ? "open" : ""}`}>
          <a className="brand" href="#/topics">
            <span className="brandmark">
              a<span>m</span>
            </span>
            <strong>Agent Manager</strong>
          </a>
          <button
            type="button"
            className="icon mobile sidebar-close"
            aria-label="Close navigation"
            onClick={() => setMenu(false)}
          >
            <X size={20} />
          </button>
          <nav className="primary-nav" aria-label="Main navigation">
            {pages.map(([id, label, Icon]) => (
              <a
                key={id}
                href={href(
                  id,
                  id === "topics"
                    ? {
                        ...lastTopics.current,
                        channel: lastTopics.current.channel || activeChannel,
                      }
                    : {},
                )}
                className={page === id ? "active" : ""}
                aria-current={page === id ? "page" : undefined}
              >
                <Icon size={18} />
                {label}
              </a>
            ))}
          </nav>
          <div className="sidebar-channels">
            <div className="sidebar-label">
              CHANNELS
              <button
                className="icon"
                aria-label="Create channel"
                onClick={() => setCreatingChannel(true)}
              >
                <Plus size={16} />
              </button>
            </div>
            {overview.channels.map((c) => (
              <a
                key={c.id}
                href={href("topics", {
                  channel: c.id,
                  view: current.params.get("view") || "threads",
                })}
                className={
                  page === "topics" && activeChannel === c.id ? "selected" : ""
                }
              >
                <Hash size={15} />
                {c.name}
                {c.subscribed && <span className="subscribed-dot" />}
              </a>
            ))}
          </div>
          <div className="shell-user">
            <div>
              <strong>{me.username}</strong>
              <small
                className={connection === "Connected" ? "online-text" : ""}
              >
                <span className="connection-dot" />
                {connection}
              </small>
            </div>
            <button className="icon" aria-label="Sign out" onClick={logout}>
              <LogOut size={17} />
            </button>
          </div>
        </aside>
        {menu && (
          <button
            className="menu-backdrop"
            aria-label="Close navigation"
            onClick={() => setMenu(false)}
          />
        )}
        <div className="shell-main">
          <header className="shell-topbar">
            <button
              className="icon mobile"
              aria-label="Open navigation"
              ref={menuTrigger}
              aria-expanded={menu}
              onClick={() => setMenu(true)}
            >
              <Menu size={20} />
            </button>
            <div className="breadcrumb">
              <span>Workspace</span>
              <span>/</span>
              <strong>{pages.find((p) => p[0] === page)?.[1]}</strong>
            </div>
            <span className="topbar-status">
              <span className="connection-dot" />
              {connection}
            </span>
          </header>
          {error && (
            <div className="global-notice" role="alert">
              <span>{friendly(error)}</span>
              <button
                className="icon"
                aria-label="Dismiss notification"
                onClick={() => setError(null)}
              >
                <X size={16} />
              </button>
            </div>
          )}
          <main className={`main-view view-${page}`}>
            {page === "topics" && <Topics params={current.params} />}{" "}
            {page === "sessions" && <Sessions params={current.params} />}{" "}
            {page === "agents" && <Agents params={current.params} />}{" "}
            {page === "inbox" && <Inbox params={current.params} />}{" "}
            {page === "settings" && <SettingsPage />}
          </main>
        </div>
        {creatingChannel && (
          <NewChannel onClose={() => setCreatingChannel(false)} />
        )}{" "}
        {viewingPerson && (
          <Modal title={viewingPerson.name} onClose={() => setPerson(null)}>
            <div className="person-detail">
              <span className="avatar">
                {viewingPerson.kind === "agent" ? (
                  <Bot size={24} />
                ) : (
                  <Users size={24} />
                )}
              </span>
              <h3>@{viewingPerson.handle}</h3>
              <p className="muted">
                {viewingPerson.kind === "agent"
                  ? "Agent persona"
                  : "Human account"}
              </p>
              {personAgent && (
                <>
                  <p>
                    {personAgent.persona || "No custom persona instructions."}
                  </p>
                  <div className="actions">
                    <a
                      className="button"
                      href={href("sessions", {
                        worker: personAgent.worker,
                        thread: personAgent.thread,
                      })}
                      onClick={() => setPerson(null)}
                    >
                      <Terminal size={15} />
                      Open session
                    </a>
                    <a
                      className="button"
                      href={href("agents", { id: personAgent.id })}
                      onClick={() => setPerson(null)}
                    >
                      Agent settings
                      <ExternalLink size={13} />
                    </a>
                  </div>
                </>
              )}
            </div>
          </Modal>
        )}
      </div>
    </AppContext.Provider>
  );
}
function Login({ onSubmit, outerError }) {
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  return (
    <section className="login-screen">
      <form
        className="login-card"
        onSubmit={async (e) => {
          e.preventDefault();
          if (busy) return;
          setBusy(true);
          setError(null);
          try {
            await onSubmit(username.trim(), password);
            setPassword("");
          } catch (e) {
            setError(e);
          } finally {
            setBusy(false);
          }
        }}
      >
        <span className="brandmark large">
          a<span>m</span>
        </span>
        <p className="eyebrow">YOUR MACHINES. ONE WORKSPACE.</p>
        <h1>Welcome back.</h1>
        <p className="muted">Conversations, agents and sessions, together.</p>
        <label>
          Username
          <input
            value={username}
            required
            maxLength={64}
            autoComplete="username"
            autoFocus
            onChange={(e) => setUsername(e.target.value)}
          />
        </label>
        <label>
          Password
          <input
            type="password"
            value={password}
            required
            minLength={12}
            maxLength={1024}
            autoComplete="current-password"
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
        <ErrorMessage error={error || outerError} />
        <button className="primary wide" disabled={busy} type="submit">
          {busy ? "Signing in…" : "Sign in ↗"}
        </button>
      </form>
    </section>
  );
}
