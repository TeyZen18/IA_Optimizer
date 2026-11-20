//! Transiciones del modo automático.
//!
//! Al ACTIVARLO: se guarda una copia exacta del estado manual (perfil, techo
//! de frecuencia, modo juego y cada ajuste manual de nodos con su valor
//! original), se restauran los valores de fábrica y la IA toma el control.
//! Al DESACTIVARLO: se devuelve todo tal como estaba antes de activarlo,
//! sin quedarse enganchado en el último perfil que tocó la IA.

use crate::state::{self, State};
use crate::{journal, warnings};
use iaopt_core::profiles::Action;
use iaopt_core::sysfs;
use iaopt_core::{tuner, Profile};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

const DIR: &str = "/data/adb/ia_optimizer";

fn overrides_path() -> PathBuf {
    PathBuf::from(DIR).join("manual_overrides.json")
}
fn snapshot_path() -> PathBuf {
    PathBuf::from(DIR).join("manual_snapshot.json")
}

/// Un ajuste manual de un nodo: el valor original (antes de tocarlo) y el
/// valor que el usuario dejó aplicado.
#[derive(Debug, Clone)]
struct Override {
    path: String,
    original: String,
    current: String,
}

fn jstr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn overrides_json(list: &[Override]) -> String {
    let items: Vec<String> = list
        .iter()
        .map(|o| {
            format!(
                "{{\"path\":{},\"original\":{},\"current\":{}}}",
                jstr(&o.path),
                jstr(&o.original),
                jstr(&o.current)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Lee una lista de overrides escrita por nosotros (objetos planos).
/// Acepta tanto el fichero puro (`[...]`) como la cola de un snapshot.
fn parse_overrides(s: &str) -> Vec<Override> {
    let Some(i) = s.find('[') else { return Vec::new() };
    let Some(j) = s.rfind(']') else { return Vec::new() };
    if j <= i {
        return Vec::new();
    }
    let body = &s[i + 1..j];
    if body.trim().is_empty() {
        return Vec::new();
    }
    body.split("},")
        .filter_map(|chunk| {
            let path = state::get_json_str(chunk, "path")?;
            let original = state::get_json_str(chunk, "original").unwrap_or_default();
            let current = state::get_json_str(chunk, "current").unwrap_or_default();
            Some(Override {
                path,
                original,
                current,
            })
        })
        .collect()
}

fn read_overrides() -> Vec<Override> {
    fs::read_to_string(overrides_path())
        .map(|s| parse_overrides(&s))
        .unwrap_or_default()
}

fn write_overrides(list: &[Override]) {
    let _ = fs::create_dir_all(DIR);
    let _ = fs::write(overrides_path(), overrides_json(list));
}

/// Registra un ajuste manual del usuario: guarda el valor ORIGINAL la primera
/// vez (para poder volver a él) y el valor aplicado las siguientes.
/// En nodos enum la lectura es el volcado completo ("noop [deadline] cfq"):
/// se guarda el token activo, que es lo que el kernel acepta al escribir.
pub fn record_override(path: &str, before: Option<&str>, after: &str) {
    let after_t = sysfs::active_token(after);
    let original = before
        .map(|b| sysfs::active_token(b))
        .unwrap_or_else(|| after_t.clone());
    let mut list = read_overrides();
    if let Some(o) = list.iter_mut().find(|o| o.path == path) {
        o.current = after_t;
    } else {
        list.push(Override {
            path: path.to_string(),
            original,
            current: after_t,
        });
    }
    write_overrides(&list);
}

fn apply_value(st: &State, path: &str, value: &str, why: &str) -> bool {
    let a = Action {
        path: path.to_string(),
        value: value.to_string(),
        why: why.to_string(),
        pin: false,
    };
    tuner::apply(&a, &st.sys_root).applied
}

/// Restaura los límites de fábrica de frecuencia. El perfil balanceado no
/// sube un techo que estaba limitado, así que se hace explícitamente.
fn restore_factory_freqs(st: &State) {
    let knobs = st.probe().scan();
    for k in &knobs {
        if !k.writable {
            continue;
        }
        let leaf = k.path.rsplit('/').next().unwrap_or("");
        let factory_leaf = match leaf {
            "scaling_max_freq" => "cpuinfo_max_freq",
            "scaling_min_freq" => "cpuinfo_min_freq",
            _ => continue,
        };
        let factory_rel = k.path.replace(leaf, factory_leaf);
        let factory = sysfs::read_trim(&st.sys_root.join(factory_rel.trim_start_matches('/')));
        let cur = k.current.as_deref().unwrap_or("");
        if let Some(f) = factory {
            if f != cur {
                apply_value(st, &k.path, &f, "modo automático: límite de fábrica restaurado");
            }
        }
    }
}

/// Activa/desactiva el modo automático con transición completa.
/// Devuelve el JSON de respuesta para la API y el CLI.
pub fn set_auto(st: &State, en: bool) -> String {
    let was = st.auto.load(Ordering::Relaxed);
    if en == was {
        return format!("{{\"ok\":true,\"auto\":{},\"changed\":false}}", en);
    }
    st.auto.store(en, Ordering::Relaxed);
    let detail;
    if en {
        // 1) Copia exacta del estado manual del usuario.
        let p = *st.profile.lock().unwrap();
        let ov = read_overrides();
        let snap = format!(
            "{{\"profile\":\"{}\",\"freq_cap\":{:.2},\"gaming\":{},\"overrides\":{}}}",
            p.as_str(),
            st.user_cap(),
            st.gaming.load(Ordering::Relaxed),
            overrides_json(&ov)
        );
        let _ = fs::create_dir_all(DIR);
        let _ = fs::write(snapshot_path(), snap);
        // 2) Deshacer cada ajuste manual (vuelve el valor original).
        for o in &ov {
            apply_value(
                st,
                &o.path,
                &sysfs::active_token(&o.original),
                "modo automático: ajuste manual deshecho",
            );
        }
        write_overrides(&[]);
        // 3) Límites de fábrica + perfil base sin techo; la IA decide desde aquí.
        restore_factory_freqs(st);
        st.set_user_cap(1.0);
        st.gaming.store(false, Ordering::Relaxed);
        if let Ok(mut g) = st.perapp.lock() {
            *g = None;
        }
        if let Ok(mut g) = st.perapp_prev.lock() {
            *g = None;
        }
        st.apply_profile(Profile::Balanced);
        journal::append_note(
            "usuario",
            "modo automático activado: ajustes manuales restablecidos (guardados para devolverlos)",
        );
        detail = "ajustes manuales restablecidos; la IA toma el control";
    } else {
        // Devolver el control manual exacto.
        match fs::read_to_string(snapshot_path()) {
            Ok(s) => {
                let p = state::get_json_str(&s, "profile")
                    .and_then(|v| Profile::parse(&v))
                    .unwrap_or(Profile::Balanced);
                let cap = state::get_json_num(&s, "freq_cap").unwrap_or(1.0) as f32;
                let gaming = state::get_json_bool(&s, "gaming").unwrap_or(false);
                st.set_user_cap(cap);
                st.gaming.store(gaming, Ordering::Relaxed);
                st.apply_profile(p);
                // Re-aplicar los ajustes manuales de nodos que tenía el usuario.
                let ov = s
                    .split_once("\"overrides\":")
                    .map(|(_, rest)| parse_overrides(rest))
                    .unwrap_or_default();
                for o in &ov {
                    apply_value(
                        st,
                        &o.path,
                        &sysfs::active_token(&o.current),
                        "modo manual: ajuste del usuario restaurado",
                    );
                }
                write_overrides(&ov);
                let _ = fs::remove_file(snapshot_path());
                journal::append_note(
                    "usuario",
                    &format!(
                        "modo automático desactivado: control manual restaurado ({})",
                        p.as_str()
                    ),
                );
                detail = "control manual restaurado tal como estaba";
            }
            Err(_) => {
                detail = "control manual devuelto (no había estado guardado)";
            }
        }
    }
    st.save();
    let _ = warnings::drain();
    format!(
        "{{\"ok\":true,\"auto\":{},\"changed\":true,\"detail\":{}}}",
        en,
        jstr(detail)
    )
}
