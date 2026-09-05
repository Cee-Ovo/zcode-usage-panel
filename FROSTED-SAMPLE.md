# Dashboard frosted-glass sample

Status: user authorized desktop replacement and GitHub delivery of the dashboard sample on 2026-09-05. Other pages retain their existing appearance.

## Source and boundary

- [OpenGlass UI](https://github.com/moekoelueker/open-glass-ui), installed version **0.3.0**, MIT. Uses its `Glass` component and unmodified `regular` / `frosted` CSS material presets, plus its `SegmentedControl`. No custom material shader, SVG refraction, noise generator, or optical pointer tracking was added.
- [Microsoft Acrylic guidance](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic): translucency, background blur, readable text, material hierarchy and fallback considerations. This sample is application-content glass, not a claim of implementing native Windows Acrylic.
- [MDN backdrop-filter](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/backdrop-filter): the backdrop is filtered, not the foreground text.
- [WCAG contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html): contrast is checked against the composited background, not an isolated colour token.

`src/styles/frosted-sample.css` supplies only a static application backdrop, layout and typography. Background colours are a project proposal, not an official glass standard. Material blur, alpha, border colour and shadows come from the library's inline styles. Existing opaque CSS remains for non-sample pages. The optional MetricCard/AccessibleDialog glass props preserve their other callers.

Scope: dashboard, its navigation surface, provider and cost dialogs. Provider dialogs share the existing keyboard-accessible dialog and portal outside glass ancestors to avoid backdrop-filter's fixed-position containing block. No data contracts or native window configuration changed.

## Review

Run `npm run dev -- --host 127.0.0.1`, then visit http://127.0.0.1:5173/.
Browser DEV mode uses synthetic data. The review strip links the material source and lets the reviewer switch between light and dark without saving settings. It is excluded from production builds.

## Verification

- `npm run build`, `npm test` (33 tests).
- `npx --yes --package @playwright/cli playwright-cli -s=frosted run-code --filename scripts/frosted-smoke.js` (open the browser session first): library CSS material/alpha/filter checks; light/dark at 1280, 980, 600 and 400px; full-window dialog bounds; keyboard trap/restore; reduced-transparency and forced-colors fallback; no glass metric migration into Models.
- Existing `scripts/browser-smoke.js`: compact mode, 620 synthetic session search/paging, dialogs, range race, single-flight and redacted retry.
- Visual inspection: `output/playwright/frosted-light.png`, `frosted-dark.png`, `frosted-dialog.png`.
- Five background samples per theme from the 1280x960 screenshots, compared with the dashboard's muted-text colours: light 5.21–5.75:1; dark 8.26–8.91:1. These samples are **not** a complete accessibility audit or a guarantee for arbitrary backgrounds.

Not covered by this delivery: broader page migration and native WebView2 drag/resize/scroll performance acceptance. Desktop-wallpaper transparency is not enabled.
