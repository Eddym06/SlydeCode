# Vendored renderers

Third-party browser bundles served from the **sandbox origin** at `/vendor`.
A slide document cannot reach the network (`connect-src 'none'`), so these ship
with the app instead of coming from a CDN.

| Folder    | Package                  | Version | License      |
|-----------|--------------------------|---------|--------------|
| `katex/`  | `katex`                  | 0.19.1  | MIT          |
| `hljs/`   | `@highlightjs/cdn-assets`| 11.12.0 | BSD-3-Clause |
| `mermaid/`| `mermaid`                | 11.12.0 | MIT          |

Every file was extracted from the npm tarball of that exact version, and each
folder keeps the upstream `LICENSE`.

`katex` and `mermaid` are loaded lazily by the slide runtime only when a slide
actually contains math (`$…$`) or a `.slyde-diagram` block, so a deck without
technical content pays nothing.

Mermaid 12 is ESM-only with code-split chunks and would require a bundler;
11.12.0 still publishes a self-contained `dist/mermaid.min.js`, which is why
that version is pinned. Revisit the pin if a bundler ever enters the build.
