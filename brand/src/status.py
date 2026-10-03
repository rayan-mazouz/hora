"""status-ok, status-incident, status-maintenance, status-scale."""
import random
from common import *

STATUS_CSS = r"""
main { padding-bottom: var(--s-4); }
.when { font: var(--t-meta); color: var(--accent); letter-spacing: 0.02em; margin-bottom: var(--s-2); }
.hero.degraded .when { color: var(--st-degraded-text); } .hero.down .when { color: var(--st-down-text); } .hero.maint .when { color: var(--ink-muted); }
.follow { margin-left: auto; }
.svc-head { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s-3); margin-bottom: var(--s-4); }
.svc-head h2 { font: var(--t-group); font-size: 1.375rem; }
.svc-head .fact { font: var(--t-secondary); color: var(--ink-muted); }
.density { margin-left: auto; display: inline-flex; padding: 3px; gap: 2px; border-radius: var(--radius-pill); background: var(--ground-raised); }
.density input { position: absolute; opacity: 0; pointer-events: none; }
.density label { min-height: 30px; padding: 0 var(--s-3); display: inline-flex; align-items: center; border-radius: var(--radius-pill); font: var(--t-meta); color: var(--ink-muted); cursor: pointer; }
.density input:checked + label { background: var(--surface); color: var(--ink); box-shadow: var(--shadow); }
.density input:focus-visible + label { outline: var(--focus-ring); outline-offset: 1px; }
.spark, .facts { display: none; }
/* cards: the same rows, each in its own frame, with a response-time line (one path) */
body:has(#dens-cards:checked) .list { background: none; border: 0; display: grid; gap: var(--s-3); overflow: visible; }
body:has(#dens-cards:checked) .list > li { border: 1px solid var(--line); border-radius: var(--radius-xl); background: var(--surface); }
body:has(#dens-cards:checked) .list > li + li { border-top: 1px solid var(--line); }
body:has(#dens-cards:checked) .row { padding: var(--s-4); }
body:has(#dens-cards:checked) .spark { display: block; grid-column: 1 / -1; height: 36px; width: 100%; }
body:has(#dens-cards:checked) .facts { display: flex; flex-wrap: wrap; grid-column: 1 / -1; gap: var(--s-1) var(--s-4); font: var(--t-secondary); color: var(--ink-muted); }
.facts b { color: var(--ink); font-weight: 500; }
.spark path.l { fill: none; stroke: var(--accent-soft); stroke-width: 1.5; vector-effect: non-scaling-stroke; }
.spark path.a { fill: var(--accent-wash); opacity: 0.7; }
.under { margin-top: var(--s-5); }
.past { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--line); max-width: var(--measure-wide); }
.past li { display: grid; grid-template-columns: 6.5rem 1fr auto; gap: var(--s-3); padding: var(--s-3) 0; border-bottom: 1px solid var(--line); align-items: baseline; }
.past time { font: var(--t-meta); color: var(--ink-muted); }
.past .t { font: var(--t-label); }
.past .t a { color: var(--ink); text-decoration: none; }
.past .t a:hover { text-decoration: underline; }
.past .t small { display: block; font: var(--t-secondary); color: var(--ink-muted); margin-top: 2px; }
.past .r { font: var(--t-meta); white-space: nowrap; display: inline-flex; gap: 6px; align-items: center; }
@media (max-width: 599px) { .past li { grid-template-columns: 1fr auto; } .past time { grid-column: 1 / -1; } }
.quiet-line { display: flex; align-items: center; gap: var(--s-3); color: var(--ink-muted); padding: var(--s-4) 0; }

/* what's wrong now: the affected services, up front, with their story */
.now { display: grid; gap: var(--s-3); }
.now-card { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); padding: var(--s-4) var(--s-5); }
.now-card .row { padding: 0; min-height: 0; }
.now-card .row .name { font-size: 1.0625rem; }
.now-card .story { margin-top: var(--s-3); display: grid; gap: var(--s-2); font: var(--t-secondary); color: var(--ink-muted); }
.now-card .story b { color: var(--ink); font-weight: 500; }
.updates { list-style: none; margin: var(--s-4) 0 0; padding: 0 0 0 var(--s-4); border-left: 2px solid var(--line); display: grid; gap: var(--s-3); }
.updates li { position: relative; }
.updates li::before { content: ''; position: absolute; left: calc(-1 * var(--s-4) - 6px); top: 6px; width: 10px; height: 10px; border-radius: 50%; background: var(--surface); border: 2px solid var(--ink-muted); }
.updates li:first-child::before { border-color: var(--accent); background: var(--accent); }
.updates .k { font: var(--t-label); }
.updates time { font: var(--t-meta); color: var(--ink-muted); margin-left: var(--s-2); }
.updates p { font: var(--t-secondary); color: var(--ink); margin-top: 2px; max-width: var(--measure); }
.seen { display: flex; flex-wrap: wrap; gap: var(--s-2); }
.seen .chip { min-height: 28px; font: var(--t-meta); }

/* scale: groups as one summary row, a disclosure; only what is wrong is open */
.filters { display: flex; flex-wrap: wrap; gap: var(--s-2); align-items: center; margin-bottom: var(--s-4); }
.search { display: flex; align-items: center; gap: var(--s-2); min-height: var(--control-h); padding: 0 var(--s-3); border: 1px solid var(--line); border-radius: var(--radius); background: var(--surface); flex: 1 1 16rem; max-width: 24rem; color: var(--ink-muted); }
.search input { border: 0; background: none; font: var(--t-body); color: var(--ink); width: 100%; min-width: 0; outline: none; }
.search:focus-within { outline: var(--focus-ring); outline-offset: var(--focus-offset); }
.seg { display: inline-flex; gap: var(--s-1); flex-wrap: wrap; }
.seg a { display: inline-flex; align-items: center; gap: 6px; min-height: var(--control-h-s); padding: 0 var(--s-3); border-radius: var(--radius-pill); font: var(--t-label); color: var(--ink-muted); text-decoration: none; border: 1px solid transparent; }
.seg a[aria-current] { background: var(--surface); border-color: var(--line); color: var(--ink); }
.seg a b { font-weight: 500; color: var(--ink-muted); }
.gl { list-style: none; margin: 0; padding: 0; background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); }
.gl > li + li { border-top: 1px solid var(--line); }
.gl details > summary { list-style: none; cursor: pointer; display: grid; grid-template-columns: 20px var(--glyph) minmax(0, 1fr) auto; gap: var(--s-3); align-items: center; padding: var(--s-3) var(--s-4); min-height: 60px; border-radius: var(--radius-xl); }
.gl details > summary::-webkit-details-marker { display: none; }
.gl details > summary:hover { background: color-mix(in srgb, var(--ground-raised) 55%, transparent); }
.gl .chev { width: 16px; height: 16px; color: var(--ink-muted); transition: transform 0.2s; }
.gl details[open] .chev { transform: rotate(90deg); }
.gl .gname { font: var(--t-group); font-size: 1.0625rem; min-width: 0; }
.gl .gname small { display: block; font: var(--t-secondary); color: var(--ink-muted); }
.gl .gsum { text-align: right; font: var(--t-secondary); color: var(--ink-muted); white-space: nowrap; }
.gl .gsum .word { font: var(--t-label); }
@media (max-width: 599px) { .gl details > summary { grid-template-columns: 20px var(--glyph) minmax(0, 1fr); } .gl .gsum { grid-column: 3; text-align: left; } }
.gl .inner { padding: 0 var(--s-4) var(--s-4); }
.compact { list-style: none; margin: 0; padding: 0; columns: 3 20rem; column-gap: var(--s-5); }
.compact li { break-inside: avoid; display: grid; grid-template-columns: var(--glyph) minmax(0, 1fr) auto; gap: var(--s-2); align-items: center; min-height: 40px; border-bottom: 1px solid var(--line); font: var(--t-secondary); }
.compact li a { color: var(--ink); text-decoration: none; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.compact li a:hover { text-decoration: underline; }
.compact li .pct { color: var(--ink-muted); }
.compact li .g { width: 12px; height: 12px; }
.more { display: flex; gap: var(--s-3); align-items: center; padding-top: var(--s-3); font: var(--t-secondary); color: var(--ink-muted); flex-wrap: wrap; }
.bar.mini { height: 14px; margin-top: 8px; max-width: 22rem; }
.bar.mini svg { width: 100%; margin: 0; -webkit-mask-image: repeating-linear-gradient(90deg, #000 0 calc(100% / 90 - 1px), transparent calc(100% / 90 - 1px) calc(100% / 90)); mask-image: repeating-linear-gradient(90deg, #000 0 calc(100% / 90 - 1px), transparent calc(100% / 90 - 1px) calc(100% / 90)); }
.weight { font: var(--t-meta); color: var(--ink-muted); }
@media (max-width: 599px) { .gl .gsum .pctx { display: none; } }
"""


def spark(seed, spike=None):
    rnd = random.Random(seed)
    pts, v = [], 40
    for i in range(48):
        v = max(10, min(70, v + rnd.uniform(-6, 6)))
        y = v + (40 if spike and spike[0] <= i <= spike[1] else 0)
        pts.append((i * 100 / 47, 100 - y))
    line = 'M' + ' '.join(f'{x:.1f} {y:.1f}' for x, y in pts)
    area = line + ' V100 H0Z'
    return (f'<svg class="spark" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">'
            f'<path class="a" d="{area}"/><path class="l" d="{line}"/></svg>')


def mrow(name, st, statetext, d, pct, why='', small='', typical='84 ms', seed=1, spike=None):
    whyh = f'<span class="why">{why}</span>' if why else ''
    sm = f'<small>{small}</small>' if small else ''
    return (f'<li><div class="row {st}">{glyph(st)}<span class="name">{name}{sm}</span>'
            f'<span class="state"><span class="word {st}">{statetext}</span> <span class="muted">· {pct}</span></span>{whyh}'
            f'{bar(d, pct)}'
            f'<span class="facts">{vfacts(typical)}</span>{spark(seed, spike)}</div></li>')


def vfacts(typical):
    if typical.endswith(' ms'):
        v = int(typical.split()[0])
        return (f'<span>Paris <b>{v} ms</b></span><span>Frankfurt <b>{round(v * 0.9 + 4)} ms</b></span>'
                f'<span>Montréal <b>{round(v * 1.6 + 70)} ms</b></span>')
    return f'<span>Response <b>{typical}</b></span><span>from 3 places</span>'


def group(title, fact, rows):
    return (f'<section class="grp" aria-labelledby="g-{title[:4]}"><div class="group-head"><h3 id="g-{title[:4]}">{title}</h3>'
            f'<span class="fact">{fact}</span></div><ul class="list">{"".join(rows)}</ul>'
            f'<div class="bar-legend" aria-hidden="true"><span class="d90">90 days ago</span><span class="d30">30 days ago</span><span>Today</span></div></section>')


DENSITY = ('<fieldset class="density" style="border:0;margin:0 0 0 auto"><legend class="vh">Layout</legend>'
           '<input type="radio" name="dens" id="dens-rows" checked><label for="dens-rows">Rows</label>'
           '<input type="radio" name="dens" id="dens-cards"><label for="dens-cards">Cards</label></fieldset>')


def services(groups, fact):
    return (f'<section class="section" aria-labelledby="svc"><div class="svc-head"><h2 id="svc">Services</h2>'
            f'<span class="fact">{fact}</span>{DENSITY}</div>'
            f'<div class="groups">{"".join(groups)}</div>'
            f'<details class="under"><summary class="btn quiet" style="display:inline-flex">How to read the bars</summary>'
            f'<div style="padding:var(--s-3) 0">{LEGEND}</div></details></section>')


def pelican_groups(variant='ok'):
    U = lambda s, **k: days(s, **k)
    g1 = [
        mrow('Website', 'up', 'Up', U(1), '100 %', small='pelican.app', seed=1),
        mrow('Web app', 'up', 'Up', U(2, slow=[61]), '99.99 %', seed=2, typical='112 ms'),
        mrow('Public API', 'up', 'Up', U(3, down=[71]), '99.97 %', seed=3, typical='64 ms'),
        mrow('Mobile sync', 'up', 'Up', U(4), '100 %', seed=4, typical='71 ms'),
    ]
    g2 = [
        mrow('Card payments', 'up', 'Up', U(5), '100 %', seed=5, typical='240 ms'),
        mrow('Bank transfers (SEPA)', 'up', 'Up', U(6), '100 %', seed=6, typical='310 ms'),
        mrow('Invoice emails', 'up', 'Up', U(7, slow=[72]), '99.98 %', seed=7, typical='1.2 s'),
        mrow('Webhooks', 'up', 'Up', U(8), '100 %', seed=8, typical='95 ms'),
    ]
    g3 = [
        mrow('Database', 'up', 'Up', U(9), '100 %', seed=9, typical='3 ms'),
        mrow('File storage', 'up', 'Up', U(10), '100 %', seed=10, typical='48 ms'),
        mrow('Search', 'up', 'Up', U(11, maint=[50]), '99.99 %', seed=11, typical='38 ms'),
        mrow('Background jobs', 'up', 'Up', U(12), '100 %', seed=12, typical='heartbeat'),
        mrow('PDF rendering', 'up', 'Up', U(13, slow=[30, 31]), '99.95 %', seed=13, typical='620 ms'),
        mrow('Status page', 'up', 'Up', U(14, none_before=20), '100 %', small='since 14 Jul', seed=14, typical='22 ms'),
    ]
    return g1, g2, g3


def page_ok():
    g1, g2, g3 = pelican_groups()
    body = f"""{header(current='Status')}
<main id="main" class="page">
<section class="hero" aria-labelledby="h">
  {owl('up', 96, cls='hero-owl')}
  <div>
    <p class="when">Saturday 3 October, 14:32 UTC</p>
    <h1 id="h">Everything is running.</h1>
    <p class="sub">All <b>14 services</b> are up. Checked 12 s ago from 3 places.</p>
  </div>
  <div class="chips">
    <span class="chip up">{glyph('up')}14 up</span>
    <span class="chip">No incident in 15 days</span>
    <a class="chip maint" href="status-maintenance.html">{glyph('maint')}Planned: Tue 7 Oct, 22:00 UTC</a>
    <a class="btn quiet follow" href="#">{icon('feed')}Follow updates</a>
  </div>
</section>
{services([group('Pelican', '4 services', g1), group('Payments and email', '4 services', g2), group('Behind the scenes', '6 services', g3)], '14 services in 3 groups')}
<section class="section" aria-labelledby="past"><div class="section-head"><h2 id="past">Recent incidents</h2><a class="fact" href="monitor.html">All history</a></div>
<ul class="past">
  <li><time datetime="2026-09-18">18 Sep</time><span class="t"><a href="incident.html">Public API was down for 41 minutes</a><small>The database ran out of connections. Fixed the same evening.</small></span><span class="r">{state('up', 'Resolved')}</span></li>
  <li><time datetime="2026-09-12">12 Sep</time><span class="t"><a href="incident.html">Invoice emails were slow</a><small>Sending took up to 6 minutes for an hour.</small></span><span class="r">{state('up', 'Resolved')}</span></li>
</ul></section>
<section class="section" aria-labelledby="wt"><div class="section-head"><h2 id="wt">Who keeps watch</h2></div>
<div class="watchers">{owls(['up','up','up'], label='Three owls asleep: all three places see every service up')}<p style="max-width:40rem">Watched from <b style="color:var(--ink);font-weight:500">Paris, Frankfurt and Montréal</b>. Each place checks every service; a service counts as down only when two places agree, so a hiccup near one of them never shows here.</p></div></section>
</main>
{footer()}"""
    return shell('Pelican status', body, 'Is Pelican working? Live status of every Pelican service.', STATUS_CSS)


def page_incident():
    U = lambda s, **k: days(s, **k)
    pay = U(5, down=[89]); mail = U(7, slow=[72, 89])
    g1 = [
        mrow('Website', 'up', 'Up', U(1), '100 %', small='pelican.app', seed=1),
        mrow('Web app', 'up', 'Up', U(2, slow=[61]), '99.99 %', seed=2, typical='112 ms'),
        mrow('Public API', 'up', 'Up', U(3, down=[71]), '99.97 %', seed=3, typical='64 ms'),
        mrow('Mobile sync', 'up', 'Up', U(4), '100 %', seed=4, typical='71 ms'),
    ]
    g2 = [
        mrow('Card payments', 'down', 'Down', pay, '99.96 %', why='Since 14:02 UTC (38 min). Payments are refused, nothing is charged.', seed=5, spike=(40, 47), typical='timeout'),
        mrow('Bank transfers (SEPA)', 'up', 'Up', U(6), '100 %', seed=6, typical='310 ms'),
        mrow('Invoice emails', 'degraded', 'Slow', mail, '99.97 %', why='Emails leave about 4 minutes late.', seed=7, spike=(42, 47), typical='4 min'),
        mrow('Webhooks', 'up', 'Up', U(8), '100 %', seed=8, typical='95 ms'),
    ]
    g3 = [
        mrow('Database', 'up', 'Up', U(9), '100 %', seed=9, typical='3 ms'),
        mrow('File storage', 'up', 'Up', U(10), '100 %', seed=10, typical='48 ms'),
        mrow('Search', 'up', 'Up', U(11, maint=[50]), '99.99 %', seed=11, typical='38 ms'),
        mrow('Background jobs', 'up', 'Up', U(12), '100 %', seed=12, typical='heartbeat'),
        mrow('PDF rendering', 'up', 'Up', U(13, slow=[30, 31]), '99.95 %', seed=13, typical='620 ms'),
        mrow('Status page', 'up', 'Up', U(14, none_before=20), '100 %', small='since 14 Jul', seed=14, typical='22 ms'),
    ]
    body = f"""{header(current='Status')}
<main id="main" class="page">
<div class="callout warning" role="note">
  {glyph('degraded')}<p class="k">Announcement</p>
  <span></span><p>Some customers of Banque Rhône need to confirm card payments twice this week. This comes from their bank, not from Pelican.</p>
  <span></span><p class="when">Posted 1 Oct, 09:10 UTC · until 9 Oct</p>
</div>
<section class="hero down" aria-labelledby="h">
  {owl('down', 96, cls='hero-owl woke')}
  <div>
    <p class="when">Saturday 3 October, 14:40 UTC</p>
    <h1 id="h" aria-live="polite">Card payments are down.</h1>
    <p class="sub">Since <b>14:02 UTC</b> (38 min). Invoice emails are also slow. The other <b>12 services</b> are up.</p>
  </div>
  <div class="chips">
    <a class="chip down" href="#now">{glyph('down')}1 down</a>
    <a class="chip degraded" href="#now">{glyph('degraded')}1 slow</a>
    <span class="chip up">{glyph('up')}12 up</span>
    <a class="btn quiet follow" href="#">{icon('feed')}Follow updates</a>
  </div>
</section>

<section class="section" aria-labelledby="now" style="margin-top:var(--s-6)">
<div class="section-head"><h2 id="now">Right now</h2><span class="fact">Updated 14:40 UTC</span></div>
<div class="now">
  <article class="now-card">
    <div class="row down">{glyph('down')}<span class="name">Card payments</span><span class="state"><span class="word down">Down</span> <span class="muted">· 38 min</span></span></div>
    <div class="story">
      <p><b>What you'll notice:</b> paying an invoice by card fails with "try again later". No card is charged.</p>
      <div class="seen"><span class="muted" style="align-self:center">Confirmed from</span>
        <span class="vchip down">{glyph('down')}Paris</span><span class="vchip down">{glyph('down')}Frankfurt</span><span class="vchip down">{glyph('down')}Montréal</span><span class="muted" style="font:var(--t-meta)">3 of 3 places: not a local problem</span></div>
    </div>
    <ol class="updates" aria-label="Updates, newest first">
      <li><span class="k">Identified</span><time datetime="14:31">14:31 UTC</time><p>Our payment provider has an outage in its EU region. We are switching card payments to the backup route.</p></li>
      <li><span class="k">Investigating</span><time datetime="14:09">14:09 UTC</time><p>Card payments fail since 14:02. Bank transfers work.</p></li>
    </ol>
  </article>
  <article class="now-card">
    <div class="row degraded">{glyph('degraded')}<span class="name">Invoice emails</span><span class="state"><span class="word degraded">Slow</span> <span class="muted">· 21 min</span></span></div>
    <div class="story"><p><b>What you'll notice:</b> invoice emails arrive about 4 minutes late. None are lost.</p>
    <p>Probably linked: receipts of failed card payments are being retried.</p></div>
  </article>
</div>
</section>
{services([group('Pelican', 'All 4 up', g1), group('Payments and email', '1 down · 1 slow', g2), group('Behind the scenes', 'All 6 up', g3)], '14 services in 3 groups')}
</main>
{footer()}"""
    return shell('Card payments are down · Pelican status', body, 'Pelican: card payments are down since 14:02 UTC.', STATUS_CSS)


def page_maint():
    U = lambda s, **k: days(s, **k)
    g1 = [
        mrow('Website', 'up', 'Up', U(1), '100 %', small='pelican.app', seed=1),
        mrow('Web app', 'up', 'Up', U(2, slow=[61]), '99.99 %', seed=2, typical='112 ms'),
        mrow('Public API', 'up', 'Up', U(3, down=[71]), '99.97 %', seed=3, typical='64 ms'),
        mrow('Mobile sync', 'up', 'Up', U(4), '100 %', seed=4, typical='71 ms'),
    ]
    g3 = [
        mrow('Database', 'maint', 'Maintenance', U(9, maint=[89]), '100 %', why='Until 23:30 UTC. Upgrading to a new version.', seed=9, typical='3 ms'),
        mrow('Search', 'maint', 'Maintenance', U(11, maint=[50, 89]), '99.99 %', why='Until 23:30 UTC. Results may be a few minutes old.', seed=11, typical='38 ms'),
        mrow('File storage', 'up', 'Up', U(10), '100 %', seed=10, typical='48 ms'),
        mrow('Background jobs', 'up', 'Up', U(12), '100 %', seed=12, typical='heartbeat'),
        mrow('PDF rendering', 'up', 'Up', U(13, slow=[30, 31]), '99.95 %', seed=13, typical='620 ms'),
        mrow('Status page', 'up', 'Up', U(14, none_before=20), '100 %', small='since 14 Jul', seed=14, typical='22 ms'),
    ]
    g2 = [
        mrow('Card payments', 'up', 'Up', U(5), '100 %', seed=5, typical='240 ms'),
        mrow('Bank transfers (SEPA)', 'up', 'Up', U(6), '100 %', seed=6, typical='310 ms'),
        mrow('Invoice emails', 'up', 'Up', U(7, slow=[72]), '99.98 %', seed=7, typical='1.2 s'),
        mrow('Webhooks', 'up', 'Up', U(8), '100 %', seed=8, typical='95 ms'),
    ]
    body = f"""{header(current='Status')}
<main id="main" class="page">
<section class="hero maint" aria-labelledby="h">
  {owl('maint', 96, cls='hero-owl')}
  <div>
    <p class="when">Tuesday 7 October, 22:41 UTC</p>
    <h1 id="h">Planned maintenance, until 23:30.</h1>
    <p class="sub">We are upgrading the database. Pelican stays open; <b>search</b> may show results a few minutes old. Everything else is up.</p>
    <div style="margin-top:var(--s-4);max-width:28rem">
      <div style="display:flex;justify-content:space-between;font:var(--t-meta);color:var(--ink-muted)"><span>22:00</span><span>41 of 90 min</span><span>23:30</span></div>
      <div role="progressbar" aria-label="Maintenance window" aria-valuemin="0" aria-valuemax="90" aria-valuenow="41" aria-valuetext="41 of 90 minutes"
        style="height:8px;border-radius:999px;background:var(--surface);margin-top:6px;overflow:hidden;box-shadow:inset 0 0 0 1px var(--line)"><div style="width:45.5%;height:100%;background:var(--ink-muted);border-radius:999px"></div></div>
    </div>
  </div>
  <div class="chips">
    <span class="chip maint">{glyph('maint')}2 in maintenance</span>
    <span class="chip up">{glyph('up')}12 up</span>
    <a class="btn quiet follow" href="#">{icon('feed')}Follow updates</a>
  </div>
</section>
<div class="callout info" role="note">{glyph('info')}<p class="k">Next</p><span></span><p>Thursday 16 October, 06:00 to 06:15 UTC: certificate renewal on the API. No interruption expected.</p></div>
{services([group('Behind the scenes', '2 in maintenance · 4 up', g3), group('Pelican', 'All 4 up', g1), group('Payments and email', 'All 4 up', g2)], '14 services in 3 groups')}
</main>
{footer()}"""
    return shell('Planned maintenance · Pelican status', body, 'Pelican: planned database maintenance until 23:30 UTC.', STATUS_CSS)


# ---------------------------------------------------------------------------
# 712 monitors
# ---------------------------------------------------------------------------
def page_scale():
    rnd = random.Random(7)
    groups = [
        ('Shared hosting · fr-par', 'web servers, PHP pools, mail', 212, 'paris'),
        ('Customer sites · eu-north', 'client websites checked over HTTPS', 248, 'site'),
        ('Managed databases', 'PostgreSQL and MariaDB clusters', 96, 'db'),
        ('Object storage', 'S3 endpoints and gateways', 48, 'obj'),
        ('DNS and mail relays', 'authoritative DNS, SMTP relays', 64, 'dns'),
        ('Control panel and API', 'what customers log in to', 22, 'panel'),
        ('Office and VPN', 'internal, shown to staff only', 22, 'office'),
    ]
    total = sum(g[2] for g in groups)  # 712
    names_site = ['atelier-loire.fr', 'boulangerie-maret.fr', 'cabinet-varenne.com', 'domaine-sauzet.fr', 'ecole-montessori-ouest.org', 'fleurs-de-lys.shop',
                  'garage-pelletier.fr', 'hotel-les-pins.com', 'kine-aubrac.fr', 'la-cave-a-mots.fr', 'lycee-jean-mace.org', 'mairie-saint-aulaye.fr']
    attn = f"""
<section class="section" aria-labelledby="attn" style="margin-top:var(--s-6)">
<div class="section-head"><h2 id="attn">Needs attention</h2><span class="fact">3 of {total}</span></div>
<ul class="list" style="max-width:var(--measure-wide)">
  <li><a class="row down" href="monitor.html">{glyph('down')}<span class="name">web-14.fr-par<small>Shared hosting · fr-par</small></span><span class="state"><span class="word down">Down</span> <span class="muted">· 9 min</span></span><span class="why">No answer on port 443. 38 sites hosted here; they are listed in its history.</span></a></li>
  <li><a class="row degraded" href="monitor.html">{glyph('degraded')}<span class="name">pg-cluster-03<small>Managed databases</small></span><span class="state"><span class="word degraded">Slow</span> <span class="muted">· 1.8 s</span></span><span class="why">Queries take 1.8 s (usually 40 ms).</span></a></li>
  <li><a class="row degraded" href="monitor.html">{glyph('degraded')}<span class="name">smtp-relay-2<small>DNS and mail relays</small></span><span class="state"><span class="word degraded">Slow</span> <span class="muted">· 3.1 s</span></span><span class="why">Mail leaves with a short delay.</span></a></li>
</ul></section>"""

    def gsum(i, g):
        name, desc, n, kind = g
        if i == 0:
            return f'{state("down", "1 down")} <span class="muted">· {n - 1} up</span>', 'down'
        if i == 2:
            return f'{state("degraded", "1 slow")} <span class="muted">· {n - 1} up</span>', 'degraded'
        if i == 4:
            return f'{state("degraded", "1 slow")} <span class="muted">· {n - 1} up</span>', 'degraded'
        return f'{state("up", f"All {n} up")}', 'up'

    items = []
    for i, g in enumerate(groups):
        name, desc, n, kind = g
        s, st = gsum(i, g)
        pct = ['99.97 %', '99.99 %', '99.96 %', '100 %', '99.99 %', '99.98 %', '100 %'][i]
        gd = days(i, down=[89] if i == 0 else ([40] if i == 1 else []), slow=[89] if i in (2, 4) else ([12, 13] if i == 5 else []))
        mini = f'<span class="bar mini">{bar_svg(gd)}</span>'
        inner = ''
        open_ = ''
        if i == 0:
            open_ = ' open'
            lis = []
            shown = 24
            for k in range(1, shown + 1):
                nm = f'web-{k:02d}.fr-par'
                stt = 'down' if k == 14 else 'up'
                p = '99.4 %' if k == 14 else rnd.choice(['100 %', '100 %', '99.99 %', '99.98 %'])
                w = 'Down' if stt == 'down' else 'Up'
                lis.append(f'<li>{glyph(stt)}<a href="monitor.html">{nm}</a><span class="pct">{"<span class=\"word down\">Down</span>" if stt=="down" else p}</span></li>')
            inner = (f'<div class="inner"><ul class="compact" aria-label="{name}: {n} services">{"".join(lis)}</ul>'
                     f'<div class="more"><span>Showing 24 of 212, problems first, then A to Z.</span><a class="btn" href="#">Show all 212</a><a class="btn quiet" href="#">Next 24</a></div></div>')
        items.append(
            f'<li><details{open_}><summary>{icon("chev", "chev")}{glyph(st)}'
            f'<span class="gname">{name}<small>{n} services · {desc}</small>{mini}</span>'
            f'<span class="gsum">{s}<br><span class="pctx">{pct} over 90 days</span></span></summary>{inner}</details></li>')

    body = f"""{header(op='Norrland Hosting', op_initial='N', current='Status')}
<main id="main" class="page">
<section class="hero degraded" aria-labelledby="h">
  {owl('degraded', 96)}
  <div>
    <p class="when">Saturday 3 October, 14:32 UTC</p>
    <h1 id="h">Almost everything is running.</h1>
    <p class="sub"><b>709 of {total}</b> services are up. One web server is down and two are slow; the rest is fine.</p>
  </div>
  <div class="chips">
    <a class="chip down" href="#attn">{glyph('down')}1 down</a>
    <a class="chip degraded" href="#attn">{glyph('degraded')}2 slow</a>
    <span class="chip up">{glyph('up')}709 up</span>
    <a class="btn quiet follow" href="#">{icon('feed')}Follow updates</a>
  </div>
</section>
{attn}
<section class="section" aria-labelledby="all">
<div class="svc-head"><h2 id="all">All services</h2><span class="fact">{total} in {len(groups)} groups · groups where all is well stay folded</span></div>
<form class="filters" role="search" action="#" onsubmit="return false">
  <label class="search">{icon('search')}<span class="vh">Find a service</span><input type="search" name="q" placeholder="Find a service, e.g. web-14"></label>
  <nav class="seg" aria-label="Show">
    <a href="#" aria-current="true">All <b>{total}</b></a>
    <a href="#attn">{glyph('down')}Needs attention <b>3</b></a>
    <a href="#">{glyph('maint')}Maintenance <b>0</b></a>
  </nav>
</form>
<ul class="gl">{"".join(items)}</ul>
<p class="mock-note"><b>Why it stays light.</b> Folded groups send one summary row and one group bar, not their monitors. The open group sends 24 compact rows (no bar, no chart); "Show all" is a plain link to <code>/status/shared-hosting</code>. This mockup weighs <b>47 KB</b>: 16 KB of content for {total} monitors, the rest is its styles (one cached file in the real app). Today's page is 6.6 MB. Response-time charts live on each monitor's own page.</p>
</section>
</main>
{footer()}"""
    return shell('Norrland Hosting status', body, 'Live status of Norrland Hosting: 712 services.', STATUS_CSS)


LOCAL_CSS = r"""
.signed { display: inline-flex; align-items: center; gap: 6px; min-height: var(--control-h-xs); padding: 0 var(--s-3); border-radius: var(--radius-pill); background: var(--accent-wash); color: var(--accent); font: var(--t-meta); white-space: nowrap; }
.signed svg { width: 14px; height: 14px; }
details.why { margin-top: var(--s-6); background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); }
details.why > summary { cursor: pointer; list-style: none; display: flex; flex-wrap: wrap; align-items: center; gap: var(--s-2) var(--s-3); padding: var(--s-4) var(--s-5); }
details.why > summary::-webkit-details-marker { display: none; }
details.why > summary .chev { width: 16px; height: 16px; color: var(--ink-muted); transition: transform 0.2s; }
details.why[open] > summary .chev { transform: rotate(90deg); }
details.why > summary h2 { font: var(--t-group); }
details.why > summary .tag { font: var(--t-meta); color: var(--ink-muted); }
.why-in { padding: 0 var(--s-5) var(--s-5); display: grid; gap: var(--s-5); grid-template-columns: minmax(0, 1fr); }
@media (min-width: 960px) { .why-in { grid-template-columns: 20rem minmax(0, 1fr); } }
.scene { background: var(--ground); border-radius: var(--radius-l); padding: var(--s-4); display: grid; gap: var(--s-3); justify-items: center; text-align: center; }
.scene .owls { width: 100%; max-width: 260px; }
.scene p { font: var(--t-secondary); color: var(--ink-muted); }
.verdict { font: 400 0.8125rem/1.6 var(--font-mono); background: var(--ground-raised); border-radius: var(--radius-l); padding: var(--s-3) var(--s-4); color: var(--ink); overflow-x: auto; }
.vt { width: 100%; border-collapse: collapse; font: var(--t-secondary); }
.vt th, .vt td { text-align: left; padding: var(--s-2) var(--s-2) var(--s-2) 0; border-bottom: 1px solid var(--line); }
.vt th { font: var(--t-meta); color: var(--ink-muted); }
.vt td:last-child, .vt th:last-child { text-align: right; padding-right: 0; }
.steps { list-style: none; margin: 0; padding: 0; display: grid; gap: var(--s-2); font: var(--t-secondary); }
.steps li { display: grid; grid-template-columns: var(--glyph) 4.5rem 1fr; gap: var(--s-2); align-items: baseline; }
.steps time { font: 400 0.8125rem/1.25rem var(--font-mono); color: var(--ink-muted); }
.paged { text-align: left; display: grid; grid-template-columns: auto 1fr; gap: var(--s-3); align-items: center; padding: var(--s-3) var(--s-4); border-radius: var(--radius-l); background: var(--accent-wash); }
.paged b { font: var(--t-label); display: block; }
.paged p { font: var(--t-secondary); }
"""


def page_local():
    U = lambda s, **k: days(s, **k)
    g1 = [
        mrow('Website', 'up', 'Up', U(1), '100 %', small='pelican.app', seed=1),
        mrow('Web app', 'up', 'Up', U(2, slow=[61]), '99.99 %', seed=2, typical='112 ms'),
        mrow('Public API', 'up', 'Up', U(3, down=[71]), '99.97 %', why='Up from Frankfurt and Montréal. Paris cannot reach it since 09:14: a network problem near Paris.', seed=3, typical='64 ms'),
        mrow('Mobile sync', 'up', 'Up', U(4), '100 %', seed=4, typical='71 ms'),
    ]
    g2 = [
        mrow('Card payments', 'up', 'Up', U(5), '100 %', seed=5, typical='240 ms'),
        mrow('Bank transfers (SEPA)', 'up', 'Up', U(6), '100 %', seed=6, typical='310 ms'),
        mrow('Invoice emails', 'up', 'Up', U(7, slow=[72]), '99.98 %', seed=7, typical='1.2 s'),
        mrow('Webhooks', 'up', 'Up', U(8), '100 %', seed=8, typical='95 ms'),
    ]
    hdr = header(current='Status').replace('<button class="btn theme-btn"', f'<span class="signed">{icon("lock")}Operator</span><button class="btn theme-btn"', 1)
    body = f"""{hdr}
<main id="main" class="page">
<section class="hero" aria-labelledby="h">
  {owl('up', 96)}
  <div>
    <p class="when">Saturday 3 October, 09:26 UTC</p>
    <h1 id="h">Everything is running.</h1>
    <p class="sub">All <b>14 services</b> are up for our users. One of our three checkpoints has a network problem; it does not affect you.</p>
  </div>
  <div class="chips">
    <span class="chip up">{glyph('up')}14 up</span>
    <span class="chip">{icon('pin')}Paris checkpoint: network issue</span>
    <a class="btn quiet follow" href="#">{icon('feed')}Follow updates</a>
  </div>
</section>
<div class="callout info" role="note">{glyph('info')}<p class="k">Not an outage</p><span></span>
<p>Since 09:14, our Paris checkpoint can't reach the Public API. Frankfurt and Montréal reach it normally, so the problem is on the road from Paris, not in Pelican.</p></div>

<details class="why" open>
<summary>{icon('chev', 'chev')}<h2>Why nobody was woken</h2><span class="tag">Operators only · hidden from the public page</span></summary>
<div class="why-in">
  <div class="scene">
    {owls(['down', 'up', 'up'], names=['Paris', 'Frankfurt', 'Montréal'], label='Three owls: Paris awake, Frankfurt and Montréal asleep. Only Paris sees a problem.')}
    <p>Paris woke up and asked the others before calling anyone. They both see the API up.</p>
    <div class="paged">{glyph('info')}<div><b>Sent as a notice, not a page</b><p>One message to #ops, worded as a local problem. No phone rang.</p></div></div>
  </div>
  <div style="display:grid;gap:var(--s-4);align-content:start">
    <div><p class="eyebrow" style="margin-bottom:var(--s-2)">Hora's verdict, as sent</p>
    <div class="verdict">Public API down here (Paris): seen UP by Frankfurt, Montréal. Down from 1/3 vantage points (network issue near this node?)</div></div>
    <table class="vt"><thead><tr><th scope="col">Checked from</th><th scope="col">Sees Public API</th><th scope="col">Answer in</th></tr></thead><tbody>
      <tr><td>Paris <span class="muted">· this Hora</span></td><td>{state('down', 'Down')}</td><td>timeout, 10 s</td></tr>
      <tr><td>Frankfurt <span class="muted">· peer</span></td><td>{state('up', 'Up')}</td><td>61 ms</td></tr>
      <tr><td>Montréal <span class="muted">· peer</span></td><td>{state('up', 'Up')}</td><td>142 ms</td></tr>
    </tbody></table>
    <ol class="steps" aria-label="What Hora did">
      <li>{glyph('degraded')}<time>09:13:32</time><span>Paris: first timeout, retried.</span></li>
      <li>{glyph('degraded')}<time>09:14:02</time><span>Paris: 3 failures in a row. Asks Frankfurt and Montréal to check.</span></li>
      <li>{glyph('up')}<time>09:14:03</time><span>Both answer within a second: up.</span></li>
      <li>{glyph('info')}<time>09:14:03</time><span>Notice sent with the verdict above. The public page keeps saying "Up".</span></li>
    </ol>
    <p class="muted" style="font:var(--t-secondary)">The Paris node's other checks work, so it is the route to the API host. <a href="watchers.html">See all watchers</a></p>
  </div>
</div>
</details>
{services([group('Pelican', 'All 4 up', g1), group('Payments and email', 'All 4 up', g2)], '14 services, 8 shown')}
<p class="mock-note"><b>Today and next.</b> Hora already asks its peers before alerting and writes this verdict into the alert; a disputed down is softened, never silenced. Sending a softened alert to a quieter channel than real outages ("a notice, not a page") is a <b>proposal</b> for a <code>notify_unconfirmed</code> setting.</p>
</main>
{footer()}"""
    return shell('Not an outage · Pelican status', body, 'Pelican: all services up; one checkpoint has a local network problem.', STATUS_CSS + LOCAL_CSS)
