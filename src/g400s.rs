//! Logitech G400s (046D:C24C): älteres, nicht veröffentlichtes Protokoll über Feature-Reports
//! an Schnittstelle 1. Quelle: github.com/rprichard/logitech-g400-config (Issue 3, an der G400s geprüft).
//!
//! Report 0x20 = Abfragerate (0 = 1000, 1 = 500, 2 = 250, 3 = 125 Hz), lesen und schreiben.
//! Report 0x8E = DPI-Stufe, nur lesen: Schreiben braucht bei der G400s einen USB-Control-Transfer,
//! den der Windows-HID-Treiber nicht durchlässt. Bit 0x80 = „Treibermodus“ (DPI-Tasten als normale Tasten).

use hidapi::{DeviceInfo, HidDevice};

pub const VID: u16 = 0x046D;
pub const PID: u16 = 0xC24C;
pub const NAME: &str = "G400s";
const REPORT_RATE: u8 = 0x20;
const REPORT_DPI: u8 = 0x8E;
/// Abfragerate je Wert in Report 0x20
const RATEN: [u32; 4] = [1000, 500, 250, 125];
/// DPI je Stufe. Die DPI-Tasten schalten 7, 6, 4, 3 durch (Werkseinstellung 4000/2000/800/400);
/// Stufe 5 erreicht man nur über die Logitech-Software, ihr Wert ist nicht belegt.
const DPI_STUFEN: [(u8, u32); 4] = [(3, 400), (4, 800), (6, 2000), (7, 4000)];

pub fn schnittstelle(i: &DeviceInfo) -> bool {
    i.vendor_id() == VID && i.product_id() == PID && i.interface_number() == 1
}

fn lesen(dev: &HidDevice, report: u8) -> Result<u8, String> {
    let mut buf = [report, 0];
    let n = dev.get_feature_report(&mut buf).map_err(|_| "Maus antwortet nicht".to_string())?;
    if n < 2 {
        return Err("Maus antwortet nicht".into());
    }
    Ok(buf[1])
}

/// Steckt die G400s noch und antwortet?
pub fn da(dev: &HidDevice) -> bool {
    lesen(dev, REPORT_RATE).is_ok()
}

pub fn stufe(dev: &HidDevice) -> Result<u8, String> {
    lesen(dev, REPORT_DPI).map(|v| v & 0x7F)
}

pub fn dpi(dev: &HidDevice) -> Result<u32, String> {
    let s = stufe(dev)?;
    DPI_STUFEN.iter().find(|(k, _)| *k == s).map(|&(_, d)| d).ok_or(format!("DPI-Stufe {s} unbekannt"))
}

pub fn raten() -> Vec<u32> {
    let mut v = RATEN.to_vec();
    v.sort();
    v
}

pub fn hz(dev: &HidDevice) -> Result<u32, String> {
    let r = lesen(dev, REPORT_RATE)?;
    RATEN.get(r as usize).copied().ok_or(format!("unbekannte Rate {r}"))
}

pub fn set_hz(dev: &HidDevice, ziel: u32) -> Result<(), String> {
    let i = RATEN.iter().position(|&r| r == ziel).ok_or(format!("{ziel} Hz kann die G400s nicht, möglich: 125, 250, 500, 1000"))?;
    dev.send_feature_report(&[REPORT_RATE, i as u8]).map_err(|e| format!("Senden fehlgeschlagen: {e}"))?;
    match hz(dev)? {
        h if h == ziel => Ok(()),
        h => Err(format!("G400s steht weiter auf {h} Hz")),
    }
}
