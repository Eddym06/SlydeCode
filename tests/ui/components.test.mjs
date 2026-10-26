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
import { ANIMATION_KINDS, THEME_CHOICES } from "../../ui/static/inspector.js";
import { canvasSize, placementFor } from "../../ui/static/canvas.js";
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

test("the inspector offers exactly the contract the backend accepts", () => {
  // These lists used to be duplicated per surface and drifted (6 of 12 kinds,
  // 3 of 6 themes). The inspector is now the single UI source; this test is the
  // tripwire against `slyde_core::ANIMATION_KIND_IDS` / `THEME_IDS`.
  assert.equal(ANIMATION_KINDS.length, 12, "every animation kind is offered");
  assert.equal(new Set(ANIMATION_KINDS).size, 12, "no duplicates");
  assert.ok(ANIMATION_KINDS.includes("fade_up"));
  assert.ok(ANIMATION_KINDS.includes("bounce_in"));

  assert.deepEqual(
    THEME_CHOICES.map((choice) => choice.id),
    ["studio", "paper", "night", "terminal", "academic", "pitch"]
  );
  for (const choice of THEME_CHOICES) {
    assert.equal(choice.dots.length, 2, `${choice.id} shows two palette dots`);
    assert.match(choice.dots[0], /^#[0-9A-Fa-f]{6}$/);
    assert.match(choice.dots[1], /^#[0-9A-Fa-f]{6}$/);
  }
});
test("a pasted image is centred at 60% of the canvas, whatever its shape", () => {
  // Landscape, portrait and a pathological panorama: the placement rule must keep
  // the picture inside the canvas and centred, because the stored geometry is
  // what the drag and resize operations start from.
  for (const [ratio, label] of [
    ["16:9", "landscape"],
    ["9:16", "portrait"],
    ["4:3", "classic"],
  ]) {
    const { w, h } = canvasSize(ratio);
    for (const natural of [
      { w: 1600, h: 900 },
      { w: 900, h: 1600 },
      { w: 4000, h: 400 },
      { w: 0, h: 0 },
    ]) {
      const rect = placementFor(natural, ratio);
      assert.ok(rect.w >= 16 && rect.h >= 16, `${label}: size stays usable`);
      assert.ok(rect.w <= w, `${label}: width stays on the canvas`);
      assert.ok(rect.h <= Math.round(h * 0.9) + 1, `${label}: height stays on the canvas`);
      assert.equal(rect.x, Math.round((w - rect.w) / 2), `${label}: horizontally centred`);
      assert.equal(rect.y, Math.round((h - rect.h) / 2), `${label}: vertically centred`);
    }
  }

  // A landscape photo gets its own aspect ratio, not the canvas's.
  const wide = placementFor({ w: 1600, h: 900 }, "16:9");
  assert.ok(Math.abs(wide.w / wide.h - 1600 / 900) < 0.02, "aspect ratio is preserved");
  assert.equal(wide.w, Math.round(1280 * 0.6));
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

// ---------- source editor, focus trap, layout previews ----------

test("the source highlighter escapes everything it prints", async () => {
  const { highlight } = await import("../../ui/static/code-editor.js");
  const hostile = `<h1 onclick="x()">A & B <script>alert("x")</script></h1><!-- <b> -->`;
  const html = highlight(hostile);
  assert.ok(!/<(?!\/?span)/.test(html), `only our own spans may be markup: ${html}`);
  assert.ok(html.includes("&lt;script&gt;") || html.includes("tok-tag\">script"));
  assert.ok(html.includes("A &amp; B"));
  // Stripping the spans gives back the escaped source: nothing is lost or invented.
  const text = html.replace(/<\/?span[^>]*>/g, "");
  const unescaped = text.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
  assert.equal(unescaped, hostile);
});

test("the highlighter colors tags, attributes, strings and css", async () => {
  const { highlight } = await import("../../ui/static/code-editor.js");
  const html = highlight(`<div class="a"><style>.a { color: #fff; margin: 4px; }</style></div>`);
  for (const kind of ["tag", "attr", "string", "selector", "prop", "number"]) {
    assert.ok(html.includes(`tok-${kind}`), `missing ${kind}`);
  }
});

test("Tab wraps at both ends of a dialog and lets go of unknown focus", async () => {
  const { wrapTarget } = await import("../../ui/static/focus-trap.js");
  const items = ["first", "middle", "last"];
  assert.equal(wrapTarget(items, "last", false), "first");
  assert.equal(wrapTarget(items, "first", true), "last");
  assert.equal(wrapTarget(items, "middle", false), null);
  assert.equal(wrapTarget(items, "outside", false), "first", "focus outside the dialog is pulled in");
  assert.equal(wrapTarget([], "x", false), null);
});

test("every theme has colors for the layout previews and every layout has a shape", async () => {
  const { THEME_COLORS, layoutPreview } = await import("../../ui/static/layout-previews.js");
  const { THEME_CHOICES } = await import("../../ui/static/inspector.js");
  for (const theme of THEME_CHOICES) assert.ok(THEME_COLORS[theme.id], `no colors for ${theme.id}`);
  for (const layout of ["hero", "bullet", "diagram", "chart", "image_layout", "code"]) {
    const svg = layoutPreview(layout, "paper");
    assert.ok(svg.includes("<rect"), `${layout} draws something`);
    assert.ok(svg.includes(THEME_COLORS.paper.bg), `${layout} wears the theme`);
  }
  assert.ok(layoutPreview("hero", "no-such-theme").includes(THEME_COLORS.studio.bg), "unknown theme falls back");
});

test("the element list speaks plain language and hides tags and handles", async () => {
  const { describeStructure, labelFor } = await import("../../ui/static/element-labels.js");
  const nodes = [
    { id: "a", tag: "DIV", text: "wrapper text" },
    { id: "b", tag: "H1", text: "Welcome" },
    { id: "c", tag: "UL", text: "" },
    { id: "d", tag: "LI", text: "one" },
    { id: "e", tag: "LI", text: "two" },
    { id: "f", tag: "LI", text: "three" },
    { id: "g", tag: "P", text: "Closing line" },
    { id: "h", tag: "STYLE", text: ".x{}" },
    { id: "i", tag: "BR", text: "" },
  ];
  const entries = describeStructure(nodes);
  const t = (key, vars = {}) => `${key}${vars.n === undefined ? "" : `:${vars.n}`}`;
  assert.deepEqual(
    entries.map((entry) => labelFor(entry, t)),
    ["element.text", "element.title", "element.list:3", "element.paragraph"]
  );
  assert.ok(entries.every((entry) => !/^(li|br|style)$/i.test(entry.tag)));
});

test("the work card reports real phases, never invented progress", async () => {
  const { expectedSlides, formatElapsed, lastThought, trackModel, workPhase } = await import(
    "../../ui/static/copilot-work.js"
  );
  assert.equal(expectedSlides("Make a 6 slide pitch"), 6);
  assert.equal(expectedSlides("hazme 8 diapositivas sobre clima"), 8);
  assert.equal(expectedSlides("make it better"), null);
  assert.equal(expectedSlides("500 slides"), null, "absurd counts are ignored");
  assert.equal(formatElapsed(83_000), "1:23");
  assert.equal(formatElapsed(-5), "0:00");

  assert.equal(workPhase({ text: "", thinking: "", tools: [] }).phase, "waiting");
  assert.equal(workPhase({ text: "", thinking: "Let me plan the deck", tools: [] }).phase, "thinking");
  const writing = workPhase({
    text: "",
    thinking: "x",
    tools: [{ name: "build_deck", status: "writing", progress: { items: 3, chars: 4200 } }],
  });
  assert.deepEqual([writing.phase, writing.slide], ["writing", 3]);
  assert.equal(workPhase({ text: "", tools: [{ name: "build_deck", status: "running" }] }).phase, "applying");
  assert.equal(workPhase({ text: "Done!", tools: [] }).phase, "replying");

  assert.equal(
    lastThought("First I will outline the story. Then I pick a palette that feels calm and confident for the audience."),
    "Then I pick a palette that feels calm and confident for the audience."
  );
  assert.ok(lastThought("a ".repeat(200) + "word").length <= 93);

  assert.deepEqual(trackModel({ expected: null, written: 2, current: true }), { segments: 0, done: 2, current: false });
  assert.deepEqual(trackModel({ expected: 6, written: 2, current: true }), { segments: 6, done: 2, current: true });
  assert.equal(trackModel({ expected: 6, written: 6, current: true }).current, false);
});
