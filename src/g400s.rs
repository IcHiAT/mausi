//! Logitech G400s (046D:C24C): älteres, nicht veröffentlichtes Protokoll über Feature-Reports
//! an Schnittstelle 1. Quelle: github.com/rprichard/logitech-g400-config (Issue 3, an der G400s geprüft).
//!
//! Report 0x20 = Abfragerate (0 = 1000, 1 = 500, 2 = 250, 3 = 125 Hz), lesen und schreiben.
//! Report 0x8E = DPI-Stufe. Bit 0x80 = „Treibermodus“ (DPI-Tasten als normale Tasten).
//! DPI setzen geht nur mit einem Hersteller-Control-Transfer (0x40, Anfrage 2, Wert 0x8E,
//! Index = Stufe), den der Windows-HID-Treiber nicht durchlässt. Dafür richtet Mausi auf Wunsch
//! WinUSB an Schnittstelle 1 ein (siehe winusb.rs); ohne WinUSB lassen sich die DPI nur lesen.

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

/// Weg zur Einstell-Schnittstelle: HID (nur Hz) oder WinUSB (Hz und DPI)
pub enum Zugang {
    Hid(HidDevice),
    #[cfg(windows)]
    Usb(crate::winusb::Usb),
}

pub fn schnittstelle(i: &DeviceInfo) -> bool {
    i.vendor_id() == VID && i.product_id() == PID && i.interface_number() == 1
}

impl Zugang {
    fn lesen(&self, report: u8) -> Result<u8, String> {
        let mut buf = [report, 0];
        let n = match self {
            Zugang::Hid(d) => d.get_feature_report(&mut buf).map_err(|_| "Maus antwortet nicht".to_string())?,
            // GET_REPORT (Feature) an Schnittstelle 1
            #[cfg(windows)]
            Zugang::Usb(u) => u.steuern(0xA1, 0x01, 0x0300 | report as u16, 1, &mut buf)?,
        };
        if n < 2 || buf[0] != report {
            return Err("Maus antwortet nicht".into());
        }
        Ok(buf[1])
    }

    fn schreiben(&self, report: u8, wert: u8) -> Result<(), String> {
        match self {
            Zugang::Hid(d) => d.send_feature_report(&[report, wert]).map_err(|e| format!("Senden fehlgeschlagen: {e}")),
            // SET_REPORT (Feature) an Schnittstelle 1
            #[cfg(windows)]
            Zugang::Usb(u) => u.steuern(0x21, 0x09, 0x0300 | report as u16, 1, &mut [report, wert]).map(|_| ()),
        }
    }

    /// Steckt die G400s noch und antwortet?
    pub fn da(&self) -> bool {
        self.lesen(REPORT_RATE).is_ok()
    }

    /// Kann Mausi über diesen Zugang die DPI setzen?
    pub fn dpi_einstellbar(&self) -> bool {
        !matches!(self, Zugang::Hid(_))
    }

    pub fn dpi(&self) -> Result<u32, String> {
        let s = self.lesen(REPORT_DPI)? & 0x7F;
        DPI_STUFEN.iter().find(|(k, _)| *k == s).map(|&(_, d)| d).ok_or(format!("DPI-Stufe {s} unbekannt"))
    }

    pub fn set_dpi(&self, ziel: u32) -> Result<(), String> {
        let Some(&(stufe, _)) = DPI_STUFEN.iter().find(|(_, d)| *d == ziel) else {
            return Err(format!("{ziel} DPI kann die G400s nicht, möglich: 400, 800, 2000, 4000"));
        };
        match self {
            Zugang::Hid(_) => return Err("Die DPI der G400s lassen sich erst nach „DPI einstellbar machen …“ setzen".into()),
            #[cfg(windows)]
            Zugang::Usb(u) => {
                // Treibermodus-Bit beibehalten
                let modus = self.lesen(REPORT_DPI).map(|v| v & 0x80).unwrap_or(0);
                u.steuern(0x40, 0x02, REPORT_DPI as u16, (stufe | modus) as u16, &mut [])?;
            }
        }
        match self.dpi()? {
            d if d == ziel => Ok(()),
            d => Err(format!("G400s steht weiter auf {d} DPI")),
        }
    }

    pub fn hz(&self) -> Result<u32, String> {
        let r = self.lesen(REPORT_RATE)?;
        RATEN.get(r as usize).copied().ok_or(format!("unbekannte Rate {r}"))
    }

    pub fn set_hz(&self, ziel: u32) -> Result<(), String> {
        let i = RATEN.iter().position(|&r| r == ziel).ok_or(format!("{ziel} Hz kann die G400s nicht, möglich: 125, 250, 500, 1000"))?;
        self.schreiben(REPORT_RATE, i as u8)?;
        match self.hz()? {
            h if h == ziel => Ok(()),
            h => Err(format!("G400s steht weiter auf {h} Hz")),
        }
    }
}

/// Die festen DPI-Stufen, aufsteigend
pub fn dpi_stufen() -> Vec<u32> {
    DPI_STUFEN.iter().map(|&(_, d)| d).collect()
}

pub fn raten() -> Vec<u32> {
    let mut v = RATEN.to_vec();
    v.sort();
    v
}
