// No private API responses or page content are cached by this service worker.
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) =>
  event.waitUntil(self.clients.claim()),
);
self.addEventListener("push", (event) => {
  let data = {};
  try {
    data = event.data?.json() || {};
  } catch {}
  const target = new URL(data.url || "/#/inbox", self.location.origin);
  const url =
    target.origin === self.location.origin
      ? target.href
      : new URL("/#/inbox", self.location.origin).href;
  event.waitUntil(
    self.registration.showNotification("Agent Manager", {
      body: "New activity in your inbox",
      icon: "/icon-192.png",
      badge: "/icon-192.png",
      tag: "agent-manager-inbox",
      data: { url },
      renotify: false,
    }),
  );
});
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const target = new URL(
    event.notification.data?.url || "/#/inbox",
    self.location.origin,
  );
  const url =
    target.origin === self.location.origin
      ? target.href
      : new URL("/#/inbox", self.location.origin).href;
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({
        type: "window",
        includeUncontrolled: true,
      });
      for (const client of windows) {
        if (new URL(client.url).origin === self.location.origin) {
          await client.navigate(url);
          await client.focus();
          return;
        }
      }
      await self.clients.openWindow(url);
    })(),
  );
});
