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
    ["studio", "paper", "night", "original", "academic", "pitch"]
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

test("transitions have keyframes for every kind, mirror by direction and cut cleanly", async () => {
  const { keyframesFor, transitionOf, TRANSITION_KINDS } = await import("../../ui/static/transitions.js");
  assert.equal(keyframesFor("none"), null);
  assert.equal(keyframesFor("unknown"), null);
  for (const kind of TRANSITION_KINDS.filter((kind) => kind !== "none")) {
    const frames = keyframesFor(kind, 1);
    assert.ok(frames.leave.length === 2 && frames.enter.length === 2, `${kind} has two halves`);
  }
  const forward = keyframesFor("push", 1);
  const back = keyframesFor("push", -1);
  assert.equal(forward.enter[0].transform, "translateX(100%)");
  assert.equal(back.enter[0].transform, "translateX(-100%)", "going back mirrors the motion");

  assert.equal(transitionOf({}), null);
  assert.equal(transitionOf({ transition: { kind: "none", duration_ms: 400 } }), null);
  assert.deepEqual(transitionOf({ transition: { kind: "fade", duration_ms: 50 } }), { kind: "fade", duration: 120 });
  assert.equal(transitionOf({ transition: { kind: "zoom", duration_ms: 99999 } }).duration, 2000);
  assert.equal(transitionOf({ transition: { kind: "spin", duration_ms: 400 } }), null);
});

test("the elements catalog is safe, unique and fits the canvas", async () => {
  const { CATALOG, CATEGORIES, centeredPlacement } = await import("../../ui/static/elements-catalog.js");
  const keys = CATALOG.map((item) => `${item.category}/${item.id}`);
  assert.equal(new Set(keys).size, keys.length, "no duplicate ids per category");
  for (const category of CATEGORIES) {
    assert.ok(CATALOG.some((item) => item.category === category), `${category} has items`);
  }
  for (const item of CATALOG) {
    assert.ok(!/<script|javascript:|\son\w+\s*=/i.test(item.html), `${item.category}/${item.id} is plain markup`);
    assert.ok(item.w > 0 && item.h > 0);
  }
  const big = CATALOG.find((item) => item.id === "4x4");
  const place = centeredPlacement(big, { w: 1280, h: 720 });
  assert.ok(place.x >= 0 && place.y >= 0 && place.x + place.w <= 1280 && place.y + place.h <= 720);
  const tiny = centeredPlacement({ w: 100, h: 50 }, { w: 1280, h: 720 });
  assert.deepEqual([tiny.w, tiny.h], [100, 50], "small blocks keep their size");
  assert.equal(tiny.x, 590);
});

test("align and distribute keep every block inside the canvas", async () => {
  const { arrangeBoxes } = await import("../../ui/static/canvas.js");
  const canvas = { w: 1280, h: 720 };
  const boxes = [
    { id: "a", x: 100, y: 50, w: 200, h: 100 },
    { id: "b", x: 400, y: 300, w: 100, h: 60 },
    { id: "c", x: 900, y: 500, w: 300, h: 120 },
  ];
  const at = (rows, id) => rows.find((row) => row.id === id);
  const left = arrangeBoxes("arrange-left", boxes, canvas);
  assert.deepEqual(left.map((row) => row.x), [100, 100, 100]);
  const right = arrangeBoxes("arrange-right", boxes, canvas);
  assert.deepEqual(right.map((row) => row.x + row.w), [1200, 1200, 1200]);
  const center = arrangeBoxes("arrange-center", boxes, canvas);
  assert.equal(at(center, "a").x + 100, 650, "centered on the middle of the union");
  assert.deepEqual(arrangeBoxes("arrange-top", boxes, canvas).map((row) => row.y), [50, 50, 50]);

  const spread = arrangeBoxes("distribute-h", boxes, canvas);
  const gapAB = at(spread, "b").x - (at(spread, "a").x + at(spread, "a").w);
  const gapBC = at(spread, "c").x - (at(spread, "b").x + at(spread, "b").w);
  assert.ok(Math.abs(gapAB - gapBC) <= 1, `equal gaps: ${gapAB} vs ${gapBC}`);
  assert.equal(at(spread, "a").x, 100);
  assert.equal(at(spread, "c").x, 900, "the outer blocks stay put");

  const outside = arrangeBoxes("arrange-right", [{ id: "x", x: 1200, y: 10, w: 200, h: 20 }, { id: "y", x: 0, y: 0, w: 50, h: 50 }], canvas);
  assert.ok(outside.every((row) => row.x >= 0 && row.x + row.w <= 1280), "clamped to the canvas");
  assert.equal(arrangeBoxes("spin", boxes, canvas), null);
});

test("the wizard picks a writing model and never an embedding or audio one", async () => {
  const { recommendedModel } = await import("../../ui/static/onboarding-models.js");
  const ids = (...list) => list.map((id) => ({ id }));
  assert.equal(recommendedModel("deepseek", ids("deepseek-reasoner", "deepseek-chat")), "deepseek-chat");
  assert.equal(
    recommendedModel("openai", ids("text-embedding-3-large", "gpt-4o-mini", "gpt-4o", "whisper-1", "gpt-4.1")),
    "gpt-4.1"
  );
  assert.equal(
    recommendedModel("anthropic", ids("claude-3-5-haiku-latest", "claude-sonnet-4-20250514", "claude-sonnet-4-5")),
    "claude-sonnet-4-5",
    "the newest of the preferred family"
  );
  assert.equal(recommendedModel("gemini", ids("gemini-2.0-flash", "gemini-2.5-flash", "embedding-001")), "gemini-2.5-flash");
  assert.equal(recommendedModel("openrouter", ids("a/b", "vendor/model:free")), "vendor/model:free");
  assert.equal(recommendedModel("unknown", ids("tts-1", "some-model", "other")), "some-model");
  assert.equal(recommendedModel("openai", []), "");
  assert.equal(recommendedModel("openai", ids("whisper-1", "tts-1")), "", "nothing suitable means no pick");
});

test("every deck template renders in both languages without inventing data", async () => {
  const { TEMPLATES, buildTemplate } = await import("../../ui/static/deck-templates.js");
  const { THEME_COLORS } = await import("../../ui/static/layout-previews.js");
  const { LAYOUTS } = await import("../../ui/static/layouts.js");
  assert.ok(TEMPLATES.length >= 5);
  for (const template of TEMPLATES) {
    assert.ok(THEME_COLORS[template.theme], `${template.id} uses a known theme`);
    for (const lang of ["en", "es"]) {
      const built = buildTemplate(template, lang, { w: 1280, h: 720 });
      assert.ok(built.slides.length >= 6, `${template.id} tells a story in ${built.slides.length} slides`);
      for (const slide of built.slides) {
        assert.ok(slide.code.includes("slide layout-"), "rendered through a layout");
        assert.ok(slide.notes && slide.notes.length > 10, `${template.id} has speaker guidance`);
        assert.ok(!/<script|javascript:|\son\w+=/i.test(slide.code));
      }
      assert.ok(built.title.length > 2);
    }
    for (const item of template.slides) {
      assert.ok(LAYOUTS.includes(item.layout), `${template.id} uses a real layout`);
      assert.ok(item.content.en.title && item.content.es.title, "both languages carry a title");
    }
  }
  // Placeholders, never fake figures.
  const numbers = TEMPLATES.flatMap((template) =>
    template.slides.flatMap((item) => item.content.en.paragraphs.filter((text) => /^\d/.test(text)))
  );
  assert.ok(numbers.length > 0 && numbers.every((value) => /^0+%?$/.test(value)), `placeholders only: ${numbers}`);
});

test("L, P and C drive the laser, the pen and the eraser in the presenter", () => {
  const press = (key) => keyToAction({ key, ctrlKey: false, metaKey: false, altKey: false });
  assert.equal(press("l"), "laser");
  assert.equal(press("L"), "laser");
  assert.equal(press("p"), "pen");
  assert.equal(press("c"), "clearDraw");
  // The existing keys keep their meaning.
  assert.equal(press("b"), "blackout");
  assert.equal(press("s"), "speaker");
  assert.equal(press("f"), "fullscreen");
  // A browser shortcut is never captured.
  assert.equal(keyToAction({ key: "p", ctrlKey: true, metaKey: false, altKey: false }), null);
});

test("the PNG quality falls back to 2x when storage is empty, refused or holds junk", async () => {
  const { readPngScale } = await import("../../ui/static/inspector.js");
  const original = globalThis.localStorage;
  try {
    globalThis.localStorage = { getItem: () => null };
    assert.equal(readPngScale(), 2);
    globalThis.localStorage = { getItem: () => "1" };
    assert.equal(readPngScale(), 1);
    globalThis.localStorage = { getItem: () => "7" };
    assert.equal(readPngScale(), 2, "an unknown scale is ignored");
    globalThis.localStorage = {
      getItem: () => {
        throw new Error("blocked");
      },
    };
    assert.equal(readPngScale(), 2, "a refusing store does not break the inspector");
  } finally {
    if (original === undefined) delete globalThis.localStorage;
    else globalThis.localStorage = original;
  }
});

test("the report formats a JSON reply, keeps other text verbatim and never injects markup", async () => {
  const { formatReply, reportText } = await import("../../ui/static/copilot-report.js");
  const escape = (value) => String(value).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

  const json = formatReply('{"error":{"message":"too <big>","code":429,"retry":true}}', escape);
  assert.match(json, /<span class="rp-key">"error"<\/span>:/);
  assert.match(json, /<span class="rp-num">429<\/span>/);
  assert.match(json, /<span class="rp-lit">true<\/span>/);
  assert.ok(!json.includes("<big>"), "the provider's markup is escaped");
  assert.ok(json.includes("&lt;big&gt;"));

  assert.equal(formatReply("<b>gateway timeout</b>", escape), "&lt;b&gt;gateway timeout&lt;/b&gt;");
  assert.equal(formatReply("", escape), "");

  const text = reportText(
    { provider: "Groq", model: "m", status: 413, code: "too_large", summary: "Too big", detail: "{}", when: "now" },
    (key) => key
  );
  assert.match(text, /^SlydeCode copilot report/);
  assert.match(text, /copilot\.report\.status: HTTP 413/);
  assert.match(text, /Too big/);
});

test("the reasoning viewport shows the tail of the thought on whole words", async () => {
  const { recentThinking } = await import("../../ui/static/copilot-work.js");
  assert.equal(recentThinking(""), "");
  assert.equal(recentThinking("  short   thought  "), "short thought");
  const long = Array.from({ length: 200 }, (_, index) => `word${index}`).join(" ");
  const tail = recentThinking(long, 120);
  assert.ok(tail.startsWith("…"), "marks that earlier text rolled out");
  assert.ok(tail.endsWith("word199"), "ends with the newest words");
  assert.ok(tail.length <= 125);
  assert.match(tail.slice(1), /^word\d+/, "never starts mid-word");
});
