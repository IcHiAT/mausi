//! Schwarzes Design (selbst gezeichnete Menüs, Eingabefenster) und die DPI/Hz-Einblendung.
//! Native Dark-Menüs sind nur dunkelgrau, deshalb werden die Einträge per Owner-Draw gemalt.

use crate::tray::{schrift, w};
use crate::zustand;
use std::cell::{Cell, RefCell};
use std::ptr::{null, null_mut};
use std::time::Instant;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const SCHLUESSEL: &str = "design_schwarz";

// Farben als COLORREF (0x00BBGGRR)
const HG: u32 = 0x000000;
const HG_HOVER: u32 = 0x2A2A2A;
const HG_FELD: u32 = 0x1C1C1C;
const TEXT: u32 = 0xF2F2F2;
const TEXT_GRAU: u32 = 0x8C8C8C;
const LINIE: u32 = 0x3A3A3A;
const AKZENT: u32 = 0xD8B400; // Farbe aus dem Tray-Symbol

const HELL_HG: u32 = 0xFFFFFF;
const HELL_TEXT: u32 = 0x1A1A1A;
const HELL_GRAU: u32 = 0x707070;
const HELL_LINIE: u32 = 0xC8C8C8;

pub const KLASSE_OSD: &str = "MausOsd";
const OSD_B: i32 = 150;
const OSD_H: i32 = 82;
const OSD_ALPHA: f32 = 240.0;
const OSD_EIN_MS: f32 = 220.0;
const OSD_HALTEN_MS: f32 = 2600.0;
const OSD_AUS_MS: f32 = 450.0;
const OSD_TIMER: usize = 1;

struct MenuText {
    text: Vec<u16>,
    untermenu: bool,
}

thread_local! {
    static SCHWARZ: Cell<bool> = Cell::new(true);
    static PINSEL: RefCell<Vec<(u32, HBRUSH)>> = RefCell::new(Vec::new());
    static MENU_TEXTE: RefCell<Vec<MenuText>> = RefCell::new(Vec::new());
    static OSD: Cell<HWND> = Cell::new(null_mut());
    static OSD_START: Cell<Option<Instant>> = Cell::new(None);
    static OSD_AKTUELL: Cell<f32> = Cell::new(0.0);
    static OSD_WERTE: RefCell<(String, String)> = RefCell::new((String::new(), String::new()));
    static FONT_GROSS: Cell<HFONT> = Cell::new(null_mut());
    static FONT_KLEIN: Cell<HFONT> = Cell::new(null_mut());
}

pub fn schwarz() -> bool {
    SCHWARZ.get()
}

fn pinsel(farbe: u32) -> HBRUSH {
    PINSEL.with(|p| {
        let mut p = p.borrow_mut();
        if let Some(&(_, b)) = p.iter().find(|(f, _)| *f == farbe) {
            return b;
        }
        let b = unsafe { CreateSolidBrush(farbe) };
        p.push((farbe, b));
        b
    })
}

/// Gespeicherte Wahl laden (Standard: schwarz) und anwenden.
pub unsafe fn laden() {
    SCHWARZ.set(zustand::lesen(SCHLUESSEL) != Some(0));
    menue_rahmen();
}

pub unsafe fn setzen(an: bool) {
    SCHWARZ.set(an);
    zustand::merken(SCHLUESSEL, Some(an as u32));
    menue_rahmen();
}

/// Macht den Menürahmen dunkel. Dafür gibt es keine offizielle API, nur diese seit
/// Windows 10 1903 stabilen uxtheme-Ordinale (135 SetPreferredAppMode, 136 FlushMenuThemes).
unsafe fn menue_rahmen() {
    let lib = LoadLibraryW(w("uxtheme.dll").as_ptr());
    if lib.is_null() {
        return;
    }
    if let Some(f) = GetProcAddress(lib, 135usize as *const u8) {
        let f: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
        f(if schwarz() { 2 } else { 0 }); // 2 = ForceDark, 0 = Default
    }
    if let Some(f) = GetProcAddress(lib, 136usize as *const u8) {
        let f: unsafe extern "system" fn() = std::mem::transmute(f);
        f();
    }
}

// ---------- Menü ----------

/// Alle Einträge (auch Untermenüs) auf Owner-Draw umstellen und den Hintergrund schwarz machen.
pub unsafe fn menu_schwarz(menu: HMENU) {
    MENU_TEXTE.with(|t| t.borrow_mut().clear());
    menu_umstellen(menu);
    let mut mi: MENUINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MENUINFO>() as u32;
    mi.fMask = MIM_BACKGROUND | MIM_APPLYTOSUBMENUS;
    mi.hbrBack = pinsel(HG);
    SetMenuInfo(menu, &mi);
}

unsafe fn menu_umstellen(menu: HMENU) {
    for i in 0..GetMenuItemCount(menu) {
        let mut buf = [0u16; 160];
        let mut mii: MENUITEMINFOW = std::mem::zeroed();
        mii.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
        mii.fMask = MIIM_FTYPE | MIIM_STRING | MIIM_SUBMENU;
        mii.dwTypeData = buf.as_mut_ptr();
        mii.cch = buf.len() as u32;
        if GetMenuItemInfoW(menu, i as u32, 1, &mut mii) == 0 {
            continue;
        }
        let trenner = mii.fType & MFT_SEPARATOR != 0;
        let untermenu = !mii.hSubMenu.is_null();
        if untermenu {
            menu_umstellen(mii.hSubMenu);
        }
        let mut neu: MENUITEMINFOW = std::mem::zeroed();
        neu.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
        neu.fMask = MIIM_FTYPE | MIIM_DATA;
        neu.fType = MFT_OWNERDRAW;
        if trenner {
            // Trenner ohne Text; deaktiviert, damit er nicht auswählbar ist
            neu.fMask |= MIIM_STATE;
            neu.fState = MFS_DISABLED;
        } else {
            let text = buf[..mii.cch as usize].to_vec();
            neu.dwItemData = MENU_TEXTE.with(|t| {
                let mut t = t.borrow_mut();
                t.push(MenuText { text, untermenu });
                t.len()
            });
        }
        SetMenuItemInfoW(menu, i as u32, 1, &neu);
    }
}

pub fn menu_fertig() {
    MENU_TEXTE.with(|t| t.borrow_mut().clear());
}

/// Text eines selbst gezeichneten Eintrags ändern. false = Eintrag ist nativ.
pub unsafe fn menu_text_setzen(menu: HMENU, id: u32, text: &str) -> bool {
    let mut mii: MENUITEMINFOW = std::mem::zeroed();
    mii.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
    mii.fMask = MIIM_DATA;
    if GetMenuItemInfoW(menu, id, 0, &mut mii) == 0 || mii.dwItemData == 0 {
        return false;
    }
    MENU_TEXTE.with(|t| {
        if let Some(e) = t.borrow_mut().get_mut(mii.dwItemData - 1) {
            e.text = text.encode_utf16().collect();
        }
    });
    true
}

pub unsafe fn menu_messen(hwnd: HWND, lp: LPARAM) -> bool {
    let m = &mut *(lp as *mut MEASUREITEMSTRUCT);
    if m.CtlType != ODT_MENU {
        return false;
    }
    if m.itemData == 0 {
        m.itemWidth = 0;
        m.itemHeight = 9;
        return true;
    }
    let mut r: RECT = std::mem::zeroed();
    let dc = GetDC(hwnd);
    let alt = SelectObject(dc, schrift() as HGDIOBJ);
    MENU_TEXTE.with(|t| {
        if let Some(e) = t.borrow().get(m.itemData - 1) {
            DrawTextW(dc, e.text.as_ptr(), e.text.len() as i32, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
        }
    });
    SelectObject(dc, alt);
    ReleaseDC(hwnd, dc);
    m.itemWidth = r.right as u32 + 56;
    m.itemHeight = 28;
    true
}

pub unsafe fn menu_malen(lp: LPARAM) -> bool {
    let d = &*(lp as *const DRAWITEMSTRUCT);
    if d.CtlType != ODT_MENU {
        return false;
    }
    let r = d.rcItem;
    if d.itemData == 0 {
        FillRect(d.hDC, &r, pinsel(HG));
        let mitte = (r.top + r.bottom) / 2;
        let l = RECT { left: r.left + 8, top: mitte, right: r.right - 8, bottom: mitte + 1 };
        FillRect(d.hDC, &l, pinsel(LINIE));
        ExcludeClipRect(d.hDC, r.left, r.top, r.right, r.bottom);
        return true;
    }
    let grau = d.itemState & (ODS_GRAYED | ODS_DISABLED) != 0;
    let aktiv = d.itemState & ODS_SELECTED != 0 && !grau;
    FillRect(d.hDC, &r, pinsel(if aktiv { HG_HOVER } else { HG }));
    SetBkMode(d.hDC, TRANSPARENT as _);
    SetTextColor(d.hDC, if grau { TEXT_GRAU } else { TEXT });
    let alt = SelectObject(d.hDC, schrift() as HGDIOBJ);
    let fmt = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
    if d.itemState & ODS_CHECKED != 0 {
        let mut f = RECT { left: r.left + 8, top: r.top, right: r.left + 28, bottom: r.bottom };
        SetTextColor(d.hDC, AKZENT);
        DrawTextW(d.hDC, w("✓").as_ptr(), -1, &mut f, fmt);
        SetTextColor(d.hDC, if grau { TEXT_GRAU } else { TEXT });
    }
    MENU_TEXTE.with(|t| {
        if let Some(e) = t.borrow().get(d.itemData - 1) {
            let mut f = RECT { left: r.left + 30, top: r.top, right: r.right - 22, bottom: r.bottom };
            DrawTextW(d.hDC, e.text.as_ptr(), e.text.len() as i32, &mut f, fmt);
            if e.untermenu {
                let mut f = RECT { left: r.right - 20, top: r.top, right: r.right - 6, bottom: r.bottom };
                DrawTextW(d.hDC, w("›").as_ptr(), -1, &mut f, fmt);
            }
        }
    });
    SelectObject(d.hDC, alt);
    // Sonst malt Windows seinen eigenen (schwarzen) Untermenü-Pfeil darüber
    ExcludeClipRect(d.hDC, r.left, r.top, r.right, r.bottom);
    true
}

// ---------- Eingabefenster und Akku-Hinweis ----------

/// Titelleiste und Rahmen schwarz (Windows 11).
pub unsafe fn fenster_schwarz(hwnd: HWND) {
    let dunkel: i32 = 1;
    DwmSetWindowAttribute(hwnd, 20, (&dunkel as *const i32).cast(), 4); // USE_IMMERSIVE_DARK_MODE
    DwmSetWindowAttribute(hwnd, 34, (&LINIE as *const u32).cast(), 4); // BORDER_COLOR
    DwmSetWindowAttribute(hwnd, 35, (&HG as *const u32).cast(), 4); // CAPTION_COLOR
}

pub unsafe fn hintergrund_malen(hwnd: HWND, dc: HDC) {
    let mut r: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut r);
    FillRect(dc, &r, pinsel(HG));
}

pub unsafe fn kind_farbe(dc: HDC, feld: bool) -> LRESULT {
    let hg = if feld { HG_FELD } else { HG };
    SetTextColor(dc, TEXT);
    SetBkColor(dc, hg);
    pinsel(hg) as LRESULT
}

pub unsafe fn knopf_malen(lp: LPARAM) {
    let d = &*(lp as *const DRAWITEMSTRUCT);
    let mut r = d.rcItem;
    let gedrueckt = d.itemState & ODS_SELECTED != 0;
    FillRect(d.hDC, &r, pinsel(if gedrueckt { LINIE } else { HG_HOVER }));
    FrameRect(d.hDC, &r, pinsel(if d.itemState & ODS_FOCUS != 0 { AKZENT } else { LINIE }));
    let mut buf = [0u16; 32];
    let n = GetWindowTextW(d.hwndItem, buf.as_mut_ptr(), buf.len() as i32);
    SetBkMode(d.hDC, TRANSPARENT as _);
    SetTextColor(d.hDC, TEXT);
    let alt = SelectObject(d.hDC, schrift() as HGDIOBJ);
    DrawTextW(d.hDC, buf.as_ptr(), n, &mut r, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
    SelectObject(d.hDC, alt);
}

// ---------- DPI/Hz-Einblendung ----------
// Rechts am Bildschirmrand, vertikal mittig. Nimmt weder Fokus noch Klicks, stört also kein Spiel.

unsafe fn font(cache: &'static std::thread::LocalKey<Cell<HFONT>>, hoehe: i32, gewicht: i32) -> HFONT {
    if cache.get().is_null() {
        cache.set(CreateFontW(-hoehe, 0, 0, 0, gewicht, 0, 0, 0, 1, 0, 0, 5, 0, w("Segoe UI").as_ptr()));
    }
    cache.get()
}

pub unsafe fn osd_zeigen(dpi: Option<u32>, hz: Option<u32>, im_spiel: bool) {
    let zahl = |v: Option<u32>| v.map(|x| x.to_string()).unwrap_or("–".into());
    OSD_WERTE.with(|o| *o.borrow_mut() = (zahl(dpi), zahl(hz)));

    let mut hwnd = OSD.get();
    if hwnd.is_null() {
        hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w(KLASSE_OSD).as_ptr(),
            w("Mausi").as_ptr(),
            WS_POPUP,
            0, 0, OSD_B, OSD_H,
            null_mut(), null_mut(), GetModuleHandleW(null()), null(),
        );
        OSD.set(hwnd);
    }

    // Monitor des Spiels, sonst der mit dem Mauszeiger (das Tray-Fenster selbst ist unsichtbar)
    let monitor = if im_spiel {
        MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST)
    } else {
        let mut p = POINT { x: 0, y: 0 };
        GetCursorPos(&mut p);
        MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST)
    };
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(monitor, &mut mi);
    let m = mi.rcMonitor;

    // Läuft die Einblendung noch, geht es von der aktuellen Helligkeit aus weiter
    let schon = OSD_AKTUELL.get();
    let bisher = std::time::Duration::from_secs_f32(schon * OSD_EIN_MS / 1000.0);
    OSD_START.set(Some(Instant::now() - bisher));
    SetLayeredWindowAttributes(hwnd, 0, (schon * OSD_ALPHA) as u8, LWA_ALPHA);
    SetWindowPos(
        hwnd, HWND_TOPMOST,
        m.right - OSD_B, (m.top + m.bottom - OSD_H) / 2, OSD_B, OSD_H,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    InvalidateRect(hwnd, null(), 0);
    UpdateWindow(hwnd);
    SetTimer(hwnd, OSD_TIMER, 15, None);
}

unsafe fn osd_takt(hwnd: HWND) {
    let Some(start) = OSD_START.get() else { return };
    let t = start.elapsed().as_secs_f32() * 1000.0;
    let h = if t < OSD_EIN_MS {
        t / OSD_EIN_MS
    } else if t < OSD_EIN_MS + OSD_HALTEN_MS {
        1.0
    } else {
        1.0 - (t - OSD_EIN_MS - OSD_HALTEN_MS) / OSD_AUS_MS
    };
    if h <= 0.0 {
        KillTimer(hwnd, OSD_TIMER);
        ShowWindow(hwnd, SW_HIDE);
        OSD_START.set(None);
        OSD_AKTUELL.set(0.0);
        return;
    }
    OSD_AKTUELL.set(h);
    SetLayeredWindowAttributes(hwnd, 0, (h * OSD_ALPHA) as u8, LWA_ALPHA);
}

unsafe fn osd_malen(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let (hg, text, grau, linie) =
        if schwarz() { (HG, TEXT, TEXT_GRAU, LINIE) } else { (HELL_HG, HELL_TEXT, HELL_GRAU, HELL_LINIE) };
    let r = RECT { left: 0, top: 0, right: OSD_B, bottom: OSD_H };
    FillRect(dc, &r, pinsel(hg));
    FrameRect(dc, &r, pinsel(linie));
    let balken = RECT { left: 0, top: 0, right: 3, bottom: OSD_H };
    FillRect(dc, &balken, pinsel(AKZENT));
    SetBkMode(dc, TRANSPARENT as _);

    let gross = font(&FONT_GROSS, 26, 600);
    let klein = font(&FONT_KLEIN, 14, 400);
    let alt = SelectObject(dc, gross as HGDIOBJ);
    let (dpi, hz) = OSD_WERTE.with(|o| o.borrow().clone());
    for (i, (wert, einheit)) in [(dpi, "DPI"), (hz, "Hz")].iter().enumerate() {
        let oben = 7 + i as i32 * 34;
        let unten = oben + 34;
        SelectObject(dc, gross as HGDIOBJ);
        SetTextColor(dc, text);
        let mut f = RECT { left: 10, top: oben, right: 104, bottom: unten };
        DrawTextW(dc, w(wert).as_ptr(), -1, &mut f, DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_NOPREFIX);
        SelectObject(dc, klein as HGDIOBJ);
        SetTextColor(dc, grau);
        let mut f = RECT { left: 110, top: oben + 3, right: OSD_B - 4, bottom: unten };
        DrawTextW(dc, w(einheit).as_ptr(), -1, &mut f, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX);
    }
    SelectObject(dc, alt);
    EndPaint(hwnd, &ps);
}

pub unsafe extern "system" fn osd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            osd_takt(hwnd);
            0
        }
        WM_PAINT => {
            osd_malen(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_NCHITTEST => -1, // HTTRANSPARENT
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
