/**
 * Viewport → tile topic subscription diffing.
 *
 * Given the tiles a map's viewport now covers, produce the set of MQTT topic
 * filters to subscribe and unsubscribe relative to the previous frame.
 */

import type { TileCoord } from "./types.js";

export function tileTopic(set: string, t: TileCoord): string {
  return `geo/${set}/${t.z}/${t.x}/${t.y}`;
}

export interface SubDiff {
  toSubscribe: string[];
  toUnsubscribe: string[];
}

export function diffSubscriptions(previous: Set<string>, next: Set<string>): SubDiff {
  const toSubscribe: string[] = [];
  const toUnsubscribe: string[] = [];
  for (const topic of next) {
    if (!previous.has(topic)) toSubscribe.push(topic);
  }
  for (const topic of previous) {
    if (!next.has(topic)) toUnsubscribe.push(topic);
  }
  return { toSubscribe, toUnsubscribe };
}
