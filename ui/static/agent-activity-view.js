// The AI activity chip and its stage sweep.
//
// The server announces what an agent is doing (`agent_activity`); the pure state
// machine in `agent-activity.js` decides which phase is showing and when it
// expires. This module is the DOM side: the chip, the polite live announcement,
// the one-shot sweep over the slide being written, and the "is the user busy"
// rule that decides whether the studio may follow the agent to a new slide.
//
// It used to be seven `let`s in the shell's boot closure. Now the shell tells it
// about events and interactions, and asks it two things: to paint, and whether
// the user is engaged.

import { idleActivity, nextTransition, receiveActivity, tickActivity } from "./agent-activity.js";

/** The user counts as busy for this long after the last pointer or key event. */
const FOLLOW_IDLE_MS = 1800;

/**
 * @param {object} options
 * @param {(key: string, vars?: object) => string} options.t
 * @param {object} options.els  needs agentChip, agentLabel, agentAnnouncer, frameWrap, stageScan
 * @param {() => string | null} options.getSelectedId  slide showing on the stage
 */
export function createAgentActivityView({ t, els, getSelectedId }) {
  let activity = idleActivity();
  let activityTimer = null;
  let lastInteractionAt = 0;
  let lastSheenKey = "";
  let sweepPending = false;
  let lastAnnouncedLabel = "";
  let chipHideTimer = null;

    function activityLabel(state) {
      if (!state) return "";
      switch (state.phase) {
        case "writing":
          if (state.action === "set_notes") return t("agent.notes");
          if (state.action === "upsert_slide") {
            return Number.isInteger(state.slideIndex)
              ? t("agent.writing", { n: state.slideIndex + 1 })
              : t("agent.writingGeneric");
          }
          return t("agent.writingGeneric");
        case "adjusting":
          if (state.action === "create_presentation") return t("agent.preparing");
          if (state.action === "remove_slide") return t("agent.removing");
          if (state.action === "move_slide") return t("agent.moving");
          return t("agent.adjusting");
        case "animating":
          return t("agent.animating");
        case "done":
          return state.action === "upsert_slide" ? t("agent.doneSlide") : t("agent.done");
        case "thinking":
          return t("agent.thinking");
        case "error":
          return t("agent.error");
        default:
          return "";
      }
    }

    function render({ announce = true } = {}) {
      const idle = activity.phase === "idle";
      const label = idle ? "" : activityLabel(activity);

      // The chip exits with a short fade instead of disappearing between two
      // frames; a new active event during the fade cancels it.
      if (idle) {
        if (!els.agentChip.hidden) {
          if (!chipHideTimer) {
            els.agentChip.classList.add("leaving");
            chipHideTimer = window.setTimeout(() => {
              chipHideTimer = null;
              els.agentChip.hidden = true;
              els.agentChip.classList.remove("leaving");
              els.agentChip.dataset.phase = "idle";
              els.agentLabel.textContent = "";
            }, 180);
          }
        } else {
          els.agentChip.classList.remove("leaving");
          els.agentChip.dataset.phase = "idle";
          els.agentLabel.textContent = "";
        }
      } else {
        if (chipHideTimer) {
          window.clearTimeout(chipHideTimer);
          chipHideTimer = null;
        }
        els.agentChip.hidden = false;
        els.agentChip.classList.remove("leaving");
        els.agentChip.dataset.phase = activity.phase;
        els.agentLabel.textContent = label;
      }

      // Live announcements follow the visual phases, not the raw events: a
      // mutation is instant, but "Slide ready" should be heard when it appears.
      if (idle) {
        if (lastAnnouncedLabel) els.agentAnnouncer.textContent = "";
        lastAnnouncedLabel = "";
      } else if (activity.phase === "thinking") {
        // Reserved for a future explicit generation channel; inferred calls never
        // emit it, so it is not announced.
      } else if (!announce) {
        lastAnnouncedLabel = label;
      } else if (label && label !== lastAnnouncedLabel) {
        lastAnnouncedLabel = label;
        els.agentAnnouncer.textContent = label;
      }

      const writingSlide =
        activity.phase === "writing" && activity.action === "upsert_slide";
      const targetVisible =
        Boolean(activity.slideId) && activity.slideId === getSelectedId();
      const sheen = writingSlide && targetVisible;

      if (!sheen) {
        lastSheenKey = "";
        sweepPending = false;
        delete els.frameWrap.dataset.agentPhase;
        return;
      }

      els.frameWrap.dataset.agentPhase = "writing";
      const key = `${activity.slideId}:${activity.action}`;
      if (key !== lastSheenKey) {
        lastSheenKey = key;
        sweepPending = true;
      }
      maybeSweep();
    }

    function maybeSweep() {
      if (!sweepPending || els.frameWrap.hidden) return;
      sweepPending = false;
      els.stageScan.classList.remove("run");
      void els.stageScan.offsetWidth;
      els.stageScan.classList.add("run");
    }

    function scheduleTick() {
      if (activityTimer) {
        window.clearTimeout(activityTimer);
        activityTimer = null;
      }
      const now = Date.now();
      const at = nextTransition(activity, now);
      if (!at) return;
      activityTimer = window.setTimeout(() => {
        activityTimer = null;
        activity = tickActivity(activity, Date.now());
        render();
        scheduleTick();
      }, Math.max(0, at - now) + 20);
    }

    function userIsEngaged() {
      if (Date.now() - lastInteractionAt < FOLLOW_IDLE_MS) return true;
      const el = document.activeElement;
      if (!el) return false;
      const tag = (el.tagName || "").toLowerCase();
      if (tag === "input" || tag === "textarea" || tag === "select" || el.isContentEditable) {
        return true;
      }
      // Never move the storyboard selection while keyboard focus is in it.
      return Boolean(el.closest(".rail"));
    }

  return {
    /** Feeds one `agent_activity` event into the state machine (does not paint). */
    receive(event) {
      activity = receiveActivity(activity, event, Date.now());
    },
    /** Paints the chip, the announcement and the sheen for the current phase. */
    render,
    /** Replays the sweep if one is waiting and the stage is visible. */
    maybeSweep,
    /** Arms the timer that moves the chip to its next phase. */
    scheduleTick,
    /** Call on every pointer or key event: people editing are not followed. */
    markInteraction() {
      lastInteractionAt = Date.now();
    },
    userIsEngaged,
  };
}
