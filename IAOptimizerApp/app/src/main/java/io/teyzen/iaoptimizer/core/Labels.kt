package io.teyzen.iaoptimizer.core

/**
 * Traduce los nombres tecnicos del kernel a lenguaje claro para el usuario.
 * Ninguna ruta o variable de sistema should mostrarse en pantalla.
 */
object Labels {

    data class Info(
        val title: String,
        val hint: String,
        val unit: String? = null,
        val numeric: Boolean = false
    )

    private val exact: Map<String, Info> = mapOf(
        "scaling_max_freq" to Info(
            "Frecuencia máxima del procesador",
            "Velocidad tope que puede alcanzar la CPU. Bajarla ahorra batería y calor.",
            "MHz", numeric = true
        ),
        "scaling_min_freq" to Info(
            "Frecuencia mínima del procesador",
            "Velocidad de reposo de la CPU. Subirla responde antes.",
            "MHz", numeric = true
        ),
        "scaling_governor" to Info(
            "Gobernador de CPU",
            "Cómo el sistema reparte el trabajo entre los núcleos.",
            null
        ),
        "cpuinfo_max_freq" to Info(
            "Frecuencia máxima soportada",
            "Tope físico del procesador; solo informativo.",
            "MHz", numeric = true
        ),
        "cpuinfo_min_freq" to Info(
            "Frecuencia mínima soportada",
            "Mínimo físico del procesador; solo informativo.",
            "MHz", numeric = true
        ),
        "scaling_cur_freq" to Info(
            "Frecuencia actual de la CPU",
            "Velocidad en este momento; solo lectura.",
            "MHz", numeric = true
        ),
        "scaling_available_governors" to Info(
            "Gobernadores disponibles",
            "Estrategias de CPU que acepta este dispositivo.",
            null
        ),
        "scheduler" to Info(
            "Planificador de disco (E/S)",
            "Orden en que se leen y escriben los datos.",
            null
        ),
        "comp_algorithm" to Info(
            "Algoritmo de compresión (ZRAM)",
            "Cómo se comprime la memoria para aprovechar más RAM.",
            null
        ),
        "gpu_max_freq" to Info(
            "Frecuencia máxima de la GPU",
            "Tope de velocidad del gráfico. Bajarla ahorra batería.",
            "MHz", numeric = true
        ),
        "gpu_min_freq" to Info(
            "Frecuencia mínima de la GPU",
            "Velocidad de reposo del gráfico.",
            "MHz", numeric = true
        ),
        "gpu_clk" to Info(
            "Frecuencia de la GPU",
            "Velocidad del gráfico.",
            "MHz", numeric = true
        ),
        "refresh_rate" to Info(
            "Tasa de refresco de pantalla",
            "Veces por segundo que se redibuja la imagen.",
            "Hz", numeric = true
        ),
        "density" to Info(
            "Densidad de pantalla",
            "Tamaño de los elementos en pantalla.",
            "dpi", numeric = true
        ),
        "resolution" to Info(
            "Resolución de pantalla",
            "Cantidad de píxeles del panel.",
            null
        ),
        "tcp_congestion_control" to Info(
            "Control de congestión de red",
            "Cómo se reparte el ancho de banda.",
            null
        ),
        "zram" to Info(
            "Memoria comprimida (ZRAM)",
            "Comprime datos en RAM para equivocar más memoria.",
            null
        ),
        "swappiness" to Info(
            "Intercambio de memoria (swap)",
            "Qué tanto se usa la memoria comprimida.",
            null, numeric = true
        ),
        "cpufreq_power_mode" to Info(
            "Modo de energía del procesador",
            "Perfil eléctrico del chip: rendimiento o ahorro.",
            null
        ),
        "hard_userlimit_max_cpu_freq" to Info(
            "Límite manual de frecuencia",
            "Techo de CPU fijado por el sistema o por ti.",
            "MHz", numeric = true
        ),
        "bypass" to Info(
            "Bypass térmico",
            "Ignora la limitación por temperatura al cargar.",
            null
        ),
        "sched_latency_ns" to Info(
            "Latencia del planificador",
            "Ventana de tiempo en la que el procesador reparte el trabajo entre tareas.",
            "ns", numeric = true
        ),
        "sched_min_granularity_ns" to Info(
            "Precisión de planificación",
            "Cómo de fino reparte el procesador el tiempo entre tareas.",
            "ns", numeric = true
        ),
        "sched_wakeup_granularity_ns" to Info(
            "Sensibilidad al despertar",
            "Qué pronto se reactiva una tarea que estaba en reposo.",
            "ns", numeric = true
        ),
        "rmem_max" to Info(
            "Memoria máxima de recepción",
            "Tamaño del búfer de red al recibir datos.",
            null, numeric = true
        ),
        "wmem_max" to Info(
            "Memoria máxima de envío",
            "Tamaño del búfer de red al enviar datos.",
            null, numeric = true
        ),
        "dirty_background_ratio" to Info(
            "Escritura en segundo plano",
            "Porcentaje de memoria que se escribe al disco sin frenar las apps.",
            "%", numeric = true
        ),
        "dirty_ratio" to Info(
            "Umbral de escritura",
            "Porcentaje de memoria que fuerza una escritura inmediata.",
            "%", numeric = true
        ),
        "tcp_congestion_control" to Info(
            "Control de congestión de red",
            "Cómo se reparte el ancho de banda.",
            null
        )
    )

    fun knob(domain: String, path: String): Info {
        val leaf = path.substringAfterLast('/')
        val base = exact[leaf] ?: exact[path] ?: fallback(domain, path, leaf)
        // El mismo nodo existe en varios clústeres de CPU: el título los
        // diferencia para que no parezcan controles duplicados.
        val cluster = Regex("policy(\\d+)").find(path)?.groupValues?.get(1)
        if (cluster != null && leaf in setOf(
                "scaling_max_freq", "scaling_min_freq",
                "scaling_governor", "scaling_available_governors"
            )
        ) {
            return base.copy(title = "${base.title} · clúster $cluster")
        }
        // Varios discos reales comparten el mismo nodo "scheduler": el título
        // lleva el dispositivo para distinguirlos (sda, sdb, sdc).
        if (leaf == "scheduler") {
            val dev = Regex("/sys/block/([^/]+)/").find(path)?.groupValues?.get(1)
            if (dev != null) return base.copy(title = "Planificador de disco · $dev")
        }
        return base
    }

    private fun fallback(domain: String, path: String, leaf: String): Info {
        return when {
            leaf.endsWith("scaling_governor") || domain.equals("cpu_governor", true) ->
                Info("Gobernador de CPU", "Cómo el sistema reparte el trabajo entre los núcleos.")
            leaf.endsWith("scaling_max_freq") ->
                Info("Frecuencia máxima del procesador", "Velocidad tope de la CPU.", "MHz", true)
            leaf.endsWith("scaling_min_freq") ->
                Info("Frecuencia mínima del procesador", "Velocidad de reposo de la CPU.", "MHz", true)
            leaf.contains("scheduler") || path.contains("queue/") -> {
                val dev = Regex("/sys/block/([^/]+)/").find(path)?.groupValues?.get(1)
                Info(
                    if (dev != null) "Planificador de disco · $dev" else "Planificador de disco (E/S)",
                    "Orden de lectura y escritura del almacenamiento."
                )
            }
            path.contains("kgsl") || path.contains("mali") || path.contains("devfreq") ||
                domain.equals("gpu", true) ->
                Info("Frecuencia de la GPU", "Velocidad del procesador gráfico.", "MHz", true)
            path.contains("thermal") || domain.equals("thermal", true) ->
                Info("Control térmico", "Límites para proteger el dispositivo del calor.")
            path.contains("zram") ->
                Info("Memoria comprimida (ZRAM)", "Más memoria útil comprimiendo datos.")
            path.contains("swappiness") || path.contains("swap") ->
                Info("Intercambio de memoria", "Qué tanto se usa la memoria comprimida.", null, true)
            path.contains("tcp") || path.contains("network") ->
                Info("Red", "Ajustes de conexión y ancho de banda.")
            path.contains("ppm") || path.contains("mtk") ->
                Info("Gestión de potencia del chip", "Modos eléctricos del procesador (PPM).")
            path.contains("battery") || path.contains("charger") ->
                Info("Batería y carga", "Comportamiento de la carga y el consumo.")
            else -> Info(
                leaf.replace('_', ' ')
                    .replaceFirstChar { it.uppercase() },
                "Ajuste avanzado del dispositivo."
            )
        }
    }

    /** Secciones con nombre claro (nunca el nombre interno del dominio). */
    fun domain(domain: String): String = when (domain.lowercase()) {
        "cpu", "cpu_governor", "cpu_freq" -> "Procesador (CPU)"
        "gpu", "devfreq", "kgsl", "mali" -> "Gráficos (GPU)"
        "memory", "io", "scheduler", "zram", "swap" -> "Memoria y almacenamiento"
        "display", "screen", "panel" -> "Pantalla"
        "network", "net", "wifi" -> "Red"
        "thermal", "temperature", "battery", "charger" -> "Energía y temperatura"
        "ppm", "mtk", "platform" -> "Gestión del chip"
        else -> domain.replace('_', ' ').replaceFirstChar { it.uppercase() }
    }

    /** Valor de un control enumerado en lenguaje claro (gobernadores, modos, etc.). */
    fun choice(domain: String, path: String, raw: String): String {
        val leaf = path.substringAfterLast('/')
        return when {
            leaf.endsWith("scaling_governor") -> when (raw) {
                "performance" -> "Rendimiento máximo"
                "powersave" -> "Ahorro de energía"
                "schedutil" -> "Adaptativo"
                "walt" -> "Adaptativo"
                "ondemand" -> "Según demanda"
                "conservative" -> "Conservador"
                "interactive" -> "Interactivo"
                "userspace" -> "Manual"
                else -> raw
            }
            leaf.contains("scheduler") -> when (raw) {
                "none" -> "Ninguno"
                "noop" -> "Sin reordenar"
                "deadline" -> "Plazo fijo"
                "mq-deadline" -> "Plazo fijo multicola"
                "kyber" -> "Kyber"
                "cfq" -> "CFQ"
                "bfq" -> "BFQ"
                else -> raw
            }
            leaf == "cpufreq_power_mode" -> when (raw.trim()) {
                "0" -> "Normal"
                "1" -> "Ahorro de energía"
                "2" -> "Modo juego"
                "3" -> "Rendimiento máximo"
                else -> {
                    val v = raw.lowercase()
                    when {
                        v.contains("default") || v.contains("normal") -> "Normal"
                        v.contains("low") -> "Ahorro de energía"
                        v.contains("perf") || v.contains("high") -> "Rendimiento máximo"
                        v.contains("save") -> "Ahorro de energía"
                        else -> raw
                    }
                }
            }
            leaf.contains("congestion") -> when (raw) {
                "cubic" -> "Cubic"
                "bic" -> "BIC"
                "reno" -> "Reno"
                "westwood" -> "Westwood"
                "vegas" -> "Vegas"
                else -> raw
            }
            else -> raw
        }
    }

    /** Resumen del motor en lenguaje claro. */
    fun reason(raw: String): String {
        val t = raw.trim()
        if (t.isEmpty() || t == "-") return "Sin novedad: todo en orden."
        return when {
            t.contains("histéresis", true) ->
                "Se mantiene el modo actual: no hizo falta ningún cambio."
            t.contains("sin cambios", true) || t.contains("Sin cambios", true) ->
                "No se cambió nada: el modo actual sigue siendo el mejor."
            t.contains("pantalla apagada", true) ->
                "Pantalla apagada: el motor está en espera."
            t.contains("SoC por encima", true) ->
                "El procesador está muy caliente: se limita el rendimiento para protegerlo."
            t.contains("batería a", true) ->
                "La batería está caliente: se limita el rendimiento para protegerla."
            t.contains("equilibrio", true) ->
                "Equilibrio entre rendimiento y consumo."
            else -> t.replaceFirstChar { it.uppercase() }
        }
    }

    /** Texto claro de lo que hace cada modo. */
    fun mode(profile: String): Info = when (profile) {
        "eco" -> Info(
            "Eco",
            "Activa el ahorro de batería del sistema, limita la CPU y usa el gobernador de bajo consumo."
        )
        "performance" -> Info(
            "Rendimiento",
            "Desbloquea la frecuencia máxima y usa el gobernador de máximo rendimiento."
        )
        else -> Info(
            "Balanceado",
            "Equilibrio entre fluidez y consumo: gobernador adaptativo y sin límites extra."
        )
    }
}
