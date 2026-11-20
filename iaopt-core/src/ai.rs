//! Núcleo de la IA de IA Motor (v0.7), escrito por nosotros.
//!
/// Conceptos: objetivo medible (satisfaccion), umbral, planificador de un solo
/// cambio ("el más pequeño que sirve") y memoria de qué cambios ayudaron.
/// No copia ni deriva de ningun otro proyecto.

use crate::engine::Metrics;
use crate::profiles::{self, Action};
use crate::sysfs::Knob;
use crate::{Features, Profile};

/// Objetivo del motor: por encima de esta satisfaccion no interviene.
pub const TARGET: f32 = 0.75;

/// Objetivo según el comportamiento de intervención elegido por el usuario.
/// Calma = solo actúa ante brechas grandes (objetivo alto); Decisivo = actúa
/// antes, mientras el error sigue siendo de bajo coste (objetivo bajo).
pub fn target_for(behavior: &str) -> f32 {
    match behavior {
        "calma" => 0.85,
        "decisivo" => 0.65,
        _ => TARGET,
    }
}

/// Satisfaccion 0..1: 1 = frio y sin carga. Baja con calor y uso.
pub fn satisfaction(m: &Metrics) -> f32 {
    let heat = ((m.battery_temp_c - 35.0) / 15.0).clamp(0.0, 1.0);
    let load = m.cpu_load.clamp(0.0, 1.0);
    (1.0 - 0.6 * heat - 0.2 * load).clamp(0.0, 1.0)
}

/// Frase de estado para la WebUI.
pub fn objective_line(m: &Metrics) -> String {
    let s = satisfaction(m);
    let heat = ((m.battery_temp_c - 35.0) / 15.0).clamp(0.0, 1.0);
    let verdict = if s >= TARGET { "dentro del objetivo" } else { "por debajo del objetivo" };
    format!(
        "satisfaccion {:.3} · objetivo {:.2} · calor {:.0}% · {}",
        s,
        TARGET,
        heat * 100.0,
        verdict
    )
}

fn rank(path: &str) -> u8 {
    if path.ends_with("scaling_governor") {
        0
    } else if path.ends_with("scaling_max_freq") {
        1
    } else if path.ends_with("scaling_min_freq") {
        2
    } else if path.contains("queue/scheduler") {
        3
    } else {
        4
    }
}

/// Un unico cambio: el de mayor efecto entre los permitidos.
/// `avoid` sirve para no repetir el ultimo camino intentado.
pub fn plan_single(
    profile: Profile,
    knobs: &[Knob],
    features: Features,
    avoid: Option<&str>,
) -> Option<Action> {
    profiles::plan(profile, knobs)
        .into_iter()
        .filter(|a| a.path != "-")
        .filter(|a| profiles::action_enabled(features, a))
        .filter(|a| avoid.map(|p| p != a.path).unwrap_or(true))
        .min_by_key(|a| rank(&a.path))
}

/// Tipo de control para la memoria de aprendizaje.
pub fn kind_of(path: &str) -> &'static str {
    if path.ends_with("scaling_governor") {
        "governor"
    } else if path.ends_with("scaling_max_freq") || path.ends_with("scaling_min_freq") {
        "freq"
    } else if path.contains("queue/scheduler") {
        "scheduler"
    } else {
        "otro"
    }
}
