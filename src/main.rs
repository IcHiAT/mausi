#![cfg_attr(windows, windows_subsystem = "windows")]
//! maus - DPI und Abfragerate von Logitech-Mäusen setzen.
//! Ohne Argumente: Tray-Symbol (Windows). Mit Argumenten: Kommandozeile.

mod g400s;
mod hidpp;
#[cfg(windows)]
mod design;
#[cfg(windows)]
mod startmenue;
#[cfg(windows)]
mod tray;

use hidpp::Maus;

const HILFE: &str = "maus - DPI und Abfragerate von Logitech-Mäusen

  maus              Tray-Symbol starten (Rechtsklick = Menü)
  maus status       Name, DPI, Hz und mögliche Werte anzeigen
  maus geraete      alle angeschlossenen Mäuse auflisten
  maus dpi 800      DPI setzen
  maus hz 1000      Abfragerate setzen
  maus hilfe        diese Hilfe";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    {
        if args.is_empty() {
            tray::starten();
            return;
        }
        // Programm läuft ohne eigenes Konsolenfenster: Ausgabe an die aufrufende Konsole hängen
        unsafe {
            use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
        println!();
    }
    let code = match cli(&args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Fehler: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn zahl(s: Option<&String>) -> Result<u32, String> {
    s.ok_or("Zahl fehlt")?.parse().map_err(|_| "keine gültige Zahl".to_string())
}

fn cli(a: &[String]) -> Result<(), String> {
    match a.first().map(String::as_str).unwrap_or("status") {
        "status" => {
            let m = Maus::finden()?;
            println!("Maus:  {}", m.name);
            if let Ok(d) = m.dpi() {
                let (min, max) = m.dpi_bereich().unwrap_or((0, 0));
                println!("DPI:   {d}  (möglich {min}-{max})");
            }
            if let Ok(r) = m.raten() {
                let l: Vec<String> = r.iter().map(|x| x.to_string()).collect();
                match (m.hz_lesewert_unsicher(), zustand::hz_lesen(&m.name)) {
                    (true, Some(h)) => println!("Hz:    {h}  (zuletzt von maus gesetzt)"),
                    _ => println!("Hz:    {}", m.hz().map(|h| h.to_string()).unwrap_or("?".into())),
                }
                println!("       möglich: {}", l.join(", "));
            }
            if let Ok((p, laedt)) = m.akku() {
                println!("Akku:  {p} %{}", if laedt { " (lädt)" } else { "" });
            }
            Ok(())
        }
        "dpi" => {
            let v = zahl(a.get(1))?;
            let m = Maus::finden()?;
            m.set_dpi(v)?;
            println!("{}: jetzt {} DPI", m.name, m.dpi()?);
            #[cfg(windows)]
            tray::einblendung_anstossen();
            Ok(())
        }
        "hz" => {
            let v = zahl(a.get(1))?;
            let m = Maus::finden()?;
            m.set_hz(v)?;
            zustand::hz_merken(&m.name, v);
            println!("{}: Abfragerate auf {v} Hz gesetzt", m.name);
            #[cfg(windows)]
            tray::einblendung_anstossen();
            Ok(())
        }
        "geraete" => {
            let t = std::time::Instant::now();
            let (mause, fremde) = hidpp::alle();
            for m in &mause {
                println!("{}  ({:04X}:{:04X})", m.name, m.vid, m.pid);
            }
            for f in &fremde {
                println!("{}  ({:04X}:{:04X}, nicht einstellbar)", f.name, f.vid, f.pid);
            }
            println!("Suche: {} ms", t.elapsed().as_millis());
            Ok(())
        }
        "liste" => {
            for z in hidpp::liste()? {
                println!("{z}");
            }
            Ok(())
        }
        _ => {
            println!("{HILFE}");
            Ok(())
        }
    }
}

/// Merkt sich die zuletzt gesetzte Hz-Zahl je Maus, weil manche Mäuse sie falsch zurückmelden.
pub mod zustand {
    use std::path::PathBuf;

    fn datei() -> Option<PathBuf> {
        let basis = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
        Some(basis.join("maus").join("zustand.txt"))
    }

    pub fn hz_lesen(name: &str) -> Option<u32> {
        lesen(name)
    }

    pub fn hz_merken(name: &str, hz: u32) {
        merken(name, Some(hz));
    }

    /// Zeile `schluessel=wert` lesen
    pub fn lesen(schluessel: &str) -> Option<u32> {
        let t = std::fs::read_to_string(datei()?).ok()?;
        let k = format!("{schluessel}=");
        t.lines().find_map(|l| l.strip_prefix(&k)?.trim().parse().ok())
    }

    /// Zeile `schluessel=wert` setzen, bei `None` entfernen
    pub fn merken(schluessel: &str, wert: Option<u32>) {
        let Some(p) = datei() else { return };
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let k = format!("{schluessel}=");
        let alt = std::fs::read_to_string(&p).unwrap_or_default();
        let mut z: Vec<String> = alt.lines().filter(|l| !l.starts_with(&k)).map(String::from).collect();
        if let Some(v) = wert {
            z.push(format!("{k}{v}"));
        }
        let _ = std::fs::write(p, z.join("\n"));
    }
}
