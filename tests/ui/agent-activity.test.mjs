// Deterministic tests for the AI activity timing model. No browser, no DOM:
// this is the state machine the browser suite drives indirectly.

import assert from "node:assert/strict";
import test from "node:test";

import {
  TIMING,
  idleActivity,
  nextTransition,
  receiveActivity,
  tickActivity,
} from "../../ui/static/agent-activity.js";

function writing(at = 1_000, extra = {}) {
  return {
    phase: "writing",
    action: "upsert_slide",
    slide_id: "sld_a",
    slide_index: 0,
    ...extra,
  };
}

test("holds the active phase, then shows done, then goes idle", () => {
  const start = 1_000;
  let state = receiveActivity(idleActivity(), writing(start), start);
  assert.equal(state.phase, "writing");
  assert.equal(state.slideId, "sld_a");
  assert.equal(state.slideIndex, 0);

  state = receiveActivity(
    state,
    {
      phase: "done",
      action: "upsert_slide",
      slide_id: "sld_a",
      slide_index: 0,
    },
    start + 10
  );
  assert.equal(state.phase, "writing", "done waits for the minimum hold");

  const stillWriting = tickActivity(state, start + TIMING.minPhase - 1);
  assert.equal(stillWriting.phase, "writing");

  const done = tickActivity(state, start + TIMING.minPhase);
  assert.equal(done.phase, "done");
  assert.equal(done.slideId, "sld_a");

  const idle = tickActivity(done, done.doneUntil);
  assert.equal(idle.phase, "idle");
});

test("a late done arrives immediately and keeps the full confirmation", () => {
  const start = 1_000;
  let state = receiveActivity(idleActivity(), writing(start), start);
  const late = start + TIMING.minPhase + 500;
  state = receiveActivity(
    state,
    { phase: "done", action: "upsert_slide", slide_id: "sld_a" },
    late
  );
  assert.equal(state.phase, "done");
  assert.equal(state.doneUntil, late + TIMING.doneHold);
  assert.equal(state.hideAt, state.doneUntil);
});

test("an error overrides the active phase and expires on its own", () => {
  const start = 1_000;
  let state = receiveActivity(idleActivity(), writing(start), start);
  state = receiveActivity(
    state,
    { phase: "error", action: "upsert_slide", slide_id: "sld_a" },
    start + 20
  );
  assert.equal(state.phase, "error");
  assert.equal(state.hideAt, start + 20 + TIMING.errorHold);
  assert.equal(tickActivity(state, state.hideAt - 1).phase, "error");
  assert.equal(tickActivity(state, state.hideAt).phase, "idle");
});

test("a new active event replaces the done confirmation", () => {
  const start = 1_000;
  let state = receiveActivity(idleActivity(), writing(start), start);
  state = receiveActivity(
    state,
    { phase: "done", action: "upsert_slide", slide_id: "sld_a" },
    start + 10
  );
  const finished = tickActivity(
    tickActivity(state, start + TIMING.minPhase),
    start + TIMING.minPhase + TIMING.doneHold
  );
  assert.equal(finished.phase, "idle");

  const at = start + TIMING.minPhase + TIMING.doneHold + 100;
  const next = receiveActivity(
    finished,
    writing(at, { slide_id: "sld_b", slide_index: 1 }),
    at
  );
  assert.equal(next.phase, "writing");
  assert.equal(next.slideId, "sld_b");
  assert.equal(next.slideIndex, 1);
  assert.equal(next.activeUntil, at + TIMING.minPhase);
});

test("done without an active phase still renders a legible done state", () => {
  const now = 5_000;
  const state = receiveActivity(
    idleActivity(),
    { phase: "done", action: "create_presentation" },
    now
  );
  assert.equal(state.phase, "done");
  assert.equal(state.doneUntil, now + TIMING.doneHold);
  assert.equal(tickActivity(state, state.doneUntil).phase, "idle");
});

test("an explicit thinking event outlives a provider timeout and expires", () => {
  const now = 2_000;
  const state = receiveActivity(idleActivity(), { phase: "thinking" }, now);
  assert.equal(state.phase, "thinking");
  assert.ok(TIMING.thinkHold > 300_000, "thinking covers the 300s provider timeout");
  assert.equal(state.hideAt, now + TIMING.thinkHold);
  assert.equal(tickActivity(state, state.hideAt - 1).phase, "thinking");
  assert.equal(tickActivity(state, state.hideAt).phase, "idle");
});

test("nextTransition reports the next boundary and nothing when idle", () => {
  assert.equal(nextTransition(idleActivity(), 0), null);

  const state = receiveActivity(idleActivity(), writing(1_000), 1_000);
  assert.equal(nextTransition(state, 1_100), state.activeUntil);

  const pending = receiveActivity(
    state,
    { phase: "done", action: "upsert_slide", slide_id: "sld_a" },
    1_010
  );
  const done = tickActivity(pending, state.activeUntil);
  assert.equal(done.phase, "done");
  assert.equal(nextTransition(done, state.activeUntil + 1), done.doneUntil);
});

test("unknown phases and duplicate events do not corrupt the model", () => {
  const state = receiveActivity(idleActivity(), writing(1_000), 1_000);
  assert.equal(receiveActivity(state, { phase: "nonsense" }, 1_100), state);
  assert.equal(receiveActivity(state, null, 1_100), state);

  const more = receiveActivity(state, writing(1_500), 1_500);
  assert.equal(more.phase, "writing");
  assert.equal(more.slideIndex, 0);
  assert.equal(more.activeUntil, 1_500 + TIMING.minPhase);
});
