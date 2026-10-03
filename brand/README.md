# Hora brand

The night watch: a little owl that dozes while everything is up, and a parliament
of owls (one per vantage point) that agrees before it wakes you. Third sibling of
the synthe.se family, after Ariane and Maison.

- **Start here:** open [`index.html`](index.html) in a browser - the gallery of
  the brand, the owl states and every mockup (light/dark, desktop/phone).
- [`brand.md`](brand.md) - concept, mascot rules, tokens with contrast ratios,
  state system, type, voice (EN/FR), do and don't, and the Starlight theme
  (section 12) for `docs/`.
- `assets/` - mark, favicon, wordmark, owl states, OG card; `fonts/` - Fraunces,
  Cal Sans, Borel, JetBrains Mono (OFL, licences alongside).

The mockups are generated: `python3 src/build.py` (status pages and the rest via
`src/*.py`), thumbnails with `node src/thumbs.mjs <page>...`, which borrows the
Playwright install of `../maison/e2e` (or set `PLAYWRIGHT_PKG` to another
`package.json` that depends on `playwright`).
