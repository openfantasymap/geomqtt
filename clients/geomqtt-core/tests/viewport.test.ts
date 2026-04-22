import { describe, expect, test } from "vitest";
import { diffSubscriptions, tileTopic } from "../src/viewport.js";

describe("tileTopic", () => {
  test("formats the slippy-tile MQTT topic", () => {
    expect(tileTopic("vehicles", { z: 10, x: 544, y: 370 })).toBe(
      "geo/vehicles/10/544/370",
    );
  });
});

describe("diffSubscriptions", () => {
  test("reports all next as toSubscribe when previous is empty", () => {
    const prev = new Set<string>();
    const next = new Set(["a", "b"]);
    const { toSubscribe, toUnsubscribe } = diffSubscriptions(prev, next);
    expect(toSubscribe.sort()).toEqual(["a", "b"]);
    expect(toUnsubscribe).toEqual([]);
  });

  test("reports all prev as toUnsubscribe when next is empty", () => {
    const { toSubscribe, toUnsubscribe } = diffSubscriptions(
      new Set(["a", "b"]),
      new Set<string>(),
    );
    expect(toSubscribe).toEqual([]);
    expect(toUnsubscribe.sort()).toEqual(["a", "b"]);
  });

  test("overlap is preserved on both sides", () => {
    const prev = new Set(["a", "b", "c"]);
    const next = new Set(["b", "c", "d"]);
    const { toSubscribe, toUnsubscribe } = diffSubscriptions(prev, next);
    expect(toSubscribe).toEqual(["d"]);
    expect(toUnsubscribe).toEqual(["a"]);
  });

  test("no-op when sets are equal", () => {
    const prev = new Set(["x", "y"]);
    const next = new Set(["x", "y"]);
    const { toSubscribe, toUnsubscribe } = diffSubscriptions(prev, next);
    expect(toSubscribe).toEqual([]);
    expect(toUnsubscribe).toEqual([]);
  });
});
