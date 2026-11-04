export const HISTORY_PAGE_SIZE = 50;
export const LIVE_ITEM_LIMIT = 100;
export const TEXT_PAGE_SIZE = 16000;

export async function historyPage(
  call,
  threadId,
  cursor = null,
  mode = "items",
) {
  if (mode === "items") {
    try {
      const page = await call("thread/items/list", {
        threadId,
        cursor,
        sortDirection: "desc",
        limit: HISTORY_PAGE_SIZE,
      });
      return {
        items: [...page.data].reverse().map((entry) => entry.item),
        nextCursor: page.nextCursor,
        mode,
      };
    } catch (error) {
      if (
        cursor ||
        !/(-32601|unsupported method|not supported|unknown.*method)/i.test(
          error.message,
        )
      )
        throw error;
      // Older Codex stores may not support item pagination. Keep this fallback
      // small and summarized; never fetch an entire long turn as a fallback.
      mode = "summary";
    }
  }
  const page = await call("thread/turns/list", {
    threadId,
    cursor,
    itemsView: "summary",
    sortDirection: "desc",
    limit: 5,
  });
  return {
    items: [...page.data].reverse().flatMap((turn) => turn.items || []),
    nextCursor: page.nextCursor,
    mode,
  };
}

export function mergeSessionEvents(items, messages) {
  const next = [...items];
  const positions = new Map(next.map((item, index) => [item.id, index]));
  for (const { method, params: p = {} } of messages) {
    const item = p.item;
    if (
      (method === "item/started" || method === "item/completed") &&
      item?.id
    ) {
      const i = positions.get(item.id);
      if (i === undefined) {
        positions.set(item.id, next.length);
        next.push(item);
      } else next[i] = item;
    } else if (method === "item/agentMessage/delta" && p.itemId) {
      const i = positions.get(p.itemId);
      if (i === undefined) {
        positions.set(p.itemId, next.length);
        next.push({ id: p.itemId, type: "agentMessage", text: p.delta || "" });
      } else
        next[i] = { ...next[i], text: (next[i].text || "") + (p.delta || "") };
    }
  }
  return next.slice(-LIVE_ITEM_LIMIT);
}
