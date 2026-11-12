import { ReminderDialog } from "./Reminders";
import React, { useEffect, useRef, useState } from "react";
import {
  Plus,
  Search,
  Columns3,
  List,
  Bell,
  BellOff,
  Archive,
  MessageSquare,
  ArrowLeft,
} from "lucide-react";
import { board, href, navigate, route, authGuard } from "./api";
import { useApp } from "./context";
import {
  Composer,
  ErrorMessage,
  RichText,
  Person,
  Attachments,
  Modal,
} from "./components";
export default function Topics({ params }) {
  const app = useApp();
  const channel = Number(params.get("channel")) || app.channels[0]?.id;
  const view = params.get("view") === "kanban" ? "kanban" : "threads";
  const sort = params.get("sort") === "created" ? "created" : "latest";
  const search = params.get("search") || "";
  const status = params.get("status") || "";
  const assignee = params.get("assignee") || "";
  const showDiscussion = params.get("discussion") === "1";
  const showArchived = params.get("archived") === "1";
  const selected = Number(params.get("topic")) || null;
  const [data, setData] = useState([]);
  const [cursor, setCursor] = useState(null);
  const [error, setError] = useState(null);
  const [loading, setLoading] = useState(false);
  const composer = useRef(null);
  const [query, setQuery] = useState(search);
  const generation = useRef(0);
  const channelInfo = app.channels.find((c) => c.id === channel);
  const current = {
    channel,
    sort,
    view,
    search,
    status,
    assignee,
    discussion: showDiscussion ? "1" : "",
    archived: showArchived ? "1" : "",
    topic: selected,
    page: params.get("page"),
    reply: params.get("reply"),
  };
  function patch(changes) {
    navigate("topics", {
      ...current,
      ...(changes.topic !== undefined && changes.topic !== selected
        ? { reply: null, page: null }
        : {}),
      ...changes,
    });
  }
  useEffect(() => {
    setQuery(search);
  }, [search]);
  useEffect(() => {
    const timer = setTimeout(() => {
      if (query !== search) patch({ search: query, topic: null });
    }, 250);
    return () => clearTimeout(timer);
  }, [query]);
  async function load(more = false) {
    const version = ++generation.current;
    setLoading(true);
    try {
      const result = await board("list_topics", {
        channel,
        sort,
        search,
        ...(status ? { status } : {}),
        ...(more && cursor ? { before: cursor } : {}),
      });
      if (version !== generation.current) return;
      setData((old) => (more ? [...old, ...result.data] : result.data));
      setCursor(result.nextCursor);
      setError(null);
    } catch (e) {
      if (version === generation.current) setError(e);
    } finally {
      if (version === generation.current) setLoading(false);
    }
  }
  useEffect(() => {
    load();
    return () => {
      generation.current++;
    };
  }, [channel, sort, search, status, app.revision]);
  const visible = data.filter(
    (t) =>
      (showArchived || t.status !== "archived") &&
      (view !== "kanban" || showDiscussion || t.status !== "discussion") &&
      (!assignee || t.assignee === assignee),
  );
  const columns = ["discussion", "working", "done", "archived"].filter(
    (s) =>
      (s !== "discussion" || showDiscussion) &&
      (s !== "archived" || showArchived),
  );
  async function subscribe() {
    try {
      await board("set_subscription", {
        kind: "channel",
        target: channel,
        subscribed: !channelInfo?.subscribed,
      });
      await app.refresh();
    } catch (e) {
      setError(e);
    }
  }
  function card(t) {
    return (
      <a
        className={`topic-card ${selected === t.id ? "selected" : ""}`}
        href={href("topics", {
          ...current,
          topic: t.id,
          reply: null,
          page: null,
        })}
        key={t.id}
      >
        <div className="card-meta">
          <span className={`status-dot ${t.status}`} />
          <span>#{t.id}</span>
          <span className="capitalize">{t.status}</span>
        </div>
        <h3>{t.title}</h3>
        <p>{t.body.slice(0, 180)}</p>
        <div className="card-meta">
          {t.assignee ? (
            <span>
              Assigned to{" "}
              {app.people.find((p) => p.id === t.assignee)?.name || t.assignee}
            </span>
          ) : (
            <span>Unassigned</span>
          )}
          <time>{new Date(t.createdAt * 1000).toLocaleDateString()}</time>
        </div>
        <div className="card-meta">
          <span>
            Latest post by{" "}
            {app.people.find((p) => p.id === (t.latestPost?.author || t.author))
              ?.name ||
              t.latestPost?.author ||
              t.author}
            {" at "}
            <time
              dateTime={new Date(
                (t.latestPost?.createdAt || t.createdAt) * 1000,
              ).toISOString()}
            >
              {new Date(
                (t.latestPost?.createdAt || t.createdAt) * 1000,
              ).toLocaleString()}
            </time>
          </span>
        </div>
      </a>
    );
  }
  return (
    <div className={`topics-workspace ${selected ? "with-detail" : ""}`}>
      <section className={`topic-browser ${selected ? "has-selection" : ""}`}>
        <div className="topic-browser-content">
          <div className="page-heading">
            <div>
              <p className="eyebrow">SHARED WORKSPACE</p>
              <h1>#{channelInfo?.name || "general"}</h1>
              <p>
                {channelInfo?.description ||
                  "Conversations and work, together."}
              </p>
            </div>
            <div className="actions">
              <button
                onClick={subscribe}
                title="Channel subscriptions notify you about new topics"
              >
                {channelInfo?.subscribed ? (
                  <BellOff size={16} />
                ) : (
                  <Bell size={16} />
                )}
                <span>
                  {channelInfo?.subscribed ? "Subscribed" : "Subscribe"}
                </span>
              </button>
              <button
                className="primary"
                onClick={() =>
                  composer.current?.querySelector("textarea")?.focus()
                }
              >
                <Plus size={16} />
                New topic
              </button>
            </div>
          </div>
          <div className="topic-controls">
            <div className="view-switch" aria-label="Topic view">
              <button
                className={view === "threads" ? "active" : ""}
                onClick={() => patch({ view: "threads" })}
              >
                <List size={16} />
                Threads
              </button>
              <button
                className={view === "kanban" ? "active" : ""}
                onClick={() => patch({ view: "kanban" })}
              >
                <Columns3 size={16} />
                Kanban
              </button>
            </div>
            <div className="search-input">
              <Search size={16} />
              <input
                aria-label="Search topics"
                placeholder="Search topics and replies"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
          </div>
          <div className="filters">
            <select
              aria-label="Sort topics"
              value={sort}
              onChange={(e) => patch({ sort: e.target.value })}
            >
              <option value="latest">Latest post</option>
              <option value="created">Newest topic</option>
            </select>
            <select
              aria-label="Filter by status"
              value={status}
              onChange={(e) => patch({ status: e.target.value })}
            >
              <option value="">All statuses</option>
              {["discussion", "working", "done", "archived"].map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
            </select>
            <select
              aria-label="Filter by assignee"
              value={assignee}
              onChange={(e) => patch({ assignee: e.target.value })}
            >
              <option value="">Everyone</option>
              {app.people.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
            {view === "kanban" && (
              <label className="check">
                <input
                  type="checkbox"
                  checked={showDiscussion}
                  onChange={(e) =>
                    patch({ discussion: e.target.checked ? "1" : "" })
                  }
                />
                Show Discussion
              </label>
            )}
            <label className="check">
              <input
                type="checkbox"
                checked={showArchived}
                onChange={(e) =>
                  patch({ archived: e.target.checked ? "1" : "" })
                }
              />
              Show Archived
            </label>
            <span className="muted result-count">{visible.length} loaded</span>
          </div>
          <ErrorMessage error={error} />
          {view === "kanban" ? (
            <div className="kanban">
              {columns.map((s) => (
                <section className="kanban-column" key={s}>
                  <h2>
                    <span className={`status-dot ${s}`} />
                    {s}
                    <small>
                      {visible.filter((t) => t.status === s).length}
                    </small>
                  </h2>
                  {visible.filter((t) => t.status === s).map(card)}
                </section>
              ))}
            </div>
          ) : (
            <div className="topic-list">{visible.map(card)}</div>
          )}
          {!visible.length && !loading && (
            <div className="empty">
              <MessageSquare size={30} />
              <h2>
                {view === "kanban"
                  ? "No topics in these columns"
                  : "No topics yet"}
              </h2>
              <p>
                {view === "kanban"
                  ? "Discussion is hidden by default. Reveal it or start something new."
                  : "Start a discussion, mention a teammate, and take it from there."}
              </p>
            </div>
          )}
          {cursor && (
            <button
              className="load-more"
              disabled={loading}
              onClick={() => load(true)}
            >
              {loading ? "Loading…" : "Load more topics"}
            </button>
          )}
        </div>
        <div className="channel-composer" ref={composer}>
          <p className="field-hint">
            New topic in #{channelInfo?.name || "general"}
          </p>
          <Composer
            key={channel}
            draftKey={`new-topic:${channel}`}
            title
            label="New topic message"
            placeholder="Start a new topic… Type @ to mention someone."
            buttonText="Post topic"
            onSubmit={async (data) => {
              const t = await board("create_topic", { ...data, channel });
              patch({ topic: t.id });
              app.changed();
            }}
          />
        </div>
      </section>
      {selected && (
        <TopicDetail
          key={selected}
          id={selected}
          view={view}
          onViewChange={(next) => patch({ view: next })}
          onClose={() => patch({ topic: null })}
        />
      )}
    </div>
  );
}
function TopicDetail({ id, onClose, view, onViewChange }) {
  const reply = new URLSearchParams(location.hash.split("?")[1] || "").get(
    "reply",
  );
  const pageValue = new URLSearchParams(location.hash.split("?")[1] || "").get(
    "page",
  );
  const page =
    /^\d+$/.test(pageValue || "") && Number.isSafeInteger(Number(pageValue))
      ? Number(pageValue)
      : 0;
  const scrolledPage = useRef(null);
  const [reminding, setReminding] = useState(false);
  const app = useApp();
  const [topic, setTopic] = useState(null);
  const [error, setError] = useState(null);
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState("");
  const sequence = useRef(0);
  const scroll = useRef(null);
  const sentReply = useRef(null);
  const scrolledReply = useRef(null);
  async function load(latest = false) {
    const version = ++sequence.current;
    const requested = route().params;
    const requestedPage = requested.get("page");
    const requestedReply = requested.get("reply");
    try {
      const t = await board("read_topic", {
        topic: id,
        page: latest ? 0 : page,
        ...(!latest && reply ? { reply: Number(reply) } : {}),
      });
      const location = route();
      if (
        version === sequence.current &&
        location.params.get("page") === requestedPage &&
        location.params.get("reply") === requestedReply
      ) {
        setTopic(t);
        setError(null);
        const current = route();
        if (
          current.page === "topics" &&
          Number(current.params.get("topic")) === id &&
          current.params.get("page") !== String(t.page)
        ) {
          history.replaceState(
            null,
            "",
            href("topics", {
              ...Object.fromEntries(current.params),
              page: t.page,
            }),
          );
          window.dispatchEvent(new HashChangeEvent("hashchange"));
        }
      }
    } catch (e) {
      if (version === sequence.current) setError(e);
    }
  }
  useEffect(() => {
    load();
    return () => sequence.current++;
  }, [id, reply, page, app.revision]);
  useEffect(() => {
    if (topic && scrolledPage.current !== topic.page) {
      scrolledPage.current = topic.page;
      if (!reply && !sentReply.current && scroll.current)
        scroll.current.scrollTop = 0;
    }
  }, [topic, reply]);
  useEffect(() => {
    if (!reply) scrolledReply.current = null;
    if (reply && topic && scrolledReply.current !== reply) {
      const target = document.getElementById("reply-" + reply);
      if (target) {
        target.scrollIntoView({ block: "center" });
        scrolledReply.current = reply;
      }
    }
  }, [reply, topic]);
  useEffect(() => {
    if (
      sentReply.current &&
      topic?.replies.some((r) => r.id === sentReply.current)
    ) {
      document
        .getElementById("reply-" + sentReply.current)
        ?.scrollIntoView({ block: "nearest" });
      sentReply.current = null;
    }
  }, [topic]);
  async function update(p) {
    try {
      await board("update_topic", { topic: id, ...p });
      await load();
      app.changed();
    } catch (e) {
      setError(e);
    }
  }
  function pageHref(number) {
    return href("topics", {
      ...Object.fromEntries(route().params),
      page: number,
      reply: null,
    });
  }
  function pagination(position) {
    if (!topic) return null;
    const numbers = [
      ...new Set([
        1,
        ...Array.from({ length: 5 }, (_, i) => topic.page + i - 2),
        topic.pageCount,
      ]),
    ]
      .filter((n) => n >= 1 && n <= topic.pageCount)
      .sort((a, b) => a - b);
    return (
      <nav
        className="reply-pagination"
        aria-label={"Reply pages (" + position + ")"}
      >
        <p>
          Page {topic.page} of {topic.pageCount} · {topic.replyCount} replies
        </p>
        <div>
          {topic.page > 1 ? (
            <>
              <a href={pageHref(1)}>First</a>
              <a href={pageHref(topic.page - 1)}>Previous</a>
            </>
          ) : (
            <>
              <span aria-disabled="true">First</span>
              <span aria-disabled="true">Previous</span>
            </>
          )}
          {numbers.map((n, i) => (
            <React.Fragment key={n}>
              {i > 0 && n > numbers[i - 1] + 1 && <span>…</span>}
              <a
                href={pageHref(n)}
                aria-label={"Page " + n}
                aria-current={n === topic.page ? "page" : undefined}
              >
                {n}
              </a>
            </React.Fragment>
          ))}
          {topic.page < topic.pageCount ? (
            <>
              <a href={pageHref(topic.page + 1)}>Next</a>
              <a href={pageHref(topic.pageCount)}>Latest</a>
            </>
          ) : (
            <>
              <span aria-disabled="true">Next</span>
              <span aria-disabled="true">Latest</span>
            </>
          )}
        </div>
      </nav>
    );
  }
  return (
    <section className="topic-detail" aria-label="Topic details" ref={scroll}>
      <div className="detail-top">
        <button className="text-button" onClick={onClose}>
          <ArrowLeft size={16} />
          Topics
        </button>
        <div className="view-switch compact">
          <button
            aria-label="Thread view"
            className={view === "threads" ? "active" : ""}
            onClick={() => onViewChange("threads")}
          >
            <List size={14} />
          </button>
          <button
            aria-label="Kanban view"
            className={view === "kanban" ? "active" : ""}
            onClick={() => onViewChange("kanban")}
          >
            <Columns3 size={14} />
          </button>
        </div>
        <span className="muted">#{id}</span>
      </div>
      <ErrorMessage error={error} />
      {topic ? (
        <>
          <button
            className="title-button"
            onClick={() => {
              setTitle(topic.title);
              setRenaming(true);
            }}
            title="Rename topic"
          >
            <h2>{topic.title}</h2>
          </button>
          <p className="muted">#{topic.channelName}</p>
          <div className="topic-properties">
            <label>
              Status
              <select
                value={topic.status}
                onChange={(e) => update({ status: e.target.value })}
              >
                {["discussion", "working", "done", "archived"].map((s) => (
                  <option key={s}>{s}</option>
                ))}
              </select>
            </label>
            <label>
              Assignee
              <select
                aria-label="Topic assignee"
                value={topic.assignee || ""}
                onChange={(e) => update({ assignee: e.target.value || null })}
              >
                <option value="">Unassigned</option>
                {app.people.map((p) => (
                  <option value={p.id} key={p.id} disabled={p.active === false}>
                    {p.name} (@{p.handle})
                    {p.active === false ? " · removed" : ""}
                  </option>
                ))}
              </select>
            </label>
            <button onClick={() => setReminding(true)}>Remind me</button>
            <button
              onClick={async () => {
                try {
                  await board("set_subscription", {
                    kind: "topic",
                    target: id,
                    subscribed: !topic.subscribed,
                  });
                  await load();
                } catch (e) {
                  setError(e);
                }
              }}
            >
              {topic.subscribed ? <BellOff size={15} /> : <Bell size={15} />}{" "}
              {topic.subscribed ? "Unsubscribe" : "Subscribe"}
            </button>
          </div>
          {topic.page === 1 && (
            <article className="post opening">
              <div className="post-by">
                <Person id={topic.author} />
                <time>{new Date(topic.createdAt * 1000).toLocaleString()}</time>
              </div>
              <RichText text={topic.body} />
              <Attachments files={topic.attachments} />
            </article>
          )}
          {reply && (
            <p className="notice">
              Linked reply highlighted on this page.{" "}
              <a
                href={href("topics", {
                  ...Object.fromEntries(route().params),
                  reply: null,
                  page: topic.pageCount,
                })}
              >
                Show latest replies
              </a>
            </p>
          )}
          {reminding && (
            <ReminderDialog
              topic={id}
              onClose={() => setReminding(false)}
              onSaved={() => app.changed()}
            />
          )}
          <div className="replies-heading">
            <h3>Replies</h3>
          </div>
          {pagination("top")}
          {topic.replies.map((r) => (
            <article
              className={
                "post" + (String(r.id) === reply ? " search-focus" : "")
              }
              id={"reply-" + r.id}
              key={r.id}
            >
              <div className="post-by">
                <Person id={r.author} />
                <time>{new Date(r.createdAt * 1000).toLocaleString()}</time>
              </div>
              <RichText text={r.body} />
              <Attachments files={r.attachments} />
            </article>
          ))}
          {pagination("bottom")}
          <Composer
            draftKey={`reply:${id}`}
            label="Reply"
            buttonText="Reply"
            onSubmit={async (data) => {
              const stillSignedIn = authGuard();
              const posted = await board("post_reply", { ...data, topic: id });
              stillSignedIn();
              const current = route();
              if (
                current.page === "topics" &&
                Number(current.params.get("topic")) === id
              ) {
                // After sending, focus the new reply and keep refreshes on latest history.
                sentReply.current = posted.id;
                navigate("topics", {
                  ...Object.fromEntries(current.params),
                  reply: null,
                  page: null,
                });
                await load(true);
              }
              app.changed();
            }}
          />
          {renaming && (
            <Modal title="Rename topic" onClose={() => setRenaming(false)}>
              <form
                onSubmit={async (e) => {
                  e.preventDefault();
                  await update({ title });
                  setRenaming(false);
                }}
              >
                <label>
                  Title
                  <input
                    value={title}
                    autoFocus
                    required
                    maxLength={500}
                    onChange={(e) => setTitle(e.target.value)}
                  />
                </label>
                <button className="primary" type="submit">
                  Save title
                </button>
              </form>
            </Modal>
          )}
        </>
      ) : (
        !error && <p className="muted">Loading topic…</p>
      )}
    </section>
  );
}
export function NewChannel({ onClose }) {
  const app = useApp();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  return (
    <Modal title="Create a channel" onClose={onClose}>
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          if (busy) return;
          setBusy(true);
          try {
            const channel = await board("create_channel", {
              name,
              description,
            });
            await app.refresh();
            navigate("topics", { channel: channel.id });
            onClose();
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
            required
            pattern={"[a-z0-9_\\-]{2,40}"}
            autoFocus
            placeholder="project-name"
          />
        </label>
        <label>
          Description
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            maxLength={4000}
          />
        </label>
        <ErrorMessage error={error} />
        <button className="primary" disabled={busy}>
          Create channel
        </button>
      </form>
    </Modal>
  );
}
