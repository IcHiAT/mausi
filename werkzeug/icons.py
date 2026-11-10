# Rendert assets/icons/*.svg in alle Icon-Groessen und baut daraus .ico-Dateien.
# Braucht Chrome (headless) zum Rendern und Pillow.
# Aufruf: python werkzeug/icons.py
import base64, io, struct, subprocess, sys, tempfile
from pathlib import Path
from PIL import Image

CHROME = r"C:\Program Files\Google\Chrome\Application\chrome.exe"
ORDNER = Path(__file__).resolve().parent.parent / "assets" / "icons"
TRAY = [16, 20, 24, 32, 40, 48, 64]
EXE = [16, 20, 24, 32, 40, 48, 64, 128, 256]


def rendern(svgs, groessen):
    """Alle SVGs in allen Groessen auf ein Blatt setzen, einmal fotografieren, ausschneiden."""
    spalte = max(groessen) + 4
    zeilen = []
    for z, svg in enumerate(svgs):
        daten = base64.b64encode(svg.read_bytes()).decode()
        x = 0
        for g in groessen:
            zeilen.append(f'<img src="data:image/svg+xml;base64,{daten}" style="position:absolute;left:{x}px;top:{z * spalte}px;width:{g}px;height:{g}px">')
            x += g + 4
    breite, hoehe = sum(g + 4 for g in groessen), spalte * len(svgs)
    with tempfile.TemporaryDirectory() as t:
        seite, bild = Path(t) / "blatt.html", Path(t) / "blatt.png"
        seite.write_text(f"<html><body style='margin:0;background:transparent'>{''.join(zeilen)}</body></html>", encoding="utf-8")
        subprocess.run([CHROME, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1",
                        "--default-background-color=00000000", f"--window-size={breite},{hoehe}",
                        f"--screenshot={bild}", seite.as_uri()], check=True, capture_output=True)
        blatt = Image.open(bild).convert("RGBA")
        blatt.load()
    ergebnis = {}
    for z, svg in enumerate(svgs):
        x = 0
        for g in groessen:
            ergebnis[(svg.stem, g)] = blatt.crop((x, z * spalte, x + g, z * spalte + g))
            x += g + 4
    return ergebnis


def ico_schreiben(ziel, bilder):
    """ICO mit PNG-Eintraegen (ab Vista erlaubt, klein)."""
    daten = []
    for b in bilder:
        p = io.BytesIO()
        b.save(p, "PNG", optimize=True)
        daten.append(p.getvalue())
    kopf = struct.pack("<HHH", 0, 1, len(bilder))
    offset = 6 + 16 * len(bilder)
    eintraege = b""
    for b, d in zip(bilder, daten):
        w = 0 if b.width >= 256 else b.width
        eintraege += struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(d), offset)
        offset += len(d)
    ziel.write_bytes(kopf + eintraege + b"".join(daten))


def main():
    svgs = sorted(ORDNER.glob("*.svg"))
    bilder = rendern(svgs, EXE)
    for svg in svgs:
        ico_schreiben(ORDNER / f"{svg.stem}.ico", [bilder[(svg.stem, g)] for g in TRAY])
        bilder[(svg.stem, 64)].save(ORDNER / f"{svg.stem}.png")
    # Programm-Icon (exe, Startmenue, Fenster): Variante 4 (hellgrau). Die Tray-Varianten bindet tray.rs ein.
    ico_schreiben(ORDNER.parent / "mausi.ico", [bilder[("4_minimal", g)] for g in EXE])
    for f in sorted(ORDNER.parent.rglob("*.ico")):
        print(f.relative_to(ORDNER.parent), f.stat().st_size, "Bytes")


if __name__ == "__main__":
    sys.exit(main())
