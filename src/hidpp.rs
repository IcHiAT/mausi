//! Logitech HID++ 2.0 - nur das, was für DPI und Abfragerate nötig ist.
//!
//! Lange Meldung (Report-ID 0x11, 20 Byte): [0x11, Gerät, Feature-Index, Funktion<<4 | SW-ID, Parameter...]
//! Feature-Nummern: 0x0005 Name, 0x2201/0x2202 DPI, 0x8060/0x8061 Abfragerate.

use crate::g400s;
use hidapi::{HidApi, HidDevice};
use std::cell::Cell;
use std::ffi::{CStr, CString};
use std::rc::Rc;
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
    _api: Rc<HidApi>,
    /// HID++-Schnittstelle (bei der G400s leer)
    dev: Option<HidDevice>,
    /// Zugang zur G400s (bei HID++-Mäusen leer)
    g4: Option<g400s::Zugang>,
    /// Hersteller- und Produktnummer der HID-Schnittstelle (beim Funkempfänger die des Empfängers)
    pub vid: u16,
    pub pid: u16,
    pfad: CString,
    /// 0xFF = Kabel, 1-6 = Funkempfänger, 0 = G400s (kein HID++)
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

/// Eine Maus, die Mausi nicht einstellen kann (anderer Hersteller oder älteres Protokoll).
pub struct Fremd {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
}

/// Fundort einer Maus: HID-Schnittstelle und Platz (0xFF = Kabel, 1-6 = Funkempfänger, 0 = G400s).
pub struct Platz {
    pub pfad: CString,
    pub idx: u8,
}

/// Platz-Nummer für die G400s (HID++ benutzt 0 nie)
const G400S: u8 = 0;

fn hidpp_schnittstelle(i: &hidapi::DeviceInfo) -> bool {
    i.vendor_id() == 0x046D && i.usage_page() == 0xFF00 && i.usage() == 0x0002
}

/// Alle Plätze abfragen, an denen eine einstellbare Maus antwortet. Dauert 2–4 s (schlafende Funkmaus),
/// deshalb läuft das im Tray in einem eigenen Thread; danach verbindet `verbinden` schnell.
pub fn suchen() -> Vec<Platz> {
    suchen_mit_luecke().0
}

/// Wie `suchen`, dazu: gab es einen Empfänger oder Anschluss, an dem keine Maus geantwortet hat?
/// (z. B. Funkmaus schläft tief oder ist aus) – dann lohnt es sich, später noch einmal zu suchen.
pub fn suchen_mit_luecke() -> (Vec<Platz>, bool) {
    let Ok(api) = HidApi::new() else { return (Vec::new(), false) };
    let api = Rc::new(api);
    let infos: Vec<_> = api.device_list().filter(|i| hidpp_schnittstelle(i)).cloned().collect();
    let mut gefunden: Vec<Platz> = Vec::new();
    let mut luecke = false;
    for info in &infos {
        let Some(m) = Maus::oeffnen(&api, info, 0xFF) else { continue };
        // Ein Empfänger kann mehrere Mäuse haben: alle Plätze zugleich anpingen.
        let vorher = gefunden.len();
        for idx in m.wer_antwortet() {
            let Some(mut m) = Maus::oeffnen(&api, info, idx) else { continue };
            if m.einrichten(3) {
                gefunden.push(Platz { pfad: info.path().to_owned(), idx });
            }
        }
        luecke |= gefunden.len() == vorher;
    }
    let mut g4: Vec<CString> = api.device_list().filter(|i| g400s::schnittstelle(i)).map(|i| i.path().to_owned()).collect();
    #[cfg(windows)]
    g4.extend(crate::winusb::pfade().into_iter().filter_map(|p| CString::new(p).ok()));
    for pfad in g4 {
        if Maus::g400s_oeffnen(&api, &pfad).is_some() {
            gefunden.push(Platz { pfad, idx: G400S });
        }
    }
    (gefunden, luecke)
}

/// Mit den gefundenen Plätzen verbinden.
pub fn verbinden(plaetze: &[Platz]) -> Vec<Maus> {
    let Ok(api) = HidApi::new() else { return Vec::new() };
    let api = Rc::new(api);
    let mut v = Vec::new();
    for p in plaetze {
        if p.idx == G400S {
            v.extend(Maus::g400s_oeffnen(&api, &p.pfad));
            continue;
        }
        let Some(info) = api.device_list().find(|i| i.path() == p.pfad.as_c_str()).cloned() else { continue };
        let Some(mut m) = Maus::oeffnen(&api, &info, p.idx) else { continue };
        if m.einrichten(3) {
            v.push(m);
        }
    }
    v
}

/// Mäuse, die Mausi nicht einstellen kann: Maus-Schnittstellen von Geräten ohne HID++.
pub fn fremde() -> Vec<Fremd> {
    let Ok(api) = HidApi::new() else { return Vec::new() };
    let hidpp_ids: Vec<(u16, u16)> =
        api.device_list().filter(|i| hidpp_schnittstelle(i)).map(|i| (i.vendor_id(), i.product_id())).collect();
    let mut fremde: Vec<Fremd> = Vec::new();
    for i in api.device_list().filter(|i| i.usage_page() == 1 && i.usage() == 2) {
        let id = (i.vendor_id(), i.product_id());
        if hidpp_ids.contains(&id) || id == (g400s::VID, g400s::PID) || fremde.iter().any(|f| (f.vid, f.pid) == id) {
            continue;
        }
        let name = i.product_string().unwrap_or("").trim();
        // Tastaturen melden für Makrotasten oft auch eine Maus-Schnittstelle
        let klein = format!(" {} ", name.to_lowercase());
        let tastatur = api.device_list().any(|t| (t.vendor_id(), t.product_id()) == id && t.usage_page() == 1 && t.usage() == 6);
        let maus_im_namen = klein.contains("mouse") || klein.contains("maus");
        if name.is_empty() || klein.contains("keyboard") || klein.contains(" kb ") || (tastatur && !maus_im_namen) {
            continue;
        }
        fremde.push(Fremd { name: name.to_string(), vid: id.0, pid: id.1 });
    }
    fremde
}

/// Alle Mäuse: die einstellbaren (HID++ 2.0 und G400s) und die übrigen.
pub fn alle() -> (Vec<Maus>, Vec<Fremd>) {
    (verbinden(&suchen()), fremde())
}

impl Maus {
    /// Erste Logitech-Maus suchen, die DPI oder Abfragerate per HID++ kann.
    pub fn finden() -> Result<Maus, String> {
        // MAUS_NR=2 wählt die zweite gefundene Maus (zum Testen)
        let nr = std::env::var("MAUS_NR").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).max(1);
        alle().0.into_iter().nth(nr - 1).ok_or_else(|| "keine Logitech-Maus mit HID++ gefunden (eingeschaltet?)".into())
    }

    fn oeffnen(api: &Rc<HidApi>, info: &hidapi::DeviceInfo, idx: u8) -> Option<Maus> {
        let dev = info.open_device(api).ok()?;
        let _ = dev.set_blocking_mode(false);
        Some(Maus {
            _api: api.clone(),
            dev: Some(dev),
            g4: None,
            vid: info.vendor_id(),
            pid: info.product_id(),
            pfad: info.path().to_owned(),
            idx,
            swid: Cell::new(1),
            name: String::new(),
            dpi_f: None,
            rate_f: None,
            akku_f: None,
        })
    }

    /// G400s über ihre HID-Schnittstelle oder, falls eingerichtet, über WinUSB öffnen
    fn g400s_oeffnen(api: &Rc<HidApi>, pfad: &CStr) -> Option<Maus> {
        let zugang = match api.device_list().find(|i| i.path() == pfad) {
            Some(info) => g400s::Zugang::Hid(info.open_device(api).ok()?),
            #[cfg(windows)]
            None => g400s::Zugang::Usb(crate::winusb::Usb::oeffnen(pfad.to_str().ok()?)?),
            #[cfg(not(windows))]
            None => return None,
        };
        if !zugang.da() {
            return None;
        }
        Some(Maus {
            _api: api.clone(),
            dev: None,
            g4: Some(zugang),
            vid: g400s::VID,
            pid: g400s::PID,
            pfad: pfad.to_owned(),
            idx: G400S,
            swid: Cell::new(1),
            name: g400s::NAME.into(),
            dpi_f: None,
            rate_f: None,
            akku_f: None,
        })
    }

    /// HID++-Schnittstelle (nur bei HID++-Mäusen aufgerufen)
    fn hid(&self) -> &HidDevice {
        self.dev.as_ref().expect("HID++-Schnittstelle")
    }

    /// Zugang zur G400s (nur bei der G400s aufgerufen)
    fn g4(&self) -> &g400s::Zugang {
        self.g4.as_ref().expect("G400s-Zugang")
    }

    /// Gleiche Maus? (gleiche Schnittstelle, gleicher Platz)
    pub fn gleich(&self, p: &Platz) -> bool {
        self.pfad == p.pfad && self.idx == p.idx
    }

    fn einrichten(&mut self, versuche: usize) -> bool {
        if !self.anpingen(versuche) {
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

    /// Welche Plätze dieser Schnittstelle antworten? Pingt kabelgebunden (0xFF) und Empfänger-Plätze 1–6
    /// gleichzeitig an und sammelt die Antworten, bis zu 2 s (schlafende Funkmaus).
    fn wer_antwortet(&self) -> Vec<u8> {
        const PLAETZE: [u8; 7] = [0xFF, 1, 2, 3, 4, 5, 6];
        let mut buf = [0u8; 20];
        while self.hid().read(&mut buf).unwrap_or(0) > 0 {}
        let mut da: Vec<u8> = Vec::new();
        let pingen = |da: &Vec<u8>| {
            for idx in PLAETZE.iter().filter(|i| !da.contains(i)) {
                let mut msg = [0u8; 20];
                msg[..4].copy_from_slice(&[0x11, *idx, 0, 0x1F]);
                let _ = self.hid().write(&msg);
            }
        };
        let start = Instant::now();
        pingen(&da);
        let mut nochmal = true;
        while start.elapsed() < Duration::from_millis(2000) {
            if nochmal && start.elapsed() > Duration::from_millis(300) {
                nochmal = false; // Funk verliert gelegentlich eine Meldung
                pingen(&da);
            }
            match self.hid().read(&mut buf) {
                Ok(n) if n >= 7 && buf[0] == 0x11 && buf[2] == 0 && buf[3] == 0x1F && PLAETZE.contains(&buf[1]) => {
                    if !da.contains(&buf[1]) {
                        da.push(buf[1]);
                    }
                    if buf[1] == 0xFF {
                        break; // kabelgebunden: keine Empfänger-Plätze
                    }
                }
                Ok(0) => std::thread::sleep(Duration::from_millis(1)),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        if std::env::var_os("MAUS_DEBUG").is_some() {
            eprintln!("antworten: {da:02x?} nach {} ms", start.elapsed().as_millis());
        }
        da
    }

    /// Ist an diesem Platz eine Maus? Eine Funkmaus im Schlaf antwortet erst nach ~1 s,
    /// deshalb zählt auch eine verspätete Antwort auf einen früheren Ping.
    fn anpingen(&self, versuche: usize) -> bool {
        let mut buf = [0u8; 20];
        while self.hid().read(&mut buf).unwrap_or(0) > 0 {} // alte Meldungen verwerfen
        let antwort = |buf: &[u8; 20], n: usize| n >= 7 && buf[0] == 0x11 && buf[1] == self.idx && buf[2] == 0 && buf[3] >> 4 == 1;
        let dbg = std::env::var_os("MAUS_DEBUG").is_some();
        for i in 0..versuche {
            let sw = self.swid.get() % 15 + 1;
            self.swid.set(sw);
            let mut msg = [0u8; 20];
            msg[..4].copy_from_slice(&[0x11, self.idx, 0, 0x10 | sw]);
            let w = self.hid().write(&msg);
            if dbg { eprintln!("ping {:02x} {:?}", self.idx, w); }
            if w.is_err() {
                return false;
            }
            // nach dem letzten Ping länger warten, damit eine aufwachende Maus noch antworten kann
            let ms = if i + 1 == versuche { 1500 } else { 250 };
            let ende = Instant::now() + Duration::from_millis(ms);
            while Instant::now() < ende {
                match self.hid().read(&mut buf) {
                    Ok(n) if antwort(&buf, n) => return true,
                    Ok(0) => std::thread::sleep(Duration::from_millis(1)),
                    Ok(_) => {}
                    Err(_) => return false,
                }
            }
        }
        false
    }

    fn req_einmal(&self, f: u8, func: u8, p: &[u8], ms: u64) -> Result<Option<[u8; 16]>, String> {
        let mut buf = [0u8; 20];
        for _ in 0..64 {
            // alte Meldungen verwerfen
            if self.hid().read(&mut buf).unwrap_or(0) == 0 {
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
        let w = self.hid().write(&msg);
        if dbg { eprintln!("-> {:02x?} {:?}", &msg[..8], w); }
        w.map_err(|e| format!("Senden fehlgeschlagen: {e}"))?;
        let ende = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < ende {
            let n = self.hid().read(&mut buf);
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

    /// Kann Mausi die DPI dieser Maus ändern? (G400s nur mit WinUSB, sonst nur anzeigen.)
    pub fn dpi_einstellbar(&self) -> bool {
        self.idx != G400S || self.g4().dpi_einstellbar()
    }

    /// Ist das eine G400s? (für „DPI einstellbar machen …“)
    pub fn ist_g400s(&self) -> bool {
        self.idx == G400S
    }

    /// Feste DPI-Stufen (G400s); None = beliebige Werte im Bereich
    pub fn dpi_stufen(&self) -> Option<Vec<u32>> {
        (self.idx == G400S).then(g400s::dpi_stufen)
    }

    pub fn dpi(&self) -> Result<u32, String> {
        if self.idx == G400S {
            return self.g4().dpi();
        }
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

    /// DPI mit einem einzigen kurzen Versuch (für das offene Menü). None = keine Antwort in der Zeit.
    pub fn dpi_schnell(&self) -> Option<u32> {
        if self.idx == G400S {
            return self.g4().dpi().ok();
        }
        let (f, func) = match self.dpi_f? {
            Dpi::Ext(f) => (f, 5),
            Dpi::Basis(f) => (f, 2),
        };
        let r = self.req_einmal(f, func, &[0], 60).ok()??;
        Some(be(r[1], r[2]))
    }

    /// (kleinster, größter) DPI-Wert des Sensors
    pub fn dpi_bereich(&self) -> Result<(u32, u32), String> {
        if self.idx == G400S {
            let s = g400s::dpi_stufen();
            return Ok((s[0], s[s.len() - 1]));
        }
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
        if self.idx == G400S {
            return self.g4().set_dpi(v);
        }
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

    /// Führt `aktion` aus. Lehnt die Maus mit Fehler 5 oder 2 ab, steht sie im Onboard-Modus
    /// (Werte kommen aus dem gespeicherten Profil). Dann auf Host-Modus (0x8100, Modus 2)
    /// umschalten und noch einmal versuchen. Die Werte sind vorher geprüft, Fehler 2 heißt
    /// hier also nicht „Wert ungültig“.
    fn mit_host_modus(&self, aktion: impl Fn() -> Result<(), String>) -> Result<(), String> {
        match aktion() {
            Err(e) if e.ends_with("Fehler 5") || e.ends_with("Fehler 2") => {
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
        if self.idx == G400S {
            return Err("kabelgebunden, kein Akku".into());
        }
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
        if self.idx == G400S {
            return Ok(g400s::raten());
        }
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
        if self.idx == G400S {
            return self.g4().hz();
        }
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
        if self.idx == G400S {
            return self.g4().set_hz(hz);
        }
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
