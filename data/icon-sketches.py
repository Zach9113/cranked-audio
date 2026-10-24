import math, pathlib
out = pathlib.Path(__file__).parent
C, R = 128, 84
def pt(deg, r=R, c=C):
    a = math.radians(deg); return (c + r * math.cos(a), c + r * math.sin(a))
def arc(a0, a1, r=R):
    (x0, y0), (x1, y1) = pt(a0, r), pt(a1, r)
    return f"M {x0:.1f} {y0:.1f} A {r} {r} 0 {1 if a1 - a0 > 180 else 0} 1 {x1:.1f} {y1:.1f}"
START, END = 135, 405
CRANK = START + 270 * 0.94

def ticks(r0, r1, n=11, color="#ffffff", opacity=0.35, width=4):
    s = []
    for i in range(n):
        a = START + 270 * i / (n - 1)
        (x0, y0), (x1, y1) = pt(a, r0), pt(a, r1)
        s.append(f'<line x1="{x0:.1f}" y1="{y0:.1f}" x2="{x1:.1f}" y2="{y1:.1f}" stroke="{color}" stroke-opacity="{opacity}" stroke-width="{width}" stroke-linecap="round"/>')
    return "\n  ".join(s)

px, py = pt(CRANK, 40)

# A: cranked to the max: fire gradient arc with glow, tick ring, brushed knob
a = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#26262e"/><stop offset="1" stop-color="#0e0e12"/></linearGradient>
    <linearGradient id="fire" x1="0" y1="1" x2="1" y2="0.6"><stop offset="0" stop-color="#ffd23f"/><stop offset="0.55" stop-color="#ff7a1a"/><stop offset="1" stop-color="#ff2d3c"/></linearGradient>
    <radialGradient id="knob" cx="0.4" cy="0.32" r="0.8"><stop offset="0" stop-color="#4a4a56"/><stop offset="0.6" stop-color="#26262e"/><stop offset="1" stop-color="#17171c"/></radialGradient>
    <filter id="glow" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="7"/></filter>
  </defs>
  <rect width="256" height="256" rx="56" fill="url(#bg)"/>
  {ticks(104, 112)}
  <path d="{arc(START, END)}" fill="none" stroke="#000000" stroke-opacity="0.45" stroke-width="20" stroke-linecap="round"/>
  <path d="{arc(START, CRANK)}" fill="none" stroke="url(#fire)" stroke-width="20" stroke-linecap="round" filter="url(#glow)" opacity="0.8"/>
  <path d="{arc(START, CRANK)}" fill="none" stroke="url(#fire)" stroke-width="20" stroke-linecap="round"/>
  <circle cx="128" cy="131" r="58" fill="#000" opacity="0.35"/>
  <circle cx="128" cy="128" r="56" fill="url(#knob)" stroke="#55555f" stroke-width="2"/>
  <line x1="128" y1="128" x2="{px:.1f}" y2="{py:.1f}" stroke="#ffffff" stroke-width="10" stroke-linecap="round"/>
  <circle cx="128" cy="128" r="9" fill="#ff7a1a"/>
</svg>'''

# B: neon: magenta to orange tube with bloom on deep violet, glossy black knob
b = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs>
    <radialGradient id="bg" cx="0.5" cy="0.45" r="0.75"><stop offset="0" stop-color="#2a1240"/><stop offset="1" stop-color="#0b0612"/></radialGradient>
    <linearGradient id="neon" x1="0" y1="1" x2="1" y2="0.4"><stop offset="0" stop-color="#ff2bd6"/><stop offset="1" stop-color="#ff9d2b"/></linearGradient>
    <radialGradient id="knob" cx="0.38" cy="0.3" r="0.85"><stop offset="0" stop-color="#3b3446"/><stop offset="0.5" stop-color="#16121c"/><stop offset="1" stop-color="#08060b"/></radialGradient>
    <filter id="bloom" x="-40%" y="-40%" width="180%" height="180%"><feGaussianBlur stdDeviation="10"/></filter>
  </defs>
  <rect width="256" height="256" rx="56" fill="url(#bg)"/>
  <path d="{arc(START, END)}" fill="none" stroke="#ffffff" stroke-opacity="0.08" stroke-width="16" stroke-linecap="round"/>
  <path d="{arc(START, CRANK)}" fill="none" stroke="url(#neon)" stroke-width="26" stroke-linecap="round" filter="url(#bloom)"/>
  <path d="{arc(START, CRANK)}" fill="none" stroke="url(#neon)" stroke-width="14" stroke-linecap="round"/>
  <path d="{arc(START, CRANK)}" fill="none" stroke="#fff" stroke-opacity="0.55" stroke-width="4" stroke-linecap="round"/>
  <circle cx="128" cy="128" r="54" fill="url(#knob)" stroke="#ff9d2b" stroke-opacity="0.35" stroke-width="2"/>
  <ellipse cx="114" cy="104" rx="30" ry="16" fill="#ffffff" opacity="0.08"/>
  <line x1="{pt(CRANK, 22)[0]:.1f}" y1="{pt(CRANK, 22)[1]:.1f}" x2="{pt(CRANK, 44)[0]:.1f}" y2="{pt(CRANK, 44)[1]:.1f}" stroke="#ff9d2b" stroke-width="9" stroke-linecap="round"/>
</svg>'''

# C: flat and bold: knob with pointer plus sound waves bursting out
wave = lambda r: f'M {C + r * math.cos(math.radians(-35)):.1f} {C + r * math.sin(math.radians(-35)):.1f} A {r} {r} 0 0 1 {C + r * math.cos(math.radians(35)):.1f} {C + r * math.sin(math.radians(35)):.1f}'
cx = 100
c = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <rect width="256" height="256" rx="56" fill="#ff6a13"/>
  <g transform="translate(-40 0) scale(1)">
    <circle cx="128" cy="128" r="58" fill="#1a1a20"/>
    <circle cx="128" cy="128" r="58" fill="none" stroke="#ffffff" stroke-opacity="0.15" stroke-width="4"/>
    <line x1="128" y1="128" x2="{pt(CRANK, 44)[0]:.1f}" y2="{pt(CRANK, 44)[1]:.1f}" stroke="#ffffff" stroke-width="12" stroke-linecap="round"/>
  </g>
  <path d="{wave(80).replace('M ', 'M ', 1)}" transform="translate(-40 0) scale(1)" fill="none" stroke="#1a1a20" stroke-width="13" stroke-linecap="round"/>
  <path d="{wave(102)}" transform="translate(-40 0) scale(1)" fill="none" stroke="#1a1a20" stroke-width="13" stroke-linecap="round" opacity="0.75"/>
  <path d="{wave(124)}" transform="translate(-40 0) scale(1)" fill="none" stroke="#1a1a20" stroke-width="13" stroke-linecap="round" opacity="0.5"/>
</svg>'''

for name, svg in (("A", a), ("B", b), ("C", c)):
    (out / f"{name}.svg").write_text(svg)
