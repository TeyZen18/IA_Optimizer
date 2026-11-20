//! Presupuesto de calor sostenido: mide en ESTE dispositivo cuánto tarda una
//! sesión intensa en provocar la primera limitación térmica real.
//!
//! No se estima nada: el reloj arranca cuando la carga de CPU sostenida supera
//! el umbral de sesión intensa y se detiene en la primera intervención del
//! limitador térmico (intensidad > 5 %). El resultado se persiste en un archivo
//! atómico para que la app y la WebUI vean las mismas mediciones.

use std::fs;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const DIR: &str = "/data/adb/ia_optimizer";
const PATH: &str = "/data/adb/ia_optimizer/heat_budget.json";

/// Carga mínima para considerar la sesión "intensa" (0..1).
const HEAVY_LOAD: f32 = 0.65;
/// Por debajo de esta carga la sesión se da por terminada tras 30 s.
const COOL_LOAD: f32 = 0.30;
const IDLE_END_S: u64 = 30;
/// Intensidad del limitador que cuenta como primera caída de seguridad.
const LIMIT_ON: f32 = 0.05;

struct Track {
    /// Inicio de la sesión intensa en curso (epoch s).
    start: Option<u64>,
    /// Ya se midió la primera caída en esta sesión.
    recorded: bool,
    /// Inicio del periodo de baja carga (para cerrar la sesión).
    idle_since: Option<u64>,
    /// Sesiones intensas observadas desde que hay registro.
    sessions: u64,
    /// Últimas mediciones (segundos hasta la primera caída).
    samples: Vec<u64>,
    /// Última temperatura de control y su instante (para la tasa EMA).
    last_temp: Option<f32>,
    last_t: u64,
    /// Tasa de calentamiento EMA (°C/s), solo informativa por API.
    rate_ema: f32,
}

fn slot() -> &'static Mutex<Track> {
    static S: OnceLock<Mutex<Track>> = OnceLock::new();
    S.get_or_init(|| {
        let mut t = Track {
            start: None,
            recorded: false,
            idle_since: None,
            sessions: 0,
            samples: Vec::new(),
            last_temp: None,
            last_t: 0,
            rate_ema: 0.0,
        };
        // Recuperar el registro persistido (solo muestras y contador; una
        // sesión a medias no se retoma: el reloj debe ser continuo).
        if let Ok(s) = fs::read_to_string(PATH) {
            t.sessions = crate::state::get_json_num(&s, "sessions")
                .map(|v| v as u64)
                .unwrap_or(0);
            if let Some(arr) = s.split("\"samples\":[").nth(1) {
                let body = arr.split(']').next().unwrap_or("");
                for tok in body.split(',') {
                    if let Ok(v) = tok.trim().parse::<u64>() {
                        t.samples.push(v);
                    }
                }
            }
        }
        Mutex::new(t)
    })
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn persist(t: &Track) {
    let samples: Vec<String> = t.samples.iter().map(|v| v.to_string()).collect();
    let body = format!(
        "{{\"sessions\":{},\"samples\":[{}]}}",
        t.sessions,
        samples.join(",")
    );
    let _ = fs::create_dir_all(DIR);
    let tmp = format!("{}.tmp", PATH);
    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, PATH);
    }
}

/// Un tick del lazo térmico. `load` es la carga de CPU instantánea (0..1),
/// `limiting` indica si el limitador está actuando de verdad y `control_c` es
/// la temperatura de control fusionada (para la tasa de calentamiento EMA).
pub fn tick(load: f32, limiting: bool, control_c: f32) {
    let Ok(mut t) = slot().lock() else { return };
    let n = now();
    let mut changed = false;

    // Tasa de calentamiento EMA (°C/s): informativa por API; la medición del
    // presupuesto sigue siendo el reloj real, no esta extrapolación.
    if let Some(prev) = t.last_temp {
        let dt = n.saturating_sub(t.last_t);
        if dt >= 1 && dt <= 30 {
            let inst = (control_c - prev) / dt as f32;
            // Un salto >10 °C entre ticks es anomalía del sensor: se descarta.
            if (control_c - prev).abs() < 10.0 {
                t.rate_ema += 0.3 * (inst - t.rate_ema);
            }
        }
    }
    t.last_temp = Some(control_c);
    t.last_t = n;

    if load >= HEAVY_LOAD {
        t.idle_since = None;
        if t.start.is_none() {
            t.start = Some(n);
            t.recorded = false;
            t.sessions += 1;
            changed = true;
        }
        if limiting && !t.recorded {
            if let Some(start) = t.start {
                let budget = n.saturating_sub(start);
                // Una caída inmediata (<5 s) suele ser ruido del arranque de
                // la sesión: se descarta para no ensuciar la medición.
                if budget >= 5 {
                    t.samples.push(budget);
                    if t.samples.len() > 8 {
                        t.samples.remove(0);
                    }
                    t.recorded = true;
                    changed = true;
                }
            }
        }
    } else if load < COOL_LOAD {
        if t.idle_since.is_none() {
            t.idle_since = Some(n);
        }
        if let Some(idle) = t.idle_since {
            if n.saturating_sub(idle) >= IDLE_END_S && t.start.is_some() {
                t.start = None;
                t.recorded = false;
                changed = true;
            }
        }
    } else {
        t.idle_since = None;
    }

    if changed {
        persist(&t);
    }
}

/// JSON público: última medición, mejor, mediana y sesiones observadas.
/// `rate_c_s` y `predicted_s` son la extrapolación lineal amortiguada de la
/// tasa EMA hasta el umbral de protección (65 °C); `predicted_s` es null si
/// la temperatura es estable o baja (no se inventa un número).
pub fn json() -> String {
    let Ok(t) = slot().lock() else {
        return "{\"measured\":false,\"sessions\":0}".into();
    };
    let temp = t.last_temp.unwrap_or(0.0);
    let predicted = if t.rate_ema > 0.005 && in_range_below_threshold(temp) {
        let raw = (65.0 - temp) / t.rate_ema;
        // Amortiguación para horizontes largos: la tasa no se sostiene lineal.
        let damped = raw / (1.0 + raw * 0.002);
        Some((damped as u64).min(3600))
    } else {
        None
    };
    let pred_str = predicted.map(|v| v.to_string()).unwrap_or_else(|| "null".into());
    if t.samples.is_empty() {
        return format!(
            "{{\"measured\":false,\"sessions\":{},\"in_session\":{},\"rate_c_s\":{:.4},\"predicted_s\":{}}}",
            t.sessions,
            t.start.is_some(),
            t.rate_ema,
            pred_str
        );
    }
    let mut sorted = t.samples.clone();
    sorted.sort_unstable();
    let last = *t.samples.last().unwrap();
    let best = sorted[0];
    let median = sorted[sorted.len() / 2];
    format!(
        "{{\"measured\":true,\"sessions\":{},\"in_session\":{},\"last_s\":{},\"best_s\":{},\"median_s\":{},\"samples\":{},\"rate_c_s\":{:.4},\"predicted_s\":{}}}",
        t.sessions,
        t.start.is_some(),
        last,
        best,
        median,
        t.samples.len(),
        t.rate_ema,
        pred_str
    )
}

fn in_range_below_threshold(temp: f32) -> bool {
    temp > 0.0 && temp < 65.0
}
