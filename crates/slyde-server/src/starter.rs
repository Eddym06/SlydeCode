//! Starter slide code.
//!
//! These are the only HTML templates the product ships. They exist so a new
//! deck is never a blank page, and so the AI has a concrete, on-brand example
//! of the structure slides should follow. Everything else is AI-authored.

use slyde_core::{AnimationKind, AnimationStep, TemplateType};

pub fn template_type(raw: &str) -> TemplateType {
    match raw.trim().to_ascii_lowercase().as_str() {
        "hero" => TemplateType::Hero,
        "bullet" | "bullets" => TemplateType::Bullet,
        "diagram" => TemplateType::Diagram,
        "chart" => TemplateType::Chart,
        "image_layout" | "image" => TemplateType::ImageLayout,
        _ => TemplateType::Code,
    }
}

/// Default animation for a starter slide. Kept short: one signature motion
/// moment per slide, never a parade of effects. Steps whose target does not
/// exist in the chosen template are dropped.
pub fn default_animation_for(template: &str) -> Vec<AnimationStep> {
    let code = default_code(template);
    let mut steps = default_animation();
    steps.retain(|step| code.contains(step.target.trim_start_matches('.')));
    steps
}

pub fn default_animation() -> Vec<AnimationStep> {
    vec![
        AnimationStep {
            target: ".title".into(),
            kind: AnimationKind::FadeUp,
            delay_ms: 0,
            duration_ms: 560,
            easing: "cubic-bezier(.16,1,.3,1)".into(),
        },
        AnimationStep {
            target: ".sub".into(),
            kind: AnimationKind::FadeUp,
            delay_ms: 90,
            duration_ms: 560,
            easing: "cubic-bezier(.16,1,.3,1)".into(),
        },
    ]
}

pub fn default_code(template: &str) -> String {
    match template.trim().to_ascii_lowercase().as_str() {
        "bullet" | "bullets" => BULLETS.to_string(),
        "diagram" => DIAGRAM.to_string(),
        "chart" => CHART.to_string(),
        "image_layout" | "image" => IMAGE_LAYOUT.to_string(),
        "code" => CODE.to_string(),
        _ => HERO.to_string(),
    }
}

const BASE_CSS: &str = r#"
  .slide { position: absolute; inset: 0; padding: 84px 92px; display: flex; flex-direction: column; }
  .title { margin: 0; font-size: 76px; line-height: 1.02; letter-spacing: -0.035em; font-weight: 600; }
  .sub { margin: 0; font-size: 22px; line-height: 1.5; color: var(--slyde-muted); max-width: 46ch; }
  .accent { color: var(--slyde-accent); }
  .grad { background: linear-gradient(135deg, #7c3aed, #ff6b4a); -webkit-background-clip: text; background-clip: text; color: transparent; }
"#;

const HERO: &str = r#"<div class="slide hero">
  <span class="eyebrow">SlydeCode</span>
  <h1 class="title">Your story<br>starts <span class="grad">here</span>.</h1>
  <p class="sub">Ask the AI for a slide. It writes real visual code, rendered exactly as your audience will see it.</p>
</div>
<style>
  .hero { justify-content: center; gap: 24px; }
  .eyebrow { font-size: 15px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--slyde-muted); }
  .title { font-size: 96px; max-width: 15ch; }
  .sub { font-size: 24px; }
</style>
"#;

const BULLETS: &str = r#"<div class="slide bullets">
  <h1 class="title">Three things<br>that <span class="accent">matter</span>.</h1>
  <ul class="list">
    <li><span class="n">01</span><span class="t">The idea in one line</span><span class="d">A short supporting sentence that earns the headline.</span></li>
    <li><span class="n">02</span><span class="t">The proof behind it</span><span class="d">A number, a quote, or a concrete example.</span></li>
    <li><span class="n">03</span><span class="t">The next step</span><span class="d">What you want the audience to do now.</span></li>
  </ul>
</div>
<style>
  .bullets { gap: 44px; justify-content: center; }
  .title { font-size: 64px; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .list li { display: grid; grid-template-columns: 64px 1fr; gap: 4px 20px; padding: 20px 0; border-top: 1px solid color-mix(in srgb, var(--slyde-fg) 14%, transparent); }
  .n { grid-row: span 2; font-size: 14px; color: var(--slyde-accent); font-variant-numeric: tabular-nums; padding-top: 6px; }
  .t { font-size: 25px; font-weight: 500; letter-spacing: -0.01em; }
  .d { font-size: 17px; color: var(--slyde-muted); }
</style>
"#;

const DIAGRAM: &str = r#"<div class="slide diagram">
  <h1 class="title">From <span class="accent">idea</span> to screen.</h1>
  <div class="flow">
    <div class="node"><span class="k">01</span><strong>You ask</strong><span>Plain language, no syntax.</span></div>
    <div class="link"></div>
    <div class="node"><span class="k">02</span><strong>AI writes</strong><span>Real HTML, CSS and motion.</span></div>
    <div class="link"></div>
    <div class="node"><span class="k">03</span><strong>Chrome renders</strong><span>What you see is the deck.</span></div>
  </div>
</div>
<style>
  .diagram { justify-content: center; gap: 56px; }
  .title { font-size: 60px; }
  .flow { display: flex; align-items: stretch; gap: 0; }
  .node { flex: 1; border: 1px solid color-mix(in srgb, var(--slyde-fg) 16%, transparent); border-radius: 16px; padding: 26px; display: flex; flex-direction: column; gap: 8px; }
  .node strong { font-size: 24px; font-weight: 600; letter-spacing: -0.01em; }
  .node span { font-size: 16px; color: var(--slyde-muted); line-height: 1.45; }
  .node .k { font-size: 13px; color: var(--slyde-accent); }
  .link { width: 48px; align-self: center; height: 1px; background: color-mix(in srgb, var(--slyde-fg) 20%, transparent); }
</style>
"#;

const CHART: &str = r#"<div class="slide chart">
  <h1 class="title">Signal, not noise.</h1>
  <div class="bars">
    <div class="bar"><span style="height:38%"></span><em>Q1</em></div>
    <div class="bar"><span style="height:56%"></span><em>Q2</em></div>
    <div class="bar"><span style="height:74%"></span><em>Q3</em></div>
    <div class="bar"><span style="height:92%"></span><em>Q4</em></div>
  </div>
</div>
<style>
  .chart { justify-content: flex-end; gap: 40px; }
  .title { font-size: 58px; }
  .bars { display: flex; align-items: flex-end; gap: 28px; height: 340px; }
  .bar { flex: 1; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; gap: 12px; height: 100%; }
  .bar span { width: 100%; border-radius: 10px 10px 4px 4px; background: linear-gradient(180deg, #ff6b4a, #7c3aed); opacity: 0.9; }
  .bar em { font-style: normal; font-size: 13px; color: var(--slyde-muted); }
</style>
"#;

const IMAGE_LAYOUT: &str = r#"<div class="slide media">
  <div class="copy">
    <h1 class="title">Make it<br>unmistakable.</h1>
    <p class="sub">Use the right half for an image, a product shot, or a portrait. Replace this block with your own media.</p>
  </div>
  <div class="plate" aria-hidden="true"></div>
</div>
<style>
  .media { flex-direction: row; align-items: center; gap: 56px; }
  .copy { flex: 1; display: flex; flex-direction: column; gap: 20px; }
  .title { font-size: 64px; }
  .plate { flex: 1; align-self: stretch; border-radius: 20px; background: radial-gradient(120% 120% at 20% 0%, #7c3aed 0%, #ff6b4a 100%); opacity: 0.85; }
</style>
"#;

const CODE: &str = r#"<div class="slide code">
  <h1 class="title">The deck is <span class="accent">code</span>.</h1>
  <pre class="block"><code>&lt;section class="slide"&gt;
  &lt;h1&gt;Every slide is real HTML&lt;/h1&gt;
  &lt;p&gt;And CSS. And motion.&lt;/p&gt;
&lt;/section&gt;</code></pre>
</div>
<style>
  .code { justify-content: center; gap: 32px; }
  .title { font-size: 56px; }
  .block { margin: 0; padding: 28px 32px; border-radius: 16px; background: color-mix(in srgb, var(--slyde-fg) 6%, transparent); border: 1px solid color-mix(in srgb, var(--slyde-fg) 14%, transparent); font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 18px; line-height: 1.6; color: var(--slyde-muted); }
</style>
"#;

/// CSS shared by every starter slide. Appended once per document.
pub fn base_css() -> &'static str {
    BASE_CSS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_produces_a_title_and_styles() {
        for name in ["hero", "bullet", "diagram", "chart", "image_layout", "code"] {
            let code = default_code(name);
            assert!(code.contains("class=\"title\""), "{name} has no .title");
            assert!(code.contains("<style>"), "{name} has no styles");
        }
    }

    #[test]
    fn unknown_template_falls_back_to_hero() {
        assert_eq!(default_code("nope"), default_code("hero"));
        assert_eq!(template_type("nope"), TemplateType::Code);
    }

    #[test]
    fn animation_targets_exist_in_starter_slides() {
        assert_eq!(default_animation().len(), 2);
        for name in ["hero", "bullet", "diagram", "chart", "image_layout", "code"] {
            let code = default_code(name);
            let steps = default_animation_for(name);
            assert!(!steps.is_empty(), "{name} has no usable animation");
            for step in &steps {
                let class = step.target.trim_start_matches('.');
                assert!(code.contains(class), "{name} is missing {}", step.target);
            }
        }
    }
}
