//! FPS real del display: contadores de frames por capa de SurfaceFlinger.
//!
//! En kernels modernos (Android 16) ni gfxinfo de juegos Vulkan ni
//! --timestats/--latency exponen frames por aplicación. Pero el volcado de
//! SurfaceFlinger sí mantiene un contador acumulativo `frame=N` por capa:
//! el delta entre dos lecturas ES el número de frames presentados.
//!
//! Se usa una ventana deslizante (~1,5 s) de muestras (t, contador) por
//! paquete: así varios consumidores (HUD, panel, grabadora) pueden sondeo
//! a ritmos distintos sin robarse el delta entre sí. Guard de stale: sin
//! muestra reciente o contador reiniciado, se devuelve 0 (no se inventa).

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn slot() -> &'static Mutex<HashMap<String, VecDeque<(u64, u64)>>> {
    static S: OnceLock<Mutex<HashMap<String, VecDeque<(u64, u64)>>> > = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Mayor contador `frame=N` entre las capas del paquete (la SurfaceView del
/// juego es la que presenta los frames renderizados).
pub fn read_counter(pkg: &str) -> Option<u64> {
    let (c, _layers) = read_counter_dbg(pkg);
    c
}

/// Como read_counter, pero devuelve también cuántas capas del paquete vio
/// (diagnóstico: distingue "no hay capas" de "no hay contador").
pub fn read_counter_dbg(pkg: &str) -> (Option<u64>, usize) {
    if pkg.is_empty() {
        return (None, 0);
    }
    let out = match std::process::Command::new("dumpsys")
        .arg("SurfaceFlinger")
        .output()
    {
        Ok(o) => o,
        Err(_) => return (None, 0),
    };
    let s = String::from_utf8_lossy(&out.stdout);
    let mut max_c: Option<u64> = None;
    let mut layers = 0usize;
    let mut lines = s.lines().peekable();
    while let Some(l) = lines.next() {
        if l.contains("Layer [") && l.contains(pkg) {
            layers += 1;
            if let Some(next) = lines.peek() {
                if let Some(pos) = next.find("frame=") {
                    let num: String = next[pos + 6..]
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    if let Ok(v) = num.parse::<u64>() {
                        max_c = Some(max_c.map(|m| m.max(v)).unwrap_or(v));
                    }
                }
            }
        }
    }
    (max_c, layers)
}

/// FPS del paquete sobre la ventana deslizante. 0 = sin dato fiable.
pub fn fps_value(pkg: &str) -> i32 {
    match read_counter(pkg) {
        Some(c) => fps_from_counter(pkg, c),
        None => 0,
    }
}

/// Núcleo del cálculo: alimenta el historial con una lectura ya hecha y
/// devuelve el FPS sobre la ventana (evita un segundo volcado por petición).
pub fn fps_from_counter(pkg: &str, c: u64) -> i32 {
    let t = now_ms();
    let Ok(mut map) = slot().lock() else { return 0 };
    let h = map.entry(pkg.to_string()).or_default();
    h.push_back((t, c));
    while h.len() > 12 {
        h.pop_front();
    }
    while let Some(front) = h.front() {
        if t.saturating_sub(front.0) > 12000 {
            h.pop_front();
        } else {
            break;
        }
    }
    // Referencia: la muestra más reciente con edad entre 0,5 y 8 s.
    // (Nunca la muestra actual: daría dt=0. El panel in-game sondea cada
    // ~3-6 s según lo que tarde el ciclo; la ventana debe cubrir ese ritmo.)
    let mut reference: Option<(u64, u64)> = None;
    for &(tt, cc) in h.iter() {
        let age = t.saturating_sub(tt);
        if age >= 500 && age <= 8000 {
            reference = Some((tt, cc));
        }
    }
    let Some((rt, rc)) = reference else { return 0 };
    let dt = t.saturating_sub(rt);
    if c < rc {
        return 0; // contador reiniciado (capa nueva): sin dato aún
    }
    (((c - rc) * 1000) / dt).min(240) as i32
}

/// GET /api/fps?package=X (vacío = app en primer plano).
pub fn json(pkg: &str) -> String {
    let pkg = if pkg.is_empty() {
        crate::read_foreground_app().unwrap_or_default()
    } else {
        pkg.to_string()
    };
    let (counter, layers) = read_counter_dbg(&pkg);
    let fps = counter.map(|c| fps_from_counter(&pkg, c)).unwrap_or(0);
    format!(
        "{{\"fps\":{},\"epoch_ms\":{},\"package\":\"{}\",\"source\":\"surfaceflinger\",\"layers\":{},\"counter\":{}}}",
        fps,
        now_ms(),
        pkg.replace('"', ""),
        layers,
        counter.map(|v| v.to_string()).unwrap_or_else(|| "null".into())
    )
}
