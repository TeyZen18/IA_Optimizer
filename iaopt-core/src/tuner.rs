//! Aplicacion de ajustes con verificacion por relectura y rollback automatico.
//!
/// Flujo de cada accion:
///   1. leer valor actual (before)
///   2. escribir el nuevo valor
///   3. releer y comparar
///   4. si no se sostuvo, restaurar `before` y marcar el fallo

use crate::profiles::Action;
use crate::sysfs::{read_trim, write_verify};
use std::path::Path;
use std::sync::Mutex;

/// Aviso de un nodo al que el kernel no deja acceder.
#[derive(Debug, Clone)]
pub struct Warning {
    pub path: String,
    pub kind: String,
    pub detail: String,
}

static WARNINGS: Mutex<Vec<Warning>> = Mutex::new(Vec::new());

pub fn push_warning(path: &str, kind: &str, detail: &str) {
    if let Ok(mut w) = WARNINGS.lock() {
        if w.iter().any(|x| x.path == path) {
            return;
        }
        w.push(Warning {
            path: path.to_string(),
            kind: kind.to_string(),
            detail: detail.to_string(),
        });
        w.truncate(30);
    }
}

pub fn drain_warnings() -> Vec<Warning> {
    WARNINGS
        .lock()
        .map(|mut w| std::mem::take(&mut *w))
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct Report {
    pub path: String,
    pub requested: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub applied: bool,
    pub rolled_back: bool,
    pub note: String,
}

pub fn apply(action: &Action, probe_root: &Path) -> Report {
    let path = probe_root.join(action.path.trim_start_matches('/'));
    let before = read_trim(&path);

    // Un nodo fijado en 444 no aceptaria el write: se desbloquea primero.
    crate::sysfs::unlock(&path);
    if !crate::sysfs::is_writable(&path) {
        push_warning(
            &action.path,
            "no escribible",
            "el kernel no permite modificar este parámetro",
        );
        return Report {
            path: action.path.clone(),
            requested: action.value.clone(),
            before,
            after: None,
            applied: false,
            rolled_back: false,
            note: "ruta no escribible".into(),
        };
    }

    // Con `pin` el nodo queda en solo lectura si el valor se sostuvo: es la
    // forma de que el daemon de potencia del vendor no nos lo revierta.
    let ok = crate::sysfs::write_verify_pin(&path, &action.value, action.pin);
    let after = read_trim(&path);
    let mut rolled_back = false;
    let mut note = if ok { "verificado".to_string() } else { "no se sostuvo".to_string() };

    if !ok {
        push_warning(
            &action.path,
            "sin efecto",
            "el valor no se sostuvo al leerlo de vuelta; el kernel lo rechazó",
        );
        // rollback al valor activo (en nodos enum, el que estaba entre corchetes)
        if let Some(b) = before.as_ref() {
            // Los nodos de texto con cifra ("Performance(Sports) mode") solo
            // aceptan la cifra al restaurar.
            let target = if action.path.ends_with("cpufreq_power_mode") {
                crate::sysfs::power_mode_value(b).unwrap_or_else(|| crate::sysfs::active_token(b))
            } else {
                crate::sysfs::active_token(b)
            };
            crate::sysfs::unlock(&path);
            rolled_back = write_verify(&path, &target);
            note = if rolled_back {
                "fallo; restaurado valor previo".to_string()
            } else {
                "fallo y no se pudo restaurar".to_string()
            };
        }
    }

    Report {
        path: action.path.clone(),
        requested: action.value.clone(),
        before,
        after,
        applied: ok,
        rolled_back,
        note,
    }
}

/// Restaura una lista de pares (ruta, valor) - usada al desinstalar o volver a stock.
pub fn restore(items: &[(String, String)], probe_root: &Path) -> Vec<Report> {
    items
        .iter()
        .map(|(p, v)| {
            let a = Action {
                path: p.clone(),
                value: v.clone(),
                why: "restauracion".into(),
                pin: false,
            };
            apply(&a, probe_root)
        })
        .collect()
}
