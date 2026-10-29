import React, { useEffect, useRef, useState } from "react";
import { Bell, CheckCheck, ArrowUpRight } from "lucide-react";
import { board, navigate, href } from "./api";
import { useApp } from "./context";
import { ErrorMessage } from "./components";
export default function Inbox({ params }) {
  const app = useApp();
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
    try {
      if (!n.read) await board("mark_read", { id: n.id });
      app.changed();
      navigate("topics", { channel: n.event.channel, topic: n.event.topic });
    } catch (e) {
      setError(e);
    }
  }
  useEffect(() => {
    const id = Number(params.get("notification"));
    if (id && clicked.current !== id) {
      const n = items.find((n) => n.id === id);
      if (n) {
        clicked.current = id;
        open(n);
      }
    }
  }, [items, params.toString()]);
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
                #{n.event.channelName} · topic #{n.event.topic}
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
