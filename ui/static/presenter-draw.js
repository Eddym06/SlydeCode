// Presenter live annotation: red-dot laser pointer (L) and pen drawing (P).
//
// Overlaid directly on top of the slide frame so strokes stay aligned with the
// content. The canvas automatically resizes with the frame and clears on slide
// change or on C.

export function mountPresenterDraw(frameContainer) {
  if (!frameContainer) return { toggle() {}, clear() {}, destroy() {} };

  let mode = "none"; // "none" | "laser" | "pen"
  let isDrawing = false;
  let lastPos = { x: 0, y: 0 };
  let laserPos = { x: -100, y: -100 };
  let rafId = null;

  const canvas = document.createElement("canvas");
  canvas.className = "present-draw-canvas";
  canvas.setAttribute("aria-hidden", "true");
  canvas.style.cssText =
    "position:absolute;inset:0;width:100%;height:100%;pointer-events:none;z-index:100;";
  frameContainer.appendChild(canvas);

  const ctx = canvas.getContext("2d");

  function syncSize() {
    const rect = frameContainer.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const w = Math.round(rect.width * dpr);
    const h = Math.round(rect.height * dpr);
    if (w > 0 && h > 0 && (canvas.width !== w || canvas.height !== h)) {
      canvas.width = w;
      canvas.height = h;
    }
  }
  syncSize();
  window.addEventListener("resize", syncSize);

  let ro = null;
  if (typeof ResizeObserver !== "undefined") {
    ro = new ResizeObserver(syncSize);
    ro.observe(frameContainer);
  }

  function clear() {
    syncSize();
    ctx.clearRect(0, 0, canvas.width, canvas.height);
  }

  function drawLaser() {
    if (mode !== "laser") return;
    const dpr = window.devicePixelRatio || 1;
    const x = laserPos.x * dpr;
    const y = laserPos.y * dpr;
    // Radial glow
    const grad = ctx.createRadialGradient(x, y, 2 * dpr, x, y, 18 * dpr);
    grad.addColorStop(0, "rgba(255, 107, 74, 1)");
    grad.addColorStop(0.3, "rgba(124, 58, 237, 0.7)");
    grad.addColorStop(1, "rgba(255, 107, 74, 0)");
    ctx.save();
    ctx.fillStyle = grad;
    ctx.beginPath();
    ctx.arc(x, y, 18 * dpr, 0, Math.PI * 2);
    ctx.fill();
    // Bright white-hot core
    ctx.fillStyle = "#ffffff";
    ctx.beginPath();
    ctx.arc(x, y, 3 * dpr, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  function loop() {
    if (mode === "laser") {
      syncSize();
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      drawLaser();
      rafId = requestAnimationFrame(loop);
    }
  }

  function onPointerMove(e) {
    const rect = frameContainer.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;

    if (mode === "laser") {
      laserPos = { x, y };
    } else if (mode === "pen" && isDrawing) {
      const dpr = window.devicePixelRatio || 1;
      ctx.save();
      ctx.strokeStyle = "#FF6B4A";
      ctx.lineWidth = 4 * dpr;
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      ctx.shadowColor = "rgba(124, 58, 237, 0.4)";
      ctx.shadowBlur = 8 * dpr;
      ctx.beginPath();
      ctx.moveTo(lastPos.x * dpr, lastPos.y * dpr);
      ctx.lineTo(x * dpr, y * dpr);
      ctx.stroke();
      ctx.restore();
      lastPos = { x, y };
    }
  }

  function onPointerDown(e) {
    if (mode !== "pen" || e.button !== 0) return;
    isDrawing = true;
    const rect = frameContainer.getBoundingClientRect();
    lastPos = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    e.preventDefault();
  }

  function onPointerUp() {
    isDrawing = false;
  }

  frameContainer.addEventListener("pointermove", onPointerMove);
  frameContainer.addEventListener("pointerdown", onPointerDown);
  window.addEventListener("pointerup", onPointerUp);
  // Finishing a stroke fires a click on the canvas. While drawing it must not
  // bubble to the presenter, or every stroke would turn the page.
  canvas.addEventListener("click", (event) => {
    if (mode === "pen") event.stopPropagation();
  });

  function setMode(next) {
    syncSize();
    if (mode === next) next = "none";
    mode = next;
    cancelAnimationFrame(rafId);
    if (mode === "laser") {
      syncSize();
      // The slide lives in a cross-origin iframe, which swallows pointer moves.
      // The overlay has to receive them or the dot would never follow the cursor.
      canvas.style.pointerEvents = "auto";
      frameContainer.style.cursor = "none";
      loop();
    } else if (mode === "pen") {
      syncSize();
      canvas.style.pointerEvents = "auto";
      frameContainer.style.cursor = "crosshair";
    } else {
      canvas.style.pointerEvents = "none";
      frameContainer.style.cursor = "";
      clear();
    }
    return mode;
  }

  return {
    toggle(target) {
      return setMode(target);
    },
    clear,
    get mode() {
      return mode;
    },
    destroy() {
      cancelAnimationFrame(rafId);
      if (ro) ro.disconnect();
      window.removeEventListener("resize", syncSize);
      frameContainer.removeEventListener("pointermove", onPointerMove);
      frameContainer.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("pointerup", onPointerUp);
      canvas.remove();
    },
  };
}
