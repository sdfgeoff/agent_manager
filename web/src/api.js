export class ApiError extends Error {
  constructor(message, status) {
    super(message);
    this.status = status;
  }
}
export const events = new EventTarget();
let authGeneration = 0;
export function newAuthScope() {
  authGeneration++;
}
export function authGuard() {
  const generation = authGeneration;
  return () => {
    if (generation !== authGeneration)
      throw new DOMException("Sign-in changed; action cancelled", "AbortError");
  };
}
export async function api(path, body, method, signal) {
  const generation = authGeneration;
  const response = await fetch(path, {
    method: method || (body === undefined ? "GET" : "POST"),
    headers: { "Content-Type": "application/json", "X-Agentmanager": "1" },
    credentials: "same-origin",
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  const result = await response.json().catch(() => ({}));
  if (!response.ok) {
    if (
      response.status === 401 &&
      path !== "/api/login" &&
      generation === authGeneration
    )
      window.dispatchEvent(new Event("auth-lost"));
    throw new ApiError(
      result.error || `Request failed (${response.status})`,
      response.status,
    );
  }
  return result;
}
const reads = new Set([
  "overview",
  "agent_activity",
  "search",
  "list_reminders",
  "read_reminder",
  "read_notification",
  "list_topics",
  "read_topic",
  "list_agents",
  "list_notifications",
  "read_attachment",
  "agent_subscriptions",
]);
export const board = (op, body = {}) =>
  api(
    `/api/board/${op}`,
    reads.has(op) ? body : { requestId: crypto.randomUUID(), ...body },
  );
export const rpc = (worker, method, params = {}) =>
  api(`/api/workers/${encodeURIComponent(worker)}/rpc`, { method, params });
export function friendly(error) {
  const text = error?.message || String(error);
  return text.includes("active writer")
    ? "This session is open in another Codex process. Close that client before sending or changing settings."
    : text;
}
export function route() {
  const [path, search = ""] = location.hash.replace(/^#/, "").split("?");
  return {
    page: path?.replace(/^\//, "") || "topics",
    params: new URLSearchParams(search),
  };
}
export function href(page, params = {}) {
  const query = new URLSearchParams(
    Object.entries(params).filter(
      ([, v]) => v !== null && v !== undefined && v !== "",
    ),
  );
  return `#/${page}${query.size ? "?" + query : ""}`;
}
export function navigate(page, params = {}) {
  location.hash = href(page, params);
}
