"""monitor, incident, report, empty, docs-home."""
import random, math
from common import *

PAGE_CSS = r"""
.crumbs { display: flex; flex-wrap: wrap; gap: var(--s-2); font: var(--t-secondary); color: var(--ink-muted); margin-top: var(--s-5); }
.crumbs a { color: var(--ink-muted); }
.crumbs .sep { opacity: 0.6; }
.title { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s-3) var(--s-4); margin-top: var(--s-3); }
.title h1 { font: var(--t-page); font-size: clamp(1.75rem, 1.4rem + 1.4vw, 2.25rem); line-height: 1.15; }
.title .state-big { display: inline-flex; align-items: center; gap: 6px; padding: 4px var(--s-3) 4px var(--s-2); border-radius: var(--radius-pill); background: var(--wash-up); }
.title .state-big .word { font: var(--t-label); }
.target { margin-top: var(--s-2); font: var(--t-secondary); color: var(--ink-muted); }
.target code { color: var(--ink); }
.tiles { display: grid; gap: var(--s-3); grid-template-columns: repeat(auto-fill, minmax(min(100%, 15rem), 1fr)); margin-top: var(--s-5); }
.tile { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-l); padding: var(--s-4); display: grid; gap: var(--s-1); align-content: start; }
.tile .k { font: var(--t-secondary); color: var(--ink-muted); }
.tile .v { font: 500 1.75rem/1.2 var(--font-display); font-variant-numeric: tabular-nums lining-nums; }
.tile .v small { font: var(--t-secondary); color: var(--ink-muted); margin-left: 4px; }
.tile .n { font: var(--t-secondary); color: var(--ink-muted); }
.meter { height: 8px; border-radius: var(--radius-pill); background: var(--ground-raised); overflow: hidden; margin-top: var(--s-2); box-shadow: inset 0 0 0 1px var(--line); }
.meter > span { display: block; height: 100%; background: var(--accent-soft); border-radius: inherit; }
.panel { overflow-x: clip; background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); padding: var(--s-4) var(--s-5) var(--s-5); }
.panel-head { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s-3); margin-bottom: var(--s-4); }
.panel-head h2 { font: var(--t-group); }
.panel-head .tabs { margin-left: auto; }
.tabs { display: inline-flex; padding: 3px; gap: 2px; border-radius: var(--radius-pill); background: var(--ground-raised); }
.tabs a { min-height: 30px; padding: 0 var(--s-3); display: inline-flex; align-items: center; border-radius: var(--radius-pill); font: var(--t-meta); color: var(--ink-muted); text-decoration: none; }
.tabs a[aria-current] { background: var(--surface); color: var(--ink); box-shadow: var(--shadow); }
.chartwrap { position: relative; padding-left: 2.5rem; padding-bottom: 1.5rem; }
.chart { width: 100%; height: 200px; display: block; overflow: visible; }
.chart line, .chart path { vector-effect: non-scaling-stroke; }
.ylab span { position: absolute; left: 0; width: 2rem; text-align: right; translate: 0 -50%; font: var(--t-tiny); color: var(--ink-muted); }
.ylab { position: absolute; left: 0; top: 0; bottom: 1.5rem; width: 2rem; }
.ylab span { top: 0; }
.xlab { position: absolute; left: 2.5rem; right: 0; bottom: 0; display: flex; justify-content: space-between; font: var(--t-tiny); color: var(--ink-muted); }
.thr-l { position: absolute; right: 0; font: var(--t-tiny); color: var(--st-degraded-text); }
.unit { position: absolute; left: 0; top: -1.4rem; font: var(--t-tiny); color: var(--ink-muted); }
@media (max-width: 599px) { .xlab span:nth-child(even) { visibility: hidden; } }
.chart .grid line { stroke: var(--line); stroke-width: 1; }
.chart .ax { font: var(--t-tiny); fill: var(--ink-muted); }
.chart .band { fill: var(--accent-wash); }
.chart .p50 { fill: none; stroke: var(--accent); stroke-width: 2; stroke-linejoin: round; }
.chart .p95 { fill: none; stroke: var(--accent-soft); stroke-width: 1.25; stroke-dasharray: 3 3; }
.chart .thr { stroke: var(--st-degraded); stroke-width: 1.25; stroke-dasharray: 6 4; }
.chart .thr-l { font: var(--t-tiny); fill: var(--st-degraded-text); }
.key { display: flex; flex-wrap: wrap; gap: var(--s-2) var(--s-4); font: var(--t-secondary); color: var(--ink-muted); margin-top: var(--s-3); }
.key i { display: inline-block; width: 18px; height: 0; border-top: 2px solid var(--accent); vertical-align: middle; margin-right: 6px; }
.key i.d { border-top: 1.5px dashed var(--accent-soft); }
.key i.t { border-top: 1.5px dashed var(--st-degraded); }
/* 90 days, one button per day: this page shows one monitor, so 90 nodes are fine here */
.days { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(90, 1fr); gap: 2px; height: 36px; align-items: end; }
.days li { height: 100%; position: relative; }
.days button { all: unset; display: block; width: 100%; height: 30px; border-radius: 1px; background: var(--st-up); cursor: pointer; position: relative; }
.days button.d { background: var(--st-degraded); height: 21px; margin-top: 9px; }
.days button.x { background: var(--st-down); height: 36px; }
.days button.m { background: var(--st-maint); height: 15px; margin-top: 15px; }
.days button.n { background: var(--st-none); height: 5px; margin-top: 25px; }
.days button:hover { filter: brightness(1.15); }
.days button:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
.days button[aria-pressed='true'] { box-shadow: 0 0 0 2px var(--ground), 0 0 0 4px var(--ink); }
.days button::after { content: attr(data-tip); visibility: hidden; position: absolute; bottom: calc(100% + 10px); left: 50%; translate: -50% 0; background: var(--ink); color: var(--ground);
  font: var(--t-meta); padding: 6px 10px; border-radius: var(--radius); white-space: nowrap; opacity: 0; pointer-events: none; transition: opacity 0.12s; z-index: 2; }
.days button:hover::after, .days button:focus-visible::after { opacity: 1; visibility: visible; }
.days li:nth-child(-n+12) button::after { left: 0; translate: 0 0; }
.days li:nth-last-child(-n+12) button::after { left: auto; right: 0; translate: 0 0; }
@media (max-width: 599px) { .days { grid-template-columns: repeat(30, 1fr); gap: 3px; } .days li:nth-child(-n+60) { display: none; } .days li:nth-child(n+61):nth-child(-n+70) button::after { left: 0; translate: 0 0; } }
.daydetail { margin-top: var(--s-4); display: grid; grid-template-columns: auto 1fr auto; gap: var(--s-1) var(--s-3); align-items: center; padding: var(--s-3) var(--s-4); border-radius: var(--radius-l); background: var(--wash-down); }
.daydetail .k { font: var(--t-label); }
.daydetail p { font: var(--t-secondary); grid-column: 2 / -1; }
.two { display: grid; gap: var(--s-5); grid-template-columns: repeat(auto-fit, minmax(min(100%, 30rem), 1fr)); margin-top: var(--s-5); align-items: start; }
.heat { display: grid; grid-template-columns: 2.6rem repeat(24, 1fr); gap: 2px; font: var(--t-tiny); color: var(--ink-muted); }
.heat span.c { aspect-ratio: 1 / 1; border-radius: 2px; min-height: 8px; }
.heat .t0 { background: var(--accent-wash); } .heat .t1 { background: color-mix(in srgb, var(--st-up) 55%, var(--accent-wash)); }
.heat .t2 { background: var(--st-up); } .heat .t3 { background: var(--st-degraded); }
.heat .t4 { background: var(--st-down); position: relative; }
.heat .t4::after { content: ''; position: absolute; inset: 28%; background: var(--surface); clip-path: polygon(50% 0, 100% 50%, 50% 100%, 0 50%); }
.heat .lbl { align-self: center; }
.heat .hr { text-align: center; }
.heatkey { display: flex; flex-wrap: wrap; gap: var(--s-2) var(--s-4); margin-top: var(--s-3); font: var(--t-secondary); color: var(--ink-muted); }
.heatkey span { display: inline-flex; align-items: center; gap: 6px; }
.heatkey i { width: 14px; height: 14px; border-radius: 2px; display: inline-block; position: relative; }
.vlist { list-style: none; margin: 0; padding: 0; }
.vlist li { display: grid; grid-template-columns: var(--glyph) 1fr auto; gap: var(--s-3); align-items: center; padding: var(--s-3) 0; border-bottom: 1px solid var(--line); }
.vlist li:last-child { border-bottom: 0; }
.vlist .n { font: var(--t-label); }
.vlist .n small { display: block; font: var(--t-secondary); color: var(--ink-muted); }
.vlist .r { font: var(--t-secondary); color: var(--ink-muted); text-align: right; display: inline-flex; align-items: center; gap: var(--s-2); white-space: nowrap; }
.lat { display: inline-block; width: 64px; height: 6px; border-radius: 999px; background: var(--ground-raised); overflow: hidden; }
.lat i { display: block; height: 100%; background: var(--accent-soft); border-radius: inherit; }
.vpwrap { display: grid; grid-template-columns: auto 1fr; gap: var(--s-4); align-items: center; padding-bottom: var(--s-3); border-bottom: 1px solid var(--line); }
@media (max-width: 599px) { .vpwrap { grid-template-columns: 1fr; } .lat { display: none; } }
details.ops { margin-top: var(--s-5); background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); }
details.ops summary { cursor: pointer; padding: var(--s-4) var(--s-5); font: var(--t-label); display: flex; gap: var(--s-2); align-items: center; }
details.ops summary .muted { font: var(--t-secondary); }
details.ops .in { padding: 0 var(--s-5) var(--s-5); display: grid; gap: var(--s-4); }
dl.kv { display: grid; grid-template-columns: max-content 1fr; gap: var(--s-2) var(--s-5); margin: 0; font: var(--t-secondary); }
dl.kv dt { color: var(--ink-muted); }
dl.kv dd { margin: 0; color: var(--ink); }
pre.code { margin: 0; padding: var(--s-4); background: var(--ground-raised); border-radius: var(--radius-l); overflow-x: auto; font: 400 0.8125rem/1.6 var(--font-mono); color: var(--ink); }
pre.code .c { color: var(--ink-muted); } pre.code .k { color: var(--accent); }
.incl { list-style: none; margin: 0; padding: 0; }
.incl li { display: grid; grid-template-columns: var(--glyph) 1fr auto; gap: var(--s-3); padding: var(--s-3) 0; border-bottom: 1px solid var(--line); align-items: baseline; }
.incl li:last-child { border-bottom: 0; }
.incl .t { font: var(--t-label); }
.incl .t small { display: block; font: var(--t-secondary); color: var(--ink-muted); margin-top: 2px; }
.incl .r { font: var(--t-meta); color: var(--ink-muted); white-space: nowrap; }
"""


def chart(seed=3, w=1000, h=200, incident=None):
    rnd = random.Random(seed)
    n = 96
    pad_l, pad_b, top = 0, 0, 8
    ih = h - pad_b - top
    ymax = 300
    p50, p95 = [], []
    v = 64
    for i in range(n):
        v = max(40, min(95, v + rnd.uniform(-6, 6)))
        bump = 0
        if 30 <= i <= 36:
            bump = 25 * math.sin((i - 30) / 6 * math.pi)
        a = v + bump
        b = a + 50 + rnd.uniform(0, 40)
        if incident and incident[0] <= i <= incident[1]:
            a, b = 260, 290
        p50.append(a); p95.append(b)
    X = lambda i: pad_l + i * (w - pad_l) / (n - 1)
    Y = lambda val: top + ih - min(val, ymax) / ymax * ih
    l50 = 'M' + ' '.join(f'{X(i):.1f} {Y(p):.1f}' for i, p in enumerate(p50))
    l95 = 'M' + ' '.join(f'{X(i):.1f} {Y(p):.1f}' for i, p in enumerate(p95))
    band = l95 + ' L' + ' '.join(f'{X(i):.1f} {Y(p):.1f}' for i, p in reversed(list(enumerate(p50)))) + 'Z'
    grid = ''.join(f'<line x1="{pad_l}" x2="{w}" y1="{Y(t):.1f}" y2="{Y(t):.1f}"/>' for t in (0, 100, 200, 300))
    yl = ''.join(f'<text class="ax" x="{pad_l - 8}" y="{Y(t) + 4:.1f}" text-anchor="end">{t}</text>' for t in (0, 100, 200, 300))
    hours = ['15:00', '18:00', '21:00', '00:00', '03:00', '06:00', '09:00', '12:00', 'now']
    xl = ''.join(f'<text class="ax" x="{X(i * 12) if i < 8 else w:.1f}" y="{h - 4}" text-anchor="{"end" if i == 8 else "middle"}">{t}</text>' for i, t in enumerate(hours))
    thr = Y(250)
    ylab = ''.join(f'<span style="top:{Y(t) / h * 100:.2f}%">{t}</span>' for t in (0, 100, 200, 300))
    xlab = ''.join(f'<span>{t}</span>' for t in hours)
    return (f'<div class="chartwrap"><div class="ylab" aria-hidden="true">{ylab}</div>'
            f'<svg class="chart" viewBox="0 0 {w} {h}" preserveAspectRatio="none" role="img" '
            f'aria-label="Response time over the last 24 hours: typically 64 ms, slowest 1 in 20 under 140 ms, never slow.">'
            f'<g class="grid">{grid}</g><path class="band" d="{band}"/><path class="p95" d="{l95}"/><path class="p50" d="{l50}"/>'
            f'<line class="thr" x1="{pad_l}" x2="{w}" y1="{thr:.1f}" y2="{thr:.1f}"/></svg>'
            f'<span class="thr-l" style="top:{thr / h * 100 - 9:.2f}%">slow above 250 ms</span>'
            f'<div class="xlab" aria-hidden="true">{xlab}</div><span class="unit">ms</span></div>')


def days_buttons():
    import datetime
    start = datetime.date(2026, 7, 6)
    out = []
    for i in range(90):
        dte = start + datetime.timedelta(days=i)
        lab = dte.strftime('%-d %b')
        cls, tip = '', f'{lab}: up all day'
        if i == 74:
            cls, tip = 'x', f'{lab}: down 41 min'
        elif i in (27,):
            cls, tip = 'd', f'{lab}: slow for 12 min'
        elif i == 50:
            cls, tip = 'm', f'{lab}: maintenance 15 min'
        pressed = 'true' if i == 74 else 'false'
        out.append(f'<li><button type="button" class="{cls}" data-tip="{tip}" aria-label="{tip}" aria-pressed="{pressed}"></button></li>')
    return '<ol class="days" aria-label="Last 90 days, one button per day">' + ''.join(out) + '</ol>'


def heat():
    rnd = random.Random(11)
    rows = []
    rows.append('<span></span>' + ''.join(f'<span class="hr">{h if h % 6 == 0 else ""}</span>' for h in range(24)))
    for d, name in enumerate(['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']):
        cells = []
        for hr in range(24):
            t = rnd.choices([0, 1, 2], [5, 4, 1])[0]
            if 9 <= hr <= 11 and d in (1, 2, 3, 4, 5):
                t = max(t, 2)
            if d == 4 and hr == 19:
                t = 4
            if d == 1 and hr == 10:
                t = 3
            cells.append(f'<span class="c t{t}"></span>')
        rows.append(f'<span class="lbl">{name}</span>' + ''.join(cells))
    return ('<div class="heat" role="img" aria-label="Response time by hour over the last 7 days. Busier mornings on weekdays; Monday 10:00 was slow; Thursday 19:00 failed.">'
            + ''.join(rows) + '</div>')


def page_monitor():
    body = f"""{header(current='History')}
<main id="main" class="page">
<nav class="crumbs" aria-label="Breadcrumb"><a href="status-ok.html">Status</a><span class="sep" aria-hidden="true">/</span><a href="status-ok.html#g-Peli">Pelican</a><span class="sep" aria-hidden="true">/</span><span aria-current="page">Public API</span></nav>
<div class="title"><h1>Public API</h1><span class="state-big">{state('up', 'Up')}<span class="muted" style="font:var(--t-secondary)">for 15 days</span></span></div>
<p class="target"><code>api.pelican.app/health</code> · checked every 30 s from 3 places</p>

<div class="tiles">
  <div class="tile"><span class="k">Up over 90 days</span><span class="v">99.97 %</span><span class="n">One outage, 41 min, on 18 Sep</span></div>
  <div class="tile"><span class="k">Response time, 24 h</span><span class="v">64 ms<small>typical</small></span><span class="n">Slowest 1 in 20: 140 ms</span></div>
  <div class="tile"><span class="k">Allowed downtime left this month</span><span class="v">31 min<small>of 43</small></span><div class="meter" role="meter" aria-valuemin="0" aria-valuemax="43" aria-valuenow="31" aria-label="31 of 43 minutes left"><span style="width:72%"></span></div><span class="n">Target 99.9 % over 30 days</span></div>
  <div class="tile"><span class="k">Certificate</span><span class="v">41 days<small>left</small></span><span class="n">Renews by itself (Let's Encrypt)</span></div>
</div>

<section class="panel" style="margin-top:var(--s-5)" aria-labelledby="rt">
  <div class="panel-head"><h2 id="rt">Response time</h2><nav class="tabs" aria-label="Period"><a href="#" aria-current="true">24 h</a><a href="#">7 days</a><a href="#">30 days</a></nav></div>
  {chart()}
  <div class="key"><span><i></i>Typical (median)</span><span><i class="d"></i>Slowest 1 in 20 (p95)</span><span><i class="t"></i>Counted as slow</span></div>
</section>

<section class="panel" style="margin-top:var(--s-5)" aria-labelledby="ninety">
  <div class="panel-head"><h2 id="ninety">Last 90 days</h2><span class="muted" style="font:var(--t-secondary);margin-left:auto">Tap or tab to a day</span></div>
  <div style="padding-top:36px">{days_buttons()}</div>
  <div class="bar-legend" style="padding-inline:0" aria-hidden="true"><span class="d90">6 Jul</span><span class="d30">3 Sep</span><span>Today</span></div>
  <div class="daydetail" aria-live="polite">{glyph('down')}<span class="k">Thursday 18 September</span><a href="incident.html">Read the post-mortem</a><p>Down for 41 minutes, 19:12 to 19:53 UTC. Cause: the database ran out of connections.</p></div>
</section>

<div class="two">
<section class="panel" aria-labelledby="hm">
  <div class="panel-head"><h2 id="hm">Hour by hour, 7 days</h2></div>
  {heat()}
  <div class="heatkey"><span><i style="background:var(--accent-wash)"></i>Fast</span><span><i style="background:color-mix(in srgb, var(--st-up) 55%, var(--accent-wash))"></i>Usual</span><span><i style="background:var(--st-up)"></i>Busy</span><span><i style="background:var(--st-degraded)"></i>Slow</span><span><i class="t4" style="background:var(--st-down)"></i>{glyph('down')} Failed</span></div>
</section>
<section class="panel" aria-labelledby="vp">
  <div class="panel-head"><h2 id="vp">Checked from 3 places</h2><span class="muted" style="font:var(--t-secondary);margin-left:auto">{state('up', 'Up from 3 of 3')}</span></div>
  <div class="vpwrap">{owls(['up', 'up', 'up'], names=['Paris', 'Frankfurt', 'Montréal'], label='Three owls asleep: Paris, Frankfurt and Montréal all see Public API up', width=200)}
  <p class="muted" style="font:var(--t-secondary)">Each place checks on its own. An outage is called only when two of them agree, so a hiccup near one place never pages anyone.</p></div>
  <ul class="vlist">
    <li>{glyph('up')}<span class="n">Paris<small>this Hora · checked 12 s ago</small></span><span class="r"><span class="word up">Up</span><span class="lat"><i style="width:43%"></i></span>61 ms</span></li>
    <li>{glyph('up')}<span class="n">Frankfurt<small>peer · seen 8 s ago</small></span><span class="r"><span class="word up">Up</span><span class="lat"><i style="width:41%"></i></span>58 ms</span></li>
    <li>{glyph('up')}<span class="n">Montréal<small>peer · seen 21 s ago</small></span><span class="r"><span class="word up">Up</span><span class="lat"><i style="width:100%"></i></span>142 ms</span></li>
  </ul>
  <p class="muted" style="font:var(--t-secondary);margin-top:var(--s-2)">Typical answer time over 24 h, from each place. Montréal is further away; that is distance, not trouble.</p>
<div class="panel-head" style="margin:var(--s-5) 0 var(--s-2)"><h2>Incidents</h2><a href="incident.html" style="margin-left:auto;font:var(--t-secondary)">Timeline</a></div>
  <ul class="incl">
    <li>{glyph('down')}<span class="t"><a href="incident.html">Down for 41 min</a><small>Database out of connections</small></span><span class="r">18 Sep</span></li>
    <li>{glyph('degraded')}<span class="t">Slow for 12 min<small>Up to 1.9 s during a deploy</small></span><span class="r">2 Aug</span></li>
  </ul>
</section>
</div>

<details class="ops"><summary>{icon('chev')}For operators <span class="muted">probe, percentiles, SLO burn</span></summary>
<div class="in">
<dl class="kv">
  <dt>Probe</dt><dd>HTTP GET, expects 200, timeout 10 s, IPv4 and IPv6</dd>
  <dt>p50 / p95 / p99, 24 h</dt><dd>64 / 140 / 212 ms</dd>
  <dt>Latency SLO</dt><dd>p95 under 250 ms: {state('up', 'met')}</dd>
  <dt>Burn rate, 1 h / 6 h</dt><dd>0.0× / 0.2×</dd>
  <dt>Depends on</dt><dd>Database, File storage</dd>
  <dt>Confirm with peers</dt><dd>on · last asked 18 Sep 19:12, verdict "confirmed down from 3/3 vantage points"</dd>
</dl>
<pre class="code"><span class="k">[[monitors]]</span>
id = "api"
name = "Public API"
target = "https://api.pelican.app/health"
interval_secs = 30
group = "pelican"
depends_on = ["db", "storage"]
slo_uptime = 99.9            <span class="c"># 43 min a month</span></pre>
</div></details>
</main>
{footer()}"""
    return shell('Public API · Pelican status', body, 'Public API: uptime, response time and incidents.', PAGE_CSS)


# ---------------------------------------------------------------------------
INC_CSS = r"""
.pm-head { display: grid; grid-template-columns: auto 1fr; gap: var(--s-4) var(--s-5); align-items: center; margin-top: var(--s-5); }
.pm-head .eyebrow { display: flex; gap: var(--s-2); align-items: center; }
.pm-head h1 { font: var(--t-hero); font-size: clamp(1.75rem, 1.35rem + 1.6vw, 2.4rem); text-wrap: balance; }
.pm-head .sub { color: var(--ink-muted); margin-top: var(--s-2); }
.pm-actions { display: flex; flex-wrap: wrap; gap: var(--s-2); margin-top: var(--s-4); }
.pm-grid { display: grid; gap: var(--s-5); grid-template-columns: minmax(0, 1fr); margin-top: var(--s-6); }
@media (min-width: 1100px) { .pm-grid { grid-template-columns: minmax(0, 1fr) 22rem; } }
.facts { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); padding: var(--s-4) var(--s-5); align-self: start; }
.facts dl { display: grid; grid-template-columns: 1fr; gap: 0; margin: 0; }
.facts dt { font: var(--t-secondary); color: var(--ink-muted); padding-top: var(--s-3); }
.facts dd { margin: 0; font: var(--t-label); padding-bottom: var(--s-3); border-bottom: 1px solid var(--line); }
.facts dd:last-child { border-bottom: 0; }
.promise { display: grid; grid-template-columns: auto 1fr; gap: var(--s-3); align-items: center; background: var(--accent-wash); border-radius: var(--radius-l); padding: var(--s-4); margin-top: var(--s-4); }
.promise b { font: var(--t-label); display: block; }
.promise p { font: var(--t-secondary); }
article h2 { font: var(--t-group); font-size: 1.375rem; margin: var(--s-6) 0 var(--s-3); }
article h2:first-child { margin-top: 0; }
article p { max-width: var(--measure); margin-bottom: var(--s-3); }
.tl { list-style: none; margin: 0; padding: 0; position: relative; }
.tl::before { content: ''; position: absolute; left: 7px; top: 8px; bottom: 8px; width: 2px; background: var(--line); }
.tl li { display: grid; grid-template-columns: var(--glyph) 5.5rem 1fr; gap: var(--s-3); padding: var(--s-2) 0; position: relative; align-items: baseline; }
.tl li > .g { background: var(--ground); border-radius: 50%; box-shadow: 0 0 0 3px var(--ground); position: relative; top: 2px; }
.tl time { font: var(--t-meta); color: var(--ink-muted); font-family: var(--font-mono); font-size: 0.8125rem; }
.tl .e b { font: var(--t-label); display: block; }
.tl .e span { font: var(--t-secondary); color: var(--ink-muted); }
.tl .e .who { display: inline-flex; align-items: center; gap: 6px; }
.tl .note { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-l); padding: var(--s-3) var(--s-4); margin-top: var(--s-1); color: var(--ink); font: var(--t-secondary); }
@media (max-width: 599px) { .tl li { grid-template-columns: var(--glyph) 1fr; } .tl time { grid-column: 2; } .tl .e { grid-column: 2; } }
.snap { font: 400 0.8125rem/1.6 var(--font-mono); background: var(--ground-raised); border-radius: var(--radius-l); padding: var(--s-4); overflow-x: auto; margin: 0; }
.snap .h { color: var(--st-down-text); }
.chips-in { display: flex; flex-wrap: wrap; gap: var(--s-2); }
"""


def page_incident():
    body = f"""{header(current='History')}
<main id="main" class="page">
<nav class="crumbs" aria-label="Breadcrumb"><a href="status-ok.html">Status</a><span class="sep" aria-hidden="true">/</span><a href="monitor.html">Public API</a><span class="sep" aria-hidden="true">/</span><span aria-current="page">Incident #42</span></nav>
<header class="pm-head">
  {owl('up', 80, label='')}
  <div>
    <p class="eyebrow"><span>Post-mortem #42</span>·<span class="word up" style="text-transform:none;letter-spacing:0">{glyph('up')} Resolved</span></p>
    <h1>Public API was down for 41 minutes</h1>
    <p class="sub">Thursday 18 September 2026, 19:12 to 19:53 UTC. Web app and Mobile sync were down with it.</p>
    <div class="pm-actions"><button class="btn" type="button">{icon('copy')}Copy as Markdown</button><a class="btn quiet" href="#">Download .md</a><a class="btn quiet" href="monitor.html">Public API's history</a></div>
  </div>
</header>

<div class="pm-grid">
<article>
  <h2>What happened</h2>
  <p>At 19:12 the API stopped answering: every request waited for a database connection that never came. A nightly export had opened connections without closing them, and the pool (100 connections) filled up.</p>
  <p>We restarted the export worker at 19:49; the API answered again at 19:53. No data was lost. Requests made during the outage failed with an error and can be retried.</p>
  <h2>What we changed</h2>
  <p>The export now uses its own pool of 10 connections, and Hora watches the pool: it says "slow" at 80 %, long before it is full.</p>

  <h2>Timeline</h2>
  <ol class="tl">
    <li>{glyph('degraded')}<time>19:11:40</time><span class="e"><b>First failed check, retried</b><span>Paris: timeout after 10 s. A single failure is retried before anything is recorded.</span></span></li>
    <li>{glyph('down')}<time>19:12:10</time><span class="e"><b>Down, confirmed from 3 of 3 places</b><span>3 failures in a row in Paris. Paris asks Frankfurt and Montréal to check: both see it down too.</span><span class="vantages" style="margin-top:6px"><span class="vchip down">{glyph('down')}Paris <small>timeout</small></span><span class="vchip down">{glyph('down')}Frankfurt <small>503</small></span><span class="vchip down">{glyph('down')}Montréal <small>503</small></span></span></span></li>
    <li>{glyph('info')}<time>19:12:11</time><span class="e"><b>One alert sent</b><span>To #ops on Slack: "Database is the cause; Public API, Web app and Mobile sync affected". Not three alerts.</span></span></li>
    <li>{glyph('info')}<time>19:20</time><span class="e"><b class="who">Note by Léonard</b><div class="note">Pool at 100/100. All held by export-worker. Restarting it.</div></span></li>
    <li>{glyph('info')}<time>19:49</time><span class="e"><b>Change: export-worker restarted</b><span>From the deploy hook, logged as an event.</span></span></li>
    <li>{glyph('up')}<time>19:53:02</time><span class="e"><b>Up again</b><span>Recovery confirmed from 3 places. One "resolved" message sent.</span></span></li>
  </ol>

  <h2>What the API answered</h2>
  <pre class="snap" aria-label="Response captured at 19:12:10"><span class="h">HTTP/1.1 503 Service Unavailable</span>
content-type: application/json
retry-after: 30

{{"error":"database unavailable","detail":"pool timed out after 10s"}}</pre>
  <p class="muted" style="margin-top:var(--s-2);font:var(--t-secondary)">Captured by Hora at the first confirmed failure, from Paris.</p>
</article>

<aside>
  <div class="facts">
    <dl>
      <dt>Started</dt><dd>18 Sep, 19:12 UTC</dd>
      <dt>Recovered</dt><dd>18 Sep, 19:53 UTC</dd>
      <dt>Duration</dt><dd>41 minutes</dd>
      <dt>Cause</dt><dd>Database: no free connection</dd>
      <dt>Also affected</dt><dd><span class="chips-in"><a class="chip" href="#">Web app</a><a class="chip" href="#">Mobile sync</a></span></dd>
      <dt>Seen down from</dt><dd>Paris, Frankfurt and Montréal (3 of 3): a real outage, not a network problem</dd>
      <dt>Allowed downtime used</dt><dd>41 of 43 min (September)</dd>
    </dl>
  </div>
  <div class="promise" style="grid-template-columns:1fr">{owls(['down', 'down', 'down'], names=['Paris', 'Frankfurt', 'Montréal'], label='Three owls awake: all three places saw the outage', width=180)}<div><b>3 places agreed, 1 alert sent</b><p>All three watchers saw it down, so Hora paged. It grouped the three services under their cause and stayed quiet through the first retry.</p></div></div>
</aside>
</div>
</main>
{footer()}"""
    return shell('Post-mortem #42: Public API · Pelican status', body, 'Public API was down for 41 minutes on 18 September 2026.', PAGE_CSS + INC_CSS)


# ---------------------------------------------------------------------------
REPORT_CSS = r"""
body { background: var(--ground-raised); }
.toolbar { display: flex; flex-wrap: wrap; gap: var(--s-2); align-items: center; justify-content: space-between; max-width: 210mm; margin: var(--s-5) auto var(--s-3); padding: 0 var(--page-margin); }
.toolbar .nav { display: flex; gap: var(--s-2); align-items: center; font: var(--t-label); }
.sheet { background: var(--surface); max-width: 210mm; margin: 0 auto var(--s-7); padding: clamp(20px, 5vw, 18mm) clamp(16px, 5vw, 18mm); border-radius: var(--radius-l); box-shadow: var(--shadow); }
.rh { display: flex; align-items: center; gap: var(--s-3); padding-bottom: var(--s-4); border-bottom: 1px solid var(--line); }
.rh .op { font: 500 1.05rem/1.2 var(--font-display); }
.rh .who { margin-left: auto; display: flex; align-items: center; gap: 8px; color: var(--ink-muted); font: var(--t-meta); }
.rh .who .b { font: 400 1rem/1 var(--font-brand); color: var(--ink); transform: translateY(0.14em); }
.rt { margin-top: var(--s-6); }
.rt h1 { font: 500 2.25rem/1.15 var(--font-display); }
.rt p { color: var(--ink-muted); margin-top: var(--s-1); }
.kpis { display: grid; grid-template-columns: repeat(auto-fit, minmax(7.25rem, 1fr)); gap: var(--s-3); margin-top: var(--s-5); }
.kpi { border: 1px solid var(--line); border-radius: var(--radius-l); padding: var(--s-3) var(--s-4); }
.kpi .k { font: var(--t-secondary); color: var(--ink-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.kpi .v { white-space: nowrap; font: 500 1.6rem/1.25 var(--font-display); font-variant-numeric: lining-nums tabular-nums; }
.kpi .n { font: var(--t-secondary); }
.kpi.lead { background: var(--accent-wash); border-color: transparent; }
.rg { margin-top: var(--s-6); break-inside: avoid; }
.rg-h { display: flex; align-items: baseline; gap: var(--s-3); margin-bottom: var(--s-2); }
.rg-h h2 { font: var(--t-group); }
.rg-h span { margin-left: auto; font: var(--t-secondary); color: var(--ink-muted); }
.tw { overflow-x: auto; }
table { width: 100%; border-collapse: collapse; font: var(--t-secondary); min-width: 34rem; }
th { font: var(--t-meta); color: var(--ink-muted); text-align: right; padding: var(--s-2) var(--s-2); border-bottom: 1px solid var(--ink-muted); white-space: nowrap; }
th:first-child, td:first-child { text-align: left; padding-left: 0; }
td { padding: var(--s-2) var(--s-2); border-bottom: 1px solid var(--line); text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
td:first-child { font: var(--t-label); }
td .word { font: var(--t-secondary); }
td .g { width: 12px; height: 12px; margin-right: 4px; }
td.strip { width: 6.5rem; }
td.strip .bar { height: 16px; pointer-events: none; }
td.strip .bar svg { width: 100%; margin: 0; -webkit-mask-image: repeating-linear-gradient(90deg, #000 0 calc(100% / 30 - 1px), transparent calc(100% / 30 - 1px) calc(100% / 30)); mask-image: repeating-linear-gradient(90deg, #000 0 calc(100% / 30 - 1px), transparent calc(100% / 30 - 1px) calc(100% / 30)); }
tr.missed td { background: var(--wash-down); }
.method { margin-top: var(--s-6); padding-top: var(--s-4); border-top: 1px solid var(--line); font: var(--t-secondary); color: var(--ink-muted); display: grid; gap: var(--s-2); }
@page { size: A4; margin: 14mm; }
@media print {
  :root { color-scheme: light !important; }
  body { background: #fff; }
  .toolbar, .skip, .foot { display: none; }
  .sheet { box-shadow: none; padding: 0; max-width: none; border-radius: 0; background: #fff; }
  .kpi.lead { background: var(--raw-petrol-wash); -webkit-print-color-adjust: exact; print-color-adjust: exact; }
  tr.missed td, .bar svg rect { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
}
"""


def strip30(d):
    runs, i = [], 0
    out = []
    while i < len(d):
        j = i
        while j < len(d) and d[j] == d[i]:
            j += 1
        y, h = {'u': (0, 12), 'd': (4, 8), 'x': (0, 16), 'm': (6, 6), 'n': (10, 2)}[d[i]]
        out.append(f'<rect class="{d[i]}" x="{i}" y="{y}" width="{j - i}" height="{h}"/>')
        i = j
    return f'<span class="bar"><svg viewBox="0 0 30 16" preserveAspectRatio="none" aria-hidden="true">{"".join(out)}</svg></span>'


def rrow(name, up, target, met, inc, down, mttr, left, d):
    st = 'up' if met else 'down'
    word = 'Met' if met else 'Missed'
    cls = '' if met else ' class="missed"'
    return (f'<tr{cls}><td>{name}</td><td>{up}</td><td>{target}</td><td>{glyph(st)}<span class="word {st}">{word}</span></td>'
            f'<td>{inc}</td><td>{down}</td><td>{left}</td><td class="strip">{strip30(d)}</td></tr>')


def page_report():
    U = ['u'] * 30
    def dd(**k):
        d = ['u'] * 30
        for key, cls in (('x', 'x'), ('d', 'd'), ('m', 'm')):
            for i in k.get(key, []):
                d[i] = cls
        return d
    thead = '<thead><tr><th scope="col">Service</th><th scope="col">Up</th><th scope="col">Target</th><th scope="col">Target met</th><th scope="col">Incidents</th><th scope="col">Downtime</th><th scope="col">Allowed left</th><th scope="col">30 days</th></tr></thead>'
    g1 = ''.join([
        rrow('Website', '100 %', '99.9 %', True, '0', '0 min', 'n/a', '43 min', U),
        rrow('Web app', '99.91 %', '99.9 %', True, '1', '41 min', '41 min', '2 min', dd(x=[17])),
        rrow('Public API', '99.89 %', '99.95 %', False, '1', '41 min', '41 min', '0 min', dd(x=[17])),
        rrow('Mobile sync', '99.91 %', '99.9 %', True, '1', '41 min', '41 min', '2 min', dd(x=[17])),
    ])
    g2 = ''.join([
        rrow('Card payments', '100 %', '99.95 %', True, '0', '0 min', 'n/a', '21 min', U),
        rrow('Bank transfers (SEPA)', '100 %', '99.9 %', True, '0', '0 min', 'n/a', '43 min', U),
        rrow('Invoice emails', '100 %', '99.5 %', True, '1', '0 min', '58 min', '3 h 36 min', dd(d=[11])),
        rrow('Webhooks', '99.99 %', '99.9 %', True, '0', '4 min', 'n/a', '39 min', dd(d=[22])),
    ])
    body = f"""<div class="toolbar"><nav class="nav" aria-label="Months"><a class="btn quiet" href="#">August</a><span>September 2026</span><a class="btn quiet" href="#">October</a></nav>
<span style="display:flex;gap:8px"><button class="btn" type="button" onclick="print()">{icon('print')}Print or save as PDF</button>{theme_button()}</span></div>
<main id="main" class="sheet">
<header class="rh"><span class="op-logo" style="width:28px;height:28px;border-radius:7px;display:grid;place-items:center;background:var(--ink);color:var(--ground);font:600 .9rem/1 var(--font-display)" aria-hidden="true">P</span><span class="op">Pelican</span>
<span class="who"><img src="assets/mark.svg" alt="" width="20" height="20" style="border-radius:5px">Report by <span class="b">Hora</span></span></header>
<div class="rt"><p class="eyebrow">Service report</p><h1>September 2026</h1><p>1 to 30 September 2026, times in UTC. 14 services, 8 shown with a target.</p></div>
<div class="kpis">
  <div class="kpi lead"><div class="k">Up overall</div><div class="v">99.97 %</div><div class="n">{state('up', 'Target met')}</div></div>
  <div class="kpi"><div class="k">Incidents</div><div class="v">2</div><div class="n muted">1 outage, 1 slow period</div></div>
  <div class="kpi"><div class="k">Downtime</div><div class="v">41 min</div><div class="n muted">all on 18 Sep</div></div>
  <div class="kpi"><div class="k">To recover</div><div class="v">41 min</div><div class="n muted">median, outages only</div></div>
  <div class="kpi"><div class="k">Targets met</div><div class="v">7 of 8</div><div class="n">{state('down', 'Public API missed')}</div></div>
</div>
<section class="rg"><div class="rg-h"><h2>Pelican</h2><span>Group: 99.93 % up</span></div><div class="tw"><table>{thead}<tbody>{g1}</tbody></table></div></section>
<section class="rg"><div class="rg-h"><h2>Payments and email</h2><span>Group: 100 % up</span></div><div class="tw"><table>{thead}<tbody>{g2}</tbody></table></div></section>
<div class="method">
<p><b style="color:var(--ink);font-weight:500">How this is counted.</b> A service is up when it answers as expected; slow answers count as up and are listed as incidents. Planned maintenance is left out. An outage counts only once two places agree.</p>
<p>Generated 1 October 2026, 00:05 UTC, by Hora 0.9 from 1 296 000 checks. <a href="#">JSON</a> · <a href="#">CSV</a></p>
</div>
</main>"""
    return shell('September 2026 · Pelican service report', body, 'Pelican service report for September 2026.', PAGE_CSS + REPORT_CSS)


# ---------------------------------------------------------------------------
EMPTY_CSS = r"""
.screen-label { max-width: 1200px; margin: var(--s-7) auto var(--s-3); padding: 0 var(--page-margin); font: var(--t-meta); text-transform: uppercase; letter-spacing: 0.06em; color: var(--ink-muted); display: flex; gap: var(--s-2); align-items: center; }
.screen-label::after { content: ''; flex: 1; border-top: 1px dashed var(--line); }
.screen { max-width: 1200px; margin: 0 auto; border: 1px solid var(--line); border-radius: var(--radius-xl); overflow: hidden; background: var(--ground); }
@media (min-width: 1240px) { .screen { margin-inline: auto; } .screen-label { padding: 0; } }
@media (max-width: 1239px) { .screen { margin-inline: var(--page-margin); } }
.first { display: grid; justify-items: center; text-align: center; padding: var(--s-7) var(--s-4); gap: var(--s-3); }
.first h1 { font: var(--t-hero); }
.first .lead { color: var(--ink-muted); max-width: 30rem; }
.first pre.code { text-align: left; width: min(100%, 34rem); position: relative; }
.first .actions { display: flex; flex-wrap: wrap; gap: var(--s-2); justify-content: center; }
pre.code { margin: 0; padding: var(--s-4); background: var(--ground-raised); border-radius: var(--radius-l); overflow-x: auto; font: 400 0.8125rem/1.6 var(--font-mono); color: var(--ink); }
pre.code .c { color: var(--ink-muted); } pre.code .k { color: var(--accent); }
.copy { position: absolute; top: var(--s-2); right: var(--s-2); }
.door { display: grid; grid-template-rows: auto 1fr; min-height: 560px; }
.door .side { position: relative; overflow: hidden; display: grid; place-items: center; padding: var(--s-6) var(--s-4) var(--s-5); background: var(--raw-petrol); color: var(--raw-cloud); }
.door .trail { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0.16; color: var(--raw-cloud); }
.door .id { position: relative; display: grid; justify-items: center; gap: var(--s-2); text-align: center; }
.door .id img { border-radius: var(--radius-2xl); box-shadow: 0 0 0 1px color-mix(in srgb, var(--raw-cloud) 35%, transparent); }
.door .name { font: 400 2.6rem/1.3 var(--font-brand); }
.door .promise { opacity: 0.9; max-width: 22rem; text-wrap: balance; }
.door .content { display: grid; align-content: start; justify-items: center; padding: var(--s-6) var(--s-4) var(--s-7); }
.door .content > * { width: min(100%, 26rem); }
.door h1 { font: var(--t-page); margin-bottom: var(--s-2); }
.door .lead { color: var(--ink-muted); margin-bottom: var(--s-5); }
.field { display: grid; gap: 6px; margin-bottom: var(--s-4); }
.field label { font: 600 0.875rem/1.25rem var(--font-text); }
.field input { min-height: var(--control-h); padding: 0 var(--s-3); border-radius: var(--radius); border: 1px solid var(--ink-muted); background: var(--surface); color: var(--ink); font: var(--t-body); font-family: var(--font-mono); font-size: 0.9375rem; }
.field input:focus-visible { outline: var(--focus-ring); outline-offset: 1px; }
.field .err { display: flex; gap: 6px; align-items: center; font: var(--t-secondary); color: var(--st-down-text); }
.field.bad input { border-color: var(--st-down); border-width: 2px; }
.big { width: 100%; min-height: var(--control-h-l); border-radius: var(--radius-xl); font: var(--t-body); font-weight: 500; }
.aside { padding-top: var(--s-4); margin-top: var(--s-5); border-top: 1px solid var(--line); font: var(--t-secondary); color: var(--ink-muted); }
@media (min-width: 840px) { .door { grid-template-rows: none; grid-template-columns: minmax(20rem, 40%) 1fr; } .door .side { padding: var(--s-7); } .door .name { font-size: 3.6rem; } .door .content { align-content: center; padding: var(--s-7); } }
.wait { display: grid; grid-template-columns: auto 1fr; gap: var(--s-4); align-items: center; padding: var(--s-5); }
.wait h2 { font: var(--t-group); font-size: 1.375rem; }
.wait p { color: var(--ink-muted); }
.wait .row { padding-inline: 0; }
.empty-wrap { padding-bottom: var(--s-7); }
"""

TRAIL = '''<svg class="trail" viewBox="0 0 400 400" preserveAspectRatio="xMidYMid slice" aria-hidden="true" focusable="false">
<circle cx="200" cy="200" r="150" fill="none" stroke="currentColor" stroke-width="2" stroke-dasharray="2 10" stroke-linecap="round"/>
<path d="M200 38v18M200 344v18M38 200h18M344 200h18M281 60l-9 15.6M128 324.4l-9 15.6M340 119l-15.6 9M75.6 272l-15.6 9M340 281l-15.6-9M75.6 128 60 119M281 340l-9-15.6M128 75.6 119 60" stroke="currentColor" stroke-width="3" stroke-linecap="round"/></svg>'''


def page_empty():
    body = f"""<header class="top"><div class="page"><a class="sign hora" href="docs-home.html"><img src="assets/mark.svg" alt="" width="32" height="32" style="border-radius:8px"><span class="name">Hora</span></a>{theme_button()}</div></header>
<main id="main" class="empty-wrap">
<p class="screen-label">1 · First run: no monitors yet</p>
<section class="screen" aria-labelledby="e1">
  <div class="first">
    {owl('none', 120)}
    <h1 id="e1">Nothing to watch yet.</h1>
    <p class="lead">Add a monitor to <code>config.toml</code>. Hora reads it again by itself; no restart.</p>
    <pre class="code"><button class="btn copy" type="button">{icon('copy')}Copy</button><span class="k">[[monitors]]</span>
id = "website"
name = "Website"
kind = "http"
target = "https://example.com"</pre>
    <div class="actions"><a class="btn primary" href="docs-home.html">Read the 5-minute guide</a><a class="btn" href="#">Import from Uptime Kuma</a></div>
  </div>
</section>

<p class="screen-label">2 · A private page: the door</p>
<section class="screen" aria-labelledby="e2">
  <div class="door">
    <div class="side">{TRAIL}<div class="id"><img src="assets/mark.svg" alt="" width="88" height="88"><p class="name">Hora</p><p class="promise">Hora keeps the hours, so you can sleep.</p></div></div>
    <div class="content"><div>
      <p class="eyebrow" style="margin-bottom:var(--s-2)">{icon('lock')} Clients ACME · status</p>
      <h1 id="e2">This page is private.</h1>
      <p class="lead">Ask the person who runs it for a link, or paste your viewer token.</p>
      <div class="field bad"><label for="tok">Viewer token</label><input id="tok" value="acme-7f3a…" aria-invalid="true" aria-describedby="tok-err"><span class="err" id="tok-err">{glyph('down')}That token doesn't open this page. Check it has no space at the end.</span></div>
      <button class="btn primary big" type="button">Open the page</button>
      <p class="aside">Running Hora? Viewer tokens live in <code>config.toml</code>, under <code>[server.group_tokens]</code>.</p>
    </div></div>
  </div>
</section>

<p class="screen-label">3 · A monitor just added</p>
<section class="screen" aria-labelledby="e3">
  <div class="wait">
    {owl('none', 72)}
    <div><h2 id="e3">Waiting for the first check.</h2><p>Search was added a moment ago. Its first result arrives within 30 s; history fills in from there.</p></div>
  </div>
  <ul class="list" style="margin:0 var(--s-5) var(--s-5)"><li><div class="row none">{glyph('none')}<span class="name">Search</span><span class="state"><span class="word none">No data yet</span></span>{bar(['n'] * 90, '0 %')}</div></li></ul>
</section>
</main>"""
    return shell('Empty and private states · Hora', body, 'Hora: first run, private page and waiting states.', PAGE_CSS + EMPTY_CSS)


PAGES = {'monitor.html': page_monitor, 'incident.html': page_incident, 'report.html': page_report, 'empty.html': page_empty}
try:
    import docs, mesh, gallery
    PAGES.update(gallery.PAGES)
    PAGES.update(docs.PAGES)
    PAGES.update(mesh.PAGES)
except ImportError as e:
    print('docs missing', e)
