// Component tests for the pure UI modules.
//
// The repo has no bundler and (deliberately) no npm dependencies, so there is no
// DOM runner: the components that carry real logic are kept free of the DOM and
// tested directly in Node. The browser-facing wiring is covered by the CDP suite.
//
// `presenter-core.js` exists because the studio overlay and the standalone
// presenter had two copies of this logic and they had already drifted once.

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  createReveal,
  keyToAction,
  progressPercent,
} from "../../ui/static/presenter-core.js";
import { formatClock, paceState, readingSeconds } from "../../ui/static/teleprompter.js";
import {
  idleActivity,
  nextTransition,
  receiveActivity,
  tickActivity,
} from "../../ui/static/agent-activity.js";

const deck = () => [
  { id: "a", code: "<h1>A</h1>", animation: [{ kind: "fade_up", target: ".t" }, { kind: "fade", target: ".s" }] },
  { id: "b", code: "<h1>B</h1>", animation: [] },
  { id: "c", code: "<h1>C</h1>", animation: [{ kind: "typewriter", target: ".t" }] },
];

test("a fresh slide plays its first step as an intro, once", () => {
  const reveal = createReveal(deck());
  assert.equal(reveal.index, 0);
  assert.equal(reveal.key, "a:10:2");
  assert.equal(reveal.pending, 2);

  // Nothing may play before the frame says it is listening.
  assert.equal(reveal.takeIntro(), null);
  assert.equal(reveal.pending, 2);

  reveal.markReady();
  const intro = reveal.takeIntro();
  assert.equal(intro.kind, "fade_up");
  assert.equal(reveal.pending, 1, "the intro consumed exactly one step");
  assert.equal(reveal.takeIntro(), null, "the intro never replays");
});

test("advancing spends steps, then moves to the next slide, then stops", () => {
  const reveal = createReveal(deck());
  reveal.markReady();
  reveal.takeIntro();

  const step = reveal.advance();
  assert.equal(step.type, "step");
  assert.equal(step.step.kind, "fade");
  assert.equal(reveal.index, 0);

  const next = reveal.advance();
  assert.equal(next.type, "slide");
  assert.equal(next.index, 1);
  assert.equal(next.slide.id, "b");
  assert.equal(reveal.key, "b:10:0", "the new slide has its own handle");

  // A slide without steps advances immediately on the next input.
  const third = reveal.advance();
  assert.equal(third.type, "slide");
  assert.equal(third.index, 2);

  // The new document is loading, so the first input waits; once it reports ready
  // the queued step plays and the next input ends the deck.
  assert.deepEqual(reveal.advance(), { type: "wait" });
  reveal.markReady();
  assert.equal(reveal.advance().type, "step");
  assert.deepEqual(reveal.advance(), { type: "end" }, "the last slide ends the deck");
  assert.equal(reveal.index, 2, "the end does not wrap around");
});

test("an advance while the frame is not ready waits instead of eating a step", () => {
  const reveal = createReveal(deck());
  assert.deepEqual(reveal.advance(), { type: "wait" });
  assert.equal(reveal.pending, 2, "no step was spent");
});

test("navigation clamps at both ends and jumping to the current slide is a no-op", () => {
  const reveal = createReveal(deck());
  assert.equal(reveal.goTo(-5), null, "already at the first slide");
  assert.equal(reveal.back(), null);

  const last = reveal.goTo(99);
  assert.equal(last.index, 2);
  assert.equal(last.slide.id, "c");
  reveal.markReady();
  assert.equal(reveal.advance().type, "step");
  assert.equal(reveal.advance().type, "end");
  assert.equal(reveal.goTo(2), null);
  assert.equal(reveal.goTo(2, { force: true }).index, 2, "force reloads the same slide");
});

test("back and first/last return slide actions", () => {
  const reveal = createReveal(deck());
  reveal.goTo(2);
  const back = reveal.back();
  assert.equal(back.type, "slide");
  assert.equal(back.index, 1);
  assert.equal(reveal.first().index, 0);
  assert.equal(reveal.last().index, 2);
});

test("reload resets the queue and the intro for the same slide", () => {
  const reveal = createReveal(deck());
  reveal.markReady();
  reveal.takeIntro();
  reveal.advance();
  assert.equal(reveal.pending, 0);

  reveal.reload();
  assert.equal(reveal.index, 0);
  assert.equal(reveal.pending, 2, "the steps are back");
  assert.equal(reveal.ready, false, "a reloaded document is not ready yet");
  assert.equal(reveal.takeIntro(), null);
});

test("setSlides keeps the position valid and copes with an empty deck", () => {
  const reveal = createReveal(deck());
  reveal.goTo(2);
  reveal.setSlides(deck().slice(0, 1));
  assert.equal(reveal.index, 0);
  assert.equal(reveal.slide.id, "a");

  reveal.setSlides([]);
  assert.equal(reveal.slide, null);
  assert.deepEqual(reveal.advance(), { type: "end" });
  assert.equal(reveal.goTo(1), null);
  reveal.markReady();
  assert.equal(reveal.takeIntro(), null);
});

test("a malformed slide never throws", () => {
  const reveal = createReveal([{ id: "x" }, null, { id: "z", animation: "nope" }]);
  assert.equal(reveal.pending, 0);
  reveal.markReady();
  assert.equal(reveal.takeIntro(), null);
  assert.equal(reveal.advance().type, "slide");
  assert.equal(reveal.advance().index, 2);
  assert.equal(reveal.advance().type, "end");
});

test("every presenter key maps to exactly one action", () => {
  const cases = [
    [{ key: "ArrowRight" }, "advance"],
    [{ key: " " }, "advance"],
    [{ key: "PageDown" }, "advance"],
    [{ key: "Enter" }, "advance"],
    [{ key: "ArrowLeft" }, "back"],
    [{ key: "PageUp" }, "back"],
    [{ key: "Backspace" }, "back"],
    [{ key: "Home" }, "first"],
    [{ key: "End" }, "last"],
    [{ key: "b" }, "blackout"],
    [{ key: "B" }, "blackout"],
    [{ key: "." }, "blackout"],
    [{ key: "s" }, "speaker"],
    [{ key: "S" }, "speaker"],
    [{ key: "f" }, "fullscreen"],
    [{ key: "Escape" }, "escape"],
    [{ key: "q" }, null],
    [{ key: "ArrowUp" }, null],
    [{}, null],
    [null, null],
  ];
  for (const [event, expected] of cases) {
    assert.equal(keyToAction(event), expected, `key ${JSON.stringify(event)}`);
  }
});

test("progress is a percentage of the deck, never out of range", () => {
  assert.equal(progressPercent(0, 4), 25);
  assert.equal(progressPercent(3, 4), 100);
  assert.equal(progressPercent(9, 4), 100, "clamped");
  assert.equal(progressPercent(0, 0), 0);
  assert.equal(progressPercent(0, undefined), 0);
});

test("speaker pacing estimates reading time and reports the pace", () => {
  assert.equal(readingSeconds(undefined), 0);
  assert.equal(readingSeconds(""), 0);
  // 130 words per minute: 65 words is half a minute.
  const words = Array.from({ length: 65 }, () => "palabra").join(" ");
  assert.ok(Math.abs(readingSeconds(words) - 30) <= 1, `got ${readingSeconds(words)}`);

  assert.equal(paceState(10, 40), "fast");
  assert.equal(paceState(40, 40), "on");
  assert.equal(paceState(70, 40), "slow");
  assert.equal(paceState(0, 0), "none");
  assert.equal(formatClock(0), "0:00");
  assert.equal(formatClock(65), "1:05");
});

test("the activity state machine still reports every phase", () => {
  const idle = idleActivity();
  const writing = receiveActivity(idle, {
    type: "agent_activity",
    phase: "writing",
    action: "upsert_slide",
    slide_index: 1,
  });
  assert.equal(writing.phase, "writing");
  assert.equal(writing.slideIndex, 1);

  // A mutation is instantaneous, so `done` is held back until the active phase
  // has been legible for the minimum hold; that is the whole point of the module.
  const start = 1_000;
  const active = receiveActivity(idleActivity(), {
    type: "agent_activity",
    phase: "writing",
    action: "upsert_slide",
    slide_index: 1,
  }, start);
  const queued = receiveActivity(active, {
    type: "agent_activity",
    phase: "done",
    action: "upsert_slide",
  }, start + 50);
  assert.equal(queued.pendingDone, true, "done waits for the minimum hold");
  const doneState = tickActivity(queued, start + 1_200);
  assert.equal(doneState.phase, "done");
  assert.equal(tickActivity(doneState, start + 1_200 + 3_000).phase, "idle");
  assert.ok(nextTransition(idle) >= 0);
});
