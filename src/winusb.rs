//! WinUSB für die Einstell-Schnittstelle der G400s (USB\VID_046D&PID_C24C&MI_01).
//!
//! Die DPI der G400s setzt ein USB-Control-Transfer, den der Windows-HID-Treiber nicht durchlässt.
//! `einrichten` hängt deshalb den in Windows eingebauten Treiber winusb.sys (winusb.inf, von Microsoft
//! signiert) an diese eine Schnittstelle, so wie es der Geräte-Manager über „Treiber aus Liste
//! auswählen“ tut. Bewegung und Tasten laufen über Schnittstelle 0 und bleiben beim HID-Treiber.
//! `entfernen` löscht den Geräteeintrag; Windows richtet die Schnittstelle dann wieder als HID ein.
//! Beides braucht Administratorrechte.

use std::ptr::{null, null_mut};
use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
use windows_sys::Win32::Devices::Usb::*;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows_sys::Win32::System::IO::OVERLAPPED;
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::Registry::{RegCloseKey, RegSetValueExW, REG_MULTI_SZ};

/// Schnittstellen-GUID, unter der Mausi die G400s per WinUSB wiederfindet (fest, nur für Mausi).
const MAUSI_GUID: GUID = GUID::from_u128(0x583ae582_f8d9_410c_b179_a9f09eb5127e);
const MAUSI_GUID_TEXT: &str = "{583AE582-F8D9-410C-B179-A9F09EB5127E}";
/// Geräteklasse „USBDevice“ aus winusb.inf
const KLASSE_USBDEVICE: &str = "{88BAE032-5A81-49f0-BC3D-A4FF138216D6}";
/// Kennung der Einstell-Schnittstelle (Teil der Geräteinstanz-ID)
const KENNUNG: &str = "VID_046D&PID_C24C&MI_01";

fn breit(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn text(w: &[u16]) -> String {
    let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..n])
}

/// Geräteliste, wird beim Verlassen freigegeben
struct Liste(HDEVINFO);

impl Drop for Liste {
    fn drop(&mut self) {
        unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

/// Offene G400s-Schnittstelle über WinUSB
pub struct Usb {
    datei: HANDLE,
    h: WINUSB_INTERFACE_HANDLE,
    /// Ereignis für überlappte Übertragungen: der Thread schläft, bis die Maus antwortet
    ereignis: HANDLE,
}

impl Drop for Usb {
    fn drop(&mut self) {
        unsafe {
            WinUsb_Free(self.h);
            CloseHandle(self.datei);
            CloseHandle(self.ereignis);
        }
    }
}

/// Pfade aller angeschlossenen G400s, die Mausi per WinUSB erreicht
pub fn pfade() -> Vec<String> {
    let mut v = Vec::new();
    unsafe {
        let set = SetupDiGetClassDevsW(&MAUSI_GUID, null(), null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
        if set == -1 {
            return v;
        }
        let _l = Liste(set);
        for i in 0.. {
            let mut ifd: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
            ifd.cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set, null(), &MAUSI_GUID, i, &mut ifd) == 0 {
                break;
            }
            let mut buf = vec![0u32; 512]; // 2 KB, 4-Byte-ausgerichtet
            let det = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*det).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &ifd, det, 2048, null_mut(), null_mut()) != 0 {
                let p = std::ptr::addr_of!((*det).DevicePath) as *const u16;
                v.push(text(std::slice::from_raw_parts(p, (2048 - 4) / 2)));
            }
        }
    }
    v
}

impl Usb {
    pub fn oeffnen(pfad: &str) -> Option<Usb> {
        unsafe {
            let datei = CreateFileW(
                breit(pfad).as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            );
            if datei == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut h: WINUSB_INTERFACE_HANDLE = null_mut();
            if WinUsb_Initialize(datei, &mut h) == 0 {
                CloseHandle(datei);
                return None;
            }
            let ms: u32 = 500;
            WinUsb_SetPipePolicy(h, 0, PIPE_TRANSFER_TIMEOUT, 4, (&ms as *const u32).cast());
            let ereignis = CreateEventW(null(), 0, 0, null());
            Some(Usb { datei, h, ereignis })
        }
    }

    /// Control-Transfer; liefert die Zahl der übertragenen Bytes
    pub fn steuern(&self, typ: u8, anfrage: u8, wert: u16, index: u16, daten: &mut [u8]) -> Result<usize, String> {
        let paket = WINUSB_SETUP_PACKET { RequestType: typ, Request: anfrage, Value: wert, Index: index, Length: daten.len() as u16 };
        let mut n = 0u32;
        unsafe {
            let mut ov: OVERLAPPED = std::mem::zeroed();
            ov.hEvent = self.ereignis;
            if WinUsb_ControlTransfer(self.h, paket, daten.as_mut_ptr(), daten.len() as u32, null_mut(), &mut ov) == 0
                && GetLastError() != ERROR_IO_PENDING
            {
                return Err(format!("USB-Übertragung fehlgeschlagen (Fehler {})", GetLastError()));
            }
            // höchstens 500 ms warten, dann abbrechen
            if WaitForSingleObject(self.ereignis, 500) != WAIT_OBJECT_0 {
                WinUsb_AbortPipe(self.h, 0);
                WinUsb_GetOverlappedResult(self.h, &ov, &mut n, 1);
                return Err("Maus antwortet nicht".into());
            }
            if WinUsb_GetOverlappedResult(self.h, &ov, &mut n, 0) == 0 {
                return Err(format!("USB-Übertragung fehlgeschlagen (Fehler {})", GetLastError()));
            }
        }
        Ok(n as usize)
    }

    /// Alle Endpunkte der Schnittstelle: (Adresse, Typ, Paketgröße, Intervall)
    pub fn endpunkte(&self) -> Vec<(u8, i32, u16, u8)> {
        let mut v = Vec::new();
        unsafe {
            let mut d: USB_INTERFACE_DESCRIPTOR = std::mem::zeroed();
            if WinUsb_QueryInterfaceSettings(self.h, 0, &mut d) == 0 {
                return v;
            }
            for i in 0..d.bNumEndpoints {
                let mut pi: WINUSB_PIPE_INFORMATION = std::mem::zeroed();
                if WinUsb_QueryPipe(self.h, 0, i, &mut pi) != 0 {
                    v.push((pi.PipeId, pi.PipeType, pi.MaximumPacketSize, pi.Interval));
                }
            }
        }
        v
    }

    /// Vom Endpunkt lesen und dabei höchstens `ms` warten; Ok(0) = nichts gekommen
    pub fn warten(&self, ep: u8, buf: &mut [u8], ms: u32) -> Result<usize, String> {
        unsafe {
            WinUsb_SetPipePolicy(self.h, ep, PIPE_TRANSFER_TIMEOUT, 4, (&ms as *const u32).cast());
            let mut n = 0u32;
            if WinUsb_ReadPipe(self.h, ep, buf.as_mut_ptr(), buf.len() as u32, &mut n, null()) == 0 {
                let e = GetLastError();
                return if e == 121 { Ok(0) } else { Err(format!("Lesen fehlgeschlagen (Fehler {e})")) };
            }
            Ok(n as usize)
        }
    }
}

// ---------- Einrichten und Entfernen (Administrator) ----------

/// Die Einstell-Schnittstelle der G400s suchen: (Liste, Eintrag, Instanz-ID)
unsafe fn schnittstelle_finden() -> Result<(Liste, SP_DEVINFO_DATA, String), String> {
    let set = SetupDiGetClassDevsW(null(), breit("USB").as_ptr(), null_mut(), DIGCF_ALLCLASSES | DIGCF_PRESENT);
    if set == -1 {
        return Err("Geräteliste nicht lesbar".into());
    }
    let l = Liste(set);
    for i in 0.. {
        let mut d: SP_DEVINFO_DATA = std::mem::zeroed();
        d.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
        if SetupDiEnumDeviceInfo(set, i, &mut d) == 0 {
            break;
        }
        let mut id = [0u16; 512];
        if SetupDiGetDeviceInstanceIdW(set, &d, id.as_mut_ptr(), 512, null_mut()) != 0 {
            let id = text(&id);
            if id.to_uppercase().contains(KENNUNG) {
                return Ok((l, d, id));
            }
        }
    }
    Err("G400s nicht gefunden (angesteckt?)".into())
}

/// Instanz-ID der Einstell-Schnittstelle, falls die G400s steckt
pub fn instanz() -> Option<String> {
    unsafe { schnittstelle_finden().ok().map(|(_, _, id)| id) }
}

fn letzter_fehler(was: &str) -> String {
    format!("{was} fehlgeschlagen (Fehler {})", unsafe { GetLastError() as i32 })
}

/// winusb.sys an die Einstell-Schnittstelle der G400s hängen.
pub fn einrichten() -> Result<(), String> {
    if !pfade().is_empty() {
        return Ok(()); // schon eingerichtet
    }
    unsafe {
        let (l, mut d, id) = schnittstelle_finden()?;
        let set = l.0;

        // 1. Schnittstellen-GUID hinterlegen, damit winusb.sys die Schnittstelle unter Mausis GUID meldet
        let schl = SetupDiCreateDevRegKeyW(set, &d, DICS_FLAG_GLOBAL, 0, DIREG_DEV, null(), null());
        if schl as isize == -1 {
            return Err(letzter_fehler("Registry-Schlüssel anlegen"));
        }
        let wert: Vec<u16> = MAUSI_GUID_TEXT.encode_utf16().chain([0, 0]).collect();
        let r = RegSetValueExW(schl, breit("DeviceInterfaceGUIDs").as_ptr(), 0, REG_MULTI_SZ, wert.as_ptr().cast(), (wert.len() * 2) as u32);
        RegCloseKey(schl);
        if r != 0 {
            return Err(format!("DeviceInterfaceGUIDs schreiben fehlgeschlagen (Fehler {r})"));
        }

        // 2. Geräteklasse auf USBDevice stellen, sonst bietet Windows winusb.inf nicht an
        let klasse = breit(KLASSE_USBDEVICE);
        if SetupDiSetDeviceRegistryPropertyW(set, &mut d, SPDRP_CLASSGUID, klasse.as_ptr().cast(), (klasse.len() * 2) as u32) == 0 {
            return Err(letzter_fehler("Geräteklasse ändern"));
        }

        // 3. Nur winusb.inf durchsuchen, auch Treiber, die sonst nicht zur Auswahl stehen
        let mut p: SP_DEVINSTALL_PARAMS_W = std::mem::zeroed();
        p.cbSize = std::mem::size_of::<SP_DEVINSTALL_PARAMS_W>() as u32;
        if SetupDiGetDeviceInstallParamsW(set, &d, &mut p) == 0 {
            return Err(letzter_fehler("Installationsparameter lesen"));
        }
        let inf = breit(&format!("{}\\INF\\winusb.inf", std::env::var("SystemRoot").unwrap_or("C:\\Windows".into())));
        p.DriverPath[..inf.len()].copy_from_slice(&inf);
        p.Flags |= DI_ENUMSINGLEINF;
        p.FlagsEx |= DI_FLAGSEX_ALLOWEXCLUDEDDRVS;
        if SetupDiSetDeviceInstallParamsW(set, &d, &p) == 0 {
            return Err(letzter_fehler("Installationsparameter setzen"));
        }

        // 4. Treiberliste bauen und den Eintrag „WinUsb-Gerät“ (Abschnitt WINUSB) wählen
        if SetupDiBuildDriverInfoList(set, &mut d, SPDIT_CLASSDRIVER) == 0 {
            return Err(letzter_fehler("Treiberliste bauen"));
        }
        let mut gewaehlt: Option<SP_DRVINFO_DATA_V2_W> = None;
        for i in 0.. {
            let mut drv: SP_DRVINFO_DATA_V2_W = std::mem::zeroed();
            drv.cbSize = std::mem::size_of::<SP_DRVINFO_DATA_V2_W>() as u32;
            if SetupDiEnumDriverInfoW(set, &d, SPDIT_CLASSDRIVER, i, &mut drv) == 0 {
                break;
            }
            let mut buf = vec![0u64; 1024];
            let det = buf.as_mut_ptr() as *mut SP_DRVINFO_DETAIL_DATA_W;
            (*det).cbSize = std::mem::size_of::<SP_DRVINFO_DETAIL_DATA_W>() as u32;
            SetupDiGetDriverInfoDetailW(set, &d, &drv, det, 8192, null_mut());
            if text(&(*det).SectionName).eq_ignore_ascii_case("WINUSB") {
                gewaehlt = Some(drv);
                break;
            }
        }
        let Some(drv) = gewaehlt else {
            return Err("WinUSB-Treiber nicht gefunden".into());
        };

        // 5. Installieren
        let mut neustart = 0;
        if DiInstallDevice(null_mut(), set, &d, &drv, 0, &mut neustart) == 0 {
            return Err(letzter_fehler("Treiber installieren"));
        }
        drop(l);

        // Meldet sich die Schnittstelle nicht, einmal neu starten (liest DeviceInterfaceGUIDs neu)
        for versuch in 0..2 {
            for _ in 0..20 {
                if !pfade().is_empty() {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            if versuch == 0 {
                let _ = pnputil(&["/restart-device", &id]);
            }
        }
        Err(if neustart != 0 {
            "WinUSB ist installiert, aber Windows braucht einen Neustart (ein anderes Programm, z. B. die Logitech-Software, hält die Maus offen). Alternativ die Maus einmal ab- und anstecken.".into()
        } else {
            "WinUSB ist installiert, aber die Schnittstelle meldet sich nicht. Maus einmal ab- und anstecken.".into()
        })
    }
}

/// WinUSB wieder entfernen: Geräteeintrag löschen, Windows richtet die Schnittstelle neu als HID ein.
pub fn entfernen() -> Result<(), String> {
    let id = instanz().ok_or("G400s nicht gefunden (angesteckt?)")?;
    pnputil(&["/remove-device", &id])?;
    pnputil(&["/scan-devices"])?;
    Ok(())
}

fn pnputil(args: &[&str]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("pnputil")
        .args(args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("pnputil nicht startbar: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("pnputil {} fehlgeschlagen: {}", args[0], String::from_utf8_lossy(&out.stdout).trim()))
    }
}

// ---------- Mit Administratorrechten neu starten ----------

pub fn ist_admin() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

/// Datei, in die der Admin-Lauf seine Fehlermeldung schreibt (das Tray hat keine Konsole)
fn meldungsdatei() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Mausi").join("winusb.txt"))
}

pub fn meldung_schreiben(t: &str) {
    if let Some(p) = meldungsdatei() {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, t);
    }
}

/// Startet diese exe mit `args` als Administrator (UAC-Abfrage) und wartet auf das Ende.
pub fn als_admin(args: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = breit(&exe.to_string_lossy());
    let verb = breit("runas");
    let par = breit(args);
    if let Some(p) = meldungsdatei() {
        let _ = std::fs::remove_file(p);
    }
    unsafe {
        let mut i: SHELLEXECUTEINFOW = std::mem::zeroed();
        i.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        i.fMask = SEE_MASK_NOCLOSEPROCESS;
        i.lpVerb = verb.as_ptr();
        i.lpFile = exe.as_ptr();
        i.lpParameters = par.as_ptr();
        i.nShow = 0;
        if ShellExecuteExW(&mut i) == 0 {
            return Err("Administratorrechte abgelehnt".into());
        }
        WaitForSingleObject(i.hProcess, INFINITE);
        let mut code = 1u32;
        GetExitCodeProcess(i.hProcess, &mut code);
        CloseHandle(i.hProcess);
        if code == 0 {
            return Ok(());
        }
    }
    let t = meldungsdatei().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    Err(if t.is_empty() { "Einrichtung fehlgeschlagen".into() } else { t })
}
