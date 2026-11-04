import test from "node:test";
import assert from "node:assert/strict";
import {
  historyPage,
  mergeSessionEvents,
  LIVE_ITEM_LIMIT,
} from "../src/sessionHistory.js";

test("history pages bound item requests and preserve opaque cursors and chronological display", async () => {
  const calls = [];
  const result = await historyPage(
    async (method, params) => {
      calls.push({ method, params });
      return {
        data: [
          { turnId: "long-turn", item: { id: "new" } },
          { turnId: "long-turn", item: { id: "old" } },
        ],
        nextCursor: "opaque-next",
      };
    },
    "session",
    "opaque-current",
  );
  assert.deepEqual(
    result.items.map((x) => x.id),
    ["old", "new"],
  );
  assert.equal(calls[0].method, "thread/items/list");
  assert.equal(calls[0].params.limit, 50);
  assert.equal(calls[0].params.cursor, "opaque-current");
  assert.equal(result.nextCursor, "opaque-next");
});

test("unsupported item stores use small summaries, never full turns, and other errors stay errors", async () => {
  const methods = [];
  const call = async (method, params) => {
    methods.push(method);
    if (method === "thread/items/list")
      throw new Error(
        '{"code":-32601,"message":"thread/items/list is not supported yet"}',
      );
    assert.equal(params.itemsView, "summary");
    assert.equal(params.limit, 5);
    return { data: [{ items: [{ id: "summary" }] }], nextCursor: "turns-next" };
  };
  const result = await historyPage(call, "session");
  assert.equal(result.mode, "summary");
  assert.deepEqual(result.items, [{ id: "summary" }]);
  await historyPage(call, "session", result.nextCursor, result.mode);
  assert.deepEqual(methods, [
    "thread/items/list",
    "thread/turns/list",
    "thread/turns/list",
  ]);
  await assert.rejects(
    historyPage(async () => {
      throw new Error("Disconnected");
    }, "session"),
    /Disconnected/,
  );
});

test("long live turns stay bounded while batched deltas and completion replace the correct item", () => {
  const messages = Array.from({ length: 5000 }, (_, i) => ({
    method: "item/completed",
    params: {
      item: {
        id: "tool-" + i,
        type: "commandExecution",
        aggregatedOutput: "output",
      },
    },
  }));
  messages.push(
    {
      method: "item/agentMessage/delta",
      params: { itemId: "answer", delta: "Hello " },
    },
    {
      method: "item/agentMessage/delta",
      params: { itemId: "answer", delta: "world" },
    },
  );
  const merged = mergeSessionEvents([], messages);
  assert.equal(merged.length, LIVE_ITEM_LIMIT);
  assert.equal(merged.at(-1).text, "Hello world");
  const completed = {
    id: "answer",
    type: "agentMessage",
    text: "Final answer",
  };
  const next = mergeSessionEvents(merged, [
    { method: "item/completed", params: { item: completed } },
  ]);
  assert.equal(next.length, LIVE_ITEM_LIMIT);
  assert.equal(next.at(-1), completed);
  assert.equal(
    merged.at(-1).text,
    "Hello world",
    "input snapshot remains unchanged",
  );
});
