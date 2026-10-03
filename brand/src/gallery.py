"""index.html: the brand, presented; every mockup and asset linked, with the reasons."""
from common import *

G_CSS = r"""
.g-top .page { gap: var(--s-3); }
.g-top .sub { font: var(--t-secondary); color: var(--ink-muted); }
.top nav.toc { display: none; gap: var(--s-1); }
@media (min-width: 1100px) { .top nav.toc { display: flex; } }
.intro { display: grid; gap: var(--s-6); align-items: center; padding-block: var(--s-7) var(--s-6); grid-template-columns: minmax(0, 1fr); }
@media (min-width: 960px) { .intro { grid-template-columns: minmax(0, 1.15fr) minmax(0, 0.85fr); } }
.intro .eyebrow { color: var(--accent); }
.intro h1 { font: 500 clamp(2.4rem, 1.6rem + 3.4vw, 4.4rem)/1.04 var(--font-display); letter-spacing: -0.02em; margin-top: var(--s-3); text-wrap: balance; }
.intro h1 em { font-style: normal; font-family: var(--font-brand); font-weight: 400; color: var(--accent); font-size: 0.92em; }
.intro .lead { font: 400 1.1875rem/1.65 var(--font-text); color: var(--ink-muted); margin-top: var(--s-4); max-width: 38rem; }
.intro .lead b { color: var(--ink); font-weight: 500; }
.demo { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-2xl); padding: var(--s-5); display: grid; gap: var(--s-4); justify-items: center; box-shadow: var(--shadow); }
.demo .stagebox { width: 100%; display: grid; place-items: center; padding: var(--s-5) 0 var(--s-3); border-radius: var(--radius-xl); background: var(--wash-up); transition: background 0.3s; position: relative; overflow: hidden; }
.demo[data-s='degraded'] .stagebox { background: var(--wash-degraded); }
.demo[data-s='down'] .stagebox { background: var(--wash-down); }
.demo[data-s='maint'] .stagebox, .demo[data-s='none'] .stagebox { background: var(--wash-maint); }
.demo .stagebox > .owl { width: min(56%, 200px); height: auto; }
.demo .said { font: 500 1.5rem/1.2 var(--font-display); text-align: center; margin-top: var(--s-3); min-height: 1.8rem; }
.demo .pose { font: var(--t-secondary); color: var(--ink-muted); text-align: center; max-width: 24rem; min-height: 2.5rem; }
.seg5 { display: flex; flex-wrap: wrap; justify-content: center; gap: var(--s-1); padding: 4px; border-radius: var(--radius-pill); background: var(--ground-raised); }
.seg5 button { all: unset; box-sizing: border-box; display: inline-flex; align-items: center; gap: 6px; min-height: var(--control-h-s); padding: 0 var(--s-3); border-radius: var(--radius-pill); font: var(--t-meta); color: var(--ink-muted); cursor: pointer; }
.seg5 button[aria-pressed='true'] { background: var(--surface); color: var(--ink); box-shadow: var(--shadow); }
.seg5 button:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
.seg5 .g { width: 12px; height: 12px; }
.chap { margin-top: var(--s-7); padding-top: var(--s-6); border-top: 1px solid var(--line); }
.chap > header { display: grid; gap: var(--s-2); margin-bottom: var(--s-5); max-width: 46rem; }
.chap > header .n { font: 600 0.875rem/1 var(--font-display); color: var(--accent); letter-spacing: 0.04em; }
.chap > header h2 { font: 500 clamp(1.75rem, 1.4rem + 1.4vw, 2.5rem)/1.12 var(--font-display); letter-spacing: -0.01em; }
.chap > header p { color: var(--ink-muted); font-size: 1.0625rem; line-height: 1.6; }
.cols { display: grid; gap: var(--s-4); grid-template-columns: repeat(auto-fit, minmax(min(100%, 17rem), 1fr)); }
.box { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); padding: var(--s-5); }
.box h3 { font: var(--t-group); margin-bottom: var(--s-2); }
.box p, .box li { color: var(--ink-muted); font: var(--t-secondary); font-size: 0.9375rem; line-height: 1.55; }
.box ul { margin: 0; padding-left: 1.1em; display: grid; gap: 6px; }
.box b { color: var(--ink); font-weight: 500; }
.quote { font: 400 clamp(1.25rem, 1.1rem + 0.7vw, 1.6rem)/1.45 var(--font-display); max-width: 44rem; border-left: 3px solid var(--accent-soft); padding-left: var(--s-5); margin: var(--s-5) 0; }
.quote small { display: block; font: var(--t-secondary); color: var(--ink-muted); margin-top: var(--s-2); }
.states { display: grid; gap: var(--s-3); grid-template-columns: repeat(auto-fill, minmax(min(100%, 13rem), 1fr)); }
.st { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); padding: var(--s-4); display: grid; gap: var(--s-2); justify-items: start; }
.st .ow { width: 100%; display: grid; place-items: center; border-radius: var(--radius-l); padding: var(--s-3); }
.st .ow .owl { width: 96px; height: 96px; }
.st h3 { display: flex; gap: 6px; align-items: center; font: var(--t-label); font-size: 1rem; }
.st p { font: var(--t-secondary); color: var(--ink-muted); }
.st code { font-size: 0.75rem; color: var(--ink-muted); }
.rules { display: grid; gap: var(--s-3); grid-template-columns: repeat(auto-fit, minmax(min(100%, 15rem), 1fr)); margin-top: var(--s-4); }
.rule { padding: var(--s-4); border-radius: var(--radius-l); background: var(--accent-wash); }
.rule b { display: block; font: var(--t-label); margin-bottom: 4px; }
.rule p { font: var(--t-secondary); color: var(--ink); }
.marks { display: grid; gap: var(--s-4); grid-template-columns: repeat(auto-fit, minmax(min(100%, 19rem), 1fr)); }
.markbox { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); overflow: hidden; display: grid; }
.markbox .art { min-height: 200px; display: flex; gap: var(--s-4); align-items: end; justify-content: center; padding: var(--s-5); flex-wrap: wrap; }
.markbox .art.petrol { background: var(--raw-petrol); }
.markbox .art.ground { background: var(--ground); }
.markbox .art.check { background-color: var(--ground-raised); background-image: linear-gradient(45deg, var(--line) 25%, transparent 25%, transparent 75%, var(--line) 75%), linear-gradient(45deg, var(--line) 25%, transparent 25%, transparent 75%, var(--line) 75%); background-size: 16px 16px; background-position: 0 0, 8px 8px; }
.markbox .cap { padding: var(--s-3) var(--s-4); border-top: 1px solid var(--line); font: var(--t-secondary); color: var(--ink-muted); display: flex; justify-content: space-between; gap: var(--s-3); flex-wrap: wrap; }
.markbox .cap b { color: var(--ink); font-weight: 500; }
.markbox .sz { display: grid; gap: 6px; justify-items: center; font: var(--t-tiny); color: var(--ink-muted); }
.markbox .art.petrol .sz { color: var(--raw-cloud); }
.swatches { display: grid; gap: var(--s-3); grid-template-columns: repeat(auto-fill, minmax(min(100%, 11.5rem), 1fr)); }
.sw { border-radius: var(--radius-xl); overflow: hidden; border: 1px solid var(--line); background: var(--surface); }
.sw .chipc { height: 76px; display: grid; grid-template-columns: 1fr 1fr; }
.sw .chipc span { display: grid; place-items: end start; padding: 6px 8px; font: var(--t-tiny); }
.sw .meta { padding: var(--s-3); display: grid; gap: 2px; font: var(--t-secondary); }
.sw .meta b { font: var(--t-label); }
.sw .meta code { font-size: 0.75rem; color: var(--ink-muted); }
.ctable { width: 100%; border-collapse: collapse; font: var(--t-secondary); min-width: 44rem; }
.ctable th, .ctable td { padding: var(--s-2) var(--s-3); border-bottom: 1px solid var(--line); text-align: left; white-space: nowrap; }
.ctable thead th { font: var(--t-meta); color: var(--ink-muted); }
.ctable td.r { text-align: right; font-variant-numeric: tabular-nums; }
.ctable td .dot { display: inline-block; width: 12px; height: 12px; border-radius: 3px; vertical-align: -2px; margin-right: 6px; border: 1px solid var(--line); }
.tw { overflow-x: auto; background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); padding: var(--s-2) var(--s-4); }
.ss { width: 100%; border-collapse: collapse; font: var(--t-secondary); min-width: 40rem; }
.ss th, .ss td { padding: var(--s-3); border-bottom: 1px solid var(--line); text-align: left; vertical-align: middle; }
.ss thead th { font: var(--t-meta); color: var(--ink-muted); }
.ss td svg.cell { width: 10px; height: 28px; overflow: visible; }
.ss .owl { width: 40px; height: 40px; }
.type { display: grid; gap: var(--s-4); grid-template-columns: repeat(auto-fit, minmax(min(100%, 20rem), 1fr)); }
.spec { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); padding: var(--s-5); display: grid; gap: var(--s-2); }
.spec .big { line-height: 1.1; color: var(--ink); }
.spec .who { font: var(--t-meta); color: var(--ink-muted); text-transform: uppercase; letter-spacing: 0.06em; }
.spec p { font: var(--t-secondary); color: var(--ink-muted); }
.scale-row { display: flex; flex-wrap: wrap; gap: var(--s-4); align-items: end; margin-top: var(--s-4); }
.scale-row div { display: grid; gap: 6px; justify-items: center; font: var(--t-tiny); color: var(--ink-muted); }
.scale-row i { display: block; background: var(--accent-soft); }
.voice { display: grid; gap: var(--s-3); }
.vrow { display: grid; grid-template-columns: minmax(0, 1fr); gap: var(--s-2); background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius-xl); padding: var(--s-4) var(--s-5); }
@media (min-width: 840px) { .vrow { grid-template-columns: 10rem 1fr 1fr 1fr; align-items: baseline; gap: var(--s-4); } }
.vrow .k { font: var(--t-meta); color: var(--ink-muted); text-transform: uppercase; letter-spacing: 0.06em; }
.vrow .no { color: var(--ink-muted); text-decoration: line-through; text-decoration-color: var(--st-down); }
.vrow .yes { font: 500 1.0625rem/1.4 var(--font-display); }
.vrow .fr { font: 500 1.0625rem/1.4 var(--font-display); color: var(--ink-muted); }
.taglines { display: grid; gap: var(--s-3); grid-template-columns: repeat(auto-fit, minmax(min(100%, 20rem), 1fr)); }
.tag { border-radius: var(--radius-xl); padding: var(--s-5); background: var(--raw-petrol); color: var(--raw-cloud); display: grid; gap: var(--s-2); }
.tag b { font: 500 1.5rem/1.25 var(--font-display); }
.tag span { opacity: 0.85; font: var(--t-secondary); }
.mocks { display: grid; gap: var(--s-5); grid-template-columns: repeat(auto-fill, minmax(min(100%, 24rem), 1fr)); }
.mock { display: grid; gap: var(--s-3); align-content: start; color: inherit; text-decoration: none; }
.mock .frames { position: relative; border-radius: var(--radius-xl); overflow: hidden; border: 1px solid var(--line); background: var(--ground-raised); aspect-ratio: 16 / 10; box-shadow: var(--shadow); transition: transform 0.2s, box-shadow 0.2s; }
.mock .frames img.d { width: 100%; height: 100%; object-fit: cover; object-position: top left; display: block; }
.mock .frames img.p { position: absolute; right: 12px; bottom: -40px; width: 24%; border-radius: 10px; box-shadow: 0 0 0 4px var(--raw-ink), var(--shadow-pop); transition: bottom 0.2s; }
.mock:hover .frames { transform: translateY(-2px); box-shadow: var(--shadow-pop); }
.mock:hover .frames img.p { bottom: -24px; }
.mock:focus-visible { outline: none; }
.mock:focus-visible .frames { outline: var(--focus-ring); outline-offset: 3px; }
.mock h3 { font: var(--t-group); display: flex; gap: var(--s-2); align-items: baseline; flex-wrap: wrap; }
.mock h3 code { font: 400 0.75rem/1 var(--font-mono); color: var(--ink-muted); }
.mock p { font: var(--t-secondary); color: var(--ink-muted); font-size: 0.9375rem; line-height: 1.55; }
.mock .new { font: var(--t-tiny); color: var(--accent); background: var(--accent-wash); border-radius: 999px; padding: 2px 8px; }
.niche { display: grid; gap: var(--s-4); grid-template-columns: repeat(auto-fit, minmax(min(100%, 18rem), 1fr)); }
.niche .box h3 { display: flex; gap: 8px; align-items: center; }
.files { columns: 2 18rem; column-gap: var(--s-6); font: var(--t-secondary); }
.files li { break-inside: avoid; padding: 6px 0; border-bottom: 1px solid var(--line); list-style: none; display: flex; gap: var(--s-2); justify-content: space-between; }
.files ul { margin: 0; padding: 0; }
.files span { color: var(--ink-muted); }
.og { width: 100%; height: auto; border-radius: var(--radius-xl); display: block; box-shadow: var(--shadow); }
.mesh-row { display: grid; gap: var(--s-4); grid-template-columns: repeat(auto-fit, minmax(min(100%, 18rem), 1fr)); margin-top: var(--s-4); }
.mesh-row .box { display: grid; gap: var(--s-3); justify-items: center; text-align: center; }
.mesh-row .owls { width: 100%; max-width: 240px; }
"""

DEMO_SCRIPT = r"""<script>
/* the owl demo: pressing a state changes the pose, the sentence and the wash together (never the owl alone) */
(function () {
  var demo = document.querySelector('.demo'); if (!demo) return;
  var owl = demo.querySelector('.stagebox > .owl'), said = demo.querySelector('.said'), pose = demo.querySelector('.pose');
  var T = {
    up: ['Everything is running.', 'Asleep. Nothing is wrong, so nothing moves.'],
    degraded: ['Invoice emails are slow.', 'One eye open, one tuft up: it keeps an eye on it.'],
    down: ['Card payments are down.', 'Awake, both eyes open, tufts up. Attentive, never alarmed, never sad. One perk of the tufts on waking, then still.'],
    maint: ['Planned maintenance, until 23:30.', 'A planned nap: eyes shut, tufts folded.'],
    none: ['Nothing to watch yet.', 'Two dashes, no blush: it cannot see yet.']
  };
  demo.addEventListener('click', function (e) {
    var b = e.target.closest('button[data-s]'); if (!b) return;
    var s = b.dataset.s;
    demo.querySelectorAll('button[data-s]').forEach(function (x) { x.setAttribute('aria-pressed', x === b ? 'true' : 'false'); });
    owl.classList.remove('woke'); owl.dataset.state = s; demo.dataset.s = s;
    if (s === 'down') { void owl.getBoundingClientRect(); owl.classList.add('woke'); }
    owl.setAttribute('aria-label', "Hora's owl: " + T[s][0]);
    said.textContent = T[s][0]; pose.textContent = T[s][1];
  });
})();
</script>"""


def sw(name, token, l, d, lr, note=''):
    return (f'<div class="sw"><div class="chipc"><span style="background:{l};color:{"#1B222C" if name in ("Cloud", "Wash", "Surface", "Line", "Raised") else "#F2F0EA"}">light</span>'
            f'<span style="background:{d};color:{"#EAE7E0" if name in ("Cloud", "Wash", "Surface", "Line", "Raised") else "#151A21"}">dark</span></div>'
            f'<div class="meta"><b>{name}</b><code>--{token}</code><code>{l} · {d}</code><span class="muted">{lr}</span>{f"<span class=muted>{note}</span>" if note else ""}</div></div>')


def mock(file, title, note, new=False):
    n = (f'<span class="new">{"added for the docs" if file == "docs-page" else "added for the mesh"}</span>') if new else ''
    return (f'<a class="mock" href="{file}.html"><div class="frames"><img class="d" src="shots/thumb-{file}-desk.jpg" alt="">'
            f'<img class="p" src="shots/thumb-{file}-phone.jpg" alt=""></div>'
            f'<h3>{title} <code>{file}.html</code>{n}</h3><p>{note}</p></a>')


def cell(color, y, h):
    return f'<svg class="cell" viewBox="0 0 4 29" aria-hidden="true"><rect style="fill:var({color})" y="{y}" width="4" height="{h}"/></svg>'


def page_index():
    states_cards = ''.join(
        f'<div class="st"><div class="ow" style="background:var({bg})">{owl(s, 96)}</div><h3>{glyph(s)}{w}</h3><p>{p}</p><code>assets/owl-{s}.svg</code></div>'
        for s, w, bg, p in [
            ('up', 'All up', '--wash-up', 'Asleep. Nothing is wrong, so nothing moves.'),
            ('degraded', 'Slow', '--wash-degraded', 'One eye open, one tuft up: an eye on it. Badge: half disc.'),
            ('down', 'Down', '--wash-down', 'Awake, tufts up. Attentive, never alarmed. Badge: diamond.'),
            ('maint', 'Maintenance', '--wash-maint', 'A planned nap, tufts folded. Badge: clock.'),
            ('none', 'No data', '--wash-maint', 'Two dashes, no blush; it cannot see yet. Badge: dashed ring.'),
        ])
    body = f"""<header class="top g-top"><div class="page">
<a class="sign hora" href="#top"><img src="assets/mark.svg" alt="" width="32" height="32" style="border-radius:8px"><span class="name">Hora</span></a>
<span class="sub">Brand and mockups · October 2026</span>
<nav class="toc" aria-label="Chapters"><a class="navlink" href="#concept">Concept</a><a class="navlink" href="#owl">The owl</a><a class="navlink" href="#mark">Mark</a><a class="navlink" href="#colour">Colour</a><a class="navlink" href="#states">States</a><a class="navlink" href="#type">Type</a><a class="navlink" href="#voice">Voice</a><a class="navlink" href="#mockups">Mockups</a></nav>
{theme_button()}</div></header>
<main id="main" class="page">
<span id="top"></span>
<section class="intro" aria-labelledby="h">
  <div>
    <p class="eyebrow">The third sibling, after Ariane and Maison</p>
    <h1 id="h"><em>Hora</em>, the night watch.</h1>
    <p class="lead">Night watchmen used to walk the town and call the hours: <b>"Ten o'clock, and all is well."</b> Hora is that watcher, as a little owl. It dozes while everything runs, opens one eye when something is slow, and wakes only for a real outage. <b>Flapping never wakes it, so it never wakes you.</b></p>
    <div class="chips" style="margin-top:var(--s-5)"><a class="btn primary" href="#mockups">See the mockups {icon('arrow')}</a><a class="btn" href="brand.md">Read brand.md</a></div>
  </div>
  <div class="demo" data-s="up">
    <div class="stagebox">{owl('up', 200, label="Hora's owl: Everything is running.")}</div>
    <p class="said" aria-live="polite">Everything is running.</p>
    <p class="pose">Asleep. Nothing is wrong, so nothing moves.</p>
    <div class="seg5" role="group" aria-label="Try a state">
      <button type="button" data-s="up" aria-pressed="true">{glyph('up')}Up</button>
      <button type="button" data-s="degraded" aria-pressed="false">{glyph('degraded')}Slow</button>
      <button type="button" data-s="down" aria-pressed="false">{glyph('down')}Down</button>
      <button type="button" data-s="maint" aria-pressed="false">{glyph('maint')}Maintenance</button>
      <button type="button" data-s="none" aria-pressed="false">{glyph('none')}No data</button>
    </div>
  </div>
</section>

<section class="chap" id="concept" aria-labelledby="c1"><header><span class="n">01</span><h2 id="c1">Who keeps watch</h2>
<p>Hora means "hours". The Horai kept the hours and the gates of Olympus. The night watch kept the hours of a town and said, every hour, that all was well. A status page does exactly that.</p></header>
<div class="cols">
  <div class="box"><h3>Why an owl</h3><p>The animal of the night watch, and Athena's little owl (<i>Athene noctua</i>), Greek like the Horai. Its tufts are Maison's ears, its round body is Ariane's ball of yarn: <b>the same family drawing.</b></p></div>
  <div class="box"><h3>Why asleep</h3><p>The promise is "flapping never wakes you up". An owl dozing in front of an all-green page says it better than any badge: <b>calm is the normal state</b>, and a closed eye is earned by retries, quorum and root-cause grouping.</p></div>
  <div class="box"><h3>Why several</h3><p>One owl can mishear. Hora nodes watch from several places and ask each other before they hoot: an outage is called when two agree. <b>A group of owls is a parliament</b>; this one votes before it wakes you.</p></div>
  <div class="box"><h3>Why cute, and still serious</h3><p>The owl lives in one place per page, at the size of a sentence's companion. Everything else is plain: Fraunces, Cal Sans, tabular numbers, words for every state. <b>Cute in the corner, precise in the table.</b></p></div>
</div>
<p class="quote">"Saturday 14:32. Everything is running."<small>The hero of every status page is the watchman's call: the hour, then the state, in one sentence.</small></p>
<div class="mesh-row">
  <div class="box">{owls(['up', 'up', 'up'], names=['Paris', 'Frankfurt', 'Montréal'], label='Three owls asleep: all is well everywhere')}<p><b>All well.</b> Three places, three owls asleep.</p></div>
  <div class="box">{owls(['down', 'up', 'up'], names=['Paris', 'Frankfurt', 'Montréal'], label='Only Paris awake: a local problem')}<p><b>Local only.</b> Paris wakes, asks, the others see it up: a notice, not a page.</p></div>
  <div class="box">{owls(['down', 'down', 'down'], names=['Paris', 'Frankfurt', 'Montréal'], label='All three awake: a real outage')}<p><b>Confirmed.</b> All three see it down: one alert, with the cause.</p></div>
</div>
</section>

<section class="chap" id="owl" aria-labelledby="c2"><header><span class="n">02</span><h2 id="c2">The owl and its states</h2>
<p>The pose is the overall state of the page. The badge repeats the state's shape, and the words beside it always say the same thing.</p></header>
<div class="states">{states_cards}</div>
<div class="rules">
  <div class="rule"><b>Never sad, never scared</b><p>No mouth, no tears, no frown. Down is "awake and attentive". It shows what Hora is doing, not how it feels about you.</p></div>
  <div class="rule"><b>Moves only in answer</b><p>No idle loop. It moves once when the state changes (the tufts perk on waking), then stays still.</p></div>
  <div class="rule"><b>Never alone</b><p>Always next to the sentence that says the state, with the state's shape in its badge. The owl is decoration for anyone who can't see it.</p></div>
  <div class="rule"><b>Reduced motion</b><p>Poses stay; transitions and the perk are dropped. A screen reader hears the sentence, not "owl".</p></div>
  <div class="rule"><b>One per page</b><p>In the hero, the door or an empty state. Never in rows, tables or charts. The mesh view is the one place where several perch together.</p></div>
  <div class="rule"><b>Can be turned off</b><p>Operators who want a neutral page set <code>[page] mascot = false</code> (proposal): the sentence and the shapes carry everything anyway.</p></div>
</div>
</section>

<section class="chap" id="mark" aria-labelledby="c3"><header><span class="n">03</span><h2 id="c3">Mark, wordmark, favicon</h2>
<p>The family recipe, unchanged: a petrol tile (48, radius 11), a cream character, a sleepy face without a mouth, the blush. The faint rings are the owl's facial disc, at the same 0.28 opacity as Ariane's yarn.</p></header>
<div class="marks">
  <div class="markbox"><div class="art ground"><div class="sz"><img src="assets/favicon.svg" width="16" height="16" alt="Favicon at 16 px">16</div><div class="sz"><img src="assets/favicon.svg" width="32" height="32" alt="">32</div><div class="sz"><img src="assets/mark.svg" width="48" height="48" alt="">48</div><div class="sz"><img src="assets/mark.svg" width="96" height="96" alt="Hora mark">96</div></div><div class="cap"><b>Mark and favicon</b><span>favicon.svg is redrawn on a 16 grid: no texture, no beak</span></div></div>
  <div class="markbox"><div class="art ground" style="align-items:center"><img src="assets/wordmark.svg" height="72" alt="Hora wordmark"></div><div class="cap"><b>The sign</b><span>Mark, then the name in Borel Display, set as outlines</span></div></div>
  <div class="markbox"><div class="art petrol" style="align-items:center"><img src="assets/wordmark-on-petrol.svg" height="84" alt="Hora wordmark on petrol"></div><div class="cap"><b>On the brand's petrol</b><span>The door, the social card: a faint cloud ring around the tile</span></div></div>
  <div class="markbox"><div class="art check" style="align-items:center"><img src="assets/mark-maskable.svg" width="96" height="96" alt="Maskable icon" style="border-radius:50%"><img src="assets/mark-maskable.svg" width="96" height="96" alt="" style="border-radius:22px"></div><div class="cap"><b>Maskable</b><span>Full-bleed petrol, owl in the 80 % safe zone</span></div></div>
  <div class="markbox"><div class="art ground" style="align-items:center"><img src="assets/family/ariane.svg" width="64" height="64" alt="Ariane"><img src="assets/family/maison.svg" width="64" height="64" alt="Maison"><img src="assets/mark.svg" width="64" height="64" alt="Hora"></div><div class="cap"><b>The family</b><span>Ariane, Maison, Hora (copies of the siblings' marks)</span></div></div>
  <div class="markbox"><div class="art ground" style="align-items:center"><img src="assets/variant-b-clock.svg" width="88" height="88" alt="Variant B: alarm clock"><img src="assets/variant-c-lantern.svg" width="88" height="88" alt="Variant C: lantern"></div><div class="cap"><b>Explored, not chosen</b><span>B: the alarm clock that does not ring. C: the watchman's lantern.</span></div></div>
</div>
<img class="og" src="assets/og-card.svg" alt="Social card: the owl asleep inside twelve hour ticks, Hora in Borel, 'Hora keeps the hours, so you can sleep.'" style="margin-top:var(--s-5)" width="1200" height="630">
</section>

<section class="chap" id="colour" aria-labelledby="c4"><header><span class="n">04</span><h2 id="c4">Colour: the family's, plus five states</h2>
<p>Ground, ink and one hue (petrol) in two steps and a wash, exactly as in synthe.se, Ariane and Maison. Up is petrol, not green: Hora looks like nobody else's status page, and the brand colour means "all is well".</p></header>
<div class="swatches">
  {sw('Cloud', 'ground', '#F2F0EA', '#151A21', 'page ground')}
  {sw('Surface', 'surface', '#FAF9F5', '#1A2029', 'cards, rows')}
  {sw('Raised', 'ground-raised', '#E7E4DB', '#1E2530', 'chips, code, tracks')}
  {sw('Line', 'line', '#D6D2C6', '#2E3744', 'hairlines only')}
  {sw('Ink', 'ink', '#1B222C', '#EAE7E0', '14.05:1 · 14.15:1')}
  {sw('Ink muted', 'ink-muted', '#525A66', '#A8AEB8', '6.12:1 · 7.83:1')}
  {sw('Petrol', 'accent', '#0E5E68', '#63C0CB', '6.54:1 · 8.27:1')}
  {sw('Petrol soft', 'accent-soft', '#4E8A92', '#3E7A84', '3.43:1 · 3.60:1, graphics')}
  {sw('Wash', 'accent-wash', '#D8E4E2', '#1D2A31', 'ink on it 12.29:1 · 11.91:1')}
  {sw('Amber', 'st-degraded', '#B4633F', '#D89372', 'text grade #8F4B22: 5.76:1')}
  {sw('Brick', 'st-down', '#A5372A', '#E07A63', '5.80:1 · 5.94:1')}
  {sw('Stone', 'st-none', '#87837A', '#687180', 'no data, 3.32:1 · 3.55:1')}
</div>
<div class="tw" style="margin-top:var(--s-5)"><table class="ctable">
<caption class="vh">Contrast ratios, WCAG 2.x</caption>
<thead><tr><th scope="col">Pair</th><th scope="col">Light</th><th scope="col" class="r">on ground</th><th scope="col" class="r">on surface</th><th scope="col">Dark</th><th scope="col" class="r">on ground</th><th scope="col" class="r">on surface</th><th scope="col">Use</th></tr></thead>
<tbody>
<tr><td>Ink</td><td><span class="dot" style="background:#1B222C"></span>#1B222C</td><td class="r">14.05</td><td class="r">15.20</td><td><span class="dot" style="background:#EAE7E0"></span>#EAE7E0</td><td class="r">14.15</td><td class="r">13.26</td><td>AAA body</td></tr>
<tr><td>Ink muted</td><td><span class="dot" style="background:#525A66"></span>#525A66</td><td class="r">6.12</td><td class="r">6.62</td><td><span class="dot" style="background:#A8AEB8"></span>#A8AEB8</td><td class="r">7.83</td><td class="r">7.34</td><td>AA secondary</td></tr>
<tr><td>Petrol (up, text)</td><td><span class="dot" style="background:#0E5E68"></span>#0E5E68</td><td class="r">6.54</td><td class="r">7.07</td><td><span class="dot" style="background:#63C0CB"></span>#63C0CB</td><td class="r">8.27</td><td class="r">7.74</td><td>AA text, links</td></tr>
<tr><td>Petrol soft (up, graphic)</td><td><span class="dot" style="background:#4E8A92"></span>#4E8A92</td><td class="r">3.43</td><td class="r">3.71</td><td><span class="dot" style="background:#3E7A84"></span>#3E7A84</td><td class="r">3.60</td><td class="r">3.37</td><td>bars, glyphs (3:1)</td></tr>
<tr><td>Amber (slow, graphic)</td><td><span class="dot" style="background:#B4633F"></span>#B4633F</td><td class="r">3.84</td><td class="r">4.16</td><td><span class="dot" style="background:#D89372"></span>#D89372</td><td class="r">6.94</td><td class="r">6.50</td><td>bars, glyphs</td></tr>
<tr><td>Amber (slow, text)</td><td><span class="dot" style="background:#8F4B22"></span>#8F4B22</td><td class="r">5.76</td><td class="r">6.23</td><td><span class="dot" style="background:#D89372"></span>#D89372</td><td class="r">6.94</td><td class="r">6.50</td><td>AA text</td></tr>
<tr><td>Brick (down)</td><td><span class="dot" style="background:#A5372A"></span>#A5372A</td><td class="r">5.80</td><td class="r">6.27</td><td><span class="dot" style="background:#E07A63"></span>#E07A63</td><td class="r">5.94</td><td class="r">5.56</td><td>AA text and graphics</td></tr>
<tr><td>Maintenance</td><td><span class="dot" style="background:#525A66"></span>#525A66</td><td class="r">6.12</td><td class="r">6.62</td><td><span class="dot" style="background:#A8AEB8"></span>#A8AEB8</td><td class="r">7.83</td><td class="r">7.34</td><td>= ink muted</td></tr>
<tr><td>No data</td><td><span class="dot" style="background:#87837A"></span>#87837A</td><td class="r">3.32</td><td class="r">3.59</td><td><span class="dot" style="background:#687180"></span>#687180</td><td class="r">3.55</td><td class="r">3.32</td><td>graphics only</td></tr>
</tbody></table></div>
<p class="mock-note"><b>On the washes.</b> Every state word keeps 5:1 or more on its own wash (petrol on wash-up 5.72, amber text on wash-slow 5.42, brick on wash-down 5.26; dark side 5.18 or more). The up glyph would only reach 3.0:1 on the up wash, so there it takes the text-grade petrol. Full table in brand.md.</p>
</section>

<section class="chap" id="states" aria-labelledby="c5"><header><span class="n">05</span><h2 id="c5">State: colour + shape + word</h2>
<p>Take the colour away and every state is still readable. Print the page in grey, use it colour-blind, read it aloud: same answer.</p></header>
<div class="tw"><table class="ss"><thead><tr><th scope="col">State</th><th scope="col">Shape</th><th scope="col">Day in the 90-day bar</th><th scope="col">Word (EN · FR)</th><th scope="col">Owl</th><th scope="col">Hero wash</th></tr></thead>
<tbody>
<tr><td>Up</td><td>{glyph('up')} filled disc</td><td>{cell('--st-up', 0, 22)} full height</td><td>Up · Opérationnel</td><td>{owl('up', 40)}</td><td>petrol wash</td></tr>
<tr><td>Degraded</td><td>{glyph('degraded')} half disc</td><td>{cell('--st-degraded', 7, 15)} shorter</td><td>Slow · Lent</td><td>{owl('degraded', 40)}</td><td>amber wash</td></tr>
<tr><td>Down</td><td>{glyph('down')} diamond, !</td><td>{cell('--st-down', 0, 29)} drops below the line</td><td>Down · En panne</td><td>{owl('down', 40)}</td><td>brick wash</td></tr>
<tr><td>Maintenance</td><td>{glyph('maint')} clock</td><td>{cell('--st-maint', 11, 11)} half</td><td>Maintenance · Maintenance</td><td>{owl('maint', 40)}</td><td>raised</td></tr>
<tr><td>No data</td><td>{glyph('none')} dashed ring</td><td>{cell('--st-none', 18, 4)} a stub</td><td>No data · Pas de données</td><td>{owl('none', 40)}</td><td>raised</td></tr>
</tbody></table></div>
</section>

<section class="chap" id="type" aria-labelledby="c6"><header><span class="n">06</span><h2 id="c6">Type, spacing, corners</h2>
<p>The family's locked pairing, self-hosted: Fraunces for what you read first, Cal Sans 2 for everything else, Borel Display for the name only. Spacing on 4, one radius scale, 44 px targets.</p></header>
<div class="type">
  <div class="spec"><span class="who">Display · Fraunces Variable 500</span><span class="big" style="font:500 2.6rem/1.1 var(--font-display)">Everything is running.</span><p>Page titles, the hero sentence, group names, big numbers (lining, tabular).</p></div>
  <div class="spec"><span class="who">Text · Cal Sans 2 VF 400 to 600</span><span class="big" style="font:400 1.25rem/1.45 var(--font-text)">Since 14:02 UTC (38 min). The other 12 services are up. 99.97 %</span><p>Body 16/24, secondary 14/20, meta 13/16, never under 12 px. Tabular figures everywhere.</p></div>
  <div class="spec"><span class="who">Name only · Borel Display</span><span class="big" style="font:400 3rem/1.3 var(--font-brand)">Hora</span><p>Beside the mark, nowhere else. Nudged down 0.16 em for its tall loops.</p></div>
  <div class="spec"><span class="who">Code · JetBrains Mono</span><span class="big" style="font:400 0.95rem/1.6 var(--font-mono)">hora probe api --confirm<br>up from 3/3 vantage points</span><p>Config, CLI, captured answers.</p></div>
</div>
<div class="scale-row" aria-label="Spacing scale">
  <div><i style="width:4px;height:4px"></i>4</div><div><i style="width:8px;height:8px"></i>8</div><div><i style="width:12px;height:12px"></i>12</div><div><i style="width:16px;height:16px"></i>16</div><div><i style="width:24px;height:24px"></i>24</div><div><i style="width:32px;height:32px"></i>32</div><div><i style="width:48px;height:48px"></i>48</div>
  <span style="width:var(--s-5)"></span>
  <div><i style="width:40px;height:40px;border-radius:2px"></i>2</div><div><i style="width:40px;height:40px;border-radius:6px"></i>6</div><div><i style="width:40px;height:40px;border-radius:8px"></i>8</div><div><i style="width:40px;height:40px;border-radius:10px"></i>10</div><div><i style="width:40px;height:40px;border-radius:12px"></i>12</div><div><i style="width:40px;height:40px;border-radius:16px"></i>16</div><div><i style="width:40px;height:40px;border-radius:999px"></i>pill</div>
</div>
</section>

<section class="chap" id="voice" aria-labelledby="c7"><header><span class="n">07</span><h2 id="c7">Voice</h2>
<p>Calm, plain, factual, a little warm. The state in words, the time, what you will notice. Jargon goes behind "For operators".</p></header>
<div class="voice">
  <div class="vrow"><span class="k">All up</span><span class="no">All Systems Operational</span><span class="yes">Everything is running.</span><span class="fr">Tout fonctionne.</span></div>
  <div class="vrow"><span class="k">Outage</span><span class="no">Major outage detected on payments-api</span><span class="yes">Card payments are down since 14:02.</span><span class="fr">Les paiements par carte sont en panne depuis 14 h 02.</span></div>
  <div class="vrow"><span class="k">Slow</span><span class="no">Degraded performance (p95 &gt; SLO)</span><span class="yes">Invoice emails arrive about 4 minutes late.</span><span class="fr">Les e-mails de facture arrivent avec 4 minutes de retard.</span></div>
  <div class="vrow"><span class="k">Local only</span><span class="no">Vantage disagreement: 1/3</span><span class="yes">Our Paris checkpoint can't reach it; the others can. Not an outage.</span><span class="fr">Notre point de contrôle de Paris ne le joint plus ; les autres oui. Pas une panne.</span></div>
  <div class="vrow"><span class="k">Budget</span><span class="no">Error budget: 31m remaining (72 %)</span><span class="yes">31 min of allowed downtime left this month.</span><span class="fr">Encore 31 min d'interruption possible ce mois-ci.</span></div>
  <div class="vrow"><span class="k">Empty</span><span class="no">No monitors configured.</span><span class="yes">Nothing to watch yet.</span><span class="fr">Rien à surveiller pour l'instant.</span></div>
</div>
<div class="taglines" style="margin-top:var(--s-5)">
  <div class="tag"><b>Hora keeps the hours, so you can sleep.</b><span>Main line. Docs, README, the door.</span></div>
  <div class="tag"><b>Hora veille, vous dormez.</b><span>French main line.</span></div>
  <div class="tag"><b>It only wakes you for real.</b><span>Feature line, for alerting. FR: « Elle ne vous réveille que pour de vrai. »</span></div>
</div>
</section>

<section class="chap" id="niche" aria-labelledby="c8"><header><span class="n">08</span><h2 id="c8">What the niche taught us</h2>
<p>From Atlassian Statuspage, Instatus, Better Stack, OpenStatus, Hyperping, incident.io, Uptime Kuma, Gatus and Cachet (from knowledge of these products; not re-checked online for this pass).</p></header>
<div class="niche">
  <div class="box"><h3>{glyph('up')}Kept</h3><ul><li><b>One headline sentence</b> for the whole page (Statuspage, incident.io).</li><li><b>90-day bars</b> with uptime % per service, 30 days on phones (Statuspage).</li><li><b>Incident updates</b> as a short, dated, newest-first timeline (Statuspage, Instatus).</li><li><b>Follow updates</b> near the headline (feed, JSON, badges).</li><li><b>"What you'll notice"</b> impact wording (incident.io, Better Stack).</li></ul></div>
  <div class="box"><h3>{glyph('down')}Avoided</h3><ul><li><b>Colour-only state</b>: green/red dots, bars with no shape (most of them).</li><li><b>Operator jargon</b> on public pages: p95, SLO, "heartbeating" (Uptime Kuma, Gatus).</li><li><b>Noisy grids</b>: hundreds of equal cards, every bar drawn as 90 nodes.</li><li><b>Hover-only details</b> that touch and keyboard users never reach.</li><li><b>Full-page refresh</b> that loses scroll and focus.</li></ul></div>
  <div class="box"><h3>{glyph('info')}Hora's own</h3><ul><li><b>Only what is wrong, up front</b>: "Needs attention", then groups folded with a summary.</li><li><b>Shapes in the bar</b>: shorter is slow, a drop below the line is down.</li><li><b>Checked from several places</b>, and a calm public sentence when they disagree.</li><li><b>The watchman's call</b>: the hour, then the state.</li><li><b>A face, with rules</b>: the owl never carries the state alone.</li></ul></div>
</div>
</section>

<section class="chap" id="mockups" aria-labelledby="c9"><header><span class="n">09</span><h2 id="c9">Mockups</h2>
<p>Plain HTML and CSS, light and dark (system, or the toggle), checked at 320, 360, 1440 and 2560 px without sideways scroll. Thumbnails: desktop light, phone dark.</p></header>
<div class="mocks">
  {mock('status-ok', 'All up', 'The hero is the watchman\'s call. Groups as columns of framed rows; Rows or Cards (cards add per-place response times and a one-path chart). "Who keeps watch" closes the page.')}
  {mock('status-incident', 'Outage, slow service, announcement', 'Brick wash, awake owl, one sentence with the time. "Right now" puts the story first: what you\'ll notice, confirmed from 3 places, dated updates.')}
  {mock('status-local', 'Local only: not an outage', 'Paris can\'t reach the API, Frankfurt and Montréal can. Visitors read "Everything is running" and one calm note; operators open "Why nobody was woken".', new=True)}
  {mock('status-maintenance', 'Planned maintenance', 'The owl naps, the clock shape marks what is paused, a progress bar shows the window. The next maintenance is announced below.')}
  {mock('status-scale', '712 monitors', 'What is wrong first, then 7 groups folded with counts and one group bar each. The open group lists 24 compact rows, paged. 16 KB of content.')}
  {mock('monitor', 'A monitor\'s history', 'Plain-language tiles, a chart with HTML labels, 90 focusable days with tooltips that work on touch, an hour-by-hour week, each place\'s view; percentiles behind "For operators".')}
  {mock('incident', 'Post-mortem', 'Title says what happened and how long. Timeline with state shapes, "seen down from 3 of 3", the captured answer, Copy as Markdown.')}
  {mock('watchers', 'Watchers (operators)', 'The mesh: heartbeats, a link split reported as a split, confirmations over 30 days, each service from each place.', new=True)}
  {mock('report', 'Monthly report', 'An A4 sheet on screen, print-ready: five headline numbers, a table per group with the target met or missed in words, how it is counted.')}
  {mock('empty', 'Empty and private', 'First run with a copyable snippet, the door for a private page (Maison\'s AuthShell), a monitor waiting for its first check.')}
  {mock('docs-page', 'A docs page', 'Every docs page wears the brand: wordmark, search, sidebar with the current page, "On this page", Fraunces headings, TOML and shell blocks with Copy, a table, and the four asides on the state system. Real content: the peers guide.', new=True)}
  {mock('docs-home', 'Docs and README', 'Starlight splash re-themed: tagline, install, the owl on the dial of hours, the blip-to-alert flow, a comparison table and the README header.')}
</div>
</section>

<section class="chap" id="files" aria-labelledby="c10"><header><span class="n">10</span><h2 id="c10">Files</h2></header>
<div class="files"><ul>
<li><a href="brand.md">brand.md</a><span>the brand book</span></li>
<li><a href="assets/mark.svg">assets/mark.svg</a><span>48 tile</span></li>
<li><a href="assets/mark-maskable.svg">assets/mark-maskable.svg</a><span>PWA</span></li>
<li><a href="assets/favicon.svg">assets/favicon.svg</a><span>16 grid</span></li>
<li><a href="assets/wordmark.svg">assets/wordmark.svg</a><span>auto dark</span></li>
<li><a href="assets/wordmark-on-petrol.svg">assets/wordmark-on-petrol.svg</a><span>on petrol</span></li>
<li><a href="assets/og-card.svg">assets/og-card.svg</a><span>1200 × 630</span></li>
<li><a href="assets/owl-up.svg">assets/owl-up.svg</a><span>+ degraded, down, maint, none</span></li>
<li><a href="assets/variant-b-clock.svg">assets/variant-b-clock.svg</a><span>alternative</span></li>
<li><a href="assets/variant-c-lantern.svg">assets/variant-c-lantern.svg</a><span>alternative</span></li>
<li><a href="fonts/">fonts/</a><span>Fraunces, Cal Sans 2, Borel, JetBrains Mono (OFL)</span></li>
<li><a href="brand.md">brand.md §12</a><span>Starlight custom.css, ready to paste</span></li>
<li><a href="src/">src/</a><span>the Python that writes every page</span></li>
</ul></div>
</section>
</main>
<footer class="foot"><div class="page"><span>Hora brand, proposal for Léonard · built on the synthe.se tokens shared with Ariane and Maison</span><span class="watched"><img src="assets/mark.svg" alt="" width="22" height="22"><span class="b">Hora</span></span></div></footer>"""
    return shell('Hora · brand and mockups', body, 'The Hora brand: a little owl who keeps the hours. Concept, mark, palette, states, voice and mockups.', G_CSS, scripts=DEMO_SCRIPT)


PAGES = {'index.html': page_index}
