import React, { useEffect, useRef, useState } from "react";
import {
  Bell,
  BellOff,
  KeyRound,
  UserPlus,
  Trash2,
  ShieldCheck,
} from "lucide-react";
import { api } from "./api";
import { useApp } from "./context";
import { ErrorMessage, BusyButton } from "./components";
function needsHomeScreen() {
  return (
    (/iPad|iPhone|iPod/.test(navigator.userAgent) ||
      (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1)) &&
    !matchMedia("(display-mode: standalone)").matches &&
    !navigator.standalone
  );
}
function supported() {
  return (
    window.isSecureContext &&
    "serviceWorker" in navigator &&
    "PushManager" in window &&
    "Notification" in window
  );
}
async function registration() {
  return navigator.serviceWorker.register("/sw.js", { scope: "/" });
}
function bytes(key) {
  return Uint8Array.from(atob(key.replace(/-/g, "+").replace(/_/g, "/")), (c) =>
    c.charCodeAt(0),
  );
}
export async function revokeLocalPush() {
  if (!("serviceWorker" in navigator)) return;
  try {
    const reg = await navigator.serviceWorker.getRegistration("/");
    const subscription = await reg?.pushManager.getSubscription();
    if (subscription) await subscription.unsubscribe();
  } catch {}
}
function PushSettings() {
  const [status, setStatus] = useState({ enabled: false });
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  const [permission, setPermission] = useState(
    "Notification" in window ? Notification.permission : "unsupported",
  );
  const [info, setInfo] = useState("");
  const sequence = useRef(0);
  async function load() {
    const version = ++sequence.current;
    if (!supported()) return;
    try {
      const reg = await registration();
      const sub = await reg.pushManager.getSubscription();
      const result = sub
        ? await api("/api/push/status", { endpoint: sub.endpoint })
        : { enabled: false };
      if (sequence.current === version) setStatus(result);
    } catch (e) {
      setError(e);
    }
  }
  useEffect(() => {
    load();
    return () => sequence.current++;
  }, []);
  async function toggle() {
    if (busy) return;
    sequence.current++;
    setBusy(true);
    setError(null);
    setInfo("");
    try {
      if (!status.enabled) {
        const granted = await Notification.requestPermission();
        setPermission(granted);
        if (granted !== "granted")
          throw new Error(
            granted === "denied"
              ? "Notifications are blocked. Allow them in this browser’s site settings, then try again."
              : "Notification permission was not granted.",
          );
        const config = await api("/api/push");
        const reg = await registration();
        await navigator.serviceWorker.ready;
        let subscription = await reg.pushManager.getSubscription();
        if (!subscription)
          subscription = await reg.pushManager.subscribe({
            userVisibleOnly: true,
            applicationServerKey: bytes(config.publicKey),
          });
        await api("/api/push", subscription.toJSON());
        setStatus({ enabled: true });
        setInfo(
          "Enabled for this browser. Notifications can arrive without an open tab.",
        );
      } else {
        const reg = await registration();
        const subscription = await reg.pushManager.getSubscription();
        if (subscription) {
          await api("/api/push/unsubscribe", {
            endpoint: subscription.endpoint,
          });
          await subscription.unsubscribe();
        }
        setStatus({ enabled: false });
        setInfo("Notifications disabled for this browser.");
      }
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="settings-card">
      <div className="section-title">
        <Bell size={20} />
        <h2>Inbox notifications</h2>
      </div>
      <p>
        Receive Web Push notifications when there’s new activity, even with no
        Agent Manager tab open. Clicking a notification opens your inbox.
        Message text stays off the lock screen.
      </p>
      {!supported() || needsHomeScreen() ? (
        <p className="info">
          Web Push isn’t available in this browser or context. Use HTTPS and a
          browser with Push API support. On iPhone or iPad, add Agent Manager to
          your Home Screen and open it there before enabling notifications.
        </p>
      ) : (
        <>
          <p className="muted">
            This browser:{" "}
            <strong>{status.enabled ? "Enabled" : "Disabled"}</strong>
            {permission === "denied" ? " · permission blocked" : ""}
          </p>
          <BusyButton
            busy={busy}
            onClick={toggle}
            className={status.enabled ? "" : "primary"}
          >
            {status.enabled ? <BellOff size={16} /> : <Bell size={16} />}{" "}
            {status.enabled ? "Disable notifications" : "Enable notifications"}
          </BusyButton>
          {status.error && <p className="error">{status.error}</p>}
        </>
      )}
      <ErrorMessage error={error} />
      {info && (
        <p className="success" role="status">
          {info}
        </p>
      )}
      <p className="field-hint">
        Delivery depends on browser/OS settings and connectivity; force-quitting
        a browser may stop delivery. Signing out disables this browser’s
        registration. Password changes revoke existing registrations. No private
        API pages are cached for offline use.
      </p>
      <a className="text-link" href="/manifest.webmanifest" hidden>
        App manifest
      </a>
    </section>
  );
}
export default function Settings() {
  const app = useApp();
  const [users, setUsers] = useState([]);
  const [error, setError] = useState(null);
  const [current, setCurrent] = useState("");
  const [password, setPassword] = useState("");
  const [repeat, setRepeat] = useState("");
  const [changing, setChanging] = useState(false);
  const [notice, setNotice] = useState("");
  const [name, setName] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [adding, setAdding] = useState(false);
  const [pushKey, setPushKey] = useState(0);
  async function load() {
    try {
      setUsers(await api("/api/settings/users"));
    } catch (e) {
      setError(e);
    }
  }
  useEffect(() => {
    load();
  }, []);
  async function change(e) {
    e.preventDefault();
    if (changing) return;
    setError(null);
    setNotice("");
    if (password !== repeat) {
      setError(new Error("The new passwords do not match."));
      return;
    }
    setChanging(true);
    try {
      await api("/api/settings/password", {
        current_password: current,
        new_password: password,
      });
      setCurrent("");
      setPassword("");
      setRepeat("");
      await revokeLocalPush();
      setPushKey((k) => k + 1);
      app.reconnect();
      setNotice(
        "Password changed. Other browser sessions were signed out; enable push again if you want notifications.",
      );
    } catch (e) {
      setError(e);
    } finally {
      setChanging(false);
    }
  }
  async function add(e) {
    e.preventDefault();
    if (adding) return;
    setAdding(true);
    setError(null);
    try {
      await api("/api/settings/users", {
        username: name,
        password: newPassword,
      });
      setName("");
      setNewPassword("");
      await load();
      await app.refresh();
    } catch (e) {
      setError(e);
    } finally {
      setAdding(false);
    }
  }
  return (
    <section className="page settings-page">
      <div className="page-heading">
        <div>
          <p className="eyebrow">ACCOUNT & PREFERENCES</p>
          <h1>Settings</h1>
          <p>
            Signed in as <strong>{app.me.username}</strong>.
          </p>
        </div>
      </div>
      <ErrorMessage error={error} />
      {notice && (
        <p className="success" role="status">
          {notice}
        </p>
      )}
      <div className="settings-grid">
        <section className="settings-card">
          <div className="section-title">
            <KeyRound size={20} />
            <h2>Change your password</h2>
          </div>
          <form onSubmit={change}>
            <label>
              Current password
              <input
                type="password"
                autoComplete="current-password"
                required
                value={current}
                onChange={(e) => setCurrent(e.target.value)}
              />
            </label>
            <label>
              New password
              <input
                type="password"
                autoComplete="new-password"
                required
                minLength={12}
                maxLength={1024}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </label>
            <label>
              Repeat new password
              <input
                type="password"
                autoComplete="new-password"
                required
                minLength={12}
                maxLength={1024}
                value={repeat}
                onChange={(e) => setRepeat(e.target.value)}
              />
            </label>
            <BusyButton className="primary" type="submit" busy={changing}>
              Change password
            </BusyButton>
          </form>
        </section>
        <PushSettings key={pushKey} />
        <section className="settings-card user-settings">
          <div className="section-title">
            <ShieldCheck size={20} />
            <h2>Users</h2>
          </div>
          <p>
            All human accounts share the workspace and can manage users and
            agents.
          </p>
          <div className="user-list">
            {users.map((user) => (
              <div key={user.username}>
                <strong>
                  {user.username}
                  {user.username === app.me.username ? " (you)" : ""}
                </strong>
                <small>
                  Joined {new Date(user.created_at * 1000).toLocaleDateString()}
                </small>
                <button
                  className="icon danger"
                  aria-label={`Remove ${user.username}`}
                  disabled={users.length <= 1}
                  onClick={async () => {
                    if (
                      !confirm(
                        `Remove ${user.username}? Their active sign-ins and push registrations will be revoked.`,
                      )
                    )
                      return;
                    try {
                      const result = await api(
                        `/api/settings/users/${encodeURIComponent(user.username)}`,
                        undefined,
                        "DELETE",
                      );
                      if (result.signedOut) {
                        app.signedOut();
                        return;
                      }
                      await load();
                      app.refresh();
                    } catch (e) {
                      setError(e);
                    }
                  }}
                >
                  <Trash2 size={16} />
                </button>
              </div>
            ))}
          </div>
          <h3>
            <UserPlus size={16} />
            Add a user
          </h3>
          <form onSubmit={add}>
            <div className="form-columns">
              <label>
                Username
                <input
                  autoComplete="off"
                  required
                  maxLength={64}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
              <label>
                Initial password
                <input
                  type="password"
                  autoComplete="new-password"
                  required
                  minLength={12}
                  maxLength={1024}
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                />
              </label>
            </div>
            <BusyButton type="submit" className="primary" busy={adding}>
              <UserPlus size={15} />
              Add user
            </BusyButton>
          </form>
        </section>
      </div>
    </section>
  );
}
