//! IA Optimizer - nucleo (sin dependencias externas).
//!
//! Todo lo que toca el kernel pasa por [`sysfs`]; el motor adaptativo ([`engine`])
//! es un heuristico determinista y auditable, no una caja negra.

pub mod ai;
pub mod engine;
pub mod profiles;
pub mod sysfs;
pub mod tuner;

/// Perfil de funcionamiento elegido por el usuario o por el motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Performance,
    Balanced,
    Eco,
}

impl Profile {
    pub const ALL: [Profile; 3] = [Profile::Performance, Profile::Balanced, Profile::Eco];

    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Performance => "performance",
            Profile::Balanced => "balanced",
            Profile::Eco => "eco",
        }
    }

    /// Multiplicador de techo de frecuencia (1.0 = sin limite).
    pub fn freq_cap(self) -> f32 {
        match self {
            Profile::Performance => 1.0,
            Profile::Balanced => 0.85,
            Profile::Eco => 0.6,
        }
    }

    pub fn parse(s: &str) -> Option<Profile> {
        match s.to_ascii_lowercase().as_str() {
            "performance" | "perf" | "rendimiento" => Some(Profile::Performance),
            "balanced" | "balance" | "normal" => Some(Profile::Balanced),
            "eco" | "economia" | "bateria" => Some(Profile::Eco),
            _ => None,
        }
    }
}

/// Caracteristicas activables por el usuario o por la IA.
/// Cada una gobierna un dominio real; desactivada, no se propone ni se escribe.
#[derive(Debug, Clone, Copy)]
pub struct Features {
    pub governor: bool,
    pub freq: bool,
    pub scheduler: bool,
}

impl Default for Features {
    fn default() -> Self {
        Features { governor: true, freq: true, scheduler: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{decide, suggest, Metrics};
    use crate::profiles::plan;
    use crate::sysfs::{Domain, Knob};

    fn knob(path: &str, domain: Domain, writable: bool, cur: &str, allowed: &[&str]) -> Knob {
        Knob {
            domain,
            path: path.to_string(),
            writable,
            current: Some(cur.to_string()),
            allowed: allowed.iter().map(|s| s.to_string()).collect(),
            min: None,
            max: None,
        }
    }

    #[test]
    fn perfil_se_parsea_en_espanol_e_ingles() {
        assert_eq!(Profile::parse("ECO"), Some(Profile::Eco));
        assert_eq!(Profile::parse("rendimiento"), Some(Profile::Performance));
        assert_eq!(Profile::parse("nonsense"), None);
    }

    #[test]
    fn temperatura_critica_manda_sobre_el_juego() {
        let s = suggest(&Metrics {
            cpu_load: 0.9,
            battery_temp_c: 50.0,
            battery_pct: 40,
            charging: false,
            gaming: true,
            foreground: Foreground::Game,
        });
        assert_eq!(s.profile, Profile::Eco);
        assert!(s.confidence >= 0.9);
    }

    #[test]
    fn histeresis_evita_oscilaciones() {
        let m = Metrics {
            cpu_load: 0.1,
            battery_temp_c: 30.0,
            battery_pct: 80,
            charging: false,
            gaming: false,
            foreground: Foreground::Other,
        };
        // senal debil (0.6) con perfil previo -> se conserva
        let d = decide(Some(Profile::Performance), &m);
        assert_eq!(d.profile, Profile::Performance);
        // sin previo, aplica la sugerencia
        let d2 = decide(None, &m);
        assert_eq!(d2.profile, Profile::Eco);
    }

    #[test]
    fn el_plan_solo_toca_lo_que_existe() {
        let knobs = vec![
            knob(
                "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
                Domain::CpuGovernor,
                true,
                "schedutil",
                &["schedutil", "performance", "powersave"],
            ),
            knob(
                "/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq",
                Domain::CpuFreq,
                true,
                "2400000",
                &[],
            ),
            knob(
                "/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq",
                Domain::CpuFreq,
                false,
                "2800000",
                &[],
            ),
        ];
        let eco = plan(Profile::Eco, &knobs);
        assert!(eco.iter().any(|a| a.value == "powersave"));
        // techo eco = 60% de 2800000 = 1680000
        let cap = eco
            .iter()
            .find(|a| a.path.ends_with("scaling_max_freq"))
            .expect("debe proponer techo");
        assert_eq!(cap.value, "1680000");

        let perf = plan(Profile::Performance, &knobs);
        assert!(perf.iter().any(|a| a.value == "performance"));
        assert!(!perf.iter().any(|a| a.path.ends_with("scaling_max_freq")));
    }
}
