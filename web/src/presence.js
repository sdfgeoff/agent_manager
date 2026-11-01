import { useEffect, useRef, useState } from "react";
import { api, events } from "./api";

export const presenceLabels = {
  active: "Active",
  away: "Away",
  offline: "Offline",
  idle: "Ready",
  working: "Working",
  waiting: "Waiting for input",
  paused: "Paused",
  error: "Needs attention",
  unknown: "Status unavailable",
};

export function usePresence(username, epoch) {
  const [actors, setActors] = useState({});
  const identity = useRef(null);
  useEffect(() => {
    setActors({});
    if (!username) return;
    const client = (identity.current ||= crypto.randomUUID());
    let live = true,
      loading = false,
      lastActivity = Date.now(),
      lastSuccess = 0,
      timer,
      heartbeatBusy = false,
      lastState;
    const active = () => !document.hidden && Date.now() - lastActivity < 120000;
    async function load() {
      if (!live || loading || document.hidden) return;
      loading = true;
      try {
        const value = await api("/api/presence");
        if (live) {
          lastSuccess = Date.now();
          setActors((old) =>
            JSON.stringify(old) === JSON.stringify(value.actors)
              ? old
              : value.actors,
          );
        }
      } catch {
        if (live && Date.now() - lastSuccess > 35000) setActors({});
      } finally {
        loading = false;
      }
    }
    async function heartbeat() {
      if (!live || heartbeatBusy) return;
      heartbeatBusy = true;
      const state = active();
      try {
        await api("/api/presence", { client, active: state });
        lastState = state;
      } catch {
      } finally {
        heartbeatBusy = false;
      }
    }
    const refresh = () => {
      clearTimeout(timer);
      timer = setTimeout(load, 100);
    };
    const event = ({ detail }) => {
      if (["presence_changed", "reconnected"].includes(detail?.type)) refresh();
    };
    const activity = () => {
      lastActivity = Date.now();
      if (lastState !== true) heartbeat();
    };
    const visibility = () => {
      if (!document.hidden) {
        lastActivity = Date.now();
        load();
      }
      heartbeat();
    };
    const tick = () => {
      heartbeat();
      load();
      if (Date.now() - lastSuccess > 35000) setActors({});
    };
    heartbeat();
    load();
    const interval = setInterval(tick, 15000);
    for (const name of ["pointerdown", "keydown", "touchstart", "scroll"])
      window.addEventListener(name, activity, { passive: true, capture: true });
    document.addEventListener("visibilitychange", visibility);
    window.addEventListener("pageshow", visibility);
    events.addEventListener("server", event);
    return () => {
      live = false;
      clearTimeout(timer);
      clearInterval(interval);
      for (const name of ["pointerdown", "keydown", "touchstart", "scroll"])
        window.removeEventListener(name, activity, { capture: true });
      document.removeEventListener("visibilitychange", visibility);
      window.removeEventListener("pageshow", visibility);
      events.removeEventListener("server", event);
    };
  }, [username, epoch]);
  return actors;
}
