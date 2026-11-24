// Plain node:test unit tests — no test framework, runner config or new dependencies.
// Run with the repo's TypeScript compiler emitting CommonJS, e.g.:
//   npx tsc src/player/party.ts src/player/party.test.ts --outDir /tmp/party-test \
//     --module commonjs --moduleResolution bundler --target ES2022 --esModuleInterop --ignoreConfig
//   printf '{"type":"commonjs"}' > /tmp/party-test/package.json
//   node /tmp/party-test/party.test.js

// @ts-ignore - @types/node is intentionally not a project dependency
import { test } from "node:test";
// @ts-ignore - @types/node is intentionally not a project dependency
import assert from "node:assert/strict";
import { clockExtrapolate, driftDecision, formatRoomCode, randomRoomCode, type PartyState } from "./party";

const base: PartyState = {
  title: "Filme",
  positionSeconds: 100,
  playing: true,
  rate: 1,
  updatedAtServerSeconds: 1000,
  actorId: "host",
  seq: 1,
};

test("clockExtrapolate advances by age × rate", () => {
  assert.equal(clockExtrapolate(base, 1010), 110);
  assert.equal(clockExtrapolate({ ...base, rate: 1.5 }, 1004), 106);
  assert.equal(clockExtrapolate({ ...base, rate: 0.5 }, 1020), 110);
});

test("clockExtrapolate freezes when paused", () => {
  assert.equal(clockExtrapolate({ ...base, playing: false }, 1200), 100);
});

test("clockExtrapolate clamps age to 0..600 seconds", () => {
  assert.equal(clockExtrapolate(base, 900), 100); // future timestamp: no negative jump
  assert.equal(clockExtrapolate(base, 5000), 700); // 4000 s capped at 600 s
  assert.equal(clockExtrapolate({ ...base, rate: 2 }, 5000), 1300);
});

test("driftDecision ignores differences up to 0.3 s", () => {
  assert.deepEqual(driftDecision(100, 100.3, false), { action: "none" });
  assert.deepEqual(driftDecision(100, 99.7, false), { action: "none" });
});

test("driftDecision nudges rate by 5% or 10%", () => {
  assert.deepEqual(driftDecision(100, 101, false), { action: "rate", rate: 1.05 });
  assert.deepEqual(driftDecision(100, 99, false), { action: "rate", rate: 0.95 });
  assert.deepEqual(driftDecision(100, 103, false), { action: "rate", rate: 1.1 });
  assert.deepEqual(driftDecision(100, 97, false), { action: "rate", rate: 0.9 });
});

test("driftDecision jumps beyond 5 s behind or 4 s ahead", () => {
  assert.deepEqual(driftDecision(100, 105.1, false), { action: "jump", targetSeconds: 105.1 });
  assert.deepEqual(driftDecision(100, 95.9, false), { action: "jump", targetSeconds: 95.9 });
  assert.deepEqual(driftDecision(100, 105, false), { action: "rate", rate: 1.1 }); // boundary
  assert.deepEqual(driftDecision(100, 96, false), { action: "rate", rate: 0.9 }); // boundary
});

test("driftDecision uses a tighter 1.5 s threshold when paused", () => {
  assert.deepEqual(driftDecision(100, 101.5, true), { action: "none" });
  assert.deepEqual(driftDecision(100, 98.5, true), { action: "none" });
  assert.deepEqual(driftDecision(100, 101.6, true), { action: "jump", targetSeconds: 101.6 });
  assert.deepEqual(driftDecision(100, 98.4, true), { action: "jump", targetSeconds: 98.4 });
});

test("formatRoomCode normalizes input", () => {
  assert.equal(formatRoomCode("  ab-cd 12 "), "ABCD12");
  assert.equal(formatRoomCode("amor+filme"), "AMORFILME");
  assert.equal(formatRoomCode(""), "");
});

test("randomRoomCode produces short, unambiguous codes", () => {
  for (let i = 0; i < 50; i++) {
    const code = randomRoomCode();
    assert.match(code, /^[A-HJ-NP-Z2-9]{6}$/);
  }
  assert.equal(randomRoomCode(0).length, 1);
  assert.equal(randomRoomCode(10).length, 10);
});
