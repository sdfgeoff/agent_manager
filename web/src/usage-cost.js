export function estimateCost(tokens, rates) {
  if (
    !rates ||
    ![rates.input, rates.cachedInput, rates.output].every(
      (n) => typeof n === "number" && Number.isFinite(n) && n >= 0,
    )
  )
    return null;
  const {
    inputTokens: input,
    cachedInputTokens: cached,
    outputTokens: output,
  } = tokens;
  if (
    ![input, cached, output].every((n) => Number.isFinite(n) && n >= 0) ||
    cached > input
  )
    return null;
  return (
    ((input - cached) * rates.input +
      cached * rates.cachedInput +
      output * rates.output) /
    1e6
  );
}
export function costSeries(rows, rates, period, now = new Date()) {
  const dates = Array.from(
    { length: period === "daily" ? 30 : 12 },
    (_, index) => {
      const d = new Date(
        Date.UTC(
          now.getUTCFullYear(),
          now.getUTCMonth(),
          period === "daily" ? now.getUTCDate() : 1,
        ),
      );
      if (period === "daily") d.setUTCDate(d.getUTCDate() - 29 + index);
      else d.setUTCMonth(d.getUTCMonth() - 11 + index);
      return d.toISOString().slice(0, period === "daily" ? 10 : 7);
    },
  );
  return dates.map((date) => {
    const values = rows.filter(
      (r) => (period === "daily" ? r.day : r.day.slice(0, 7)) === date,
    );
    const costs = values.map((r) => estimateCost(r, rates));
    return {
      date,
      value:
        values.length && costs.every((c) => c !== null)
          ? costs.reduce((a, b) => a + b, 0)
          : null,
      partial: values.some((r) => r.partial),
    };
  });
}
