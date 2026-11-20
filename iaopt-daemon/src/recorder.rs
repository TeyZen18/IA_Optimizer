//! Grabadora de sesiones de juego: muestras de 1 s (FPS real + telemetría)
//! a CSV, y resumen JSON con media, mínimo, peor 1 % (low1) y segundos de
//! jank. Honestidad de granularidad: este kernel no expone tiempos por
//! frame, así que low1/jank se calculan sobre muestras de 1 s y se declara.
//!
//! - Sesiones < 30 s se descartan (too_short).
//! - Retención: 30 sesiones (se borra la más antigua, CSV + JSON).
//! - Auto-parada a los 30 min o si el juego deja el primer plano > 10 s.

use crate::state;
use std::fs;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DIR: &str = "/data/adb/ia_optimizer/sessions";
const MAX_SESSIONS: usize = 30;
const MAX_DURATION: u64 = 1800;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn jstr(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

struct Rec {
    pkg: String,
    start: u64,
    csv: String,
    fps_samples: Vec<i32>,
    refresh: f32,
    frames_start: Option<u64>,
    last_counter: Option<u64>,
    away: u64,
}

fn slot() -> &'static Mutex<Option<Rec>> {
    static S: OnceLock<Mutex<Option<Rec>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

/// Hilo de 1 Hz: muestrea mientras haya sesión activa.
pub fn run() {
    // CSV huérfanos de un crash (sin resumen JSON): sesión incompleta, se
    // descarta al arrancar para no acumular basura.
    if let Ok(rd) = fs::read_dir(DIR) {
        for e in rd.filter_map(|e| e.ok()) {
            let name = e.file_name().into_string().unwrap_or_default();
            if name.ends_with(".csv") && !fs::exists(format!("{}/{}", DIR, name.replace(".csv", ".json"))).unwrap_or(false) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(1));
        let Ok(mut g) = slot().lock() else { continue };
        let Some(rec) = g.as_mut() else { continue };

        // El juego dejó el primer plano: auto-parada tras 10 s.
        let fg = crate::read_foreground_app().unwrap_or_default();
        if fg != rec.pkg {
            rec.away += 1;
            if rec.away > 10 {
                finalize(rec);
                *g = None;
                continue;
            }
        } else {
            rec.away = 0;
        }

        let elapsed = now().saturating_sub(rec.start);
        if elapsed > MAX_DURATION {
            finalize(rec);
            *g = None;
            continue;
        }

        let fps = match crate::fps::read_counter(&rec.pkg) {
            Some(c) => {
                rec.last_counter = Some(c);
                crate::fps::fps_from_counter(&rec.pkg, c)
            }
            None => 0,
        };
        let tel = crate::telemetry::sample(false);
        let cpu_temp = tel.thermal_zones.iter().map(|z| z.2).max().unwrap_or(-1);
        let refresh = tel.refresh_hz.unwrap_or(rec.refresh);
        rec.refresh = refresh;
        rec.fps_samples.push(fps);
        let row = format!(
            "{},{},{:.0},{},{},{:.0},{:.0},{},{}\n",
            now(),
            fps,
            refresh,
            cpu_temp,
            tel.bat_temp_c.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "-1".into()),
            tel.cpu_cur_mhz.unwrap_or(0),
            crate::state::cpu_pct().unwrap_or(0),
            tel.bat_current_ma.unwrap_or(-1),
            tel.bat_pct.unwrap_or(0),
        );
        if let Ok(mut f) = fs::OpenOptions::new().append(true).open(&rec.csv) {
            let _ = f.write_all(row.as_bytes());
        }
    });
}

/// POST /api/recorder {action: start|stop, package?}.
pub fn handle(body: &str) -> String {
    let action = state::get_json_str(body, "action").unwrap_or_default();
    let Ok(mut g) = slot().lock() else {
        return "{\"ok\":false}".into();
    };
    match action.as_str() {
        "start" => {
            if g.is_some() {
                return "{\"ok\":false,\"detail\":\"ya hay una sesión en curso\"}".into();
            }
            let pkg = state::get_json_str(body, "package")
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| crate::read_foreground_app().unwrap_or_default());
            if pkg.is_empty() {
                return "{\"ok\":false,\"detail\":\"no hay app en primer plano\"}".into();
            }
            let _ = fs::create_dir_all(DIR);
            let csv = format!("{}/{}_{}.csv", DIR, pkg.replace('.', "_"), now());
            if fs::write(&csv, "t,fps,refresh_hz,cpu_temp_c,bat_temp_c,cpu_mhz,cpu_load_pct,ma,bat_pct\n").is_err() {
                return "{\"ok\":false,\"detail\":\"no se pudo crear el archivo\"}".into();
            }
            *g = Some(Rec {
                pkg: pkg.clone(),
                start: now(),
                csv,
                fps_samples: Vec::new(),
                refresh: 60.0,
                frames_start: crate::fps::read_counter(&pkg),
                last_counter: None,
                away: 0,
            });
            crate::journal::append_note("sesion", &format!("grabación de sesión iniciada ({})", pkg));
            "{\"ok\":true,\"recording\":true}".into()
        }
        "stop" => {
            match g.as_mut() {
                Some(rec) => {
                    finalize(rec);
                    *g = None;
                    "{\"ok\":true,\"recording\":false}".into()
                }
                None => "{\"ok\":false,\"detail\":\"no hay sesión en curso\"}".into(),
            }
        }
        _ => "{\"ok\":false,\"detail\":\"accion desconocida\"}".into(),
    }
}

/// Cierra la sesión: resumen JSON + retención. < 30 s se descarta.
fn finalize(rec: &Rec) {
    let duration = now().saturating_sub(rec.start);
    let json_path = rec.csv.replace(".csv", ".json");
    if duration < 30 || rec.fps_samples.is_empty() {
        let _ = fs::remove_file(&rec.csv);
        crate::journal::append_note("sesion", "sesión descartada por corta (<30 s)");
        return;
    }
    let n = rec.fps_samples.len();
    let sum: i64 = rec.fps_samples.iter().map(|v| *v as i64).sum();
    let avg = sum as f32 / n as f32;
    let min = *rec.fps_samples.iter().min().unwrap();
    let max = *rec.fps_samples.iter().max().unwrap();
    // low1: media del peor 1 % de las muestras (mín. 100 muestras; si no, null).
    let low1 = if n >= 100 {
        let mut s = rec.fps_samples.clone();
        s.sort_unstable();
        let k = (n / 100).max(1);
        let worst: i64 = s[..k].iter().map(|v| *v as i64).sum();
        format!("{:.1}", worst as f32 / k as f32)
    } else {
        "null".into()
    };
    // jank: segundos con FPS por debajo de la mitad del refresco (y > 0).
    let half = rec.refresh / 2.0;
    let jank = rec.fps_samples.iter().filter(|v| **v > 0 && (**v as f32) < half).count();
    let frames_total = rec.last_counter
        .zip(rec.frames_start)
        .map(|(a, b)| a.saturating_sub(b))
        .map(|v| v.to_string())
        .unwrap_or_else(|| "null".into());
    let body = format!(
        "{{\"package\":{},\"t\":{},\"duration_s\":{},\"samples\":{},\"avg_fps\":{:.1},\"min_fps\":{},\"max_fps\":{},\"low1_fps\":{},\"jank_s\":{},\"refresh_hz\":{:.0},\"frames_total\":{},\"granularity\":\"1s\",\"csv\":{}}}",
        jstr(&rec.pkg), rec.start, duration, n, avg, min, max, low1, jank, rec.refresh,
        frames_total, jstr(&rec.csv)
    );
    let _ = fs::write(&json_path, body);
    crate::journal::append_note(
        "sesion",
        &format!("sesión guardada: {} s, media {:.1} FPS, jank {} s", duration, avg, jank),
    );
    retention();
}

/// Conserva las 30 sesiones más recientes (CSV + JSON).
fn retention() {
    let Ok(rd) = fs::read_dir(DIR) else { return };
    let mut files: Vec<(String, u64)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !name.ends_with(".csv") { return None; }
            let t = e.metadata().ok()?.modified().ok()?
                .duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some((name, t))
        })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    for (name, _) in files.iter().skip(MAX_SESSIONS) {
        let _ = fs::remove_file(format!("{}/{}", DIR, name));
        let _ = fs::remove_file(format!("{}/{}", DIR, name.replace(".csv", ".json")));
    }
}

/// GET /api/recorder: estado + listado de sesiones con su resumen.
pub fn status() -> String {
    let g = slot().lock().ok();
    let (recording, pkg, elapsed) = match g.as_deref().and_then(|x| x.as_ref()) {
        Some(r) => (true, r.pkg.clone(), now().saturating_sub(r.start)),
        None => (false, String::new(), 0),
    };
    let mut sessions: Vec<(u64, String)> = Vec::new();
    if let Ok(rd) = fs::read_dir(DIR) {
        for e in rd.filter_map(|e| e.ok()) {
            let name = e.file_name().into_string().unwrap_or_default();
            if !name.ends_with(".json") { continue; }
            if let Ok(s) = fs::read_to_string(e.path()) {
                let t = state::get_json_num(&s, "t").unwrap_or(0.0) as u64;
                let size_kb = fs::metadata(e.path()).map(|m| m.len() / 1024).unwrap_or(0);
                // El JSON ya es un objeto: se incrusta tal cual con el tamaño.
                let trimmed = s.trim();
                let with_size = format!(
                    "{},\"size_kb\":{}}}",
                    trimmed.trim_end_matches('}'),
                    size_kb
                );
                sessions.push((t, with_size));
            }
        }
    }
    sessions.sort_by(|a, b| b.0.cmp(&a.0));
    let items: Vec<String> = sessions.iter().take(MAX_SESSIONS).map(|(_, s)| s.clone()).collect();
    format!(
        "{{\"recording\":{},\"package\":{},\"elapsed_s\":{},\"sessions\":[{}]}}",
        recording, jstr(&pkg), elapsed, items.join(",")
    )
}
