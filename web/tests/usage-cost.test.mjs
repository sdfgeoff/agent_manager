import test from "node:test";
import assert from "node:assert/strict";
import { estimateCost, costSeries } from "../src/usage-cost.js";
test("cached input is billed once and reasoning is already part of output", () => {
  assert.equal(estimateCost({ inputTokens: 1e6, cachedInputTokens: 0.5e6, outputTokens: 1e6, reasoningOutputTokens: 0.5e6 }, { input: 2, cachedInput: 1, output: 10 }), 11.5);
  assert.equal(estimateCost({inputTokens: 1, cachedInputTokens: 2, outputTokens: 0}, {input: 1, cachedInput: 1, output: 1}), null);
  assert.equal(estimateCost({}, null), null);
});
test("UTC day and month series preserve unknown gaps and observed zeros", () => {
  const rates={input: 1, cachedInput: 0.5, output: 2};
  const rows=[
    {day:"2026-10-31",inputTokens:1e6,cachedInputTokens:0,outputTokens:0,partial:true},
    {day:"2026-11-01",inputTokens:1e6,cachedInputTokens:0,outputTokens:0},
    {day:"2026-11-02",inputTokens:0,cachedInputTokens:0,outputTokens:0}
  ];
  const now=new Date("2026-11-04T00:00:00Z");
  const daily=costSeries(rows,rates,"daily",now);
  assert.equal(daily.length,30);
  assert.equal(daily.at(-1).date,"2026-11-04");
  assert.equal(daily.at(-1).value,null);
  assert.equal(daily.find(r=>r.date==="2026-11-02").value,0);
  const monthly=costSeries(rows,rates,"monthly",now);
  assert.equal(monthly.length,12);
  assert.equal(monthly.at(-1).value,1);
  assert.equal(monthly.at(-2).partial,true);
  assert.equal(monthly.at(-3).value,null);
});
