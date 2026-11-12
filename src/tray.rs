//! Tray-Symbol mit Menü: Hz-Messung, DPI-Stufen + eigener Wert, Hz, Autostart.
//! Mehrere Mäuse: jede bekommt ein eigenes Untermenü mit ihren Einstellungen.

use crate::design;
use crate::hidpp::{self, Fremd, Maus, Platz};
use crate::startmenue;
use crate::zustand;
use std::cell::{Cell, RefCell};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::Instant;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::{CreateFontIndirectW, InvalidateRect, HFONT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::Input::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 1;
const WM_EINBLENDEN: u32 = WM_APP + 2;
const WM_GEFUNDEN: u32 = WM_APP + 3;
/// WinUSB-Einrichtung (Admin-Lauf) ist fertig
const WM_WINUSB: u32 = WM_APP + 4;
// Allgemeine Menüpunkte
const ID_AUTO: usize = 3;
const ID_NEU: usize = 4;
const ID_ENDE: usize = 9;
const ID_DESIGN: usize = 11;
const ID_STARTMENUE: usize = 12;
/// Hz-Messung im Hauptmenü (oben), per Klick an und aus
const ID_MESSEN: usize = 13;
/// Einträge im Hauptmenü vor den Mäusen: Messzeile und Trennstrich
const VOR_MAEUSEN: u32 = 2;
const ID_SYMBOL: usize = 5000;
// Menüpunkte je Maus: GERAET * (Nummer + 1) + lokale ID
const GERAET: usize = 10000;
const ID_DPI: usize = 1000;
const ID_HZ: usize = 2000;
const ID_VDPI: usize = 3000;
const ID_VHZ: usize = 4000;
const ID_DPI_EIGEN: usize = 5;
const ID_VDPI_AUS: usize = 7;
const ID_VDPI_EIGEN: usize = 8;
const ID_VHZ_AUS: usize = 10;
const ID_DPI_INFO: usize = 9;
/// G400s: WinUSB einrichten, damit sich die DPI setzen lassen
const ID_WINUSB: usize = 11;
const MAX_GERAETE: usize = 8;
const ID_EDIT: usize = 101;
const TIMER_LIVE: usize = 1;
const TIMER_AKKU: usize = 2;
const TIMER_VOLL: usize = 3;
const TIMER_GERAETE: usize = 4;
const TIMER_SUCHE: usize = 5;
/// G400s: DPI-Stufe nachsehen, die Maus meldet einen Tastendruck nicht von selbst
const TIMER_G400S: usize = 6;
const AKKU_WARNUNG: u8 = 10;
const EM_SETSEL: u32 = 0x00B1;
const COLOR_BTNFACE: i32 = 15;
const MN_GETHMENU: u32 = 0x01E1;
const KLASSE: &str = "MausTray";
const KLASSE_EINGABE: &str = "MausEingabe";
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const PERSONALISIERUNG: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const STANDARD_DPI: [u32; 6] = [400, 800, 1200, 1600, 3200, 6400];

pub fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Eine einstellbare Maus und was Mausi sich zu ihr merkt.
struct Geraet {
    maus: Maus,
    hz_gesetzt: Option<u32>,
    // Vollbild (Spiele): eigene Werte, None = wie Desktop
    spiel_dpi: Option<u32>,
    spiel_hz: Option<u32>,
    im_spiel: bool,
    desk_dpi: Option<u32>,
    desk_hz: Option<u32>,
    /// Zuletzt gelesene Werte: das Menü öffnet damit sofort, ohne auf die Maus zu warten
    stand: Option<Stand>,
    /// Antwortet die Maus im offenen Menü nicht, ein paar Takte nicht fragen (sonst ruckelt es)
    pause: u8,
}

impl Geraet {
    fn neu(maus: Maus) -> Geraet {
        Geraet {
            hz_gesetzt: zustand::hz_lesen(&maus.name),
            spiel_dpi: zustand::lesen(&schluessel(&maus.name, "dpi")),
            spiel_hz: zustand::lesen(&schluessel(&maus.name, "hz")),
            maus,
            im_spiel: false,
            desk_dpi: None,
            desk_hz: None,
            stand: None,
            pause: 0,
        }
    }
}

struct App {
    geraete: Vec<Geraet>,
    fremde: Vec<Fremd>,
    presets: Vec<u32>,
    suche_laeuft: bool,
    /// Letzte Suche fand an einem Empfänger keine Maus: regelmäßig weitersuchen
    luecke: bool,
}

fn schluessel(name: &str, was: &str) -> String {
    format!("{name}.vollbild_{was}")
}

/// Hersteller und Produkt als eine Zahl, damit Raw-Input-Meldungen der richtigen Maus zugeordnet werden.
fn kennung(vid: u16, pid: u16) -> u32 {
    (vid as u32) << 16 | pid as u32
}

/// Live-Messung einer Maus (nach Hersteller und Produkt)
struct Messung {
    kennung: u32,
    zaehler: u32,
    verlauf: [u32; 4], // letzte 4 Viertelsekunden
}

/// Eine einstellbare Maus im offenen Menü: DPI nachsehen, falls sie an der Maus umgestellt wird.
#[derive(Clone)]
struct LiveDpi {
    k: usize,
    dpi: Option<u32>,
    liste: Vec<u32>,
}

/// Solange das Menü offen ist: DPI nachsehen und, wenn eingeschaltet, Raw-Input-Meldungen je Maus zählen.
struct Live {
    menu: HMENU,
    messen: bool,
    messungen: Vec<Messung>,
    /// Kennung -> Name, für die Anzeige der gerade bewegten Maus
    namen: Vec<(u32, String)>,
    /// zuletzt angezeigter Text der Messzeile: nur bei Änderung neu zeichnen
    text: String,
    start: Instant,
    pos: usize,
    // Raw-Input-Gerät -> Kennung (Gerätename nur einmal nachschlagen)
    zuordnung: Vec<(HANDLE, u32)>,
    dpi: Vec<LiveDpi>,
    takt: u32,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App { geraete: Vec::new(), fremde: Vec::new(), presets: Vec::new(), suche_laeuft: false, luecke: false });
    static EINGABE_SPIEL: Cell<bool> = Cell::new(false);
    static EINGABE_GERAET: Cell<usize> = Cell::new(0);
    static ICON: Cell<HICON> = Cell::new(null_mut());
    static TRAY_ICON: Cell<HICON> = Cell::new(null_mut());
    static TRAY_NR: Cell<usize> = Cell::new(usize::MAX);
    static TASKBAR: Cell<u32> = Cell::new(0);
    static HAUPT: Cell<HWND> = Cell::new(null_mut());
    static EINGABE: Cell<HWND> = Cell::new(null_mut());
    static FONT: Cell<HFONT> = Cell::new(null_mut());
    static LIVE: RefCell<Option<Live>> = RefCell::new(None);
    static GEWARNT: Cell<bool> = Cell::new(false);
    static MENU_OFFEN: Cell<bool> = Cell::new(false);
    /// Hz-Messung eingeschaltet: gilt nur, solange das Menü offen ist
    static MESSEN: Cell<bool> = Cell::new(false);
}

/// Ergebnis der Suche aus dem Hintergrund-Thread
static GEFUNDEN: Mutex<Option<(Vec<Platz>, bool)>> = Mutex::new(None);

// ---------- Mäuse suchen ----------

/// Mäuse im Hintergrund suchen (dauert ohne Maus bis ~14 s); das Ergebnis kommt als WM_GEFUNDEN.
fn suche_starten() {
    let laeuft = APP.with(|c| std::mem::replace(&mut c.borrow_mut().suche_laeuft, true));
    if laeuft {
        return;
    }
    let hwnd = HAUPT.get() as isize;
    std::thread::spawn(move || {
        *GEFUNDEN.lock().unwrap() = Some(hidpp::suchen_mit_luecke());
        unsafe { PostMessageW(hwnd as HWND, WM_GEFUNDEN, 0, 0) };
    });
}

/// Suchergebnis übernehmen. Schon verbundene Mäuse behalten ihren Zustand (z. B. „im Spiel“).
fn suche_uebernehmen() {
    let Some((plaetze, luecke)) = GEFUNDEN.lock().unwrap().take() else { return };
    APP.with(|c| {
        let mut a = c.borrow_mut();
        a.suche_laeuft = false;
        a.luecke = luecke;
        let mut alt = std::mem::take(&mut a.geraete);
        let mut neu = Vec::new();
        let mut zu_verbinden = Vec::new();
        for p in plaetze {
            match alt.iter().position(|g| g.maus.gleich(&p)) {
                Some(i) => neu.push(alt.swap_remove(i)),
                None => zu_verbinden.push(p),
            }
        }
        neu.extend(hidpp::verbinden(&zu_verbinden).into_iter().map(Geraet::neu));
        neu.truncate(MAX_GERAETE);
        a.geraete = neu;
        a.fremde = hidpp::fremde();
    });
}

static WINUSB_ERGEBNIS: std::sync::Mutex<Option<Result<(), String>>> = std::sync::Mutex::new(None);

/// G400s: WinUSB als Administrator einrichten (UAC-Abfrage), danach neu suchen.
fn winusb_starten() {
    // Die offene HID-Verbindung zur G400s schließen, sonst kann Windows den Treiber nicht tauschen
    APP.with(|c| c.borrow_mut().geraete.retain(|g| !g.maus.ist_g400s()));
    let hwnd = HAUPT.get() as isize;
    std::thread::spawn(move || {
        *WINUSB_ERGEBNIS.lock().unwrap() = Some(crate::winusb::als_admin("winusb ein"));
        unsafe { PostMessageW(hwnd as HWND, WM_WINUSB, 0, 0) };
    });
}

// ---------- Stand der Mäuse ----------

#[derive(Clone)]
struct Stand {
    name: String,
    dpi: Option<u32>,
    hz: Option<u32>,
    raten: Vec<u32>,
    dpi_liste: Vec<u32>,
    dpi_bereich: (u32, u32),
    /// DPI nur anzeigen, nicht ändern (G400s ohne WinUSB)
    dpi_fest: bool,
    /// Nur feste DPI-Stufen, kein eigener Wert (G400s)
    dpi_stufen: bool,
    /// G400s: bietet „DPI einstellbar machen …“ an, solange `dpi_fest`
    g400s: bool,
    akku: Option<(u8, bool)>,
}

/// Eine Maus im Menü: Name, Kennung für die Messung und die Werte (None = antwortet nicht).
struct Eintrag {
    name: String,
    kennung: u32,
    stand: Option<Stand>,
}

fn stand_von(g: &Geraet, presets: &[u32]) -> Option<Stand> {
    let m = &g.maus;
    let dpi = m.dpi();
    if matches!(&dpi, Err(e) if e == "Maus antwortet nicht") {
        return None; // aus, schläft oder außer Reichweite: nicht weiter warten
    }
    let dpi = dpi.ok();
    let raten = m.raten().unwrap_or_default();
    if dpi.is_none() && raten.is_empty() {
        return None;
    }
    let dpi_fest = !m.dpi_einstellbar();
    let (min, max) = m.dpi_bereich().unwrap_or((0, 0));
    let stufen = m.dpi_stufen();
    let dpi_stufen = stufen.is_some();
    let mut dpi_liste: Vec<u32> =
        stufen.unwrap_or_else(|| presets.iter().copied().filter(|&v| v >= min && v <= max).collect());
    if let (Some(d), false, false) = (dpi, dpi_fest, dpi_stufen) {
        if !dpi_liste.contains(&d) {
            dpi_liste.push(d);
            dpi_liste.sort();
        }
    }
    let hz = if m.hz_lesewert_unsicher() { g.hz_gesetzt.or_else(|| m.hz().ok()) } else { m.hz().ok() };
    let akku = m.akku().ok();
    Some(Stand { name: m.name.clone(), dpi, hz, raten, dpi_liste, dpi_bereich: (min, max), dpi_fest, dpi_stufen, g400s: m.ist_g400s(), akku })
}

/// Alle Mäuse vollständig abfragen und den Stand merken (kann bei schlafender Maus dauern).
fn auffrischen() {
    APP.with(|c| {
        let mut a = c.borrow_mut();
        let App { geraete, presets, .. } = &mut *a;
        for g in geraete.iter_mut() {
            let s = stand_von(g, presets);
            g.stand = s;
        }
    });
}

fn dpi_eintragen(s: &mut Stand, d: u32) {
    s.dpi = Some(d);
    if !s.dpi_fest && !s.dpi_stufen && !s.dpi_liste.contains(&d) {
        s.dpi_liste.push(d);
        s.dpi_liste.sort();
    }
}

/// Vor dem Öffnen des Menüs: nur die DPI kurz nachlesen (kann an der Maus umgestellt worden sein).
/// Eine Maus ohne gemerkten Stand wird nur dann ganz abgefragt, wenn sie sofort antwortet.
fn schnell_auffrischen() {
    APP.with(|c| {
        let mut a = c.borrow_mut();
        let App { geraete, presets, .. } = &mut *a;
        for g in geraete.iter_mut() {
            let Some(d) = g.maus.dpi_schnell() else { continue };
            if g.stand.is_some() {
                dpi_eintragen(g.stand.as_mut().unwrap(), d);
            } else {
                let s = stand_von(g, presets);
                g.stand = s;
            }
        }
    });
}

/// Gemerkter Stand aller Mäuse (ohne die Mäuse zu fragen).
fn uebersicht() -> Vec<Eintrag> {
    APP.with(|c| {
        c.borrow()
            .geraete
            .iter()
            .map(|g| Eintrag { name: g.maus.name.clone(), kennung: kennung(g.maus.vid, g.maus.pid), stand: g.stand.clone() })
            .collect()
    })
}

fn mit_maus(k: usize, f: impl FnOnce(&Maus) -> Result<(), String>) -> Result<String, String> {
    APP.with(|c| {
        let a = c.borrow();
        let m = &a.geraete.get(k).ok_or("Maus nicht mehr verbunden")?.maus;
        f(m)?;
        Ok(m.name.clone())
    })
}

fn kopf(s: &Stand) -> String {
    let d = s.dpi.map(|d| format!("{d} DPI")).unwrap_or_default();
    let h = s.hz.map(|h| format!("{h} Hz")).unwrap_or_default();
    let a = match s.akku {
        Some((p, true)) => format!(" · Akku {p} % (lädt)"),
        Some((p, false)) => format!(" · Akku {p} %"),
        None => String::new(),
    };
    format!("{} - {d} · {h}{a}", s.name)
}

unsafe fn tray(hwnd: HWND, aktion: u32, tip: &str) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = TRAY_ICON.get();
    for (i, c) in tip.encode_utf16().take(127).enumerate() {
        nid.szTip[i] = c;
    }
    Shell_NotifyIconW(aktion, &nid);
}

/// Nullterminierten UTF-16-Text mit `text` vergleichen.
unsafe fn breit_gleich(p: *const u16, text: &str) -> bool {
    let mut i = 0;
    for c in text.encode_utf16() {
        if *p.add(i) != c {
            return false;
        }
        i += 1;
    }
    *p.add(i) == 0
}

unsafe fn tip_aktualisieren(hwnd: HWND, aktion: u32) -> Vec<Eintrag> {
    auffrischen();
    tip_zeigen(hwnd, aktion)
}

/// Tooltip und Symbol aus dem gemerkten Stand, ohne die Mäuse abzufragen
unsafe fn tip_zeigen(hwnd: HWND, aktion: u32) -> Vec<Eintrag> {
    let liste = uebersicht();
    let inhalt = if liste.is_empty() {
        if APP.with(|c| c.borrow().suche_laeuft) { "suche Maus …".to_string() } else { "keine einstellbare Maus gefunden".to_string() }
    } else {
        let zeilen: Vec<String> =
            liste.iter().map(|e| e.stand.as_ref().map(kopf).unwrap_or_else(|| format!("{}: antwortet nicht", e.name))).collect();
        zeilen.join("\n")
    };
    // Erste Zeile: Programmname und Version (Tooltip ist auf 127 Zeichen begrenzt, Version bleibt vorn)
    let tip = format!("Mausi {}\n{}", env!("CARGO_PKG_VERSION"), inhalt);
    symbol_waehlen(liste.iter().any(|e| e.stand.is_some()));
    tray(hwnd, aktion, &tip);
    for e in &liste {
        if let Some((p, laedt)) = e.stand.as_ref().and_then(|s| s.akku) {
            akku_warnen(&e.name, p, laedt);
        }
    }
    liste
}

/// Mit MAUS_DEBUG=1: G400s-Abfrage nach %LOCALAPPDATA%\Mausi\g400s.txt schreiben
fn g4_protokoll(t: &str) {
    if std::env::var_os("MAUS_DEBUG").is_none() {
        return;
    }
    let Some(d) = std::env::var_os("LOCALAPPDATA") else { return };
    let p = std::path::Path::new(&d).join("Mausi").join("g400s.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        use std::io::Write;
        let _ = writeln!(f, "{t}");
    }
}

/// G400s: Wurde die DPI-Stufe an der Maus umgeschaltet, Stand merken und einblenden.
unsafe fn g400s_nachsehen(hwnd: HWND) {
    let neu = APP.with(|c| {
        let mut a = c.borrow_mut();
        for (k, g) in a.geraete.iter_mut().enumerate() {
            if !g.maus.ist_g400s() {
                continue;
            }
            let gelesen = g.maus.dpi_schnell();
            g4_protokoll(&format!("{:?} k={k} stand={:?} gelesen={gelesen:?}", std::time::SystemTime::now(), g.stand.as_ref().map(|s| s.dpi)));
            let Some(s) = g.stand.as_mut() else { continue };
            match gelesen {
                Some(d) if s.dpi.is_some() && s.dpi != Some(d) => {
                    dpi_eintragen(s, d);
                    return Some(k);
                }
                _ => {}
            }
        }
        None
    });
    // Sofort einblenden; nicht alle Mäuse neu abfragen, eine schlafende Funkmaus hält sonst sekundenlang auf
    if let Some(k) = neu {
        let st = APP.with(|c| c.borrow().geraete.get(k).and_then(|g| g.stand.clone()));
        einblenden(st.as_ref());
        tip_zeigen(hwnd, NIM_MODIFY);
    }
}

/// DPI/Hz am rechten Bildschirmrand einblenden.
unsafe fn einblenden(st: Option<&Stand>) {
    if let Some(s) = st {
        design::osd_zeigen(s.dpi, s.hz, im_spiel());
    }
}

/// Für die Kommandozeile: dem laufenden Tray sagen, dass sich etwas geändert hat.
pub fn einblendung_anstossen() {
    unsafe {
        let h = FindWindowW(w(KLASSE).as_ptr(), null());
        if !h.is_null() {
            PostMessageW(h, WM_EINBLENDEN, 0, 0);
        }
    }
}

/// Einmal pro Entladung warnen, sobald der Akku bei 10 % oder darunter ist.
/// Wieder scharf, wenn die Maus lädt oder über 10 % liegt.
unsafe fn akku_warnen(name: &str, prozent: u8, laedt: bool) {
    // MAUS_WARNUNG=<Prozent> setzt die Schwelle zum Testen hoch
    let schwelle = std::env::var("MAUS_WARNUNG").ok().and_then(|v| v.parse().ok()).unwrap_or(AKKU_WARNUNG);
    if laedt || prozent > schwelle {
        GEWARNT.set(false);
        return;
    }
    if GEWARNT.replace(true) {
        return;
    }
    hinweis_zeigen(&format!("Akku von {name} fast leer\nNoch {prozent} % – bitte bald laden."));
}

// ---------- Vollbild (Spiele) ----------

/// Läuft gerade ein Programm im Vollbild? Exklusives D3D-Vollbild meldet Windows selbst;
/// randloses Vollbild erkennt man daran, dass das Vordergrundfenster den ganzen Monitor bedeckt.
unsafe fn vollbild_erkannt() -> bool {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    let mut q = 0;
    if SHQueryUserNotificationState(&mut q) == 0 && q == QUNS_RUNNING_D3D_FULL_SCREEN {
        return true;
    }
    let fg = GetForegroundWindow();
    if fg.is_null() || fg == GetShellWindow() || fg == GetDesktopWindow() {
        return false;
    }
    let mut buf = [0u16; 32];
    let n = GetClassNameW(fg, buf.as_mut_ptr(), buf.len() as i32) as usize;
    let klasse = String::from_utf16_lossy(&buf[..n]);
    if matches!(klasse.as_str(), "WorkerW" | "Progman" | "Shell_TrayWnd") {
        return false; // Desktop / Taskleiste
    }
    let mut r: RECT = std::mem::zeroed();
    GetWindowRect(fg, &mut r);
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST), &mut mi);
    let m = mi.rcMonitor;
    r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom && ist_spiel(fg)
}

/// Gehört das Fenster zu einem Spiel? Ein Browser im Vollbild soll die Maus nicht umstellen.
unsafe fn ist_spiel(fenster: HWND) -> bool {
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    let mut pid = 0;
    GetWindowThreadProcessId(fenster, &mut pid);
    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if h.is_null() {
        return true; // Von Anti-Cheat geschützte Prozesse lassen sich nicht öffnen: das sind Spiele
    }
    let mut buf = [0u16; 1024];
    let mut n = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut n) != 0;
    CloseHandle(h);
    ok && pfad_ist_spiel(&String::from_utf16_lossy(&buf[..n as usize]), &spiele_laden())
}

/// Spiele-Ordner der üblichen Launcher, dazu eigene Einträge aus maus.ini.
fn pfad_ist_spiel(pfad: &str, eigene: &[String]) -> bool {
    const ORDNER: [&str; 9] = [
        "\\steamapps\\common\\",
        "\\epic games\\",
        "\\riot games\\",
        "\\ea games\\",
        "\\origin games\\",
        "\\gog games\\",
        "\\gog galaxy\\games\\",
        "\\ubisoft game launcher\\games\\",
        "\\xboxgames\\",
    ];
    let p = pfad.to_lowercase();
    ORDNER.iter().any(|o| p.contains(o)) || eigene.iter().any(|e| p.contains(e.as_str()))
}

/// Zeile `spiele = valorant.exe, D:\Spiele` aus maus.ini, kleingeschrieben.
fn spiele_laden() -> Vec<String> {
    let ini = std::env::current_exe().ok().and_then(|p| std::fs::read_to_string(p.with_file_name("maus.ini")).ok());
    ini.and_then(|t| {
        t.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "spiele").then(|| {
                v.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect()
            })
        })
    })
    .unwrap_or_default()
}

/// Wechselt eine Maus zwischen Desktop- und Vollbild-Werten. true = gewechselt.
/// Schlägt das Setzen fehl (Maus schläft), bleibt der Zustand und der nächste Takt versucht es erneut.
fn geraet_wechseln(g: &mut Geraet, an: bool) -> bool {
    if g.im_spiel == an {
        return false;
    }
    let (sd, sh, dd, dh, hz_g) = (g.spiel_dpi, g.spiel_hz, g.desk_dpi, g.desk_hz, g.hz_gesetzt);
    if an && sd.is_none() && sh.is_none() {
        g.im_spiel = true; // nichts umzustellen
        return true;
    }
    let m = &g.maus;
    if an {
        let akt = if sd.is_some() { m.dpi().ok() } else { None };
        let ok_d = sd.map_or(true, |v| akt == Some(v) || m.set_dpi(v).is_ok());
        let ok_h = sh.map_or(true, |v| hz_g == Some(v) || m.set_hz(v).is_ok());
        if !(ok_d && ok_h) {
            return false;
        }
        g.desk_dpi = akt;
        g.desk_hz = hz_g;
        if sh.is_some() {
            g.hz_gesetzt = sh;
        }
    } else {
        let ok_d = sd.is_none() || dd.map_or(true, |v| m.set_dpi(v).is_ok());
        let ok_h = sh.is_none() || dh.map_or(true, |v| m.set_hz(v).is_ok());
        if !(ok_d && ok_h) {
            return false;
        }
        if sh.is_some() {
            g.hz_gesetzt = dh;
        }
    }
    g.im_spiel = an;
    true
}

/// Alle Mäuse umstellen. Ergebnis: Nummern der Mäuse, die gewechselt haben und eigene Vollbild-Werte haben.
fn spiel_wechseln(an: bool) -> Vec<usize> {
    APP.with(|c| {
        let mut a = c.borrow_mut();
        let mut v = Vec::new();
        for (k, g) in a.geraete.iter_mut().enumerate() {
            if geraet_wechseln(g, an) && (g.spiel_dpi.is_some() || g.spiel_hz.is_some()) {
                v.push(k);
            }
        }
        v
    })
}

fn im_spiel() -> bool {
    APP.with(|c| c.borrow().geraete.iter().any(|g| g.im_spiel))
}

/// Vollbild-Werte einer Maus ändern (äußeres None = unverändert, Some(None) = wie Desktop) und speichern.
fn spiel_merken(k: usize, dpi: Option<Option<u32>>, hz: Option<Option<u32>>) {
    APP.with(|c| {
        let mut a = c.borrow_mut();
        let Some(g) = a.geraete.get_mut(k) else { return };
        // Läuft gerade ein Vollbild, erst auf Desktop zurück; der nächste Takt setzt die neuen Werte.
        if g.im_spiel {
            geraet_wechseln(g, false);
        }
        let name = g.maus.name.clone();
        if let Some(d) = dpi {
            g.spiel_dpi = d;
            zustand::merken(&schluessel(&name, "dpi"), d);
        }
        if let Some(h) = hz {
            g.spiel_hz = h;
            zustand::merken(&schluessel(&name, "hz"), h);
        }
    });
}

// ---------- Eigenes Hinweis-Popup ----------
// Windows-Benachrichtigungen können abgeschaltet sein, daher ein eigenes kleines Fenster
// unten rechts. Es nimmt keinen Fokus (stört kein Spiel) und schließt per Klick oder nach 15 s.

const KLASSE_HINWEIS: &str = "MausHinweis";

unsafe fn hinweis_zeigen(text: &str) {
    let hinst = GetModuleHandleW(null());
    let mut wa: RECT = std::mem::zeroed();
    SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut wa as *mut RECT).cast(), 0);
    let (b, h) = (290, 64);
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        w(KLASSE_HINWEIS).as_ptr(),
        w("Mausi").as_ptr(),
        WS_POPUP | WS_BORDER,
        wa.right - b - 12, wa.bottom - h - 12, b, h,
        null_mut(), null_mut(), hinst, null(),
    );
    let s = CreateWindowExW(
        0, w("STATIC").as_ptr(), w(text).as_ptr(), WS_CHILD | WS_VISIBLE | 0x1, // 0x1 = SS_CENTER
        8, 12, b - 18, h - 24, hwnd, null_mut(), hinst, null(),
    );
    SendMessageW(s, WM_SETFONT, schrift() as WPARAM, 1);
    if design::schwarz() {
        design::fenster_schwarz(hwnd);
    }
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    SetTimer(hwnd, 1, 15_000, None);
}

unsafe extern "system" fn hinweis_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_TIMER => {
            DestroyWindow(hwnd);
            0
        }
        WM_ERASEBKGND if design::schwarz() => {
            design::hintergrund_malen(hwnd, wp as _);
            1
        }
        WM_CTLCOLORSTATIC if design::schwarz() => design::kind_farbe(wp as _, false),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn autostart_an() -> bool {
    let mut n = 0u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, w(RUN).as_ptr(), w("Maus").as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut n)
            == ERROR_SUCCESS
    }
}

fn autostart_setzen(an: bool) {
    unsafe {
        if an {
            let Ok(exe) = std::env::current_exe() else { return };
            let v = w(&format!("\"{}\"", exe.display()));
            RegSetKeyValueW(HKEY_CURRENT_USER, w(RUN).as_ptr(), w("Maus").as_ptr(), REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32);
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, w(RUN).as_ptr(), w("Maus").as_ptr());
        }
    }
}

/// Liegt die exe woanders als beim letzten Mal (z. B. Ordner umbenannt), zeigen Autostart und
/// Startmenü-Verknüpfung noch auf den alten Pfad. Beide werden beim Start neu geschrieben, falls aktiv.
fn pfade_nachziehen() {
    if autostart_an() {
        autostart_setzen(true);
    }
    if startmenue::vorhanden("Mausi") {
        startmenue::setzen("Mausi", true);
    }
}

/// DPI-Stufen aus maus.ini neben der exe (Zeile `dpi = 400, 800, 1600`), sonst Standard.
fn presets_laden() -> Vec<u32> {
    let ini = std::env::current_exe().ok().and_then(|p| std::fs::read_to_string(p.with_file_name("maus.ini")).ok());
    let aus_ini = ini.and_then(|t| {
        t.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "dpi").then(|| v.split(',').filter_map(|x| x.trim().parse().ok()).collect::<Vec<u32>>())
        })
    });
    match aus_ini {
        Some(v) if !v.is_empty() => v,
        _ => STANDARD_DPI.to_vec(),
    }
}

unsafe fn melden(hwnd: HWND, text: &str) {
    MessageBoxW(hwnd, w(text).as_ptr(), w("Mausi").as_ptr(), MB_ICONWARNING);
}

// ---------- Live-Messung ----------

const LIVE_LEER: &str = "Gemessen: Maus bewegen …";
const MESSEN_AUS: &str = "Hz messen";

unsafe fn live_starten(hwnd: HWND, menu: HMENU, messen: bool, namen: Vec<(u32, String)>, dpi: Vec<LiveDpi>) {
    if messen {
        let rid = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
        RegisterRawInputDevices(&rid, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32);
    }
    let live = Live {
        menu, messen, messungen: Vec::new(), namen, text: LIVE_LEER.to_string(),
        start: Instant::now(), pos: 0, zuordnung: Vec::new(), dpi, takt: 0,
    };
    LIVE.with(|l| *l.borrow_mut() = Some(live));
    SetTimer(hwnd, TIMER_LIVE, 250, None);
}

unsafe fn live_stoppen(hwnd: HWND) {
    KillTimer(hwnd, TIMER_LIVE);
    let rid = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_REMOVE, hwndTarget: null_mut() };
    RegisterRawInputDevices(&rid, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32);
    LIVE.with(|l| *l.borrow_mut() = None);
}

/// Kennung (Hersteller, Produkt) eines Raw-Input-Geräts aus seinem Namen, z. B.
/// `\\?\HID#VID_046D&PID_C54D&MI_01&Col01#...`. 0 = unbekannt (z. B. Bluetooth oder Fernsteuerung).
unsafe fn raw_kennung(geraet: HANDLE) -> u32 {
    let mut buf = [0u16; 512];
    let mut n = buf.len() as u32;
    let r = GetRawInputDeviceInfoW(geraet, RIDI_DEVICENAME, buf.as_mut_ptr().cast(), &mut n);
    if r == u32::MAX || r == 0 {
        return 0;
    }
    let name = String::from_utf16_lossy(&buf[..r as usize]).to_uppercase();
    let hex = |marke: &str| {
        let i = name.find(marke)? + marke.len();
        u16::from_str_radix(name.get(i..i + 4)?, 16).ok()
    };
    match (hex("VID_"), hex("PID_")) {
        (Some(v), Some(p)) => kennung(v, p),
        _ => 0,
    }
}

/// Eine Raw-Input-Meldung der richtigen Maus zuzählen.
unsafe fn live_zaehlen(lp: LPARAM) {
    let mut kopf: RAWINPUTHEADER = std::mem::zeroed();
    let mut n = std::mem::size_of::<RAWINPUTHEADER>() as u32;
    let r = GetRawInputData(lp as HRAWINPUT, RID_HEADER, (&mut kopf as *mut RAWINPUTHEADER).cast(), &mut n, n);
    if r == u32::MAX {
        return;
    }
    LIVE.with(|l| {
        let mut l = l.borrow_mut();
        let Some(l) = l.as_mut() else { return };
        if !l.messen {
            return;
        }
        let k = match l.zuordnung.iter().find(|(h, _)| *h == kopf.hDevice) {
            Some(&(_, k)) => k,
            None => {
                let k = raw_kennung(kopf.hDevice);
                l.zuordnung.push((kopf.hDevice, k));
                k
            }
        };
        match l.messungen.iter_mut().find(|m| m.kennung == k) {
            Some(m) => m.zaehler += 1,
            None => l.messungen.push(Messung { kennung: k, zaehler: 1, verlauf: [0; 4] }),
        }
    });
}

/// Gemessene Rate auf die übliche Stufe runden, damit die Zeile nicht ständig zwischen 996 und 1003 springt.
fn rate_runden(hz: u32) -> u32 {
    for r in [125, 250, 500, 1000, 2000, 4000, 8000] {
        if hz.abs_diff(r) * 100 <= r * 8 {
            return r;
        }
    }
    (hz + 5) / 10 * 10
}

/// Eintrag (per ID im ganzen Menübaum) finden: (Menü, Position).
unsafe fn eintrag_finden(menu: HMENU, id: u32) -> Option<(HMENU, u32)> {
    for pos in 0..GetMenuItemCount(menu).max(0) as u32 {
        let sub = GetSubMenu(menu, pos as i32);
        if !sub.is_null() {
            if let Some(t) = eintrag_finden(sub, id) {
                return Some(t);
            }
        } else if GetMenuItemID(menu, pos as i32) == id {
            return Some((menu, pos));
        }
    }
    None
}

/// Text eines Eintrags ändern, selbst gezeichnet oder nativ.
unsafe fn eintrag_text(menu: HMENU, pos: u32, text: &str) {
    if !design::menu_text_setzen(menu, pos, true, text) {
        let mut t = w(text);
        let mut mii: MENUITEMINFOW = std::mem::zeroed();
        mii.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
        mii.fMask = MIIM_STRING;
        mii.dwTypeData = t.as_mut_ptr();
        SetMenuItemInfoW(menu, pos, 1, &mii);
    }
}

/// Nur diesen einen Eintrag neu zeichnen, ohne den Hintergrund vorher zu löschen (kein Flackern).
unsafe fn eintrag_malen(menu: HMENU, pos: u32) {
    use windows_sys::Win32::Graphics::Gdi::MapWindowPoints;
    let klasse = w("#32768");
    let mut f = FindWindowExW(null_mut(), null_mut(), klasse.as_ptr(), null());
    while !f.is_null() {
        if SendMessageW(f, MN_GETHMENU, 0, 0) as HMENU == menu {
            let mut r: RECT = std::mem::zeroed();
            if GetMenuItemRect(f, menu, pos, &mut r) != 0 {
                MapWindowPoints(null_mut(), f, (&mut r as *mut RECT).cast(), 2);
                InvalidateRect(f, &r, 0);
            }
        }
        f = FindWindowExW(null_mut(), f, klasse.as_ptr(), null());
    }
}

unsafe fn live_takt() {
    let neu = LIVE.with(|l| {
        let mut l = l.borrow_mut();
        let l = l.as_mut()?;
        let sek = l.start.elapsed().as_secs_f32().max(0.05);
        l.start = Instant::now();
        let pos = l.pos;
        l.pos = (l.pos + 1) % 4;
        l.takt += 1;
        // Angezeigt wird die Maus, die gerade bewegt wird: höchste Rate der letzten Sekunde
        // (Höchstwert, weil die Maus beim Abbremsen weniger sendet)
        let mut beste: Option<(u32, u32)> = None;
        for m in l.messungen.iter_mut() {
            m.verlauf[pos] = (m.zaehler as f32 / sek).round() as u32;
            m.zaehler = 0;
            let best = *m.verlauf.iter().max().unwrap();
            if best >= 20 && beste.map_or(true, |(b, _)| best > b) {
                beste = Some((best, m.kennung));
            }
        }
        let mut texte: Vec<(usize, String)> = Vec::new();
        if l.messen {
            let t = match beste {
                None => LIVE_LEER.to_string(),
                Some((hz, k)) => match l.namen.iter().find(|(kk, _)| *kk == k) {
                    Some((_, n)) => format!("Gemessen: {} Hz – {n}", rate_runden(hz)),
                    None => format!("Gemessen: {} Hz", rate_runden(hz)),
                },
            };
            if t != l.text {
                l.text = t.clone();
                texte.push((ID_MESSEN, t));
            }
        }
        // DPI jede halbe Sekunde nachsehen
        let dpi = if l.takt % 2 == 0 { l.dpi.clone() } else { Vec::new() };
        Some((l.menu, texte, dpi))
    });
    let Some((menu, texte, dpi)) = neu else { return };
    for (id, text) in texte {
        if let Some((m, pos)) = eintrag_finden(menu, id as u32) {
            eintrag_text(m, pos, &text);
            eintrag_malen(m, pos);
        }
    }
    for lg in dpi {
        let gelesen = APP.with(|c| {
            let mut a = c.borrow_mut();
            let g = a.geraete.get_mut(lg.k)?;
            if g.pause > 0 {
                g.pause -= 1;
                return None;
            }
            let Some(d) = g.maus.dpi_schnell() else {
                g.pause = 8; // ~4 s Ruhe, eine schlafende Maus bremst sonst das Menü
                return None;
            };
            let s = g.stand.as_mut()?;
            dpi_eintragen(s, d);
            Some((d, kopf(s), s.dpi_fest))
        });
        let Some((d, kopf_text, fest)) = gelesen else { continue };
        if lg.dpi == Some(d) {
            continue;
        }
        LIVE.with(|l| {
            if let Some(l) = l.borrow_mut().as_mut() {
                if let Some(x) = l.dpi.iter_mut().find(|x| x.k == lg.k) {
                    x.dpi = Some(d);
                }
            }
        });
        // Kopfzeile bzw. Untermenü-Titel der Maus steht im Hauptmenü nach Messzeile und Trennstrich
        let pos = VOR_MAEUSEN + lg.k as u32;
        eintrag_text(menu, pos, &kopf_text);
        eintrag_malen(menu, pos);
        let basis = GERAET * (lg.k + 1);
        if fest {
            if let Some((m, pos)) = eintrag_finden(menu, (basis + ID_DPI_INFO) as u32) {
                eintrag_text(m, pos, &dpi_info(Some(d)));
                eintrag_malen(m, pos);
            }
        }
        for (i, &v) in lg.liste.iter().enumerate() {
            if let Some((m, pos)) = eintrag_finden(menu, (basis + ID_DPI + i) as u32) {
                CheckMenuItem(m, pos, MF_BYPOSITION | if v == d { MF_CHECKED } else { MF_UNCHECKED });
                eintrag_malen(m, pos);
            }
        }
    }
}

fn dpi_info(dpi: Option<u32>) -> String {
    let d = dpi.map(|d| format!("DPI: {d}")).unwrap_or_else(|| "DPI".into());
    format!("{d} – nur mit den DPI-Tasten der Maus")
}

// ---------- Eingabefeld für eigene DPI ----------

pub unsafe fn schrift() -> HFONT {
    if FONT.get().is_null() {
        let mut ncm: NONCLIENTMETRICSW = std::mem::zeroed();
        ncm.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, ncm.cbSize, (&mut ncm as *mut NONCLIENTMETRICSW).cast(), 0);
        FONT.set(CreateFontIndirectW(&ncm.lfMessageFont));
    }
    FONT.get()
}

unsafe fn eingabe_zeigen(k: usize, titel_name: Option<&str>, aktuell: Option<u32>, (min, max): (u32, u32), spiel: bool) {
    let alt = EINGABE.get();
    if !alt.is_null() {
        SetForegroundWindow(alt);
        return;
    }
    EINGABE_SPIEL.set(spiel);
    EINGABE_GERAET.set(k);
    let hinst = GetModuleHandleW(null());
    let (bw, bh) = (300, 150);
    let x = (GetSystemMetrics(SM_CXSCREEN) - bw) / 2;
    let y = (GetSystemMetrics(SM_CYSCREEN) - bh) / 2;
    let mut titel = if spiel { "Eigene DPI im Vollbild".to_string() } else { "Eigene DPI".to_string() };
    if let Some(n) = titel_name {
        titel = format!("{titel} – {n}");
    }
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
        w(KLASSE_EINGABE).as_ptr(),
        w(&titel).as_ptr(),
        WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
        x, y, bw, bh, null_mut(), null_mut(), hinst, null(),
    );
    EINGABE.set(hwnd);
    let schwarz = design::schwarz();
    if schwarz {
        design::fenster_schwarz(hwnd);
    }
    let kind = |klasse: &str, text: &str, stil: u32, x, y, b, h, id: usize| {
        let c = CreateWindowExW(
            if klasse == "EDIT" && !schwarz { WS_EX_CLIENTEDGE } else { 0 },
            w(klasse).as_ptr(), w(text).as_ptr(), WS_CHILD | WS_VISIBLE | stil,
            x, y, b, h, hwnd, id as HMENU, hinst, null(),
        );
        SendMessageW(c, WM_SETFONT, schrift() as WPARAM, 1);
        c
    };
    kind("STATIC", &format!("DPI ({min}–{max}):"), 0, 16, 18, 120, 20, 0);
    let wert = aktuell.map(|d| d.to_string()).unwrap_or_default();
    let edit = kind("EDIT", &wert, WS_TABSTOP | ES_NUMBER as u32 | ES_AUTOHSCROLL as u32, 140, 14, 128, 24, ID_EDIT);
    // Selbst gezeichnete Knöpfe können nicht zugleich Default-Knopf sein; Enter löst trotzdem IDOK aus.
    let (ok, abbr) = if schwarz { (BS_OWNERDRAW, BS_OWNERDRAW) } else { (BS_DEFPUSHBUTTON, BS_PUSHBUTTON) };
    kind("BUTTON", "OK", WS_TABSTOP | ok as u32, 92, 62, 84, 28, IDOK as usize);
    kind("BUTTON", "Abbrechen", WS_TABSTOP | abbr as u32, 184, 62, 84, 28, IDCANCEL as usize);
    SetForegroundWindow(hwnd);
    SetFocus(edit);
    SendMessageW(edit, EM_SETSEL, 0, -1);
}

unsafe fn eingabe_ok(hwnd: HWND) {
    let mut buf = [0u16; 16];
    let n = GetDlgItemTextW(hwnd, ID_EDIT as i32, buf.as_mut_ptr(), buf.len() as i32) as usize;
    let text = String::from_utf16_lossy(&buf[..n]);
    let spiel = EINGABE_SPIEL.get();
    let k = EINGABE_GERAET.get();
    let ergebnis = match text.trim().parse::<u32>() {
        Ok(v) if spiel => mit_maus(k, |m| {
            let (min, max) = m.dpi_bereich()?;
            if v < min || v > max {
                return Err(format!("{v} DPI geht nicht, erlaubt sind {min}-{max}"));
            }
            Ok(())
        })
        .map(|_| spiel_merken(k, Some(Some(v)), None)),
        Ok(v) => mit_maus(k, |m| m.set_dpi(v)).map(|_| ()),
        Err(_) => Err("Bitte eine Zahl eingeben".into()),
    };
    match ergebnis {
        Ok(()) => {
            DestroyWindow(hwnd);
            let liste = tip_aktualisieren(HAUPT.get(), NIM_MODIFY);
            if !spiel {
                einblenden(liste.get(k).and_then(|e| e.stand.as_ref()));
            }
        }
        Err(e) => {
            melden(hwnd, &e);
            SetFocus(GetDlgItem(hwnd, ID_EDIT as i32));
        }
    }
}

unsafe extern "system" fn eingabe_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND if design::schwarz() => {
            design::hintergrund_malen(hwnd, wp as _);
            1
        }
        WM_CTLCOLORSTATIC if design::schwarz() => design::kind_farbe(wp as _, false),
        WM_CTLCOLOREDIT if design::schwarz() => design::kind_farbe(wp as _, true),
        WM_DRAWITEM => {
            design::knopf_malen(lp);
            1
        }
        WM_COMMAND => {
            match (wp & 0xFFFF) as i32 {
                IDOK => eingabe_ok(hwnd),
                IDCANCEL => {
                    DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            EINGABE.set(null_mut());
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ---------- Menü ----------

/// Werte, die das Menü einer Maus anbietet (zum Auswerten des gewählten Punkts).
#[derive(Default)]
struct Angebot {
    dpi: Vec<u32>,
    raten: Vec<u32>,
    vdpi: Vec<u32>,
}

/// Desktop- und Vollbild-Untermenü einer Maus an `ziel` anhängen.
unsafe fn geraet_einfuegen(ziel: HMENU, k: usize, s: &Stand) -> Angebot {
    let basis = GERAET * (k + 1);
    let desk = CreatePopupMenu();
    if s.dpi_fest {
        AppendMenuW(desk, MF_STRING | MF_GRAYED, basis + ID_DPI_INFO, w(&dpi_info(s.dpi)).as_ptr());
        if s.g400s {
            AppendMenuW(desk, MF_STRING, basis + ID_WINUSB, w("DPI einstellbar machen …").as_ptr());
        }
    } else if s.dpi.is_some() {
        let sub = CreatePopupMenu();
        for (i, &v) in s.dpi_liste.iter().enumerate() {
            let f = if Some(v) == s.dpi { MF_CHECKED } else { 0 };
            AppendMenuW(sub, MF_STRING | f, basis + ID_DPI + i, w(&format!("{v} DPI")).as_ptr());
        }
        if !s.dpi_stufen {
            AppendMenuW(sub, MF_SEPARATOR, 0, null());
            AppendMenuW(sub, MF_STRING, basis + ID_DPI_EIGEN, w("Eigener Wert …").as_ptr());
        }
        AppendMenuW(desk, MF_POPUP, sub as usize, w("DPI").as_ptr());
    }
    if !s.raten.is_empty() {
        let sub = CreatePopupMenu();
        for (i, &v) in s.raten.iter().enumerate() {
            let f = if Some(v) == s.hz { MF_CHECKED } else { 0 };
            AppendMenuW(sub, MF_STRING | f, basis + ID_HZ + i, w(&format!("{v} Hz")).as_ptr());
        }
        AppendMenuW(desk, MF_POPUP, sub as usize, w("Hz").as_ptr());
    }
    AppendMenuW(ziel, MF_POPUP, desk as usize, w("Desktop").as_ptr());

    let (sd, sh, im) = APP.with(|c| {
        let a = c.borrow();
        a.geraete.get(k).map_or((None, None, false), |g| (g.spiel_dpi, g.spiel_hz, g.im_spiel))
    });
    let mut vdpi = s.dpi_liste.clone();
    if let Some(v) = sd {
        if !vdpi.contains(&v) {
            vdpi.push(v);
            vdpi.sort();
        }
    }
    let voll = CreatePopupMenu();
    if s.dpi.is_some() && !s.dpi_fest {
        let sub = CreatePopupMenu();
        let f = if sd.is_none() { MF_CHECKED } else { 0 };
        AppendMenuW(sub, MF_STRING | f, basis + ID_VDPI_AUS, w("Wie Desktop").as_ptr());
        AppendMenuW(sub, MF_SEPARATOR, 0, null());
        for (i, &v) in vdpi.iter().enumerate() {
            let f = if Some(v) == sd { MF_CHECKED } else { 0 };
            AppendMenuW(sub, MF_STRING | f, basis + ID_VDPI + i, w(&format!("{v} DPI")).as_ptr());
        }
        if !s.dpi_stufen {
            AppendMenuW(sub, MF_SEPARATOR, 0, null());
            AppendMenuW(sub, MF_STRING, basis + ID_VDPI_EIGEN, w("Eigener Wert …").as_ptr());
        }
        AppendMenuW(voll, MF_POPUP, sub as usize, w("DPI").as_ptr());
    }
    if !s.raten.is_empty() {
        let sub = CreatePopupMenu();
        let f = if sh.is_none() { MF_CHECKED } else { 0 };
        AppendMenuW(sub, MF_STRING | f, basis + ID_VHZ_AUS, w("Wie Desktop").as_ptr());
        AppendMenuW(sub, MF_SEPARATOR, 0, null());
        for (i, &v) in s.raten.iter().enumerate() {
            let f = if Some(v) == sh { MF_CHECKED } else { 0 };
            AppendMenuW(sub, MF_STRING | f, basis + ID_VHZ + i, w(&format!("{v} Hz")).as_ptr());
        }
        AppendMenuW(voll, MF_POPUP, sub as usize, w("Hz").as_ptr());
    }
    let titel = if im { "Vollbild (Spiel) – gerade aktiv" } else { "Vollbild (Spiel)" };
    AppendMenuW(ziel, MF_POPUP, voll as usize, w(titel).as_ptr());
    Angebot { dpi: s.dpi_liste.clone(), raten: s.raten.clone(), vdpi }
}

unsafe fn menu_zeigen(hwnd: HWND) {
    // Kein zweites Menü, solange eins offen ist: Klicks, die während des Abfragens der Maus
    // auflaufen, kämen sonst mitten im offenen Menü an und würden dessen Texte löschen.
    if MENU_OFFEN.replace(true) {
        return;
    }
    // Klick auf „Hz messen“ öffnet das Menü an derselben Stelle neu, mit Messung an bzw. aus.
    // Beim nächsten Öffnen ist sie wieder aus.
    let mut p = POINT { x: 0, y: 0 };
    GetCursorPos(&mut p);
    while menu_zeigen_innen(hwnd, p) {}
    MESSEN.set(false);
    MENU_OFFEN.set(false);
}

/// Zeigt das Menü und führt den gewählten Punkt aus. `true` = Menü erneut zeigen.
unsafe fn menu_zeigen_innen(hwnd: HWND, p: POINT) -> bool {
    schnell_auffrischen();
    let liste = uebersicht();
    let (mut namen, suche): (Vec<(u32, String)>, bool) = APP.with(|c| {
        let a = c.borrow();
        (a.fremde.iter().map(|f| (kennung(f.vid, f.pid), f.name.clone())).collect(), a.suche_laeuft)
    });
    namen.extend(liste.iter().map(|e| (e.kennung, e.name.clone())));
    let menu = CreatePopupMenu();
    let messen = MESSEN.get();
    let (t, f) = if messen { (LIVE_LEER, MF_CHECKED) } else { (MESSEN_AUS, 0) };
    AppendMenuW(menu, MF_STRING | f, ID_MESSEN, w(t).as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    let mut angebote: Vec<Angebot> = Vec::new();
    let mut live_dpi: Vec<LiveDpi> = Vec::new();
    // Eine Maus: Einstellungen direkt im Hauptmenü
    let flach = liste.len() == 1;
    for (k, e) in liste.iter().enumerate() {
        match &e.stand {
            Some(s) if flach => {
                AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w(&kopf(s)).as_ptr());
                angebote.push(geraet_einfuegen(menu, k, s));
            }
            Some(s) => {
                let sub = CreatePopupMenu();
                angebote.push(geraet_einfuegen(sub, k, s));
                AppendMenuW(menu, MF_POPUP, sub as usize, w(&kopf(s)).as_ptr());
            }
            None => {
                angebote.push(Angebot::default());
                AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w(&format!("{} – antwortet nicht", e.name)).as_ptr());
            }
        }
        if let Some(s) = &e.stand {
            live_dpi.push(LiveDpi { k, dpi: s.dpi, liste: angebote[k].dpi.clone() });
        }
    }
    if liste.is_empty() {
        let t = if suche { "Suche Maus …" } else { "Keine einstellbare Maus gefunden" };
        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w(t).as_ptr());
    }
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    let auto = if autostart_an() { MF_CHECKED } else { 0 };
    AppendMenuW(menu, MF_STRING | auto, ID_AUTO, w("Mit Windows starten").as_ptr());
    let im_startmenue = startmenue::vorhanden("Mausi");
    AppendMenuW(menu, MF_STRING | if im_startmenue { MF_CHECKED } else { 0 }, ID_STARTMENUE, w("Im Startmenü anzeigen").as_ptr());
    let schwarz = if design::schwarz() { MF_CHECKED } else { 0 };
    AppendMenuW(menu, MF_STRING | schwarz, ID_DESIGN, w("Schwarzes Design").as_ptr());
    let sub = CreatePopupMenu();
    let gewaehlt = symbol_einstellung();
    for (i, name) in SYMBOL_NAMEN.iter().enumerate() {
        let f = if gewaehlt == i { MF_CHECKED } else { 0 };
        AppendMenuW(sub, MF_STRING | f, ID_SYMBOL + i, w(name).as_ptr());
        if i == 0 {
            AppendMenuW(sub, MF_SEPARATOR, 0, null());
        }
    }
    AppendMenuW(menu, MF_POPUP, sub as usize, w("Tray-Symbol").as_ptr());
    AppendMenuW(menu, MF_STRING, ID_NEU, w("Mäuse neu suchen").as_ptr());
    AppendMenuW(menu, MF_STRING, ID_ENDE, w("Beenden").as_ptr());

    if design::schwarz() {
        design::menu_schwarz(menu);
    }
    SetForegroundWindow(hwnd);
    live_starten(hwnd, menu, messen, namen, live_dpi);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, p.x, p.y, 0, hwnd, null());
    live_stoppen(hwnd);
    DestroyMenu(menu);
    design::menu_fertig();
    PostMessageW(hwnd, WM_NULL, 0, 0);
    let cmd = cmd.max(0) as usize;

    let mut gewechselt = None;
    let ergebnis = match cmd {
        ID_ENDE => {
            DestroyWindow(hwnd);
            return false;
        }
        ID_MESSEN => {
            MESSEN.set(!messen);
            return true;
        }
        ID_AUTO => {
            autostart_setzen(auto == 0);
            Ok(())
        }
        ID_STARTMENUE => {
            startmenue::setzen("Mausi", !im_startmenue);
            Ok(())
        }
        ID_NEU => {
            suche_starten();
            Ok(())
        }
        ID_DESIGN => {
            design::setzen(schwarz == 0);
            Ok(())
        }
        c if (ID_SYMBOL..ID_SYMBOL + SYMBOL_NAMEN.len()).contains(&c) => {
            let nr = c - ID_SYMBOL;
            zustand::merken(SYMBOL_SCHLUESSEL, (nr != 0).then_some(SYMBOL_WERTE[nr]));
            Ok(())
        }
        c if (GERAET..GERAET * (MAX_GERAETE + 1)).contains(&c) => {
            let k = c / GERAET - 1;
            let (Some(e), Some(an)) = (liste.get(k), angebote.get(k)) else { return false };
            let Some(s) = &e.stand else { return false };
            let name_im_titel = (!flach).then_some(e.name.as_str());
            match c % GERAET {
                ID_DPI_EIGEN => {
                    eingabe_zeigen(k, name_im_titel, s.dpi, s.dpi_bereich, false);
                    return false;
                }
                ID_VDPI_EIGEN => {
                    let sd = APP.with(|c| c.borrow().geraete.get(k).and_then(|g| g.spiel_dpi));
                    eingabe_zeigen(k, name_im_titel, sd.or(s.dpi), s.dpi_bereich, true);
                    return false;
                }
                ID_WINUSB => {
                    winusb_starten();
                    Ok(())
                }
                ID_VDPI_AUS => {
                    spiel_merken(k, Some(None), None);
                    Ok(())
                }
                ID_VHZ_AUS => {
                    spiel_merken(k, None, Some(None));
                    Ok(())
                }
                l if (ID_VDPI..ID_VDPI + an.vdpi.len()).contains(&l) => {
                    spiel_merken(k, Some(Some(an.vdpi[l - ID_VDPI])), None);
                    Ok(())
                }
                l if (ID_VHZ..ID_VHZ + an.raten.len()).contains(&l) => {
                    spiel_merken(k, None, Some(Some(an.raten[l - ID_VHZ])));
                    Ok(())
                }
                l if (ID_DPI..ID_DPI + an.dpi.len()).contains(&l) => {
                    let v = an.dpi[l - ID_DPI];
                    let r = mit_maus(k, |m| m.set_dpi(v)).map(|_| ());
                    gewechselt = r.is_ok().then_some(k);
                    r
                }
                l if (ID_HZ..ID_HZ + an.raten.len()).contains(&l) => {
                    let v = an.raten[l - ID_HZ];
                    let r = mit_maus(k, |m| m.set_hz(v)).map(|name| {
                        zustand::hz_merken(&name, v);
                        APP.with(|c| {
                            if let Some(g) = c.borrow_mut().geraete.get_mut(k) {
                                g.hz_gesetzt = Some(v);
                            }
                        });
                    });
                    gewechselt = r.is_ok().then_some(k);
                    r
                }
                _ => return false,
            }
        }
        _ => return false,
    };
    if let Err(e) = ergebnis {
        melden(hwnd, &e);
    }
    let liste = tip_aktualisieren(hwnd, NIM_MODIFY);
    if let Some(k) = gewechselt {
        einblenden(liste.get(k).and_then(|e| e.stand.as_ref()));
    }
    false
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let ev = (lp as u32) & 0xFFFF;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
                menu_zeigen(hwnd);
            }
            0
        }
        WM_MEASUREITEM if design::menu_messen(hwnd, lp) => 1,
        WM_DRAWITEM if design::menu_malen(lp) => 1,
        WM_EINBLENDEN => {
            // Änderung kam von der Kommandozeile: gemerkte Hz neu einlesen
            APP.with(|c| {
                for g in c.borrow_mut().geraete.iter_mut() {
                    g.hz_gesetzt = zustand::hz_lesen(&g.maus.name);
                }
            });
            let liste = tip_aktualisieren(hwnd, NIM_MODIFY);
            einblenden(liste.iter().find_map(|e| e.stand.as_ref()));
            0
        }
        WM_WINUSB => {
            if let Some(Err(e)) = WINUSB_ERGEBNIS.lock().unwrap().take() {
                melden(hwnd, &format!("G400s: DPI einstellbar machen hat nicht geklappt.

{e}"));
            }
            suche_starten();
            0
        }
        WM_GEFUNDEN => {
            suche_uebernehmen();
            tip_aktualisieren(hwnd, NIM_MODIFY);
            0
        }
        WM_DEVICECHANGE => {
            // Gerät an- oder abgesteckt: kurz warten, bis Windows fertig ist, dann neu suchen
            SetTimer(hwnd, TIMER_GERAETE, 1500, None);
            1
        }
        WM_INPUT => {
            live_zaehlen(lp);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_TIMER if wp == TIMER_LIVE => {
            live_takt();
            0
        }
        WM_TIMER if wp == TIMER_GERAETE => {
            KillTimer(hwnd, TIMER_GERAETE);
            suche_starten();
            0
        }
        WM_TIMER if wp == TIMER_SUCHE => {
            // Keine Maus verbunden oder ein Empfänger ohne Maus (z. B. Funkmaus erst später
            // eingeschaltet oder beim Start zu tief geschlafen): regelmäßig nachsehen
            if !MENU_OFFEN.get() && APP.with(|c| { let a = c.borrow(); a.geraete.is_empty() || a.luecke }) {
                suche_starten();
            }
            0
        }
        WM_TIMER if wp == TIMER_VOLL => {
            let an = vollbild_erkannt();
            let gewechselt = spiel_wechseln(an);
            if let Some(&k) = gewechselt.first() {
                let liste = tip_aktualisieren(hwnd, NIM_MODIFY);
                einblenden(liste.get(k).and_then(|e| e.stand.as_ref()));
            }
            0
        }
        WM_TIMER if wp == TIMER_G400S => {
            // Bei offenem Menü sieht die Live-Abfrage nach
            if !MENU_OFFEN.get() {
                g400s_nachsehen(hwnd);
            }
            0
        }
        WM_TIMER if wp == TIMER_AKKU => {
            tip_aktualisieren(hwnd, NIM_MODIFY); // Akkustand im Tooltip frisch halten
            0
        }
        WM_SETTINGCHANGE if lp != 0 && breit_gleich(lp as *const u16, "ImmersiveColorSet") => {
            // Windows hat zwischen hell und dunkel gewechselt: Tray-Symbol anpassen
            tip_aktualisieren(hwnd, NIM_MODIFY);
            0
        }
        WM_DESTROY => {
            tray(hwnd, NIM_DELETE, "");
            PostQuitMessage(0);
            0
        }
        m if m != 0 && m == TASKBAR.get() => {
            // Explorer neu gestartet: Symbol wieder anlegen
            tip_aktualisieren(hwnd, NIM_ADD);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Tray-Symbole: cyan (dunkle Taskleiste), schwarz (helle Taskleiste), hellgrau (keine Maus, Programm).
const SYMBOLE: [&[u8]; 3] = [
    include_bytes!("../assets/icons/1_klar.ico"),
    include_bytes!("../assets/icons/2_schwarz.ico"),
    include_bytes!("../assets/icons/4_minimal.ico"),
];

/// Das passende Bild aus einer ICO-Datei (PNG-Einträge) als HICON laden:
/// die kleinste Größe, die mindestens so groß ist wie gewünscht, sonst die größte.
unsafe fn ico_laden(ico: &[u8], groesse: i32) -> HICON {
    let zahl = |o: usize| u32::from_le_bytes([ico[o], ico[o + 1], ico[o + 2], ico[o + 3]]);
    let eintrag = |i: usize| {
        let o = 6 + 16 * i;
        let breite = if ico[o] == 0 { 256 } else { ico[o] as i32 };
        (breite, zahl(o + 12) as usize, zahl(o + 8))
    };
    let alle = 0..u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let passend = alle.clone().map(eintrag).filter(|e| e.0 >= groesse).min_by_key(|e| e.0);
    let Some((_, off, len)) = passend.or_else(|| alle.map(eintrag).max_by_key(|e| e.0)) else {
        return null_mut();
    };
    CreateIconFromResourceEx(ico[off..].as_ptr(), len, 1, 0x0003_0000, groesse, groesse, LR_DEFAULTCOLOR)
}

/// Größe eines kleinen Symbols bei der echten Bildschirmskalierung.
/// Mausi selbst ist nicht DPI-aware und bekäme sonst immer 16 Pixel gemeldet.
unsafe fn symbolgroesse() -> i32 {
    use windows_sys::Win32::UI::HiDpi::*;
    let alt = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    let dpi = GetDpiForSystem();
    if alt as usize != 0 {
        SetThreadDpiAwarenessContext(alt);
    }
    GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16)
}

/// Hat Windows die Taskleiste hell eingestellt?
fn taskleiste_hell() -> bool {
    let (mut wert, mut n) = (0u32, 4u32);
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, w(PERSONALISIERUNG).as_ptr(), w("SystemUsesLightTheme").as_ptr(), RRF_RT_REG_DWORD,
            null_mut(), (&mut wert as *mut u32).cast(), &mut n) == ERROR_SUCCESS && wert == 1
    }
}

const SYMBOL_SCHLUESSEL: &str = "tray_symbol";
const SYMBOL_NAMEN: [&str; 4] = ["Automatisch (nach Taskleiste)", "Cyan", "Schwarz", "Hellgrau"];
/// Gespeicherter Wert je Menüpunkt. 3 war „Mausi mit Ohren“ (entfernt) und gilt jetzt als automatisch.
const SYMBOL_WERTE: [u32; 4] = [0, 1, 2, 4];

/// Menüpunkt: 0 = automatisch, 1–3 = fest gewähltes Symbol.
fn symbol_einstellung() -> usize {
    let v = zustand::lesen(SYMBOL_SCHLUESSEL).unwrap_or(0);
    SYMBOL_WERTE.iter().position(|&x| x == v).unwrap_or(0)
}

/// Tray-Symbol: fest gewählt, oder automatisch: keine Maus = hellgrau, sonst Taskleiste hell (schwarz) oder dunkel (cyan).
unsafe fn symbol_waehlen(maus_da: bool) {
    let nr = match symbol_einstellung() {
        0 if !maus_da => 2,
        0 if taskleiste_hell() => 1,
        0 => 0,
        n => n - 1,
    };
    if TRAY_NR.replace(nr) != nr || TRAY_ICON.get().is_null() {
        let alt = TRAY_ICON.replace(ico_laden(SYMBOLE[nr], symbolgroesse()));
        if !alt.is_null() {
            DestroyIcon(alt);
        }
    }
}

unsafe fn klasse(name: &[u16], proc_: WNDPROC, hinst: HINSTANCE, hintergrund: bool) {
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: proc_,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: if hintergrund { ICON.get() } else { null_mut() },
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: if hintergrund { (COLOR_BTNFACE + 1) as usize as _ } else { null_mut() },
        lpszMenuName: null(),
        lpszClassName: name.as_ptr(),
    };
    RegisterClassW(&wc);
}

pub fn starten() {
    unsafe {
        let name = w(KLASSE);
        if !FindWindowW(name.as_ptr(), null()).is_null() {
            return; // läuft schon
        }
        zustand::umziehen();
        pfade_nachziehen();
        let hinst = GetModuleHandleW(null());
        ICON.set(ico_laden(SYMBOLE[2], 32)); // Programm-Icon: hellgrau
        design::laden();
        klasse(&name, Some(wndproc), hinst, false);
        let name_eingabe = w(KLASSE_EINGABE);
        klasse(&name_eingabe, Some(eingabe_proc), hinst, true);
        let name_hinweis = w(KLASSE_HINWEIS);
        klasse(&name_hinweis, Some(hinweis_proc), hinst, true);
        let name_osd = w(design::KLASSE_OSD);
        klasse(&name_osd, Some(design::osd_proc), hinst, false);
        let hwnd = CreateWindowExW(0, name.as_ptr(), w("Maus").as_ptr(), 0, 0, 0, 0, 0, null_mut(), null_mut(), hinst, null());
        HAUPT.set(hwnd);
        // Erster ShowWindow-Aufruf im Prozess übernimmt den Startmodus (oft minimiert).
        // Hier am unsichtbaren Hauptfenster verbrauchen, sonst startet das Hinweis-Popup minimiert.
        ShowWindow(hwnd, SW_HIDE);
        TASKBAR.set(RegisterWindowMessageW(w("TaskbarCreated").as_ptr()));
        APP.with(|c| c.borrow_mut().presets = presets_laden());
        tip_aktualisieren(hwnd, NIM_ADD);
        suche_starten();
        SetTimer(hwnd, TIMER_AKKU, 15 * 60 * 1000, None);
        SetTimer(hwnd, TIMER_VOLL, 2000, None);
        SetTimer(hwnd, TIMER_SUCHE, 30 * 1000, None);
        // MAUS_G4_MS=<ms> ändert den Takt (0 = aus), zum Messen
        let g4_ms = std::env::var("MAUS_G4_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(500);
        if g4_ms > 0 {
            SetTimer(hwnd, TIMER_G400S, g4_ms, None);
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            let e = EINGABE.get();
            if !e.is_null() && IsDialogMessageW(e, &msg) != 0 {
                continue; // Enter, Esc, Tab im Eingabefenster
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
