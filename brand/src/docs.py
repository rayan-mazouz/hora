"""docs-home.html: the Starlight splash, re-themed, and the README header."""
from common import *

DOCS_CSS = r"""
.spark, .facts { display: none; }
.dtop .page { gap: var(--s-4); }
.dtop .search { display: none; align-items: center; gap: var(--s-2); min-height: var(--control-h-s); padding: 0 var(--s-3); border: 1px solid var(--line); border-radius: var(--radius); background: var(--surface); color: var(--ink-muted); font: var(--t-secondary); width: 18rem; }
.dtop .search kbd { margin-left: auto; font: var(--t-tiny); border: 1px solid var(--line); border-radius: var(--radius-s); padding: 0 6px; }
@media (min-width: 840px) { .dtop .search { display: flex; } }
.dtop .links { display: none; gap: var(--s-1); }
@media (min-width: 700px) { .dtop .links { display: flex; } }
.splash { display: grid; gap: var(--s-6); align-items: center; padding-block: var(--s-7) var(--s-6); grid-template-columns: minmax(0, 1fr); }
@media (min-width: 960px) { .splash { grid-template-columns: minmax(0, 1.1fr) minmax(0, 0.9fr); } }
.splash .eyebrow { color: var(--accent); }
.splash h1 { font: 500 clamp(2.25rem, 1.5rem + 3.2vw, 4rem)/1.05 var(--font-display); letter-spacing: -0.015em; margin-top: var(--s-3); text-wrap: balance; }
.splash .lead { font: 400 1.1875rem/1.6 var(--font-text); color: var(--ink-muted); margin-top: var(--s-4); max-width: 36rem; }
.splash .actions { display: flex; flex-wrap: wrap; gap: var(--s-2); margin-top: var(--s-5); }
.splash .actions .btn { min-height: var(--control-h); padding: 0 var(--s-5); border-radius: var(--radius-l); }
.install { margin-top: var(--s-5); max-width: 36rem; position: relative; }
.install pre { margin: 0; padding: var(--s-4); padding-right: 5.5rem; background: var(--ink); color: var(--ground); border-radius: var(--radius-l); font: 400 0.8125rem/1.7 var(--font-mono); overflow-x: auto; }
.install pre .c { opacity: 0.6; }
.install .btn { position: absolute; top: var(--s-2); right: var(--s-2); background: transparent; color: var(--ground); border-color: color-mix(in srgb, var(--ground) 30%, transparent); min-height: 32px; }
.stage { position: relative; display: grid; place-items: center; min-height: 360px; }
.stage .dial { position: absolute; inset: 0; margin: auto; width: min(100%, 420px); aspect-ratio: 1; color: var(--accent-soft); opacity: 0.55; }
.stage > .owl { width: min(60%, 240px); height: auto; position: relative; }
.stage .say { position: absolute; bottom: 4%; left: 50%; translate: -50% 0; display: inline-flex; gap: 8px; align-items: center; padding: 8px 14px; border-radius: var(--radius-pill); background: var(--surface); box-shadow: var(--shadow); font: var(--t-label); white-space: nowrap; }
.why { display: grid; gap: var(--s-4); grid-template-columns: repeat(auto-fit, minmax(min(100%, 16rem), 1fr)); }
.card { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); padding: var(--s-5); }
.card .ic { width: 40px; height: 40px; border-radius: var(--radius-l); background: var(--accent-wash); color: var(--accent); display: grid; place-items: center; margin-bottom: var(--s-3); }
.card .ic svg { width: 22px; height: 22px; }
.card h3 { font: var(--t-group); margin-bottom: var(--s-2); }
.card p { color: var(--ink-muted); font: var(--t-secondary); font-size: 0.9375rem; line-height: 1.5; }
.h2 { font: 500 clamp(1.5rem, 1.3rem + 0.8vw, 2rem)/1.2 var(--font-display); margin-bottom: var(--s-4); }
.flow { display: grid; gap: var(--s-2); grid-template-columns: repeat(auto-fit, minmax(min(100%, 11rem), 1fr)); counter-reset: f; }
.flow li { list-style: none; background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-l); padding: var(--s-4); position: relative; }
.flow li b { display: flex; gap: 8px; align-items: center; font: var(--t-label); margin-bottom: 4px; }
.flow li b::before { counter-increment: f; content: counter(f); width: 22px; height: 22px; border-radius: 50%; display: grid; place-items: center; background: var(--accent-wash); color: var(--accent); font: var(--t-tiny); }
.flow li span { font: var(--t-secondary); color: var(--ink-muted); }
.flow li.end { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
.flow li.end span { color: var(--on-accent); opacity: 0.85; }
.flow li.end b::before { background: var(--on-accent); color: var(--accent); }
.flow { padding: 0; margin: 0; }
.cmp { width: 100%; border-collapse: collapse; font: var(--t-secondary); min-width: 34rem; }
.cmp th, .cmp td { padding: var(--s-3); border-bottom: 1px solid var(--line); text-align: left; vertical-align: top; }
.cmp thead th { font: var(--t-meta); color: var(--ink-muted); }
.cmp thead th.us { color: var(--accent); }
.cmp td.us { background: color-mix(in srgb, var(--accent-wash) 60%, transparent); font-weight: 500; }
.cmp th[scope='row'] { font: var(--t-label); }
.tw { overflow-x: auto; border: 1px solid var(--line); border-radius: var(--radius-xl); background: var(--surface); }
.y, .n { display: inline-flex; gap: 6px; align-items: center; }
.y::before { content: ''; width: 10px; height: 10px; border-radius: 50%; background: var(--st-up); }
.n::before { content: ''; width: 10px; height: 10px; border-radius: 50%; border: 1.5px solid var(--st-none); }
/* README, as GitHub renders it */
.gh { border: 1px solid var(--line); border-radius: var(--radius-xl); background: var(--surface); overflow: hidden; max-width: 56rem; }
.gh .bar0 { display: flex; align-items: center; gap: var(--s-2); padding: var(--s-3) var(--s-4); border-bottom: 1px solid var(--line); font: var(--t-meta); color: var(--ink-muted); background: var(--ground-raised); }
.gh .md { padding: var(--s-5) var(--s-5) var(--s-6); font-family: -apple-system, 'Segoe UI', system-ui, sans-serif; }
.gh h1 { font: 600 2rem/1.25 -apple-system, 'Segoe UI', system-ui, sans-serif; display: flex; align-items: center; gap: 10px; padding-bottom: 8px; border-bottom: 1px solid var(--line); }
.gh .shields { display: flex; flex-wrap: wrap; gap: 6px; margin: var(--s-3) 0; }
.sh { display: inline-flex; font: 11px/20px Verdana, sans-serif; border-radius: 3px; overflow: hidden; }
.sh b { font-weight: 400; background: #555; color: #fff; padding: 0 6px; }
.sh i { font-style: normal; background: #0E5E68; color: #fff; padding: 0 6px; }
.sh i.w { background: #B4633F; }
.gh p { margin: var(--s-3) 0; line-height: 1.6; max-width: 46rem; }
.gh blockquote { margin: var(--s-3) 0; padding: 0 1em; border-left: 0.25em solid var(--line); color: var(--ink-muted); }
.gh .shot { border: 1px solid var(--line); border-radius: var(--radius-m); overflow: hidden; margin-top: var(--s-4); }
.mini-status { background: var(--ground); padding: var(--s-4); display: grid; gap: var(--s-3); }
.mini-status .hero { margin: 0; padding: var(--s-4); grid-template-columns: auto 1fr; }
.mini-status .hero h1 { font: 500 1.5rem/1.2 var(--font-display); border: 0; padding: 0; display: block; }
.mini-status .hero > .owl { width: 52px; }
.mini-status .groups { --group-col: 16rem; gap: var(--s-3); }
"""

DIAL = '''<svg class="dial" viewBox="0 0 200 200" aria-hidden="true"><circle cx="100" cy="100" r="88" fill="none" stroke="currentColor" stroke-width="1.2" stroke-dasharray="1.5 6" stroke-linecap="round"/>
<path d="M100 4v10M100 186v10M4 100h10M186 100h10M148 16.9l-5 8.7M57 174.4l-5 8.7M183.1 52l-8.7 5M25.6 143l-8.7 5M183.1 148l-8.7-5M25.6 57 16.9 52M148 183.1l-5-8.7M57 25.6 52 16.9" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>'''

IC = {
    'moon': '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M19.5 14.6A7.8 7.8 0 0 1 9.4 4.5a7.8 7.8 0 1 0 10.1 10.1Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/></svg>',
    'box': '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 20 7.5v9L12 21l-8-4.5v-9Z M4 7.5l8 4.5 8-4.5M12 12v9" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/></svg>',
    'gauge': '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 17a8 8 0 1 1 16 0" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/><path d="m12 17 4-5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>',
    'globe': '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="8.5" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="M3.5 12h17M12 3.5c2.6 2.6 2.6 14.4 0 17M12 3.5c-2.6 2.6-2.6 14.4 0 17" fill="none" stroke="currentColor" stroke-width="1.6"/></svg>',
}


def page_docs():
    from status import pelican_groups, group
    g1, g2, g3 = pelican_groups()
    body = f"""<header class="top dtop"><div class="page">
<a class="sign hora" href="index.html"><img src="assets/mark.svg" alt="" width="32" height="32" style="border-radius:8px"><span class="name">Hora</span></a>
<span class="search">{icon('search')}Search the docs<kbd>Ctrl K</kbd></span>
<nav class="links" aria-label="Docs"><a class="navlink" href="docs-page.html">Guides</a><a class="navlink" href="#">Reference</a><a class="navlink" href="#">Français</a></nav>
<a class="btn quiet" href="#" aria-label="Hora on GitHub">{icon('gh')}</a>{theme_button()}</div></header>
<main id="main" class="page">
<section class="splash" aria-labelledby="h">
  <div>
    <p class="eyebrow">Self-hosted uptime monitor</p>
    <h1 id="h">Hora keeps the hours, so you can sleep.</h1>
    <p class="lead">One small binary checks your services, keeps their history, and serves a calm status page. It wakes you only when something has really broken: never for a blip.</p>
    <div class="actions"><a class="btn primary" href="docs-page.html">Get started {icon("arrow")}</a><a class="btn" href="status-ok.html">See a status page</a></div>
    <div class="install"><pre><span class="c"># one container, about 15 MB</span>
docker run -d -p 8787:8787 \\
  -v "$PWD/hora-config:/etc/hora" \\
  -v hora-data:/data ghcr.io/uplg/hora</pre><button class="btn" type="button">{icon('copy')}Copy</button></div>
  </div>
  <div class="stage">{DIAL}{owl('up', 240, label="Hora's owl, asleep: everything is up")}<span class="say">{glyph('up')}All 14 services are up</span></div>
</section>

<section class="section" aria-labelledby="why"><h2 class="h2" id="why">Why Hora</h2>
<div class="why">
  <article class="card"><div class="ic">{IC['moon']}</div><h3>Flapping never wakes you up</h3><p>A failed check is retried, an outage needs several failures in a row and a second place to agree, and ten services down behind one database send one alert: the cause.</p></article>
  <article class="card"><div class="ic">{IC['box']}</div><h3>One small binary</h3><p>HTTP, TCP, ping, DNS and push checks, SQLite, the status page and ten alert channels in one static binary. No Node, no Redis, no browser running in the background.</p></article>
  <article class="card"><div class="ic">{IC['gauge']}</div><h3>Targets you can keep</h3><p>Set 99.9 % and Hora shows the downtime you have left, warns when it burns too fast, and writes the monthly report for you.</p></article>
  <article class="card"><div class="ic">{IC['globe']}</div><h3>A page people trust</h3><p>Plain words on every state, light and dark, readable on a phone, fast with 700 services. Also as an Atom feed, JSON and badges.</p></article>
</div></section>

<section class="section" aria-labelledby="flow"><h2 class="h2" id="flow">From a blip to an alert, or not</h2>
<ol class="flow">
  <li><b>A check fails</b><span>Retried at once. A single blip stops here and leaves no trace.</span></li>
  <li><b>It fails again</b><span>3 failures in a row (you choose) before anything is called down.</span></li>
  <li><b>Others agree</b><span>Peers in other places check too: a local network hiccup is not an outage.</span></li>
  <li><b>Find the cause</b><span>Services that depend on a down one are grouped under it.</span></li>
  <li class="end"><b>One alert</b><span>The cause and what it affects. Then quiet until it recovers.</span></li>
</ol></section>

<section class="section" aria-labelledby="cmp"><h2 class="h2" id="cmp">How it compares</h2>
<div class="tw"><table class="cmp">
<thead><tr><th scope="col"></th><th scope="col" class="us">Hora</th><th scope="col">Uptime Kuma</th><th scope="col">Gatus</th></tr></thead>
<tbody>
<tr><th scope="row">Runs as</th><td class="us">One static binary, ~15 MB image</td><td>Node.js app</td><td>Go binary</td></tr>
<tr><th scope="row">Configured in</th><td class="us">A TOML file, reloaded live</td><td>A web UI</td><td>A YAML file</td></tr>
<tr><th scope="row">One alert for a cascade</th><td class="us"><span class="y">Yes, by dependencies</span></td><td><span class="n">No</span></td><td><span class="n">No</span></td></tr>
<tr><th scope="row">Confirmed from other places</th><td class="us"><span class="y">Yes, peers</span></td><td><span class="n">No</span></td><td><span class="n">No</span></td></tr>
<tr><th scope="row">Error budget, burn-rate alerts</th><td class="us"><span class="y">Yes</span></td><td><span class="n">No</span></td><td><span class="n">No</span></td></tr>
<tr><th scope="row">Monthly report</th><td class="us"><span class="y">Yes, printable</span></td><td><span class="n">No</span></td><td><span class="n">No</span></td></tr>
</tbody></table></div>
<p class="mock-note"><b>To verify before shipping.</b> The rival columns come from memory, not from a fresh read of their docs. Check them, add measured numbers (image size, RAM) and links, as Maison's README does.</p>
</section>

<section class="section" aria-labelledby="readme"><h2 class="h2" id="readme">The README header</h2>
<div class="gh"><div class="bar0">{icon('gh')}uplg / hora · README.md</div><div class="md">
<h1><img src="assets/mark.svg" width="36" height="36" alt="" style="border-radius:8px">Hora</h1>
<div class="shields"><span class="sh"><b>image</b><i>~15 MB</i></span><span class="sh"><b>checks</b><i>HTTP · TCP · ICMP · DNS · push</i></span><span class="sh"><b>alerts</b><i>10 channels</i></span><span class="sh"><b>wakes you for</b><i class="w">real outages only</i></span></div>
<p><b>Hora keeps the hours, so you can sleep.</b> A tiny self-hosted uptime monitor: one small binary checks your services, keeps their history in SQLite, serves a calm status page and alerts you when something has really broken.</p>
<blockquote>Named after the Horai, the Greek keepers of the hours. Its owl dozes while all is well.</blockquote>
<div class="shot"><div class="mini-status">
  <section class="hero">{owl('up', 52)}<div><h1>Everything is running.</h1><p class="sub" style="margin-top:4px;font:var(--t-secondary)">All 14 services are up.</p></div></section>
  <div class="groups">{group('Pelican', '4 services', g1[:3])}{group('Payments and email', '4 services', g2[:3])}</div>
</div></div>
</div></div>
</section>
</main>
<footer class="foot"><div class="page"><nav aria-label="More"><a href="#">Getting started</a><a href="#">Configuration</a><a href="#">Status page</a><a href="#">CLI</a><a href="#">API</a><a href="#">Roadmap</a></nav>
<span class="watched"><img src="assets/mark.svg" alt="" width="22" height="22"><span>MIT licence ·</span><span class="b">Hora</span></span></div></footer>"""
    return shell('Hora · keeps the hours, so you can sleep', body, 'Hora: a tiny self-hosted uptime monitor. One small binary, a calm status page, alerts that never wake you for a blip.', DOCS_CSS)


PAGES = {'docs-home.html': page_docs}


# ---------------------------------------------------------------------------
# docs-page.html: a Starlight content page, Hora theme (the peers guide)
# ---------------------------------------------------------------------------
PAGE_CSS = r"""
.spark, .facts { display: none; }
body { --nav-h: 56px; }
.sl-top { position: sticky; top: 0; z-index: 10; background: color-mix(in srgb, var(--ground) 92%, transparent); backdrop-filter: blur(8px); border-bottom: 1px solid var(--line); }
.sl-top .in { height: var(--nav-h); display: flex; align-items: center; gap: var(--s-3); padding: 0 var(--page-margin); }
.sl-top .sign { margin-right: 0; }
.sl-top .search { display: flex; align-items: center; gap: var(--s-2); min-height: var(--control-h-s); padding: 0 var(--s-3); border: 1px solid var(--line); border-radius: var(--radius); background: var(--surface); color: var(--ink-muted); font: var(--t-secondary); flex: 0 1 22rem; margin-inline: auto; cursor: text; }
.sl-top .search kbd { margin-left: auto; font: var(--t-tiny); border: 1px solid var(--line); border-radius: var(--radius-s); padding: 0 6px; }
.sl-top .right { display: flex; align-items: center; gap: var(--s-1); }
@media (max-width: 599px) { .sl-top .search { flex: 0 0 auto; margin-left: auto; margin-right: 0; padding: 0; width: var(--control-h-s); justify-content: center; } .sl-top .search span, .sl-top .search kbd { display: none; } .theme-btn .lbl { display: none; } .theme-btn { padding: 0; width: var(--control-h-xs); } }
.layout { display: grid; grid-template-columns: minmax(0, 1fr); }
@media (min-width: 800px) { .layout { grid-template-columns: 17.5rem minmax(0, 1fr); } }
@media (min-width: 1152px) { .layout { grid-template-columns: 17.5rem minmax(0, 1fr) 15rem; } }
/* sidebar */
.side { display: none; border-right: 1px solid var(--line); background: var(--ground); }
@media (min-width: 800px) { .side { display: block; position: sticky; top: var(--nav-h); height: calc(100dvh - var(--nav-h)); overflow-y: auto; padding: var(--s-5) var(--s-4) var(--s-6); } }
.side h2 { font: var(--t-meta); text-transform: uppercase; letter-spacing: 0.06em; color: var(--ink-muted); margin: var(--s-5) var(--s-3) var(--s-2); }
.side h2:first-child { margin-top: 0; }
.side ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 2px; }
.side a { display: flex; align-items: center; gap: 8px; min-height: 36px; padding: 0 var(--s-3); border-radius: var(--radius); color: var(--ink); text-decoration: none; font: var(--t-secondary); font-size: 0.9375rem; }
.side a:hover { background: var(--ground-raised); }
.side a[aria-current='page'] { background: var(--accent-wash); color: var(--accent); font-weight: 500; box-shadow: inset 3px 0 0 var(--accent); }
.side a .ext { margin-left: auto; width: 12px; height: 12px; color: var(--ink-muted); }
/* mobile menu and toc */
.mob { display: grid; gap: var(--s-2); padding: var(--s-3) var(--page-margin) 0; }
@media (min-width: 800px) { .mob .menu { display: none; } }
@media (min-width: 1152px) { .mob { display: none; } }
.mob details { border: 1px solid var(--line); border-radius: var(--radius-l); background: var(--surface); }
.mob summary { cursor: pointer; min-height: var(--control-h); display: flex; align-items: center; gap: var(--s-2); padding: 0 var(--s-4); font: var(--t-label); list-style: none; }
.mob summary::-webkit-details-marker { display: none; }
.mob summary .muted { margin-left: auto; font: var(--t-secondary); }
.mob details > div { padding: 0 var(--s-2) var(--s-3); }
.mob .side { display: block; position: static; height: auto; border: 0; padding: var(--s-2); background: none; }
/* content */
.content { padding: var(--s-6) var(--page-margin) var(--s-7); min-width: 0; }
@media (min-width: 800px) { .content { padding-inline: var(--s-6); } }
.content > * { max-width: 46rem; }
.content .crumb { font: var(--t-secondary); color: var(--ink-muted); }
.content h1 { font: 500 clamp(2rem, 1.6rem + 1.6vw, 2.6rem)/1.12 var(--font-display); letter-spacing: -0.015em; margin: var(--s-2) 0 var(--s-3); text-wrap: balance; }
.content .lede { font: 400 1.1875rem/1.6 var(--font-text); color: var(--ink-muted); margin-bottom: var(--s-6); }
.prose h2 { font: 500 1.75rem/1.2 var(--font-display); margin: var(--s-7) 0 var(--s-3); scroll-margin-top: calc(var(--nav-h) + 16px); }
.prose h3 { font: 500 1.3125rem/1.3 var(--font-display); margin: var(--s-6) 0 var(--s-2); scroll-margin-top: calc(var(--nav-h) + 16px); }
.prose h2 a.anchor, .prose h3 a.anchor { color: var(--ink-muted); text-decoration: none; opacity: 0; margin-left: 6px; font-family: var(--font-text); font-size: 0.8em; }
.prose h2:hover a.anchor, .prose h3:hover a.anchor, .prose a.anchor:focus-visible { opacity: 1; }
.prose p, .prose li { font: 400 1rem/1.75 var(--font-text); }
.prose p { margin: 0 0 var(--s-4); }
.prose ul, .prose ol { margin: 0 0 var(--s-4); padding-left: 1.4em; }
.prose li { margin-bottom: 6px; }
.prose li::marker { color: var(--accent-soft); }
.prose strong { font-weight: 600; }
.prose em { font-style: italic; }
.prose :not(pre) > code { font: 400 0.875em/1 var(--font-mono); background: var(--ground-raised); border: 1px solid var(--line); border-radius: var(--radius-s); padding: 0.12em 0.36em; overflow-wrap: anywhere; }
.prose a code { color: var(--accent); }
/* code blocks (Expressive Code, re-skinned) */
.ec { margin: 0 0 var(--s-5); border: 1px solid var(--line); border-radius: var(--radius-l); overflow: hidden; background: var(--surface); }
.ec .ecbar { display: flex; align-items: center; gap: var(--s-2); min-height: 36px; padding: 0 var(--s-2) 0 var(--s-4); border-bottom: 1px solid var(--line); background: var(--ground-raised); font: var(--t-meta); color: var(--ink-muted); }
.ec .ecbar .title { font-family: var(--font-mono); font-weight: 400; }
.ec .ecbar .copy { margin-left: auto; min-height: 28px; padding: 0 10px; font: var(--t-meta); }
.ec .ecbar .copy svg { width: 14px; height: 14px; }
.ec pre { margin: 0; padding: var(--s-4); overflow-x: auto; font: 400 0.84375rem/1.7 var(--font-mono); color: var(--ink); }
.ec.term { background: var(--raw-ink); border-color: var(--raw-ink); }
.ec.term .ecbar { background: color-mix(in srgb, var(--raw-paper-dark) 8%, var(--raw-ink)); border-color: color-mix(in srgb, var(--raw-paper-dark) 14%, var(--raw-ink)); color: #A8AEB8; }
.ec.term .ecbar .dots { display: inline-flex; gap: 6px; margin-right: 4px; } .ec.term .ecbar .dots i { width: 9px; height: 9px; border-radius: 50%; background: #525A66; }
.ec.term .copy { background: transparent; color: #EAE7E0; border-color: #525A66; }
.ec.term pre { color: #EAE7E0; }
.ec.term .p { color: #63C0CB; user-select: none; } .ec.term .o { color: #A8AEB8; } .ec.term .ok { color: #63C0CB; } .ec.term .w { color: #D89372; }
.tk { color: var(--accent); } .ts { color: light-dark(#8F4B22, #D89372); } .tc { color: var(--ink-muted); font-style: italic; } .tb { color: light-dark(#6B3F8F, #C3A6E0); }
/* asides, on the state system: note = info, tip = up, caution = slow, danger = down */
.aside { display: grid; grid-template-columns: auto 1fr; gap: 4px var(--s-3); padding: var(--s-4) var(--s-5) var(--s-4) var(--s-4); border-radius: var(--radius-l); margin: 0 0 var(--s-5); background: var(--accent-wash); }
.aside > .g { margin-top: 3px; }
.aside .t { font: var(--t-label); font-size: 0.9375rem; }
.aside p { grid-column: 2; margin: 0; font: 400 0.9375rem/1.65 var(--font-text); }
.aside.note .t { color: var(--accent); }
.aside.tip { background: var(--wash-up); } .aside.tip .t { color: var(--st-up-text); } .aside.tip > .g { color: var(--accent); }
.aside.caution { background: var(--wash-degraded); } .aside.caution .t { color: var(--st-degraded-text); }
.aside.danger { background: var(--wash-down); } .aside.danger .t { color: var(--st-down-text); }
/* tables */
.tbl { overflow-x: auto; margin: 0 0 var(--s-5); border: 1px solid var(--line); border-radius: var(--radius-l); background: var(--surface); }
.tbl table { width: 100%; border-collapse: collapse; font: var(--t-secondary); font-size: 0.9375rem; min-width: 30rem; }
.tbl th, .tbl td { text-align: left; padding: var(--s-3) var(--s-4); border-bottom: 1px solid var(--line); vertical-align: top; }
.tbl tr:last-child td { border-bottom: 0; }
.tbl thead th { font: var(--t-meta); color: var(--ink-muted); background: var(--ground-raised); }
.tbl td .word { white-space: nowrap; }
/* figure: the verdicts as owls */
.verdicts { display: grid; gap: var(--s-3); grid-template-columns: repeat(auto-fit, minmax(min(100%, 13rem), 1fr)); margin: 0 0 var(--s-5); }
.verdicts figure { min-width: 0; margin: 0; background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-l); padding: var(--s-4); display: grid; gap: var(--s-2); justify-items: center; text-align: center; }
.verdicts .owls { width: 100%; max-width: 190px; }
.verdicts figcaption { font: var(--t-secondary); color: var(--ink-muted); }
.verdicts figcaption code { font: 400 0.8rem/1.4 var(--font-mono); color: var(--ink); display: block; margin-top: 4px; }
/* toc */
.toc { display: none; }
@media (min-width: 1152px) { .toc { display: block; position: sticky; top: var(--nav-h); height: calc(100dvh - var(--nav-h)); overflow-y: auto; padding: var(--s-6) var(--s-4); border-left: 1px solid var(--line); } }
.toc h2, .mob .tocl h2 { font: var(--t-meta); text-transform: uppercase; letter-spacing: 0.06em; color: var(--ink-muted); margin-bottom: var(--s-3); }
.toc ul, .tocl ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 2px; }
.toc a, .tocl a { display: block; padding: 5px 0 5px var(--s-3); border-left: 2px solid var(--line); color: var(--ink-muted); text-decoration: none; font: var(--t-secondary); }
.toc a.sub, .tocl a.sub { padding-left: var(--s-5); }
.toc a:hover, .tocl a:hover { color: var(--ink); }
.toc a[aria-current='true'] { color: var(--accent); border-left-color: var(--accent); font-weight: 500; }
.toc .meta { margin-top: var(--s-5); padding-top: var(--s-4); border-top: 1px solid var(--line); display: grid; gap: var(--s-2); font: var(--t-secondary); }
.toc .meta a { border: 0; padding: 0; color: var(--ink-muted); display: inline-flex; gap: 6px; align-items: center; }
/* after the article */
.edit { display: flex; flex-wrap: wrap; gap: var(--s-2) var(--s-4); justify-content: space-between; margin-top: var(--s-7); padding-top: var(--s-4); border-top: 1px solid var(--line); font: var(--t-secondary); color: var(--ink-muted); }
.edit a { color: var(--ink-muted); display: inline-flex; align-items: center; gap: 6px; }
.pager { display: grid; gap: var(--s-3); grid-template-columns: repeat(auto-fit, minmax(min(100%, 14rem), 1fr)); margin-top: var(--s-5); }
.pager a { display: grid; gap: 2px; padding: var(--s-4); border: 1px solid var(--line); border-radius: var(--radius-l); background: var(--surface); text-decoration: none; color: var(--ink); }
.pager a:hover { border-color: var(--accent-soft); }
.pager a span { font: var(--t-meta); color: var(--ink-muted); display: inline-flex; align-items: center; gap: 6px; }
.pager a b { font: 500 1.125rem/1.3 var(--font-display); color: var(--accent); }
.pager a.next { text-align: right; justify-items: end; }
.pager a.prev span svg { transform: rotate(180deg); }
.sl-foot { margin-top: var(--s-6); display: flex; flex-wrap: wrap; align-items: center; gap: var(--s-2) var(--s-4); font: var(--t-secondary); color: var(--ink-muted); }
.sl-foot .watched { margin-left: 0; }
"""

SIDEBAR = [
    ('Start here', [('Getting started', ''), ('Configuration', ''), ('Upgrading', '')]),
    ('Guides', [('Monitors', ''), ('Alerting & notifications', ''), ('SLOs & error budgets', ''), ('Incidents & history', ''),
                ('Per-group pages & SLA reports', ''), ('Mutual surveillance (peers)', 'cur'), ('Importing from Uptime Kuma', '')]),
    ('Reference', [('CLI', ''), ('HTTP API', '')]),
    ('Project', [('Roadmap', ''), ('Changelog', 'ext')]),
]
TOC = [('heartbeat', 'The outbound heartbeat', 0), ('peers', 'Peers: two independent halves', 0), ('quorum', 'Quorum: outage or partition?', 0),
       ('vantage', 'Multi-vantage confirmation', 0), ('probe', 'How a probe request works', 1), ('failure', 'Failure behaviour', 1),
       ('external', 'External receivers', 0)]


def sidebar():
    out = []
    for title, items in SIDEBAR:
        lis = []
        for label, flag in items:
            cur = ' aria-current="page"' if flag == 'cur' else ''
            ext = '<svg class="ext" viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3h7v7M13 3 4 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg><span class="vh">(opens GitHub)</span>' if flag == 'ext' else ''
            lis.append(f'<li><a href="#"{cur}>{label}{ext}</a></li>')
        out.append(f'<h2>{title}</h2><ul>{"".join(lis)}</ul>')
    return ''.join(out)


def toc_list(current=True):
    return '<ul>' + ''.join(
        f'<li><a href="#{i}" class="{"sub" if d else ""}"{" aria-current=\"true\"" if current and i == "vantage" else ""}>{t}</a></li>'
        for i, t, d in TOC) + '</ul>'


def ec(title, code, term=False):
    cls = 'ec term' if term else 'ec'
    dots = '<span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>' if term else ''
    return (f'<div class="{cls}"><div class="ecbar">{dots}<span class="title">{title}</span>'
            f'<button class="btn copy" type="button" aria-label="Copy {title}">{icon("copy")}Copy</button></div><pre><code>{code}</code></pre></div>')


def aside(kind, title, text):
    g = {'note': 'info', 'tip': 'up', 'caution': 'degraded', 'danger': 'down'}[kind]
    return f'<aside class="aside {kind}" aria-label="{title}">{glyph(g)}<p class="t">{title}</p><p>{text}</p></aside>'


def h(level, id_, text):
    return f'<h{level} id="{id_}">{text}<a class="anchor" href="#{id_}" aria-label="Link to this section">#</a></h{level}>'


def page_docs_page():
    K = lambda s: f'<span class="tk">{s}</span>'
    S = lambda s: f'<span class="ts">{s}</span>'
    C = lambda s: f'<span class="tc">{s}</span>'
    B = lambda s: f'<span class="tb">{s}</span>'
    health = (f'{K("[health]")}\n{K("id")} = {S(chr(34) + "hora-a" + chr(34))}        {C("# this node&#39;s identity (how peers refer to it)")}\n'
              f'{K("interval_secs")} = {B("60")}   {C("# heartbeat cadence while healthy")}\n'
              f'{K("grace_secs")} = {B("180")}     {C("# startup grace before a never-seen peer is alerted")}\n'
              f'{C("# quorum = true      # see below")}\n{C("# heartbeat_url = &quot;${HC_PING_URL}&quot;   # optional extra dead-man target")}')
    peers = (f'{K("[[peers]]")}\n{K("id")} = {S("&quot;hora-b&quot;")}                                    {C("# the peer&#39;s [health].id")}\n'
             f'{K("name")} = {S("&quot;Hora B (Paris)&quot;")}\n{C("# OUT - I heartbeat the peer while I&#39;m healthy:")}\n'
             f'{K("ping_url")} = {S("&quot;https://b.example/api/push/hora-a&quot;")}\n'
             f'{K("ping_token")} = {S("&quot;${PEER_B_TOKEN}&quot;")}                 {C("# sent as X-Push-Token")}\n'
             f'{C("# IN - I watch the peer and alert if it goes silent:")}\n{K("expect_every_secs")} = {B("90")}\n'
             f'{K("listen_token")} = {S("&quot;${PEER_B_IN}&quot;")}                    {C("# required from the peer&#39;s pings")}')
    confirm = (f'{K("[health]")}\n{K("id")} = {S("&quot;hora-a&quot;")}\n'
               f'{K("confirm_with_peers")} = {B("true")}   {C("# per-monitor override: confirm_with_peers = true/false")}')
    term = ('<span class="p">$ </span>hora probe api --confirm\n'
            '<span class="o">api  https://api.example.com/health</span>\n'
            '<span class="w">down here</span>  timeout after 10s\n'
            '<span class="o">asking 2 peers…</span>\n'
            '<span class="w">seen UP by hora-b</span> - down from 1/3 vantage points (network issue near this node?)')
    ext = (f'{K("[[peers]]")}\n{K("id")} = {S("&quot;healthchecks&quot;")}\n{K("name")} = {S("&quot;healthchecks.io&quot;")}\n'
           f'{K("ping_url")} = {S("&quot;https://hc-ping.com/&lt;uuid&gt;&quot;")}')
    article = f"""
<p class="crumb">Guides</p>
<h1>Mutual surveillance (peers)</h1>
<p class="lede">Who watches the watcher: dead-man heartbeats between Hora nodes, quorum, and external receivers.</p>
<div class="prose">
<p>A monitor is only as good as the machine it runs on. Two Hora nodes (two Raspberry Pi at two friends' places, a VPS and a homelab...) can watch each other with <strong>dead-man heartbeats</strong>: each node pings the other while it is healthy; when the pings stop, the survivor alerts.</p>
<p>Nothing here is a bespoke protocol: every exchange is plain HTTP, so a peer can be another Hora, a healthchecks.io / UptimeRobot endpoint, or a cron job.</p>

{h(2, 'heartbeat', 'The outbound heartbeat: <code>[health]</code>')}
{ec('config.toml', health)}
<p>The node POSTs to each peer's <code>ping_url</code> every <code>interval_secs</code>, but <strong>only while it is locally healthy</strong> (its scheduler is ticking and its database is writable). A hung process stops pinging, and the receiver notices.</p>

{h(2, 'peers', 'Peers: two independent halves')}
<p>Each <code>[[peers]]</code> entry can declare either or both directions:</p>
{ec('config.toml', peers)}
{aside('tip', 'Reloads live', '<code>[health]</code> and <code>[[peers]]</code> reload live like everything else: edit, save, and the next tick uses the new mesh. No restart.')}
<p>Watched peers appear in their own section on the status page. Their state does not roll into the overall badge: it tracks your services, not the surveillance mesh.</p>

{h(2, 'quorum', 'Quorum: outage or partition?')}
<p>With three or more nodes, set <code>quorum = true</code>: before alerting a peer down, the node asks the <em>other</em> peers' <code>/healthz</code> whether they still see it. If any does, it is a network partition between you two, reported as a degraded peer link, not a false "node down".</p>
{aside('note', 'Note', 'Quorum is a no-op with fewer than three nodes: with two, nobody else can witness.')}

{h(2, 'vantage', 'Multi-vantage confirmation')}
<p>The peers don't just watch <em>each other</em>: they can confirm <strong>your monitors' outages</strong> from their vantage point.</p>
{ec('config.toml', confirm)}
<p>A monitor that confirms down locally triggers one concurrent round of probe requests to the peers before the alert goes out, and the alert carries the verdict:</p>
<div class="verdicts">
  <figure>{owls(['down', 'down', 'down'], names=['hora-a', 'hora-b', 'hora-c'], label='All three nodes see it down')}<figcaption><b style="color:var(--ink);font-weight:500">A real outage.</b><code>confirmed down from 3/3 vantage points</code></figcaption></figure>
  <figure>{owls(['down', 'up', 'up'], names=['hora-a', 'hora-b', 'hora-c'], label='Only this node sees it down')}<figcaption><b style="color:var(--ink);font-weight:500">Probably your fibre, not the service.</b><code>seen UP by hora-b - down from 1/3 vantage points</code></figcaption></figure>
</div>
{aside('caution', 'Softened, never silenced', 'A disputed down still alerts, with the softer wording. Geo-partial outages are real outages.')}
<p>Two Raspberry Pi at two homes become a distributed Pingdom. The same round is available on demand from the terminal with <a href="#"><code>hora probe &lt;id&gt; --confirm</code></a>: it prints the verdict both ways, "down for everyone or just me?" when the target is down here, and "up from 3/3 vantage points" when it is up.</p>
{ec('Terminal', term, term=True)}

{h(3, 'probe', 'How a probe request works')}
<p>The requester <code>POST</code>s to the peer's <code>/api/peer/probe</code> (derived from the origin of <code>ping_url</code>), authenticated with the same token pair as heartbeats: it presents its <code>ping_token</code>, which the responder checks against its <code>listen_token</code> for that peer.</p>
{aside('danger', 'The token is required', 'The id alone never authorizes a probe. The responder only probes targets present in its own configuration, matched on kind and target, so a leaked token cannot turn a peer into an SSRF relay. Both nodes must therefore know the monitor: share the config in git.')}

{h(3, 'failure', 'Failure behaviour')}
<p>Strictly fail-open, by construction. The incident record is written before the peers are even consulted.</p>
<div class="tbl"><table>
<thead><tr><th scope="col">When the peers…</th><th scope="col">The alert</th></tr></thead>
<tbody>
<tr><td>all see it down</td><td>{state('down', 'Sent')} "confirmed down from 3/3 vantage points"</td></tr>
<tr><td>see it up</td><td>{state('degraded', 'Sent, softened')} "seen UP by hora-b"</td></tr>
<tr><td>are slow, unreachable, or behind a wrong token</td><td>{state('down', 'Sent')} without the vantage annotation, never later than 10 s</td></tr>
<tr><td>don't know the target (404)</td><td>{state('none', 'Counts for nothing')} on either side</td></tr>
</tbody></table></div>
<p>The worst possible outcome is an alert <em>without</em> the vantage annotation: exactly what Hora sent before the feature.</p>

{h(2, 'external', 'External receivers')}
<ul>
<li><strong>OUT-only peer</strong>: a plain dead-man to an external service.</li>
</ul>
{ec('config.toml', ext)}
<ul>
<li><strong>Be watched by UptimeRobot</strong>: point a keyword monitor at this node's <code>/healthz</code> and match the keyword <code>ok</code> (the top-level <code>status</code> field is <code>ok</code> only while the node is fully healthy). No peer entry needed.</li>
</ul>
</div>
<div class="edit"><a href="#">{icon('gh')}Edit this page</a><span>Last updated: 2 Oct 2026</span></div>
<nav class="pager" aria-label="Pagination">
  <a class="prev" href="#"><span>{icon('arrow')}Previous</span><b>Per-group pages &amp; SLA reports</b></a>
  <a class="next" href="#"><span>Next{icon('arrow')}</span><b>Importing from Uptime Kuma</b></a>
</nav>
<footer class="sl-foot"><a class="watched" href="docs-home.html"><img src="assets/mark.svg" alt="" width="22" height="22"><span class="b">Hora</span></a><span>MIT licence · built with Starlight</span></footer>
"""
    body = f"""<header class="sl-top"><div class="in">
<a class="sign hora" href="docs-home.html"><img src="assets/mark.svg" alt="" width="32" height="32" style="border-radius:8px"><span class="name">Hora</span></a>
<button class="search" type="button" aria-label="Search the docs">{icon('search')}<span>Search</span><kbd>Ctrl K</kbd></button>
<span class="right"><a class="btn quiet" href="#" aria-label="Hora on GitHub">{icon('gh')}</a>{theme_button()}</span>
</div></header>
<div class="layout">
<nav class="side" aria-label="Main">{sidebar()}</nav>
<div style="min-width:0">
<div class="mob">
  <details class="menu"><summary>Menu<span class="muted">Guides</span></summary><div><nav class="side" aria-label="Main (mobile)">{sidebar()}</nav></div></details>
  <details class="tocl"><summary>On this page<span class="muted">Multi-vantage confirmation</span></summary><div style="padding:0 var(--s-4) var(--s-3)">{toc_list(False)}</div></details>
</div>
<main id="main" class="content">{article}</main>
</div>
<aside class="toc" aria-label="On this page"><h2>On this page</h2>{toc_list()}
<div class="meta"><a href="#">{icon('gh')}Edit this page</a><a href="#">{icon('arrow')}Back to top</a></div></aside>
</div>"""
    return shell('Mutual surveillance (peers) · Hora', body, 'Who watches the watcher: dead-man heartbeats between Hora nodes, quorum, and external receivers.', PAGE_CSS)


PAGES['docs-page.html'] = page_docs_page
