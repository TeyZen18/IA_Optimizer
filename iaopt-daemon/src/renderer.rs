//! Motor de renderizado (HWUI): lee el actual y permite elegir entre los
//! motores conocidos. Requiere root para `setprop`; algunos cambios piden
//! reiniciar la app o el sistema.

use std::process::Command;

pub const OPTIONS: &[&str] = &["OpenGL", "SkiaGL", "skiagl", "skiavk", "meow"];

fn getprop(prop: &str) -> Option<String> {
    let out = Command::new("getprop").arg(prop).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

pub fn current() -> String {
    getprop("debug.hwui.renderer")
        .or_else(|| getprop("ro.hardware.egl"))
        .or_else(|| getprop("ro.hardware.vulkan"))
        .unwrap_or_else(|| "DEFAULT".into())
}

pub fn status_json() -> String {
    let cur = current();
    let opts: Vec<String> = OPTIONS
        .iter()
        .map(|o| {
            format!(
                "{{\"name\":\"{}\",\"active\":{}}}",
                o,
                if *o == cur { "true" } else { "false" }
            )
        })
        .collect();
    format!(
        "{{\"current\":{},\"options\":[{}]}}",
        jstr(&cur),
        opts.join(",")
    )
}

pub fn set(value: &str) -> (bool, String) {
    if !OPTIONS.contains(&value) {
        return (false, "motor de renderizado no reconocido".into());
    }
    let out = Command::new("setprop")
        .args(["debug.hwui.renderer", value])
        .output();
    if out.is_err() {
        return (false, "no se pudo ejecutar setprop".into());
    }
    // Persistir para el proximo arranque (lo aplica service.sh).
    let _ = Command::new("setprop")
        .args(["persist.sys.iaopt.renderer", value])
        .output();
    let now = current();
    if now == value {
        (
            true,
            format!("motor de renderizado: {} (reinicia apps para aplicarlo)", value),
        )
    } else {
        (
            false,
            format!("no se aplico (actual: {}). Algunos equipos piden reinicio", now),
        )
    }
}

fn jstr(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
