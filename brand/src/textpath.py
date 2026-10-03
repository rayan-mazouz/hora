"""Set a string in a font as one SVG path (for the wordmark and the OG card)."""
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
import os
F = os.path.join(os.path.dirname(__file__), '..', 'fonts')
_cache = {}
def font(name, wght=None):
    key = (name, wght)
    if key in _cache: return _cache[key]
    f = TTFont(os.path.join(F, name))
    if wght and 'fvar' in f:
        from fontTools.varLib.instancer import instantiateVariableFont
        f = instantiateVariableFont(f, {'wght': wght})
    _cache[key] = f
    return f
def text_path(s, name, size, x=0, y=0, wght=None, tracking=0):
    f = font(name, wght)
    gs = f.getGlyphSet(); cmap = f.getBestCmap(); upm = f['head'].unitsPerEm
    sc = size / upm; pen = SVGPathPen(gs); cx = 0
    hmtx = f['hmtx']
    kern = {}
    for ch in s:
        g = cmap[ord(ch)]
        tp = TransformPen(pen, (sc, 0, 0, -sc, x + cx * sc, y))
        gs[g].draw(tp)
        cx += hmtx[g][0] + tracking
    return pen.getCommands(), cx * sc
