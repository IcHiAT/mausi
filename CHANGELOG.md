# Changelog

## 0.2.1 (12.11.2026)

- Neu: Der Tooltip über dem Tray-Symbol zeigt in der ersten Zeile die Version (z. B. „Mausi 0.2.1“).
- Geändert: Die Einstellungen liegen jetzt unter `%LOCALAPPDATA%\Mausi` statt `%LOCALAPPDATA%\maus`. Der alte Ordner wird beim ersten Start übernommen. Als Programmordner empfiehlt das README jetzt `%LOCALAPPDATA%\Programs\Mausi`.
- Verbessert: Wird `Mausi.exe` verschoben, zeigen Autostart und Startmenü-Verknüpfung beim nächsten Start von selbst auf den neuen Ort.

## 0.2.0 (12.11.2026)

- Neu: Überarbeitete Icons. Das Tray-Symbol folgt der Windows-Taskleiste: Auf dunkler Taskleiste ist es cyan, auf heller schwarz, ohne erreichbare Maus hellgrau. Es passt sich der Bildschirmskalierung an. Das Programm-Icon (Startmenü, Fenster) ist hellgrau.
- Neu: Menüpunkt „Tray-Symbol“. Dort lässt sich das Symbol fest wählen, Standard ist „Automatisch“.
- Neu: Mehrere Mäuse. Jede einstellbare Maus bekommt im Tray-Menü ein eigenes Untermenü mit Live-Messung, Desktop- und Vollbild-Werten. Mäuse, die Mausi nicht einstellen kann, erscheinen mit eigener Hz-Messung. Bei nur einer Maus bleibt das Menü wie bisher.
- Neu: Mausi sucht Mäuse im Hintergrund und erneut, wenn ein Gerät angesteckt oder abgezogen wird. Der Kommandozeilenbefehl `maus geraete` listet alle gefundenen Mäuse.
- Behoben: Leere Menüeinträge und ein winziges leeres Untermenü, wenn während des Öffnens erneut auf das Tray-Symbol geklickt wurde.
- Neu: Logitech G400s. Mausi stellt ihre Abfragerate ein (125, 250, 500, 1000 Hz) und zeigt die DPI-Stufe an. Die DPI lassen sich bei ihr nur mit den Tasten an der Maus umschalten, weil Windows den nötigen USB-Befehl nicht durchlässt.
- Behoben: Eine schlafende Funkmaus am Empfänger wurde oft nicht gefunden, weil sie erst nach etwa einer Sekunde antwortet. Die Suche pingt jetzt alle Empfänger-Plätze zugleich an und dauert 2–4 s statt bis zu 14 s.
- Behoben: Die Live-Messung zählte alle Mäuse zusammen, jetzt misst sie jede Maus einzeln.
- Behoben: Schlief die Funkmaus beim Start von Mausi zu tief, fehlte sie, solange eine andere Maus angeschlossen war. Mausi sucht jetzt alle 30 s weiter, solange ein Empfänger ohne Maus da ist.
- Verbessert: Das Tray-Menü öffnet sofort. Mausi merkt sich die Werte der Mäuse und liest beim Öffnen nur die DPI kurz nach.
- Neu: Wird die DPI an der Maus umgeschaltet (z. B. DPI-Tasten der G400s), zeigt das offene Menü den neuen Wert innerhalb einer halben Sekunde.
- Verbessert: Das offene Menü flackert nicht mehr. Neu gezeichnet werden nur Zeilen, die sich ändern. Die gemessene Rate wird auf die übliche Stufe gerundet (z. B. 1000 Hz statt 996–1003 Hz).
- Behoben: „Maus meldet Fehler 2“ beim Ändern der Abfragerate. Die Maus meldet im Onboard-Modus auch Code 2, Mausi schaltet jetzt auch dann auf Host-Modus um.

## 0.1.1 (08.11.2026)

- Neu: Haken „Im Startmenü anzeigen“ im Tray-Menü. Er legt die Verknüpfung im Startmenü an oder entfernt sie.

## 0.1.0 (08.11.2026)

- Erste Veröffentlichung: DPI und Abfragerate von Logitech-Mäusen ohne G HUB.
