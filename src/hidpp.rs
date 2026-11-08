//! Logitech HID++ 2.0 - nur das, was für DPI und Abfragerate nötig ist.
//!
//! Lange Meldung (Report-ID 0x11, 20 Byte): [0x11, Gerät, Feature-Index, Funktion<<4 | SW-ID, Parameter...]
//! Feature-Nummern: 0x0005 Name, 0x2201/0x2202 DPI, 0x8060/0x8061 Abfragerate.

use hidapi::{HidApi, HidDevice};
use std::cell::Cell;
use std::time::{Duration, Instant};

/// Index -> Hz für Feature 0x8061 (8 ms ... 125 µs)
pub const RATEN_EXT: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];

#[derive(Clone, Copy)]
enum Dpi {
    Ext(u8),   // 0x2202
    Basis(u8), // 0x2201
}

#[derive(Clone, Copy)]
enum Akku {
    Unified(u8), // 0x1004
    Alt(u8),     // 0x1000
}

#[derive(Clone, Copy)]
enum Rate {
    Ext(u8),   // 0x8061
    Basis(u8), // 0x8060
}

pub struct Maus {
    _api: HidApi,
    dev: HidDevice,
    idx: u8,
    swid: Cell<u8>,
    pub name: String,
    dpi_f: Option<Dpi>,
    rate_f: Option<Rate>,
    akku_f: Option<Akku>,
}

fn be(a: u8, b: u8) -> u32 {
    ((a as u32) << 8) | b as u32
}

/// DPI-Liste: Werte, dazwischen Schrittweiten (obere 3 Bit gesetzt), Ende bei 0.
fn dpi_liste(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(2)
        .map(|p| be(p[0], p[1]))
        .take_while(|&v| v != 0)
        .filter(|&v| v >> 13 != 7)
        .collect()
}

/// Alle Logitech-HID-Schnittstellen (zur Fehlersuche)
pub fn liste() -> Result<Vec<String>, String> {
    let api = HidApi::new().map_err(|e| format!("HID nicht verfügbar: {e}"))?;
    Ok(api
        .device_list()
        .filter(|i| i.vendor_id() == 0x046D)
        .map(|i| {
            format!(
                "{:04X}:{:04X} usage {:04X}/{:04X} {}",
                i.vendor_id(),
                i.product_id(),
                i.usage_page(),
                i.usage(),
                i.product_string().unwrap_or("")
            )
        })
        .collect())
}

impl Maus {
    /// Erste Logitech-Maus suchen, die DPI oder Abfragerate per HID++ kann.
    pub fn finden() -> Result<Maus, String> {
        let api = HidApi::new().map_err(|e| format!("HID nicht verfügbar: {e}"))?;
        let infos: Vec<_> = api
            .device_list()
            .filter(|i| i.vendor_id() == 0x046D && i.usage_page() == 0xFF00 && i.usage() == 0x0002)
            .cloned()
            .collect();
        let mut api = Some(api);
        for info in infos {
            let Ok(dev) = info.open_device(api.as_ref().unwrap()) else { continue };
            let _ = dev.set_blocking_mode(false);
            let mut m = Maus {
                _api: api.take().unwrap(),
                dev,
                idx: 0,
                swid: Cell::new(1),
                name: String::new(),
                dpi_f: None,
                rate_f: None,
                akku_f: None,
            };
            // 1-6 = Platz am Funkempfänger, 0xFF = per Kabel.
            // Zweite Runde mit mehr Geduld: eine schlafende Maus antwortet erst verzögert.
            for versuche in [2, 6] {
                for idx in [1u8, 2, 3, 4, 5, 6, 0xFF] {
                    m.idx = idx;
                    if m.einrichten(versuche) {
                        return Ok(m);
                    }
                }
            }
            api = Some(m._api);
        }
        Err("keine Logitech-Maus mit HID++ gefunden (eingeschaltet?)".into())
    }

    fn einrichten(&mut self, versuche: usize) -> bool {
        let da = (0..versuche).any(|_| matches!(self.req_einmal(0, 1, &[], 250), Ok(Some(_))));
        if !da {
            return false;
        }
        let f = |id| self.feature(id).ok().flatten();
        let dpi = f(0x2202).map(Dpi::Ext).or_else(|| f(0x2201).map(Dpi::Basis));
        let rate = f(0x8061).map(Rate::Ext).or_else(|| f(0x8060).map(Rate::Basis));
        if dpi.is_none() && rate.is_none() {
            return false;
        }
        let akku = f(0x1004).map(Akku::Unified).or_else(|| f(0x1000).map(Akku::Alt));
        self.akku_f = akku;
        self.dpi_f = dpi;
        self.rate_f = rate;
        self.name = self.name_lesen().unwrap_or_else(|_| "Logitech-Maus".into());
        true
    }

    fn req_einmal(&self, f: u8, func: u8, p: &[u8], ms: u64) -> Result<Option<[u8; 16]>, String> {
        let mut buf = [0u8; 20];
        for _ in 0..64 {
            // alte Meldungen verwerfen
            if self.dev.read(&mut buf).unwrap_or(0) == 0 {
                break;
            }
        }
        let sw = self.swid.get() % 15 + 1; // wechselnde SW-ID: alte Antworten passen nie
        self.swid.set(sw);
        let fs = (func << 4) | sw;
        let mut msg = [0u8; 20];
        msg[..4].copy_from_slice(&[0x11, self.idx, f, fs]);
        msg[4..4 + p.len()].copy_from_slice(p);
        let dbg = std::env::var_os("MAUS_DEBUG").is_some();
        let w = self.dev.write(&msg);
        if dbg { eprintln!("-> {:02x?} {:?}", &msg[..8], w); }
        w.map_err(|e| format!("Senden fehlgeschlagen: {e}"))?;
        let ende = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < ende {
            let n = self.dev.read(&mut buf);
            if matches!(n, Ok(0)) {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            if dbg { if let Ok(k) = n { if k > 0 { eprintln!("<- {:02x?}", &buf[..k.min(10)]); } } else { eprintln!("<- {:?}", n); } }
            let n = n.map_err(|e| format!("Lesen fehlgeschlagen: {e}"))?;
            if n < 7 || buf[1] != self.idx {
                continue;
            }
            if buf[2] == 0xFF && buf[3] == f && buf[4] == fs {
                return Err(format!("Maus meldet Fehler {}", buf[5]));
            }
            if buf[2] == f && buf[3] == fs {
                let mut r = [0u8; 16];
                r.copy_from_slice(&buf[4..20]);
                return Ok(Some(r));
            }
        }
        Ok(None)
    }

    /// Anfrage mit Wiederholung (Funk verliert gelegentlich eine Meldung).
    fn req(&self, f: u8, func: u8, p: &[u8]) -> Result<[u8; 16], String> {
        for _ in 0..4 {
            if let Some(r) = self.req_einmal(f, func, p, 400)? {
                return Ok(r);
            }
        }
        Err("Maus antwortet nicht".into())
    }

    fn feature(&self, id: u16) -> Result<Option<u8>, String> {
        let r = self.req(0, 0, &[(id >> 8) as u8, id as u8])?;
        Ok((r[0] != 0).then_some(r[0]))
    }

    fn name_lesen(&self) -> Result<String, String> {
        let f = self.feature(0x0005)?.ok_or("kein Name")?;
        let n = self.req(f, 0, &[])?[0] as usize;
        let mut name = Vec::new();
        while name.len() < n {
            let r = self.req(f, 1, &[name.len() as u8])?;
            let rest = (n - name.len()).min(16);
            name.extend_from_slice(&r[..rest]);
        }
        Ok(String::from_utf8_lossy(&name).trim_end_matches('\0').to_string())
    }

    // ---------- DPI ----------

    pub fn dpi(&self) -> Result<u32, String> {
        match self.dpi_f.ok_or("DPI nicht unterstützt")? {
            Dpi::Ext(f) => {
                let r = self.req(f, 5, &[0])?;
                Ok(be(r[1], r[2]))
            }
            Dpi::Basis(f) => {
                let r = self.req(f, 2, &[0])?;
                Ok(be(r[1], r[2]))
            }
        }
    }

    /// (kleinster, größter) DPI-Wert des Sensors
    pub fn dpi_bereich(&self) -> Result<(u32, u32), String> {
        let werte = match self.dpi_f.ok_or("DPI nicht unterstützt")? {
            Dpi::Ext(f) => {
                // Liste ist ein Bytestrom über mehrere Seiten (je 13 Byte)
                let mut strom = Vec::new();
                for seite in 0..8u8 {
                    match self.req(f, 2, &[0, 0, seite]) {
                        Ok(r) => strom.extend_from_slice(&r[3..16]),
                        Err(_) if seite > 0 => break,
                        Err(e) => return Err(e),
                    }
                    if strom.chunks_exact(2).any(|p| p == [0, 0]) {
                        break;
                    }
                }
                dpi_liste(&strom)
            }
            Dpi::Basis(f) => dpi_liste(&self.req(f, 1, &[0])?[1..15]),
        };
        match (werte.first(), werte.last()) {
            (Some(&a), Some(&b)) => Ok((a, b)),
            _ => Err("DPI-Bereich unbekannt".into()),
        }
    }

    pub fn set_dpi(&self, v: u32) -> Result<(), String> {
        let (min, max) = self.dpi_bereich()?;
        if v < min || v > max {
            return Err(format!("{v} DPI geht nicht, erlaubt sind {min}-{max}"));
        }
        let (h, l) = ((v >> 8) as u8, v as u8);
        let setzen = || -> Result<(), String> {
            match self.dpi_f.ok_or("DPI nicht unterstützt")? {
                Dpi::Ext(f) => {
                    let lod = self.req(f, 5, &[0])?[9];
                    self.req(f, 6, &[0, h, l, h, l, lod])?;
                }
                Dpi::Basis(f) => {
                    self.req(f, 3, &[0, h, l])?;
                }
            }
            Ok(())
        };
        self.mit_host_modus(setzen)
    }

    /// Führt `aktion` aus. Lehnt die Maus mit Fehler 5 ab, steht sie im Onboard-Modus
    /// (Werte kommen aus dem gespeicherten Profil). Dann auf Host-Modus (0x8100, Modus 2)
    /// umschalten und noch einmal versuchen.
    fn mit_host_modus(&self, aktion: impl Fn() -> Result<(), String>) -> Result<(), String> {
        match aktion() {
            Err(e) if e.ends_with("Fehler 5") => {
                let f = self.feature(0x8100)?.ok_or(e)?;
                self.req(f, 1, &[2])?; // setOnboardMode(host)
                aktion()
            }
            r => r,
        }
    }

    // ---------- Akku ----------

    /// (Ladestand in %, lädt gerade)
    pub fn akku(&self) -> Result<(u8, bool), String> {
        match self.akku_f.ok_or("Akkustand nicht unterstützt")? {
            Akku::Unified(f) => {
                let r = self.req(f, 1, &[])?;
                Ok((r[0], matches!(r[2], 1 | 2)))
            }
            Akku::Alt(f) => {
                let r = self.req(f, 0, &[])?;
                Ok((r[0], matches!(r[2], 1 | 2)))
            }
        }
    }

    // ---------- Abfragerate ----------

    /// Unterstützte Raten in Hz, aufsteigend
    pub fn raten(&self) -> Result<Vec<u32>, String> {
        match self.rate_f.ok_or("Abfragerate nicht unterstützt")? {
            Rate::Ext(f) => {
                let r = self.req(f, 1, &[])?;
                let m = be(r[0], r[1]);
                Ok((0..7).filter(|i| m & (1 << i) != 0).map(|i| RATEN_EXT[i]).collect())
            }
            Rate::Basis(f) => {
                let m = self.req(f, 0, &[])?[0] as u32;
                Ok((0..8u32).rev().filter(|i| m & (1 << i) != 0).map(|i| 1000 / (i + 1)).collect())
            }
        }
    }

    /// Wert, den die Maus selbst meldet. Bei 0x8061 (z. B. PRO X Superlight 2) bleibt er
    /// nach dem Setzen auf dem alten Stand stehen, obwohl die echte Rate wechselt.
    pub fn hz(&self) -> Result<u32, String> {
        match self.rate_f.ok_or("Abfragerate nicht unterstützt")? {
            Rate::Ext(f) => {
                let i = self.req(f, 2, &[])?[0] as usize;
                RATEN_EXT.get(i).copied().ok_or_else(|| format!("unbekannte Rate {i}"))
            }
            Rate::Basis(f) => Ok(1000 / (self.req(f, 1, &[])?[0].max(1) as u32)),
        }
    }

    pub fn hz_lesewert_unsicher(&self) -> bool {
        matches!(self.rate_f, Some(Rate::Ext(_)))
    }

    pub fn set_hz(&self, hz: u32) -> Result<(), String> {
        let raten = self.raten()?;
        if !raten.contains(&hz) {
            let l: Vec<String> = raten.iter().map(|r| r.to_string()).collect();
            return Err(format!("{hz} Hz geht nicht, möglich: {}", l.join(", ")));
        }
        self.mit_host_modus(|| {
            match self.rate_f.ok_or("Abfragerate nicht unterstützt")? {
                Rate::Ext(f) => {
                    let i = RATEN_EXT.iter().position(|&r| r == hz).unwrap() as u8;
                    self.req(f, 3, &[i])?;
                }
                Rate::Basis(f) => {
                    self.req(f, 2, &[(1000 / hz) as u8])?;
                }
            }
            Ok(())
        })
    }
}
