//! Tray-Symbol mit Menü: DPI-Stufen + eigener Wert, Abfragerate, Live-Messung, Autostart.

use crate::design;
use crate::hidpp::Maus;
use crate::startmenue;
use crate::zustand;
use std::cell::{Cell, RefCell};
use std::ptr::{null, null_mut};
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
const ID_DPI: usize = 1000;
const ID_HZ: usize = 2000;
const ID_AUTO: usize = 3;
const ID_NEU: usize = 4;
const ID_DPI_EIGEN: usize = 5;
const ID_LIVE: usize = 6;
const ID_ENDE: usize = 9;
const ID_VDPI: usize = 3000;
const ID_VHZ: usize = 4000;
const ID_VDPI_AUS: usize = 7;
const ID_VDPI_EIGEN: usize = 8;
const ID_VHZ_AUS: usize = 10;
const ID_DESIGN: usize = 11;
const ID_STARTMENUE: usize = 12;
const ID_EDIT: usize = 101;
const TIMER_LIVE: usize = 1;
const TIMER_AKKU: usize = 2;
const TIMER_VOLL: usize = 3;
const AKKU_WARNUNG: u8 = 10;
const EM_SETSEL: u32 = 0x00B1;
const COLOR_BTNFACE: i32 = 15;
const KLASSE: &str = "MausTray";
const KLASSE_EINGABE: &str = "MausEingabe";
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STANDARD_DPI: [u32; 6] = [400, 800, 1200, 1600, 3200, 6400];

pub fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct App {
    maus: Option<Maus>,
    hz_gesetzt: Option<u32>,
    presets: Vec<u32>,
    // Vollbild (Spiele): eigene Werte, None = wie Desktop
    spiel_dpi: Option<u32>,
    spiel_hz: Option<u32>,
    im_spiel: bool,
    desk_dpi: Option<u32>,
    desk_hz: Option<u32>,
}

fn schluessel(name: &str, was: &str) -> String {
    format!("{name}.vollbild_{was}")
}

impl App {
    fn maus(&mut self) -> Option<&Maus> {
        if self.maus.is_none() {
            self.maus = Maus::finden().ok();
            self.hz_gesetzt = self.maus.as_ref().and_then(|m| zustand::hz_lesen(&m.name));
            if let Some(m) = &self.maus {
                self.spiel_dpi = zustand::lesen(&schluessel(&m.name, "dpi"));
                self.spiel_hz = zustand::lesen(&schluessel(&m.name, "hz"));
            }
        }
        self.maus.as_ref()
    }
}

/// Live-Messung: zählt Raw-Input-Meldungen der Maus, solange das Menü offen ist.
struct Live {
    menu: HMENU,
    zaehler: u32,
    start: Instant,
    verlauf: [u32; 4], // letzte 4 Viertelsekunden
    pos: usize,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App {
        maus: None, hz_gesetzt: None, presets: Vec::new(),
        spiel_dpi: None, spiel_hz: None, im_spiel: false, desk_dpi: None, desk_hz: None,
    });
    static EINGABE_SPIEL: Cell<bool> = Cell::new(false);
    static ICON: Cell<HICON> = Cell::new(null_mut());
    static TASKBAR: Cell<u32> = Cell::new(0);
    static HAUPT: Cell<HWND> = Cell::new(null_mut());
    static EINGABE: Cell<HWND> = Cell::new(null_mut());
    static FONT: Cell<HFONT> = Cell::new(null_mut());
    static LIVE: RefCell<Option<Live>> = RefCell::new(None);
    static GEWARNT: Cell<bool> = Cell::new(false);
}

struct Stand {
    name: String,
    dpi: Option<u32>,
    hz: Option<u32>,
    raten: Vec<u32>,
    dpi_liste: Vec<u32>,
    dpi_bereich: (u32, u32),
    akku: Option<(u8, bool)>,
}

fn stand_holen() -> Option<Stand> {
    APP.with(|c| {
        let mut a = c.borrow_mut();
        for _ in 0..2 {
            let presets = a.presets.clone();
            let hz_g = a.hz_gesetzt;
            let s = a.maus().and_then(|m| {
                let dpi = m.dpi().ok();
                let raten = m.raten().unwrap_or_default();
                if dpi.is_none() && raten.is_empty() {
                    return None; // Maus weg (aus, außer Reichweite)
                }
                let (min, max) = m.dpi_bereich().unwrap_or((0, 0));
                let mut dpi_liste: Vec<u32> = presets.into_iter().filter(|&v| v >= min && v <= max).collect();
                if let Some(d) = dpi {
                    if !dpi_liste.contains(&d) {
                        dpi_liste.push(d);
                        dpi_liste.sort();
                    }
                }
                let hz = if m.hz_lesewert_unsicher() { hz_g.or_else(|| m.hz().ok()) } else { m.hz().ok() };
                let akku = m.akku().ok();
                Some(Stand { name: m.name.clone(), dpi, hz, raten, dpi_liste, dpi_bereich: (min, max), akku })
            });
            if s.is_some() {
                return s;
            }
            a.maus = None; // beim zweiten Durchlauf neu verbinden
        }
        None
    })
}

fn mit_maus(f: impl FnOnce(&Maus) -> Result<(), String>) -> Result<String, String> {
    APP.with(|c| {
        let mut a = c.borrow_mut();
        let m = a.maus().ok_or("Keine Maus gefunden")?;
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
    nid.hIcon = ICON.get();
    for (i, c) in tip.encode_utf16().take(127).enumerate() {
        nid.szTip[i] = c;
    }
    Shell_NotifyIconW(aktion, &nid);
}

unsafe fn tip_aktualisieren(hwnd: HWND, aktion: u32) -> Option<Stand> {
    let st = stand_holen();
    let tip = st.as_ref().map(kopf).unwrap_or("Maus: keine Maus gefunden".into());
    tray(hwnd, aktion, &tip);
    if let Some((p, laedt)) = st.as_ref().and_then(|s| s.akku) {
        akku_warnen(hwnd, p, laedt);
    }
    st
}

/// DPI/Hz am rechten Bildschirmrand einblenden.
unsafe fn einblenden(st: Option<Stand>) {
    if let Some(s) = st {
        design::osd_zeigen(s.dpi, s.hz, APP.with(|c| c.borrow().im_spiel));
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
unsafe fn akku_warnen(hwnd: HWND, prozent: u8, laedt: bool) {
    // MAUS_WARNUNG=<Prozent> setzt die Schwelle zum Testen hoch
    let schwelle = std::env::var("MAUS_WARNUNG").ok().and_then(|v| v.parse().ok()).unwrap_or(AKKU_WARNUNG);
    if laedt || prozent > schwelle {
        GEWARNT.set(false);
        return;
    }
    if GEWARNT.replace(true) {
        return;
    }
    let _ = hwnd;
    hinweis_zeigen(&format!("Maus-Akku fast leer\nNoch {prozent} % – bitte bald laden."));
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

/// Wechselt zwischen Desktop- und Vollbild-Werten. true = gewechselt.
/// Schlägt das Setzen fehl (Maus schläft), bleibt der Zustand und der nächste Takt versucht es erneut.
fn spiel_wechseln(an: bool) -> bool {
    APP.with(|c| {
        let mut a = c.borrow_mut();
        if a.im_spiel == an {
            return false;
        }
        let (sd, sh, dd, dh, hz_g) = (a.spiel_dpi, a.spiel_hz, a.desk_dpi, a.desk_hz, a.hz_gesetzt);
        if an && sd.is_none() && sh.is_none() {
            a.im_spiel = true; // nichts umzustellen
            return true;
        }
        let Some(m) = a.maus() else { return false };
        if an {
            let akt = if sd.is_some() { m.dpi().ok() } else { None };
            let ok_d = sd.map_or(true, |v| akt == Some(v) || m.set_dpi(v).is_ok());
            let ok_h = sh.map_or(true, |v| hz_g == Some(v) || m.set_hz(v).is_ok());
            if !(ok_d && ok_h) {
                return false;
            }
            a.desk_dpi = akt;
            a.desk_hz = hz_g;
            if sh.is_some() {
                a.hz_gesetzt = sh;
            }
        } else {
            let ok_d = sd.is_none() || dd.map_or(true, |v| m.set_dpi(v).is_ok());
            let ok_h = sh.is_none() || dh.map_or(true, |v| m.set_hz(v).is_ok());
            if !(ok_d && ok_h) {
                return false;
            }
            if sh.is_some() {
                a.hz_gesetzt = dh;
            }
        }
        a.im_spiel = an;
        true
    })
}

/// Vollbild-Werte ändern (äußeres None = unverändert, Some(None) = wie Desktop) und speichern.
fn spiel_merken(dpi: Option<Option<u32>>, hz: Option<Option<u32>>) {
    // Läuft gerade ein Vollbild, erst auf Desktop zurück; der nächste Takt setzt die neuen Werte.
    if APP.with(|c| c.borrow().im_spiel) {
        spiel_wechseln(false);
    }
    APP.with(|c| {
        let mut a = c.borrow_mut();
        let name = a.maus().map(|m| m.name.clone()).unwrap_or_default();
        if let Some(d) = dpi {
            a.spiel_dpi = d;
            zustand::merken(&schluessel(&name, "dpi"), d);
        }
        if let Some(h) = hz {
            a.spiel_hz = h;
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

unsafe fn live_starten(hwnd: HWND, menu: HMENU) {
    let rid = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
    RegisterRawInputDevices(&rid, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32);
    LIVE.with(|l| *l.borrow_mut() = Some(Live { menu, zaehler: 0, start: Instant::now(), verlauf: [0; 4], pos: 0 }));
    SetTimer(hwnd, TIMER_LIVE, 250, None);
}

unsafe fn live_stoppen(hwnd: HWND) {
    KillTimer(hwnd, TIMER_LIVE);
    let rid = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_REMOVE, hwndTarget: null_mut() };
    RegisterRawInputDevices(&rid, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32);
    LIVE.with(|l| *l.borrow_mut() = None);
}

unsafe fn live_takt() {
    let neu = LIVE.with(|l| {
        let mut l = l.borrow_mut();
        let l = l.as_mut()?;
        let sek = l.start.elapsed().as_secs_f32();
        let rate = (l.zaehler as f32 / sek.max(0.05)).round() as u32;
        l.zaehler = 0;
        l.start = Instant::now();
        l.verlauf[l.pos] = rate;
        l.pos = (l.pos + 1) % l.verlauf.len();
        // Höchstwert der letzten Sekunde: beim Abbremsen sendet die Maus weniger
        let best = *l.verlauf.iter().max().unwrap();
        let text = if best < 20 { LIVE_LEER.to_string() } else { format!("Gemessen: {best} Hz") };
        Some((l.menu, text))
    });
    let Some((menu, text)) = neu else { return };
    if !design::menu_text_setzen(menu, ID_LIVE as u32, &text) {
        ModifyMenuW(menu, ID_LIVE as u32, MF_BYCOMMAND | MF_STRING | MF_GRAYED, ID_LIVE, w(&text).as_ptr());
    }
    let fenster = FindWindowW(w("#32768").as_ptr(), null()); // offenes Menüfenster neu zeichnen
    if !fenster.is_null() {
        InvalidateRect(fenster, null(), 1);
    }
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

unsafe fn eingabe_zeigen(aktuell: Option<u32>, (min, max): (u32, u32), spiel: bool) {
    let alt = EINGABE.get();
    if !alt.is_null() {
        SetForegroundWindow(alt);
        return;
    }
    EINGABE_SPIEL.set(spiel);
    let hinst = GetModuleHandleW(null());
    let (bw, bh) = (300, 150);
    let x = (GetSystemMetrics(SM_CXSCREEN) - bw) / 2;
    let y = (GetSystemMetrics(SM_CYSCREEN) - bh) / 2;
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
        w(KLASSE_EINGABE).as_ptr(),
        w(if spiel { "Eigene DPI im Vollbild" } else { "Eigene DPI" }).as_ptr(),
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
    let desktop = !EINGABE_SPIEL.get();
    let ergebnis = match text.trim().parse::<u32>() {
        Ok(v) if EINGABE_SPIEL.get() => mit_maus(|m| {
            let (min, max) = m.dpi_bereich()?;
            if v < min || v > max {
                return Err(format!("{v} DPI geht nicht, erlaubt sind {min}-{max}"));
            }
            Ok(())
        })
        .map(|_| spiel_merken(Some(Some(v)), None)),
        Ok(v) => mit_maus(|m| m.set_dpi(v)).map(|_| ()),
        Err(_) => Err("Bitte eine Zahl eingeben".into()),
    };
    match ergebnis {
        Ok(()) => {
            DestroyWindow(hwnd);
            let st = tip_aktualisieren(HAUPT.get(), NIM_MODIFY);
            if desktop {
                einblenden(st);
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

unsafe fn menu_zeigen(hwnd: HWND) {
    let st = stand_holen();
    let menu = CreatePopupMenu();
    let (mut dpi_liste, mut raten, mut vdpi_liste) = (Vec::new(), Vec::new(), Vec::new());
    match &st {
        None => {
            AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w("Keine Maus gefunden").as_ptr());
        }
        Some(s) => {
            AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w(&kopf(s)).as_ptr());
            AppendMenuW(menu, MF_STRING | MF_GRAYED, ID_LIVE, w(LIVE_LEER).as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            let desk = CreatePopupMenu();
            if s.dpi.is_some() {
                let sub = CreatePopupMenu();
                for (i, &v) in s.dpi_liste.iter().enumerate() {
                    let f = if Some(v) == s.dpi { MF_CHECKED } else { 0 };
                    AppendMenuW(sub, MF_STRING | f, ID_DPI + i, w(&format!("{v} DPI")).as_ptr());
                }
                AppendMenuW(sub, MF_SEPARATOR, 0, null());
                AppendMenuW(sub, MF_STRING, ID_DPI_EIGEN, w("Eigener Wert …").as_ptr());
                AppendMenuW(desk, MF_POPUP, sub as usize, w("DPI").as_ptr());
            }
            if !s.raten.is_empty() {
                let sub = CreatePopupMenu();
                for (i, &v) in s.raten.iter().enumerate() {
                    let f = if Some(v) == s.hz { MF_CHECKED } else { 0 };
                    AppendMenuW(sub, MF_STRING | f, ID_HZ + i, w(&format!("{v} Hz")).as_ptr());
                }
                AppendMenuW(desk, MF_POPUP, sub as usize, w("Abfragerate").as_ptr());
            }
            AppendMenuW(menu, MF_POPUP, desk as usize, w("Desktop").as_ptr());
            dpi_liste = s.dpi_liste.clone();
            raten = s.raten.clone();
            let (sd, sh, im) = APP.with(|c| {
                let a = c.borrow();
                (a.spiel_dpi, a.spiel_hz, a.im_spiel)
            });
            vdpi_liste = dpi_liste.clone();
            if let Some(v) = sd {
                if !vdpi_liste.contains(&v) {
                    vdpi_liste.push(v);
                    vdpi_liste.sort();
                }
            }
            let voll = CreatePopupMenu();
            if s.dpi.is_some() {
                let sub = CreatePopupMenu();
                let f = if sd.is_none() { MF_CHECKED } else { 0 };
                AppendMenuW(sub, MF_STRING | f, ID_VDPI_AUS, w("Wie Desktop").as_ptr());
                AppendMenuW(sub, MF_SEPARATOR, 0, null());
                for (i, &v) in vdpi_liste.iter().enumerate() {
                    let f = if Some(v) == sd { MF_CHECKED } else { 0 };
                    AppendMenuW(sub, MF_STRING | f, ID_VDPI + i, w(&format!("{v} DPI")).as_ptr());
                }
                AppendMenuW(sub, MF_SEPARATOR, 0, null());
                AppendMenuW(sub, MF_STRING, ID_VDPI_EIGEN, w("Eigener Wert …").as_ptr());
                AppendMenuW(voll, MF_POPUP, sub as usize, w("DPI").as_ptr());
            }
            if !s.raten.is_empty() {
                let sub = CreatePopupMenu();
                let f = if sh.is_none() { MF_CHECKED } else { 0 };
                AppendMenuW(sub, MF_STRING | f, ID_VHZ_AUS, w("Wie Desktop").as_ptr());
                AppendMenuW(sub, MF_SEPARATOR, 0, null());
                for (i, &v) in s.raten.iter().enumerate() {
                    let f = if Some(v) == sh { MF_CHECKED } else { 0 };
                    AppendMenuW(sub, MF_STRING | f, ID_VHZ + i, w(&format!("{v} Hz")).as_ptr());
                }
                AppendMenuW(voll, MF_POPUP, sub as usize, w("Abfragerate").as_ptr());
            }
            let titel = if im { "Vollbild (Spiel) – gerade aktiv" } else { "Vollbild (Spiel)" };
            AppendMenuW(menu, MF_POPUP, voll as usize, w(titel).as_ptr());
        }
    }
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    let auto = if autostart_an() { MF_CHECKED } else { 0 };
    AppendMenuW(menu, MF_STRING | auto, ID_AUTO, w("Mit Windows starten").as_ptr());
    let im_startmenue = startmenue::vorhanden("Mausi");
    AppendMenuW(menu, MF_STRING | if im_startmenue { MF_CHECKED } else { 0 }, ID_STARTMENUE, w("Im Startmenü anzeigen").as_ptr());
    let schwarz = if design::schwarz() { MF_CHECKED } else { 0 };
    AppendMenuW(menu, MF_STRING | schwarz, ID_DESIGN, w("Schwarzes Design").as_ptr());
    AppendMenuW(menu, MF_STRING, ID_NEU, w("Maus neu suchen").as_ptr());
    AppendMenuW(menu, MF_STRING, ID_ENDE, w("Beenden").as_ptr());

    if design::schwarz() {
        design::menu_schwarz(menu);
    }
    let mut p = POINT { x: 0, y: 0 };
    GetCursorPos(&mut p);
    SetForegroundWindow(hwnd);
    if st.is_some() {
        live_starten(hwnd, menu);
    }
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, p.x, p.y, 0, hwnd, null());
    live_stoppen(hwnd);
    DestroyMenu(menu);
    design::menu_fertig();
    PostMessageW(hwnd, WM_NULL, 0, 0);
    let cmd = cmd.max(0) as usize;

    let mut gewechselt = false;
    let ergebnis = match cmd {
        ID_ENDE => {
            DestroyWindow(hwnd);
            return;
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
            APP.with(|c| c.borrow_mut().maus = None);
            Ok(())
        }
        ID_DESIGN => {
            design::setzen(schwarz == 0);
            Ok(())
        }
        ID_DPI_EIGEN => {
            if let Some(s) = &st {
                eingabe_zeigen(s.dpi, s.dpi_bereich, false);
            }
            return;
        }
        ID_VDPI_EIGEN => {
            if let Some(s) = &st {
                let sd = APP.with(|c| c.borrow().spiel_dpi);
                eingabe_zeigen(sd.or(s.dpi), s.dpi_bereich, true);
            }
            return;
        }
        ID_VDPI_AUS => {
            spiel_merken(Some(None), None);
            Ok(())
        }
        ID_VHZ_AUS => {
            spiel_merken(None, Some(None));
            Ok(())
        }
        c if (ID_VDPI..ID_VDPI + vdpi_liste.len()).contains(&c) => {
            spiel_merken(Some(Some(vdpi_liste[c - ID_VDPI])), None);
            Ok(())
        }
        c if (ID_VHZ..ID_VHZ + raten.len()).contains(&c) => {
            spiel_merken(None, Some(Some(raten[c - ID_VHZ])));
            Ok(())
        }
        c if (ID_DPI..ID_DPI + dpi_liste.len()).contains(&c) => {
            let v = dpi_liste[c - ID_DPI];
            let r = mit_maus(|m| m.set_dpi(v)).map(|_| ());
            gewechselt = r.is_ok();
            r
        }
        c if (ID_HZ..ID_HZ + raten.len()).contains(&c) => {
            let v = raten[c - ID_HZ];
            let r = mit_maus(|m| m.set_hz(v)).map(|name| {
                zustand::hz_merken(&name, v);
                APP.with(|c| c.borrow_mut().hz_gesetzt = Some(v));
            });
            gewechselt = r.is_ok();
            r
        }
        _ => return,
    };
    if let Err(e) = ergebnis {
        melden(hwnd, &e);
    }
    let st = tip_aktualisieren(hwnd, NIM_MODIFY);
    if gewechselt {
        einblenden(st);
    }
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
                let mut a = c.borrow_mut();
                if let Some(name) = a.maus().map(|m| m.name.clone()) {
                    a.hz_gesetzt = zustand::hz_lesen(&name);
                }
            });
            einblenden(tip_aktualisieren(hwnd, NIM_MODIFY));
            0
        }
        WM_INPUT => {
            LIVE.with(|l| {
                if let Some(l) = l.borrow_mut().as_mut() {
                    l.zaehler += 1;
                }
            });
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_TIMER if wp == TIMER_LIVE => {
            live_takt();
            0
        }
        WM_TIMER if wp == TIMER_VOLL => {
            let an = vollbild_erkannt();
            if spiel_wechseln(an) {
                let st = tip_aktualisieren(hwnd, NIM_MODIFY);
                // Beim Verlassen nur zeigen, wenn wirklich Werte zurückgestellt wurden
                let eigene = APP.with(|c| {
                    let a = c.borrow();
                    a.spiel_dpi.is_some() || a.spiel_hz.is_some()
                });
                if an || eigene {
                    einblenden(st);
                }
            }
            0
        }
        WM_TIMER if wp == TIMER_AKKU => {
            tip_aktualisieren(hwnd, NIM_MODIFY); // Akkustand im Tooltip frisch halten
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

/// Kleines Maus-Symbol (32x32) zeichnen.
unsafe fn icon_bauen() -> HICON {
    const N: usize = 32;
    let mut farbe = vec![0u8; N * N * 4]; // BGRA
    let mut maske = vec![0xFFu8; N * N / 8]; // 1 = durchsichtig
    for y in 0..N {
        for x in 0..N {
            let dx = (x as f32 - 15.5) / 10.5;
            let dy = (y as f32 - 15.5) / 15.0;
            let r2 = dx * dx + dy * dy;
            if r2 > 1.0 {
                continue;
            }
            let linie = ((x == 15 || x == 16) && y < 12) || y == 12;
            let (b, g, r) = if linie {
                (40, 40, 40)
            } else if r2 > 0.75 {
                (230, 230, 230)
            } else {
                (0xD8, 0xB4, 0x00)
            };
            let i = (y * N + x) * 4;
            farbe[i..i + 4].copy_from_slice(&[b, g, r, 255]);
            maske[(y * N + x) / 8] &= !(0x80 >> (x % 8));
        }
    }
    CreateIcon(GetModuleHandleW(null()), N as i32, N as i32, 1, 32, maske.as_ptr(), farbe.as_ptr())
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
        let hinst = GetModuleHandleW(null());
        ICON.set(icon_bauen());
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
        SetTimer(hwnd, TIMER_AKKU, 15 * 60 * 1000, None);
        SetTimer(hwnd, TIMER_VOLL, 2000, None);
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
