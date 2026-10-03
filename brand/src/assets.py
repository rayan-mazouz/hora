"""Write the SVG assets: favicon, maskable mark, owl states, wordmark, OG card, variants."""
import os
from textpath import text_path

A = os.path.join(os.path.dirname(__file__), '..', 'assets')
PETROL, CLOUD, INK, BLUSH = '#0E5E68', '#F2F0EA', '#1B222C', '#E6A49A'


def w(name, s):
    with open(os.path.join(A, name), 'w') as f:
        f.write(s.strip() + '\n')


MARK_INNER = open(os.path.join(A, 'mark.svg')).read().split('<rect width="48" height="48" rx="11" fill="#0E5E68"/>')[1].rsplit('</svg>', 1)[0]

# maskable: full-bleed petrol, the owl inside the 80 % safe zone
w('mark-maskable.svg', f'''
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">
  <!-- Hora, maskable (PWA / Android): full-bleed petrol, the owl scaled into the safe zone -->
  <rect width="48" height="48" fill="#0E5E68"/>
  <g transform="translate(4.8 4.8) scale(.8)">{MARK_INNER}</g>
</svg>''')

# favicon: drawn on a 16 grid, no texture, no blush, no beak; the tufts and two shut eyes carry it
w('favicon.svg', '''
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
  <!-- Hora at 16 px: tile, owl, tufts, two shut eyes. Everything else is dropped. -->
  <rect width="16" height="16" rx="3.6" fill="#0E5E68"/>
  <path d="M5.2 5.3 3.6 2.6 6.9 4.1ZM10.8 5.3 12.4 2.6 9.1 4.1Z" fill="#F2F0EA" stroke="#F2F0EA" stroke-width=".9" stroke-linejoin="round"/>
  <ellipse cx="8" cy="8.9" rx="4.5" ry="4.9" fill="#F2F0EA"/>
  <path d="M4.9 8.3a1.15 1.15 0 0 0 2.3 0M8.8 8.3a1.15 1.15 0 0 0 2.3 0" fill="none" stroke="#0E5E68" stroke-width="1.05" stroke-linecap="round"/>
</svg>''')

# ---------------------------------------------------------------- owl states
STYLE = '''<style>
  .ln { fill: #FAF9F5; stroke: #1B222C; stroke-width: 1.5; stroke-linejoin: round; }
  .tx { fill: none; stroke: #1B222C; stroke-width: 1; stroke-linecap: round; opacity: .2; }
  .ft, .eo { fill: #1B222C; } .gl { fill: #FAF9F5; }
  .ey { fill: none; stroke: #1B222C; stroke-width: 1.8; stroke-linecap: round; }
  .bk { fill: #0E5E68; stroke: #0E5E68; stroke-width: .9; stroke-linejoin: round; }
  .ck { fill: #E6A49A; opacity: .85; }
  .bd { fill: #F2F0EA; } .s-degraded { fill: #B4633F; } .r-degraded { stroke: #B4633F; } .s-down { fill: #A5372A; } .s-maint, .s-none { stroke: #525A66; }
  .cut { fill: #F2F0EA; stroke: #F2F0EA; }
  @media (prefers-color-scheme: dark) {
    .ln { fill: #1A2029; stroke: #EAE7E0; } .tx, .ey { stroke: #EAE7E0; } .ft, .eo { fill: #EAE7E0; } .gl { fill: #1A2029; }
    .bk { fill: #63C0CB; stroke: #63C0CB; } .ck { fill: #B8776E; }
    .bd, .cut { fill: #151A21; stroke: #151A21; } .s-degraded { fill: #D89372; } .r-degraded { stroke: #D89372; } .s-down { fill: #E07A63; } .s-maint, .s-none { stroke: #A8AEB8; }
  }
</style>'''

EYES = {
    'shut-l': '<path class="ey" d="M16.7 24.4a2.2 2.2 0 0 0 4.4 0"/>',
    'shut-r': '<path class="ey" d="M26.9 24.4a2.2 2.2 0 0 0 4.4 0"/>',
    'open-l': '<ellipse class="eo" cx="18.9" cy="24.6" rx="2.2" ry="2.6"/><circle class="gl" cx="19.7" cy="23.6" r=".75"/>',
    'open-r': '<ellipse class="eo" cx="29.1" cy="24.6" rx="2.2" ry="2.6"/><circle class="gl" cx="29.9" cy="23.6" r=".75"/>',
    'dash': '<path class="ey" d="M16.9 24.8h4M27.1 24.8h4"/>',
}
BADGE = {
    'degraded': '<circle cx="39.5" cy="38.5" r="4.4" fill="none" class="r-degraded" stroke-width="1.5"/><path class="s-degraded" d="M39.5 34.1a4.4 4.4 0 0 0 0 8.8z"/>',
    'down': '<path class="s-down" d="M39.5 32.9 45.1 38.5 39.5 44.1 33.9 38.5Z" stroke-linejoin="round"/><path class="cut" d="M39.5 35.9v3" stroke-width="1.5" stroke-linecap="round"/><circle class="cut" cx="39.5" cy="41" r=".8" stroke="none"/>',
    'maint': '<circle cx="39.5" cy="38.5" r="4.6" fill="none" class="s-maint" stroke-width="1.5"/><path d="M39.5 35.9v2.6l1.9 1.3" fill="none" class="s-maint" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>',
    'none': '<circle cx="39.5" cy="38.5" r="4.6" fill="none" class="s-none" stroke-width="1.5" stroke-dasharray="2.1 1.8"/>',
}
POSES = {  # eyes, left tuft angle, right tuft angle
    'up': (['shut-l', 'shut-r'], -5, 5, 'All up: asleep. Nothing is wrong, so nothing moves.'),
    'degraded': (['shut-l', 'open-r'], -5, -6, 'Degraded: one eye open, one tuft up.'),
    'down': (['open-l', 'open-r'], 5, -5, 'Down: awake, both eyes open, tufts up. Attentive, never alarmed.'),
    'maint': (['shut-l', 'shut-r'], -22, 22, 'Maintenance: a planned nap, tufts folded.'),
    'none': (['dash'], 0, 0, 'No data: two dashes, no blush. It cannot see yet.'),
}
LABEL = {'up': 'all up', 'degraded': 'degraded', 'down': 'down', 'maint': 'maintenance', 'none': 'no data'}


def owl_static(st):
    eyes, a, b, note = POSES[st]
    ck = '' if st == 'none' else '<ellipse class="ck" cx="16.4" cy="30.6" rx="2" ry="1.2"/><ellipse class="ck" cx="31.6" cy="30.6" rx="2" ry="1.2"/>'
    badge = '' if st == 'up' else f'<circle class="bd" cx="39.5" cy="38.5" r="7.2"/>{BADGE[st]}'
    return f'''
<svg xmlns="http://www.w3.org/2000/svg" viewBox="6 6 42 42" width="168" height="168" role="img" aria-label="Hora's owl, {LABEL[st]}">
  <!-- {note} The word beside it always says the same thing. -->
  {STYLE}
  <path class="ln" transform="rotate({a} 20.4 16.8)" d="M17.6 16.8C15.4 15.9 13.9 14 13.4 11.3 15.9 11.7 18.4 13 20.2 15.2Z"/>
  <path class="ln" transform="rotate({b} 27.6 16.8)" d="M30.4 16.8C32.6 15.9 34.1 14 34.6 11.3 32.1 11.7 29.6 13 27.8 15.2Z"/>
  <ellipse class="ft" cx="20.6" cy="41.1" rx="1.9" ry="1.2"/><ellipse class="ft" cx="27.4" cy="41.1" rx="1.9" ry="1.2"/>
  <path class="ln" d="M24 14C30.9 14 35.6 19.2 35.6 26.6 35.6 34.6 30.6 40.4 24 40.4S12.4 34.6 12.4 26.6C12.4 19.2 17.1 14 24 14Z"/>
  <g class="tx"><circle cx="18.9" cy="24.6" r="4.9"/><circle cx="29.1" cy="24.6" r="4.9"/><path d="M14.4 30.8C15.8 33.7 18.1 36.3 21 37.8M33.6 30.8C32.2 33.7 29.9 36.3 27 37.8"/></g>
  {''.join(EYES[e] for e in eyes)}
  <path class="bk" d="M22.9 28.3h2.2L24 30.1Z"/>
  {ck}
  {badge}
</svg>'''


for st in POSES:
    w(f'owl-{st}.svg', owl_static(st))

# ---------------------------------------------------------------- wordmark
d, width = text_path('Hora', 'BorelDisplay-Regular.woff2', 34, x=58, y=38)
W = 58 + width + 4
w('wordmark.svg', f'''
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W:.1f} 56" width="{W * 2:.0f}" height="112" role="img" aria-label="Hora">
  <!-- The sign: the mark, then the name in Borel Display (Rosalie Wagner, OFL), set as outlines.
       Borel's loops are the owl's slow round of the hours. Gap: the mark's width / 6. -->
  <style>.n {{ fill: #1B222C; }} @media (prefers-color-scheme: dark) {{ .n {{ fill: #EAE7E0; }} }}</style>
  <g transform="translate(0 4)"><rect width="48" height="48" rx="11" fill="#0E5E68"/>{MARK_INNER}</g>
  <path class="n" d="{d}"/>
</svg>''')
d2, width2 = text_path('Hora', 'BorelDisplay-Regular.woff2', 34, x=58, y=38)
w('wordmark-on-petrol.svg', f'''
<svg xmlns="http://www.w3.org/2000/svg" viewBox="-12 -8 {W + 24:.1f} 72" width="{(W + 24) * 2:.0f}" height="144" role="img" aria-label="Hora">
  <!-- The sign on the brand's own petrol (the door, the OG card): the tile gets a faint cloud ring -->
  <rect x="-12" y="-8" width="{W + 24:.1f}" height="72" rx="16" fill="#0E5E68"/>
  <g transform="translate(0 4)"><rect width="48" height="48" rx="11" fill="#0E5E68" stroke="#F2F0EA" stroke-opacity=".35" stroke-width="1"/>{MARK_INNER}</g>
  <path fill="#F2F0EA" d="{d2}"/>
</svg>''')

# ---------------------------------------------------------------- OG card (1200x630)
name_d, name_w = text_path('Hora', 'BorelDisplay-Regular.woff2', 120, x=560, y=268)
tag1, _ = text_path('Hora keeps the hours,', 'Fraunces-VF.woff2', 54, x=560, y=380, wght=500)
tag2, _ = text_path('so you can sleep.', 'Fraunces-VF.woff2', 54, x=560, y=446, wght=500)
sub, _ = text_path('Self-hosted uptime monitor, one small binary.', 'CalSansVF.woff2', 26, x=560, y=530)
ticks = []
import math
for i in range(12):
    ang = math.radians(i * 30 - 90)
    r1, r2 = (238, 262) if i % 3 == 0 else (248, 262)
    cx, cy = 300, 330
    ticks.append(f'M{cx + r1 * math.cos(ang):.1f} {cy + r1 * math.sin(ang):.1f}L{cx + r2 * math.cos(ang):.1f} {cy + r2 * math.sin(ang):.1f}')
w('og-card.svg', f'''
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 630" width="1200" height="630">
  <!-- Social card: the brand's petrol, the owl asleep inside the twelve hours, the name in Borel. -->
  <rect width="1200" height="630" fill="#0E5E68"/>
  <g fill="none" stroke="#F2F0EA" stroke-linecap="round" opacity=".22">
    <circle cx="300" cy="330" r="200" stroke-width="2" stroke-dasharray="2 12"/>
    <path d="{' '.join(ticks)}" stroke-width="4"/>
  </g>
  <g transform="translate(78 92) scale(9.25)">
    <g fill="#F2F0EA">
      <path d="M17.6 15.8C15.1 14.8 13.3 12.5 12.7 9.2 15.8 9.6 18.6 11.3 20.4 13.9Z"/>
      <path d="M30.4 15.8C32.9 14.8 34.7 12.5 35.3 9.2 32.2 9.6 29.4 11.3 27.6 13.9Z"/>
      <ellipse cx="20.6" cy="40.1" rx="2.1" ry="1.4"/><ellipse cx="27.4" cy="40.1" rx="2.1" ry="1.4"/>
      <path d="M24 13C30.9 13 35.6 18.2 35.6 25.6 35.6 33.6 30.6 39.4 24 39.4S12.4 33.6 12.4 25.6C12.4 18.2 17.1 13 24 13Z"/>
    </g>
    <g fill="none" stroke="#0E5E68" stroke-width="1" stroke-linecap="round" opacity=".28">
      <circle cx="18.9" cy="23.6" r="4.9"/><circle cx="29.1" cy="23.6" r="4.9"/>
      <path d="M14.4 29.8C15.8 32.7 18.1 35.3 21 36.8M33.6 29.8C32.2 32.7 29.9 35.3 27 36.8"/>
    </g>
    <path d="M16.7 23.4a2.2 2.2 0 0 0 4.4 0M26.9 23.4a2.2 2.2 0 0 0 4.4 0" fill="none" stroke="#0E5E68" stroke-width="1.7" stroke-linecap="round"/>
    <path d="M22.9 27.3h2.2L24 29.1Z" fill="#0E5E68" stroke="#0E5E68" stroke-width=".9" stroke-linejoin="round"/>
    <ellipse cx="16.4" cy="29.6" rx="2" ry="1.2" fill="#E6A49A" opacity=".85"/>
    <ellipse cx="31.6" cy="29.6" rx="2" ry="1.2" fill="#E6A49A" opacity=".85"/>
  </g>
  <path fill="#F2F0EA" d="{name_d}"/>
  <path fill="#F2F0EA" d="{tag1}"/>
  <path fill="#F2F0EA" d="{tag2}"/>
  <path fill="#F2F0EA" fill-opacity=".82" d="{sub}"/>
</svg>''')

# ---------------------------------------------------------------- explored alternatives
w('variant-b-clock.svg', '''
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">
  <!-- Variant B: the alarm clock that does not ring. Its bells lean like Maison's ears. -->
  <rect width="48" height="48" rx="11" fill="#0E5E68"/>
  <g fill="#F2F0EA">
    <path d="M10.6 17.4a6.4 6.4 0 0 1 9.4-6.8Z" transform="rotate(-6 15 14)"/>
    <path d="M28 10.6a6.4 6.4 0 0 1 9.4 6.8Z" transform="rotate(10 33 14)"/>
  </g>
  <path d="M15.6 39.6l-2.2 2.8M32.4 39.6l2.2 2.8M24 11.6v2.6" stroke="#F2F0EA" stroke-width="2.4" stroke-linecap="round"/>
  <circle cx="24" cy="27" r="13" fill="#F2F0EA"/>
  <g stroke="#0E5E68" stroke-width="1.2" stroke-linecap="round" opacity=".28"><path d="M24 16v2M24 36v2M13 27h2M33 27h2"/></g>
  <path d="M17.4 27.2a2.3 2.3 0 0 0 4.4 0M26.2 27.2a2.3 2.3 0 0 0 4.4 0" fill="none" stroke="#0E5E68" stroke-width="1.7" stroke-linecap="round"/>
  <ellipse cx="16.2" cy="31.4" rx="2" ry="1.2" fill="#E6A49A" opacity=".85"/>
  <ellipse cx="31.8" cy="31.4" rx="2" ry="1.2" fill="#E6A49A" opacity=".85"/>
</svg>''')

w('variant-c-lantern.svg', '''
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">
  <!-- Variant C: the night watchman's lantern, its flame a sleepy face. Calls the hours, lights the way. -->
  <rect width="48" height="48" rx="11" fill="#0E5E68"/>
  <path d="M19.4 12.4a4.6 4.6 0 0 1 9.2 0" fill="none" stroke="#F2F0EA" stroke-width="2" stroke-linecap="round"/>
  <path d="M15.6 17.4h16.8l-2.2-3.4H17.8Z" fill="#F2F0EA" stroke="#F2F0EA" stroke-width="1.6" stroke-linejoin="round"/>
  <rect x="14.6" y="18.6" width="18.8" height="19.6" rx="5" fill="#F2F0EA"/>
  <path d="M17 40.4h14" stroke="#F2F0EA" stroke-width="2.6" stroke-linecap="round"/>
  <g stroke="#0E5E68" stroke-width="1" opacity=".28"><path d="M19.6 19v18.8M28.4 19v18.8"/></g>
  <path d="M18.3 27.6a2.2 2.2 0 0 0 4.4 0M25.3 27.6a2.2 2.2 0 0 0 4.4 0" fill="none" stroke="#0E5E68" stroke-width="1.7" stroke-linecap="round"/>
  <ellipse cx="17.6" cy="31.8" rx="1.8" ry="1.1" fill="#E6A49A" opacity=".85"/>
  <ellipse cx="30.4" cy="31.8" rx="1.8" ry="1.1" fill="#E6A49A" opacity=".85"/>
</svg>''')
print('assets written')
