"""watchers.html: the operator's view of the mesh (peers, heartbeats, confirmations, a partition)."""
from common import *
from pages2 import PAGE_CSS

MESH_CSS = r"""
.signed { display: inline-flex; align-items: center; gap: 6px; min-height: var(--control-h-xs); padding: 0 var(--s-3); border-radius: var(--radius-pill); background: var(--accent-wash); color: var(--accent); font: var(--t-meta); white-space: nowrap; }
.signed svg { width: 14px; height: 14px; }
.lede { color: var(--ink-muted); max-width: var(--measure); margin-top: var(--s-2); }
.meshgrid { display: grid; gap: var(--s-5); grid-template-columns: minmax(0, 1fr); margin-top: var(--s-5); }
@media (min-width: 1100px) { .meshgrid { grid-template-columns: minmax(0, 1.1fr) minmax(0, 0.9fr); } }
.map { width: 100%; height: auto; display: block; overflow: visible; }
.map .link { fill: none; stroke: var(--st-up); stroke-width: 2.5; stroke-linecap: round; }
.map .link.split { stroke: var(--st-degraded); stroke-dasharray: 2 7; }
.map .pulse { fill: var(--st-up); }
.map .lab { font: 500 13px var(--font-text); fill: var(--ink); }
.map .sub { font: 400 11.5px var(--font-text); fill: var(--ink-muted); }
.map .ll { font: 500 11.5px var(--font-text); fill: var(--ink-muted); }
.map .ll.split { fill: var(--st-degraded-text); }
.map .ring { fill: var(--surface); stroke: var(--line); stroke-width: 1.5; }
.map .ring.me { stroke: var(--accent); stroke-width: 2; }
.nodes { list-style: none; margin: 0; padding: 0; display: grid; gap: var(--s-3); }
.node { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); padding: var(--s-4); display: grid; grid-template-columns: 44px 1fr auto; gap: var(--s-1) var(--s-3); align-items: center; }
.node .owl { width: 44px; height: 44px; grid-row: span 2; }
.node .n { font: var(--t-label); font-size: 1rem; }
.node .n small { font: var(--t-secondary); color: var(--ink-muted); margin-left: 6px; }
.node .s { text-align: right; font: var(--t-secondary); white-space: nowrap; }
.node .d { grid-column: 2 / -1; font: var(--t-secondary); color: var(--ink-muted); }
.node .d b { color: var(--ink); font-weight: 500; }
.node.split { background: var(--wash-degraded); border-color: transparent; }
.stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr)); gap: var(--s-3); }
.mx { width: 100%; border-collapse: collapse; font: var(--t-secondary); min-width: 32rem; }
.mx th, .mx td { padding: var(--s-2) var(--s-3); border-bottom: 1px solid var(--line); text-align: right; white-space: nowrap; }
.mx th:first-child, .mx td:first-child { text-align: left; padding-left: 0; }
.mx thead th { font: var(--t-meta); color: var(--ink-muted); }
.mx td .g { width: 12px; height: 12px; margin-right: 4px; }
.mx tr.disagree td { background: var(--wash-degraded); }
.tw { overflow-x: auto; }
"""


def page_watchers():
    hdr = header(current=None).replace('<button class="btn theme-btn"', f'<span class="signed">{icon("lock")}Operator</span><button class="btn theme-btn"', 1)
    # positions: Paris (left), Frankfurt (right), Montréal (top)
    P, F, M = (110, 250), (450, 250), (280, 70)
    mapsvg = f'''<svg class="map" viewBox="0 0 560 330" role="img" aria-label="Three watchers. Paris and Frankfurt exchange heartbeats normally; Frankfurt and Montréal too. Paris cannot reach Montréal, but Frankfurt still sees it: a network split between Paris and Montréal, not an outage.">
<path class="link" d="M{P[0]} {P[1]}L{F[0]} {F[1]}"/><path class="link" d="M{F[0]} {F[1]}L{M[0]} {M[1]}"/><path class="link split" d="M{P[0]} {P[1]}L{M[0]} {M[1]}"/>
<text class="ll" x="280" y="272" text-anchor="middle">heartbeat 0.4 s ago</text>
<text class="ll" x="388" y="150" text-anchor="start">heartbeat 21 s ago</text>
<text class="ll split" x="172" y="150" text-anchor="end">no answer for 6 min</text>
<g transform="translate({P[0]} {P[1]})"><circle class="ring me" r="34"/><svg class="owl" data-state="up" viewBox="6 6 40 40" width="48" height="48" x="-24" y="-26">{OWL_PARTS}</svg><text class="lab" y="56" text-anchor="middle">Paris</text><text class="sub" y="72" text-anchor="middle">this Hora</text></g>
<g transform="translate({F[0]} {F[1]})"><circle class="ring" r="34"/><svg class="owl" data-state="up" viewBox="6 6 40 40" width="48" height="48" x="-24" y="-26">{OWL_PARTS}</svg><text class="lab" y="56" text-anchor="middle">Frankfurt</text><text class="sub" y="72" text-anchor="middle">VPS</text></g>
<g transform="translate({M[0]} {M[1]})"><circle class="ring" r="34"/><svg class="owl" data-state="degraded" viewBox="6 6 40 40" width="48" height="48" x="-24" y="-26">{OWL_PARTS}</svg><text class="lab" x="48" y="-4">Montréal</text><text class="sub" x="48" y="12">homelab</text></g>
</svg>'''
    body = f"""{hdr}
<main id="main" class="page">
<nav class="crumbs" aria-label="Breadcrumb"><a href="status-ok.html">Status</a><span class="sep" aria-hidden="true">/</span><span aria-current="page">Watchers</span></nav>
<div class="title"><h1>Watchers</h1><span class="state-big" style="background:var(--wash-degraded)">{state('degraded', '1 link split')}</span></div>
<p class="lede">Three <b>Hora</b> nodes watch your services and each other. Each one pings the others while it is healthy, and asks them to check before it calls an outage.</p>

<div class="meshgrid">
  <section class="panel" aria-labelledby="mp"><div class="panel-head"><h2 id="mp">The mesh</h2><span class="muted" style="font:var(--t-secondary);margin-left:auto">Quorum on · 3 nodes</span></div>
    {mapsvg}
    <div class="key"><span><i></i>Heartbeats flowing</span><span><i class="t"></i>Can't reach directly</span></div>
  </section>
  <section aria-labelledby="nd"><div class="section-head"><h2 id="nd" style="font:var(--t-group)">Nodes</h2></div>
  <ul class="nodes">
    <li class="node">{owl('up', 44)}<span class="n">Paris<small>hora-par · this node</small></span><span class="s">{state('up', 'Well')}</span><span class="d">Scheduler ticking, database writable. Sends heartbeats to 2 peers.</span></li>
    <li class="node">{owl('up', 44)}<span class="n">Frankfurt<small>hora-fra</small></span><span class="s">{state('up', 'Well')}</span><span class="d">Last heartbeat <b>0.4 s ago</b> · v0.9.0 · answers probe requests</span></li>
    <li class="node split">{owl('degraded', 44)}<span class="n">Montréal<small>hora-mtl</small></span><span class="s">{state('degraded', 'Split')}</span><span class="d">No heartbeat here for <b>6 min</b>, but Frankfurt saw it <b>21 s ago</b>. The route between Paris and Montréal is broken; Montréal itself is fine. Reported as a split, not as down.</span></li>
  </ul></section>
</div>

<section class="section" aria-labelledby="cf"><div class="section-head"><h2 id="cf">Confirmations, last 30 days</h2><span class="fact">When a node sees a service down, it asks the others first</span></div>
<div class="stats">
  <div class="tile"><span class="k">Asked</span><span class="v">17</span><span class="n">times a node saw a service down</span></div>
  <div class="tile"><span class="k">Confirmed</span><span class="v">12</span><span class="n">{state('down', 'Real outages: paged')}</span></div>
  <div class="tile"><span class="k">Local only</span><span class="v">5</span><span class="n">{state('degraded', 'Network near one node')}</span></div>
  <div class="tile"><span class="k">No answer</span><span class="v">0</span><span class="n">Peers never delay an alert past 10 s</span></div>
</div></section>

<section class="section" aria-labelledby="lx"><div class="section-head"><h2 id="lx">Each service, from each place</h2><span class="fact">Typical answer time over 24 h</span></div>
<div class="tw panel" style="padding:var(--s-2) var(--s-4)"><table class="mx">
<thead><tr><th scope="col">Service</th><th scope="col">Paris</th><th scope="col">Frankfurt</th><th scope="col">Montréal</th></tr></thead>
<tbody>
<tr><td>Website</td><td>{glyph('up')}42 ms</td><td>{glyph('up')}38 ms</td><td>{glyph('up')}131 ms</td></tr>
<tr class="disagree"><td>Public API</td><td>{glyph('down')}<span class="word down">Down here</span></td><td>{glyph('up')}61 ms</td><td>{glyph('up')}142 ms</td></tr>
<tr><td>Card payments</td><td>{glyph('up')}240 ms</td><td>{glyph('up')}221 ms</td><td>{glyph('up')}386 ms</td></tr>
<tr><td>Invoice emails</td><td>{glyph('up')}1.2 s</td><td>{glyph('up')}1.1 s</td><td>{glyph('none')}<span class="word none">Not watched there</span></td></tr>
<tr><td>Database</td><td>{glyph('up')}3 ms</td><td>{glyph('none')}<span class="word none">Private, Paris only</span></td><td>{glyph('none')}<span class="word none">Private</span></td></tr>
</tbody></table></div>
<p class="mock-note"><b>Public or not.</b> Visitors of the status page see "checked from 3 places" and, when places disagree, one calm sentence. Node names, heartbeats and this table stay behind the operator token.</p>
</section>
</main>
{footer()}"""
    return shell('Watchers · Pelican status', body, 'The Hora mesh: who watches what, and each other.', PAGE_CSS + MESH_CSS)


PAGES = {'watchers.html': page_watchers}
