//! Autotest ejecutable en el dispositivo (`iaopt selftest`).
///
/// Crea un arbol sysfs falso en /data/local/tmp y verifica:
///   - deteccion de interfaces existentes
///   - planificacion por perfil
///   - escritura con verificacion por relectura
///   - rechazo de rutas no escribibles
/// Imprime PASS/FAIL y termina con codigo 0 si todo pasa.

use iaopt_core::profiles::{self, Action};
use iaopt_core::sysfs::{Domain, Probe};
use iaopt_core::{tuner, Profile};
use std::fs;
use std::path::Path;

fn setup(root: &Path) {
    let dirs = [
        "sys/devices/system/cpu/cpu0/cpufreq",
        "sys/class/thermal/thermal_zone0",
    ];
    for d in dirs {
        fs::create_dir_all(root.join(d)).unwrap();
    }
    let f = |rel: &str, val: &str| {
        fs::write(root.join(rel), val).unwrap();
    };
    f(
        "sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors",
        "schedutil performance powersave\n",
    );
    f(
        "sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
        "schedutil\n",
    );
    f(
        "sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq",
        "2800000\n",
    );
    f(
        "sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq",
        "2800000\n",
    );
    f("sys/class/thermal/thermal_zone0/temp", "375\n");

    // permisos de escritura amplios (el demonio corre como root en Magisk)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for rel in [
            "sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
            "sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq",
        ] {
            let p = root.join(rel);
            fs::set_permissions(&p, fs::Permissions::from_mode(0o666)).unwrap();
        }
    }
}

fn check(name: &str, ok: bool) -> bool {
    println!("[{}] {}", if ok { "PASS" } else { "FAIL" }, name);
    ok
}

pub fn run(root: &Path) -> i32 {
    let _ = fs::remove_dir_all(root);
    setup(root);
    let mut all = true;

    let probe = Probe::new(root);
    let knobs = probe.scan();

    all &= check(
        "detecta governor y frecuencias",
        knobs.iter().any(|k| k.path.ends_with("scaling_governor"))
            && knobs.iter().any(|k| k.path.ends_with("scaling_max_freq")),
    );

    let gov = knobs
        .iter()
        .find(|k| k.path.ends_with("scaling_governor"))
        .expect("governor presente");
    all &= check(
        "governor reporta valores permitidos",
        gov.allowed.contains(&"performance".to_string())
            && gov.allowed.contains(&"powersave".to_string()),
    );
    all &= check("governor es escribible", gov.writable);
    all &= check(
        "thermal detectado y legible",
        knobs.iter().any(|k| k.domain == Domain::Thermal && k.current.as_deref() == Some("375")),
    );

    // Plan ECO
    let plan = profiles::plan(Profile::Eco, &knobs);
    all &= check(
        "plan eco elige powersave",
        plan.iter().any(|a| a.value == "powersave"),
    );
    let cap = plan
        .iter()
        .find(|a| a.path.ends_with("scaling_max_freq"))
        .expect("plan incluye techo");
    all &= check("techo eco = 60% de 2800000", cap.value == "1680000");

    // Aplicacion real con verificacion
    let rep = tuner::apply(cap, root);
    all &= check(
        "aplicacion verificada por relectura",
        rep.applied && rep.after.as_deref() == Some("1680000"),
    );

    // Nodo fijado en solo lectura (444): el tuner lo desbloquea antes de
    // escribir (patron de nodos fijados por el vendor) y debe aplicar.
    let ro = root.join("sys/devices/system/cpu/cpu0/cpufreq/scaling_min_freq");
    fs::write(&ro, "500000\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o444)).unwrap();
    }
    // La puerta de seguridad real de las escrituras HTTP es validate: un nodo
    // escaneado como no escribible se rechaza antes de tocar el kernel.
    let knobs2 = probe.scan();
    let ro_knob = knobs2
        .iter()
        .find(|k| k.path.ends_with("scaling_min_freq"))
        .expect("min_freq presente");
    all &= check(
        "validate rechaza nodo no escribible",
        !ro_knob.writable && crate::validate::check(ro_knob, "1000000").is_err(),
    );
    let bad = tuner::apply(
        &Action {
            path: "/sys/devices/system/cpu/cpu0/cpufreq/scaling_min_freq".into(),
            value: "1000000".into(),
            why: "prueba".into(),
            pin: false,
        },
        root,
    );
    all &= check(
        "nodo fijado en 444 se desbloquea y aplica",
        bad.applied && bad.after.as_deref() == Some("1000000"),
    );
    // Ruta inexistente: rechazada sin tocar nada.
    let missing = tuner::apply(
        &Action {
            path: "/sys/devices/system/cpu/cpu9/cpufreq/scaling_max_freq".into(),
            value: "1000000".into(),
            why: "prueba".into(),
            pin: false,
        },
        root,
    );
    all &= check("ruta inexistente rechazada", !missing.applied);

    // Restauracion
    let rest = tuner::restore(
        &[(
            "/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq".into(),
            "2800000".into(),
        )],
        root,
    );
    all &= check("restauracion a valor original", rest[0].applied);

    let _ = fs::remove_dir_all(root);
    println!("{}", if all { "SELFTEST OK" } else { "SELFTEST FALLO" });
    if all {
        0
    } else {
        1
    }
}
