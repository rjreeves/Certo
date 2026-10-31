"""Regenerate crates/cli/assets/certo.ico and certo-icon.svg (the Certo compiler icon).

Design: a "C" ring with a check mark through its opening -- Certo = certain/verified.
Usage: python scripts/make-certo-icon.py   (needs Pillow)
"""
import math, os
from PIL import Image, ImageDraw

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "cli", "assets")
S = 1024
BG_TOP, BG_BOT = (74, 99, 190), (38, 52, 120)
WHITE, MINT = (255, 255, 255), (94, 234, 170)
CX = CY = 512
R, W = 275, 112            # C ring radius / stroke
GAP = 42                   # half-angle of the opening (degrees)
CHECK = [(385, 515), (485, 615), (655, 415)]
CW = 76


def render(size=S):
    k = 2  # supersample
    n = S * k
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    # gradient background
    grad = Image.new("RGBA", (n, n))
    px = grad.load()
    for y in range(n):
        t = y / (n - 1)
        c = tuple(int(BG_TOP[i] + (BG_BOT[i] - BG_TOP[i]) * t) for i in range(3)) + (255,)
        for x in range(n):
            px[x, y] = c
    mask = Image.new("L", (n, n), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, n - 1, n - 1], radius=230 * k, fill=255)
    img.paste(grad, (0, 0), mask)
    d = ImageDraw.Draw(img)

    def cap(p, r, fill):
        d.ellipse([p[0] * k - r * k, p[1] * k - r * k, p[0] * k + r * k, p[1] * k + r * k], fill=fill)

    # C ring: arc from +GAP to 360-GAP (PIL angles run clockwise from 3 o'clock)
    Ro = R + W // 2
    box = [(CX - Ro) * k, (CY - Ro) * k, (CX + Ro) * k, (CY + Ro) * k]
    d.arc(box, GAP, 360 - GAP, fill=WHITE, width=W * k)
    for a in (GAP, 360 - GAP):
        cap((CX + R * math.cos(math.radians(a)), CY + R * math.sin(math.radians(a))), W / 2, WHITE)
    # check mark
    d.line([(x * k, y * k) for x, y in CHECK], fill=MINT, width=CW * k, joint="curve")
    for p in (CHECK[0], CHECK[-1]):
        cap(p, CW / 2, MINT)
    return img.resize((size, size), Image.LANCZOS)


def svg():
    a0, a1 = math.radians(GAP), math.radians(-GAP)
    x0, y0 = CX + R * math.cos(a0), CY + R * math.sin(a0)
    x1, y1 = CX + R * math.cos(a1), CY + R * math.sin(a1)
    pts = " ".join(f"{x},{y}" for x, y in CHECK)
    hexc = lambda c: "#%02x%02x%02x" % c
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}">
  <defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="{hexc(BG_TOP)}"/><stop offset="1" stop-color="{hexc(BG_BOT)}"/>
  </linearGradient></defs>
  <rect width="{S}" height="{S}" rx="230" fill="url(#g)"/>
  <path d="M {x0:.1f} {y0:.1f} A {R} {R} 0 1 1 {x1:.1f} {y1:.1f}" fill="none" stroke="#fff" stroke-width="{W}" stroke-linecap="round"/>
  <polyline points="{pts}" fill="none" stroke="{hexc(MINT)}" stroke-width="{CW}" stroke-linecap="round" stroke-linejoin="round"/>
</svg>
'''


os.makedirs(OUT, exist_ok=True)
big = render()
big.save(os.path.join(OUT, "certo.ico"), sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
big.save(os.path.join(OUT, "certo-icon.png"))
open(os.path.join(OUT, "certo-icon.svg"), "w").write(svg())
print("wrote", os.path.normpath(OUT))
