import React, { useEffect, useRef, useState } from "react";
import { Bell, CheckCheck, ArrowUpRight } from "lucide-react";
import { board, navigate, href, authGuard } from "./api";
import { useApp } from "./context";
import { ErrorMessage, Modal, RichText } from "./components";
import Reminders from "./Reminders";
export default function Inbox({ params }) {
  const app = useApp();
  const [selected, setSelected] = useState(null);
  const [items, setItems] = useState([]);
  const [cursor, setCursor] = useState(null);
  const [error, setError] = useState(null);
  const [loading, setLoading] = useState(false);
  const seq = useRef(0);
  const clicked = useRef(null);
  async function load(more = false) {
    const current = ++seq.current;
    setLoading(true);
    try {
      const data = await board(
        "list_notifications",
        more ? { before: cursor } : {},
      );
      if (current !== seq.current) return;
      setItems((old) => (more ? [...old, ...data.data] : data.data));
      setCursor(data.data.length === 100 ? data.data.at(-1).id : null);
    } catch (e) {
      if (current === seq.current) setError(e);
    } finally {
      if (current === seq.current) setLoading(false);
    }
  }
  useEffect(() => {
    load();
    return () => seq.current++;
  }, [app.revision]);
  async function open(n) {
    const stillSignedIn = authGuard();
    try {
      if (!n.read) await board("mark_read", { id: n.id });
      stillSignedIn();
      app.changed();
      if (n.event.kind === "reminder") {
        const full = await board("read_notification", { id: n.id });
        stillSignedIn();
        setSelected(full);
      } else
        navigate("topics", {
          channel: n.event.channel,
          topic: n.event.topic,
          reply: n.event.reply,
        });
    } catch (e) {
      setError(e);
    }
  }
  useEffect(() => {
    const id = Number(params.get("notification"));
    if (!id || clicked.current === id) return;
    clicked.current = id;
    let alive = true;
    board("read_notification", { id })
      .then((n) => {
        if (alive) open(n);
      })
      .catch((e) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, [params.get("notification")]);
  return (
    <section className="page inbox-page">
      <div className="page-heading">
        <div>
          <p className="eyebrow">STAY IN THE LOOP</p>
          <h1>Inbox</h1>
          <p>Mentions, assignments and activity from your subscriptions.</p>
        </div>
        <a className="button" href={href("settings")}>
          <Bell size={16} />
          Notification settings
        </a>
      </div>
      <Reminders params={params} />
      <h2>Inbox activity</h2>
      <ErrorMessage error={error} />
      <div className="inbox-list">
        {items.map((n) => (
          <button
            key={n.id}
            className={`inbox-item ${n.read ? "" : "unread"}`}
            onClick={() => open(n)}
          >
            <span className="inbox-indicator" />
            <div>
              <div className="card-meta">
                <strong>{n.event.author?.name}</strong>
                <span>{n.event.kind}</span>
                <time>{new Date(n.createdAt * 1000).toLocaleString()}</time>
              </div>
              <h2>{n.event.title}</h2>
              <p>{n.event.body.slice(0, 200) || `Status: ${n.event.status}`}</p>
              <small>
                {n.event.kind === "reminder"
                  ? "Private reminder"
                  : `#${n.event.channelName} · topic #${n.event.topic}`}
              </small>
            </div>
            <ArrowUpRight size={18} />
          </button>
        ))}
      </div>
      {!items.length && !loading && (
        <div className="empty">
          <CheckCheck size={32} />
          <h2>You’re all caught up</h2>
          <p>
            Subscribe to a channel to hear about new topics, or join a
            conversation.
          </p>
        </div>
      )}
      {selected && (
        <Modal title={selected.event.title} onClose={() => setSelected(null)}>
          <p className="muted">
            Private reminder ·{" "}
            {new Date(selected.createdAt * 1000).toLocaleString()}
          </p>
          <RichText text={selected.event.body} />
          {selected.event.topic && (
            <a
              className="button"
              href={href("topics", {
                channel: selected.event.channel,
                topic: selected.event.topic,
              })}
            >
              Open linked topic
            </a>
          )}
        </Modal>
      )}
      {cursor && (
        <button
          className="load-more"
          disabled={loading}
          onClick={() => load(true)}
        >
          Load older notifications
        </button>
      )}
    </section>
  );
}
