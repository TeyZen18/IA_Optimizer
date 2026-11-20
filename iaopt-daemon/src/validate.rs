//! Validación de parámetros antes de escribir en el kernel.
//! Regla: nada se escribe si el valor no es numérico, está vacío o se sale del
//! rango seguro del nodo (o no pertenece a la lista de valores permitidos).

use iaopt_core::sysfs::Knob;
use std::path::Path;

/// Resultado de validar un valor para un nodo.
pub fn check(knob: &Knob, value: &str) -> Result<(), String> {
    let v = value.trim();
    if v.is_empty() {
        return Err("valor vacio".into());
    }
    let path = Path::new(&knob.path);
    if !path.exists() {
        return Err("el nodo no existe en este dispositivo".into());
    }
    if !knob.writable {
        return Err("el nodo no es escribible".into());
    }
    // Enum explicito (gobernador, planificador, congestión, compresor…)
    if !knob.allowed.is_empty() {
        if !knob.allowed.iter().any(|a| a == v) {
            return Err(format!(
                "valor no permitido; opciones: {}",
                knob.allowed.join(", ")
            ));
        }
        return Ok(());
    }
    // Numero dentro del rango seguro
    let n: i64 = v
        .parse()
        .map_err(|_| "el valor debe ser numerico".to_string())?;
    if let Some(lo) = knob.min {
        if n < lo {
            return Err(format!("por debajo del minimo seguro ({})", lo));
        }
    }
    if let Some(hi) = knob.max {
        if n > hi {
            return Err(format!("por encima del maximo seguro ({})", hi));
        }
    }
    if knob.min.is_none() && knob.max.is_none() {
        // Sin rango conocido: solo aceptamos numeros no negativos razonables.
        if n < 0 {
            return Err("valor negativo no permitido".into());
        }
    }
    Ok(())
}

/// Valida usando el escaneo actual del daemon.
pub fn check_scanned(knobs: &[Knob], path: &str, value: &str) -> Result<(), String> {
    let k = knobs
        .iter()
        .find(|k| k.path == path)
        .ok_or_else(|| "nodo desconocido; reescanea controles".to_string())?;
    check(k, value)
}
