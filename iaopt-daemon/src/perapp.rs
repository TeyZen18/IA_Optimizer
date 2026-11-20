//! Perfil propio por app, aplicado cuando la app pasa a primer plano.
//!
//! Antes el perfil elegido en Game Space solo se guardaba: el ciclo de IA
//! Motors nunca lo consultaba. Este hilo vigila la app en primer plano y, si
//! tiene un perfil propio (eco / balanced / performance), lo aplica; cuando la
//! app deja de estar en primer plano, restaura el perfil que estaba activo.
//!
//! Para no pelearse con el ciclo del agente (que decidiría otro perfil y los dos
//! oscilarían cada pocos segundos), el estado comparte `State::perapp`: mientras
//! la app con perfil propio esté en primer plano, el agente no vuelve a aplicar
//! su propia decisión.

use std::sync::Arc;
use std::time::Duration;

use crate::games;
use crate::journal;
use crate::state::State;

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

pub fn run(period: Duration, st: Arc<State>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(period);
        tick(&st);
    });
}

fn tick(st: &State) {
    let fg = match crate::read_foreground_app() {
        Some(p) if !p.is_empty() => p,
        _ => return,
    };

    let held = st.perapp.lock().ok().and_then(|g| g.clone());

    // La app con perfil propio dejó de estar en primer plano: restaurar.
    if let Some(held_pkg) = held.clone() {
        if held_pkg != fg {
            let prev = st.perapp_prev.lock().ok().and_then(|g| g.clone());
            if let Some(p) = prev {
                let _ = st.apply_profile(p);
                journal::append_note(
                    "perfil",
                    &format!(
                        "{} salió de primer plano: se restaura {}",
                        held_pkg,
                        p.as_str()
                    ),
                );
            }
            if let Ok(mut g) = st.perapp.lock() {
                *g = None;
            }
            if let Ok(mut g) = st.perapp_prev.lock() {
                *g = None;
            }
        } else {
            // Sigue en primer plano: nada que hacer.
            return;
        }
    }

    // ¿Esta app tiene perfil propio?
    let Some(nombre) = games::profile_of(&fg) else {
        return;
    };
    let Some(perfil) = iaopt_core::Profile::parse(&nombre) else {
        return;
    };
    let prev = *st.profile.lock().unwrap();
    if prev == perfil {
        // Ya está aplicado; solo registrar el hold para que el agente no lo pise.
        if let Ok(mut g) = st.perapp.lock() {
            *g = Some(fg.clone());
        }
        if let Ok(mut g) = st.perapp_prev.lock() {
            *g = Some(prev);
        }
        return;
    }
    let _ = st.apply_profile(perfil);
    if let Ok(mut g) = st.perapp.lock() {
        *g = Some(fg.clone());
    }
    if let Ok(mut g) = st.perapp_prev.lock() {
        *g = Some(prev);
    }
    journal::append_note(
        "perfil",
        &format!(
            "{} en primer plano: se aplica su perfil {}",
            jstr(&fg),
            perfil.as_str()
        ),
    );
}
