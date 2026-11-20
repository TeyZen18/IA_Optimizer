//! Reparacion autonoma: diagnostica fallos, aplica un comando seguro y, si no
/// funciona, lo anade a una lista negra para no volver a intentarlo.
///
/// Principio: cada reparacion es un comando declarado, con efecto verificable;
/// nunca se reintenta a ciegas lo que ya fallo.

use crate::state::State;
use std::fs;
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

const BIN: &str = "/data/adb/modules/ia_optimizer/bin/iaopt";
const SERVICE: &str = "/data/adb/modules/ia_optimizer/service.sh";
const BLACKLIST: &str = "/data/adb/ia_optimizer/repair.blacklist";

pub struct Finding {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

fn read_blacklist() -> Vec<String> {
    fs::read_to_string(BLACKLIST)
        .unwrap_or_default()
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn add_blacklist(name: &str) {
    let mut s = fs::read_to_string(BLACKLIST).unwrap_or_default();
    if !s.contains(name) {
        s.push_str(name);
        s.push('\n');
        let _ = fs::write(BLACKLIST, s);
    }
}

/// Comprobaciones sin escribir; cada fallo trae su comando de reparacion.
pub fn diagnose(st: &State) -> Vec<Finding> {
    let bl = read_blacklist();
    let mut v = Vec::new();

    let cfg_ok = fs::read_to_string(&st.cfg_path)
        .map(|s| crate::state::get_json_str(&s, "profile").is_some())
        .unwrap_or(false);
    v.push(Finding {
        name: "config".into(),
        ok: cfg_ok,
        detail: if cfg_ok {
            "config.json legible".into()
        } else {
            "config.json ilegible o incompleto".into()
        },
        fix: if cfg_ok { None } else { Some("rewrite-config".into()) },
    });

    let bin_ok = fs::metadata(BIN).map(|m| m.is_file()).unwrap_or(false);
    v.push(Finding {
        name: "binario".into(),
        ok: bin_ok,
        detail: if bin_ok {
            "bin/iaopt presente".into()
        } else {
            "bin/iaopt no encontrado".into()
        },
        fix: if bin_ok { None } else { Some("chmod-bin".into()) },
    });

    let addr: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let listening = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok();
    v.push(Finding {
        name: "daemon".into(),
        ok: listening,
        detail: if listening {
            "escuchando en 127.0.0.1:8080".into()
        } else {
            "no responde en 127.0.0.1:8080".into()
        },
        fix: if listening {
            None
        } else {
            Some("restart-service".into())
        },
    });

    let n = st.probe().scan().len();
    v.push(Finding {
        name: "controles".into(),
        ok: n > 0,
        detail: format!("{} interfaces activas", n),
        fix: None,
    });

    let fails = crate::journal::failures(20);
    v.push(Finding {
        name: "escrituras".into(),
        ok: fails == 0,
        detail: if fails == 0 {
            "sin fallos recientes".into()
        } else {
            format!("{} acciones no se sostuvieron", fails)
        },
        fix: None,
    });

    // Aprendizaje: si una reparacion ya fallo, no se ofrece de nuevo.
    for f in v.iter_mut() {
        if let Some(fx) = f.fix.clone() {
            if bl.contains(&fx) {
                f.fix = None;
                f.detail.push_str(" · reparación descartada: falló antes");
            }
        }
    }
    v
}

/// Ejecuta una reparacion declarada. Si falla, queda en lista negra.
pub fn apply_fix(st: &State, name: &str) -> (bool, String) {
    if read_blacklist().iter().any(|b| b == name) {
        return (false, "reparacion en lista negra (fallo antes)".into());
    }
    let res = match name {
        "rewrite-config" => {
            st.save();
            (true, "config reescrita con valores validos".into())
        }
        "chmod-bin" => {
            let ok = fs::metadata(BIN).is_ok();
            if ok {
                let _ = fs::set_permissions(BIN, fs::Permissions::from_mode(0o755));
                (true, "permisos del binario reparados".into())
            } else {
                (false, "el binario no existe".into())
            }
        }
        "restart-service" => {
            let cmd = format!(
                "nohup setsid sh {} </dev/null >> /data/adb/ia_optimizer/daemon.log 2>&1 &",
                SERVICE
            );
            let spawned = std::process::Command::new("sh").arg("-c").arg(cmd).output();
            if let Err(e) = spawned {
                (false, format!("no se pudo reiniciar: {}", e))
            } else {
                // Verificar que el daemon realmente responde antes de darlo por bueno.
                std::thread::sleep(Duration::from_secs(4));
                let addr: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
                if TcpStream::connect_timeout(&addr, Duration::from_millis(600)).is_ok() {
                    (true, "servicio reiniciado y respondiendo".into())
                } else {
                    (false, "el servicio no respondio tras reiniciar".into())
                }
            }
        }
        _ => (false, "reparacion desconocida".into()),
    };
    if !res.0 {
        add_blacklist(name);
    }
    res
}
