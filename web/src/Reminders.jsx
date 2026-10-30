import React, { useEffect, useRef, useState } from "react";
import { AlarmClock, Plus, Pencil, X } from "lucide-react";
import { board, authGuard, href } from "./api";
import { useApp, useDraft } from "./context";
import { Modal, ErrorMessage, BusyButton } from "./components";

export function intervalLabel(seconds) {
  for (const [unit, size] of [
    ["week", 604800],
    ["day", 86400],
    ["hour", 3600],
    ["minute", 60],
    ["second", 1],
  ])
    if (seconds % size === 0) {
      const n = seconds / size;
      return `Every ${n} ${unit}${n === 1 ? "" : "s"}`;
    }
  return "";
}
function localDate(ms) {
  const d = new Date(ms);
  return new Date(ms - d.getTimezoneOffset() * 60000)
    .toISOString()
    .slice(0, 16);
}
function presetTime(preset) {
  const now = Date.now();
  if (preset === "tomorrow") {
    const d = new Date(now);
    d.setDate(d.getDate() + 1);
    d.setHours(9, 0, 0, 0);
    return localDate(d.getTime());
  }
  return localDate(now + (preset === "hour" ? 3600000 : 900000));
}
export function ReminderDialog({ reminder, topic, onClose, onSaved }) {
  const [message, setMessage] = useDraft(
    `reminder:${reminder?.id || "new"}:${topic || ""}`,
    reminder?.message || "",
  );
  const [when, setWhen] = useState(() =>
    reminder ? localDate(reminder.dueAt * 1000) : presetTime("15"),
  );
  const [preset, setPreset] = useState(reminder ? "custom" : "15");
  const [repeating, setRepeating] = useState(Boolean(reminder?.repeatSeconds));
  const [unit, setUnit] = useState(() => {
    const s = reminder?.repeatSeconds || 86400;
    return [604800, 86400, 3600, 60, 1].find((n) => s % n === 0) || 1;
  });
  const [amount, setAmount] = useState(() => {
    const s = reminder?.repeatSeconds || 86400;
    const n = [604800, 86400, 3600, 60, 1].find((n) => s % n === 0) || 1;
    return s / n;
  });
  const [linkedTopic, setLinkedTopic] = useState(
    reminder?.topic || topic || "",
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const submitting = useRef(false);
  const request = useRef(null);
  const date = new Date(when);
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  async function save(e) {
    e.preventDefault();
    if (submitting.current) return;
    setError(null);
    // Date() normalizes nonexistent DST times; reject those instead of silently moving.
    if (
      !Number.isFinite(date.getTime()) ||
      localDate(date.getTime()) !== when
    ) {
      setError(
        new Error("That local time does not exist. Choose another time."),
      );
      return;
    }
    const seconds = Math.floor(date.getTime() / 1000);
    if (seconds <= Date.now() / 1000) {
      setError(new Error("Choose a future time"));
      return;
    }
    const repeat = repeating ? Number(amount) * Number(unit) : null;
    if (
      repeating &&
      (!Number.isSafeInteger(repeat) || repeat < 1 || repeat > 31536000)
    ) {
      setError(new Error("Repeat interval must be 1 second to 365 days"));
      return;
    }
    const payload = {
      message: message.trim(),
      dueAt: seconds,
      repeatSeconds: repeat,
      topic: linkedTopic ? Number(linkedTopic) : null,
      ...(reminder ? { id: reminder.id } : {}),
    };
    const serialized = JSON.stringify(payload);
    if (request.current?.input !== serialized)
      request.current = { input: serialized, id: crypto.randomUUID() };
    const stillSignedIn = authGuard();
    submitting.current = true;
    setBusy(true);
    try {
      const saved = await board(
        reminder ? "update_reminder" : "create_reminder",
        { ...payload, requestId: request.current.id },
      );
      stillSignedIn();
      setMessage("");
      onSaved(saved);
      onClose();
    } catch (e) {
      setError(e);
    } finally {
      submitting.current = false;
      setBusy(false);
    }
  }
  return (
    <Modal
      title={reminder ? "Edit reminder" : "New reminder"}
      onClose={() => {
        if (!busy) onClose();
      }}
    >
      <form onSubmit={save} className="reminder-form">
        <p className="muted">
          Private to you. Delivery appears in your inbox and enabled browser
          notifications.
        </p>
        <label>
          Message
          <textarea
            aria-label="Reminder message"
            autoFocus
            required
            maxLength={20000}
            rows={4}
            value={message}
            onChange={(e) => setMessage(e.target.value)}
          />
        </label>
        <label>
          When
          <select
            value={preset}
            onChange={(e) => {
              setPreset(e.target.value);
              if (e.target.value !== "custom")
                setWhen(presetTime(e.target.value));
            }}
          >
            <option value="15">In 15 minutes</option>
            <option value="hour">In one hour</option>
            <option value="tomorrow">Tomorrow at 9am</option>
            <option value="custom">Custom date & time</option>
          </select>
        </label>
        <label>
          Date & time
          <input
            type="datetime-local"
            required
            value={when}
            onChange={(e) => {
              setPreset("custom");
              setWhen(e.target.value);
            }}
          />
        </label>
        <p className="muted">
          Timezone: {zone}.{" "}
          {Number.isFinite(date.getTime()) ? date.toString() : "Choose a time."}
        </p>
        <label className="checkbox-label">
          <input
            type="checkbox"
            checked={repeating}
            onChange={(e) => setRepeating(e.target.checked)}
          />
          Repeat
        </label>
        {repeating && (
          <>
            <div className="repeat-controls">
              <label>
                Every
                <input
                  type="number"
                  min={1}
                  max={31536000}
                  required
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                />
              </label>
              <label>
                Units
                <select
                  value={unit}
                  onChange={(e) => setUnit(Number(e.target.value))}
                >
                  <option value={1}>seconds</option>
                  <option value={60}>minutes</option>
                  <option value={3600}>hours</option>
                  <option value={86400}>days</option>
                  <option value={604800}>weeks</option>
                </select>
              </label>
            </div>
            <p className="muted">
              Fixed elapsed intervals. A day is 24 hours, including across
              daylight-saving changes. Missed repeats deliver once.
            </p>
          </>
        )}
        <label>
          Topic number (optional)
          <input
            type="number"
            min={1}
            value={linkedTopic}
            onChange={(e) => setLinkedTopic(e.target.value)}
            placeholder="Link context without posting to it"
          />
        </label>
        <ErrorMessage error={error} />
        <BusyButton type="submit" className="primary" busy={busy}>
          {reminder ? "Save reminder" : "Schedule reminder"}
        </BusyButton>
      </form>
    </Modal>
  );
}
export default function Reminders({ params }) {
  const app = useApp();
  const [items, setItems] = useState([]);
  const [cursor, setCursor] = useState(null);
  const [history, setHistory] = useState(false);
  const [editing, setEditing] = useState(undefined);
  const [error, setError] = useState(null);
  const [info, setInfo] = useState(null);
  const [loading, setLoading] = useState(false);
  const [cancelling, setCancelling] = useState(null);
  const seq = useRef(0);
  const opened = useRef(null);
  async function load(more = false) {
    const n = ++seq.current;
    setLoading(true);
    try {
      const r = await board("list_reminders", {
        history,
        ...(more ? { before: cursor } : {}),
      });
      if (n === seq.current) {
        setItems((old) => (more ? [...old, ...r.data] : r.data));
        setCursor(r.nextCursor);
        setError(null);
      }
    } catch (e) {
      if (n === seq.current) setError(e);
    } finally {
      if (n === seq.current) setLoading(false);
    }
  }
  useEffect(() => {
    load();
    return () => seq.current++;
  }, [history, app.revision]);
  useEffect(() => {
    const id = Number(params?.get("reminder"));
    if (!id || opened.current === id) return;
    opened.current = id;
    let alive = true;
    board("read_reminder", { id })
      .then((r) => {
        if (alive) {
          if (r.state === "active") setEditing(r);
          else {
            setHistory(true);
            setInfo(r.message);
          }
        }
      })
      .catch((e) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, [params?.get("reminder")]);
  async function cancel(r) {
    if (cancelling) return;
    setCancelling(r.id);
    try {
      await board("cancel_reminder", { id: r.id });
      setInfo("Reminder cancelled.");
      app.changed();
      await load();
    } catch (e) {
      setError(e);
    } finally {
      setCancelling(null);
    }
  }
  return (
    <section className="reminders-panel">
      <div className="section-heading">
        <h2>
          <AlarmClock size={20} /> Your reminders
        </h2>
        <button onClick={() => setEditing(null)}>
          <Plus size={16} />
          New reminder
        </button>
      </div>
      <label className="checkbox-label">
        <input
          type="checkbox"
          checked={history}
          onChange={(e) => setHistory(e.target.checked)}
        />
        Include completed & cancelled
      </label>
      <ErrorMessage error={error} />
      {info && (
        <p className="success" role="status">
          {info}
        </p>
      )}
      <div className="reminder-list">
        {items.map((r) => (
          <article className="reminder-row" key={r.id}>
            <div>
              <p>{r.message}</p>
              <small>
                {r.state === "active" ? "Next: " : r.state + " · "}
                {new Date(r.dueAt * 1000).toLocaleString()} ·{" "}
                {r.repeatSeconds ? intervalLabel(r.repeatSeconds) : "Once"}
                {r.topic && (
                  <>
                    {" "}
                    ·{" "}
                    <a href={href("topics", { topic: r.topic })}>
                      Topic #{r.topic}
                    </a>
                  </>
                )}
              </small>
            </div>
            {r.state === "active" && (
              <div className="row-actions">
                <button
                  className="icon"
                  aria-label={`Edit reminder ${r.id}`}
                  onClick={() => setEditing(r)}
                >
                  <Pencil size={16} />
                </button>
                <button
                  disabled={Boolean(cancelling)}
                  aria-label={`Cancel reminder ${r.id}`}
                  onClick={() => cancel(r)}
                >
                  <X size={16} />
                  Cancel
                </button>
              </div>
            )}
          </article>
        ))}
      </div>
      {!items.length && !loading && (
        <p className="muted">No {history ? "" : "upcoming "}reminders.</p>
      )}
      {cursor && (
        <button disabled={loading} onClick={() => load(true)}>
          Load more reminders
        </button>
      )}
      {editing !== undefined && (
        <ReminderDialog
          reminder={editing}
          onClose={() => setEditing(undefined)}
          onSaved={(r) => {
            setInfo(
              `Scheduled for ${new Date(r.dueAt * 1000).toLocaleString()}`,
            );
            app.changed();
            load();
          }}
        />
      )}
    </section>
  );
}
