# Fonts

SlydeCode ships **Geist** (sans) and **Geist Mono**, both variable, so the UI
does not depend on the user having the font installed and offline rendering
(PDF/PNG export) is deterministic.

| File | Family | Style | Weight axis |
|---|---|---|---|
| `Geist-Variable.woff2` | Geist | normal | 100–900 |
| `Geist-Italic-Variable.woff2` | Geist | italic | 100–900 |
| `GeistMono-Variable.woff2` | Geist Mono | normal | 100–900 |
| `GeistMono-Italic-Variable.woff2` | Geist Mono | italic | 100–900 |

Source: <https://registry.npmjs.org/geist> (package `geist` 1.8.2, files under
`dist/fonts/geist-sans` and `dist/fonts/geist-mono`).

License: SIL Open Font License 1.1 — see `LICENSE-Geist.txt`. The license is
compatible with commercial distribution; the fonts may be bundled and served
with the app as long as the license travels with them.

Served at `/fonts/*` by `slyde-server` from this directory, on both the app
origin (editor UI) and the sandbox origin (slide documents), so each origin
caches the face once across every iframe it renders.
