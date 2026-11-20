package io.teyzen.iaoptimizer.core

import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * Puente local con el backend Rust (demonio iaopt).
 *
 * Orden de preferencia:
 *  1) HTTP a 127.0.0.1:8080 (el demonio ya expone su API).
 *  2) Ejecucion root del binario del modulo (`iaopt <cmd>`) como respaldo.
 *
 * El binario vive en /data/adb/modules/ia_optimizer/bin/iaopt y solo se ejecuta
 * con su (KernelSU/Magisk). Nunca se piden credenciales.
 */
object RustBridge {

    private const val BIN = "/data/adb/modules/ia_optimizer/bin/iaopt"
    private const val DAEMON = "http://127.0.0.1:8080"
    private val SU_CANDIDATES = listOf(
        "/system/bin/su", "/system/xbin/su", "/sbin/su",
        "/debug_ramdisk/su", "/system/sbin/su", "su"
    )

    @Volatile
    private var suPath: String? = null

    @Synchronized
    private fun su(): String? {
        if (suPath != null) return suPath
        for (p in SU_CANDIDATES) {
            if (p == "su") {
                val found = try {
                    val proc = ProcessBuilder("sh", "-c", "command -v su")
                        .redirectErrorStream(true).start()
                    val out = proc.inputStream.bufferedReader().readText().trim()
                    proc.waitFor()
                    out
                } catch (e: Exception) { "" }
                if (found.isNotEmpty() && File(found).exists()) {
                    suPath = found
                    return suPath
                }
            } else if (File(p).exists()) {
                suPath = p
                return suPath
            }
        }
        return null
    }

    fun hasRoot(): Boolean = su() != null

    /** Ejecuta un comando del binario Rust con root y devuelve la salida. */
    fun rootCommand(vararg args: String): String {
        val cmd = (listOf(BIN) + args).joinToString(" ") { shellQuote(it) }
        return try {
            val su = su()
            val pb = if (su != null) {
                ProcessBuilder(su, "-c", cmd)
            } else {
                ProcessBuilder("sh", "-c", cmd)
            }
            pb.redirectErrorStream(true)
            val proc = pb.start()
            val out = proc.inputStream.bufferedReader().readText().trim()
            proc.waitFor()
            out
        } catch (e: Exception) {
            "error: ${e.message}"
        }
    }

    private fun shellQuote(s: String): String =
        "'" + s.replace("'", "'\\''") + "'"

    /** GET HTTP al demonio. Devuelve "" si no responde. */
    fun http(path: String): String = httpRequest("GET", path, null)

    fun post(path: String, json: String): String = httpRequest("POST", path, json)

    private fun httpRequest(method: String, path: String, body: String?): String {
        return try {
            val url = URL(DAEMON + path)
            val conn = url.openConnection() as HttpURLConnection
            conn.connectTimeout = 4000
            conn.readTimeout = 15000
            conn.requestMethod = method
            if (body != null) {
                conn.doOutput = true
                conn.setRequestProperty("Content-Type", "application/json")
                conn.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
            }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val text = stream?.bufferedReader()?.readText() ?: ""
            conn.disconnect()
            // Cada GET correcto alimenta la caché: las pantallas pintan al
            // instante con lo último conocido mientras llega el dato nuevo.
            if (method == "GET" && code in 200..299) StateCache.put(path, text)
            text
        } catch (e: Exception) {
            ""
        }
    }

    /** State con respaldo: HTTP primero, binario root si no hay demonio. */
    fun stateJson(): String {
        http("/api/state").takeIf { it.isNotEmpty() }?.let { return it }
        return rootCommand("state")
    }

    fun telemetryJson(): String {
        http("/api/telemetry").takeIf { it.isNotEmpty() }?.let { return it }
        return rootCommand("telemetry")
    }

    fun agentJson(): String {
        http("/api/agent").takeIf { it.isNotEmpty() }?.let { return it }
        return rootCommand("agent")
    }

    fun appsJson(filter: String): String {
        http("/api/apps?filter=$filter").takeIf { it.isNotEmpty() }?.let { return it }
        return rootCommand("apps", filter)
    }

    fun appInfoJson(pkg: String): String {
        http("/api/app-info?package=" + android.net.Uri.encode(pkg)).takeIf { it.isNotEmpty() }
            ?.let { return it }
        return rootCommand("app-info", pkg)
    }

    fun setProfile(profile: String): String =
        post("/api/profile", JSONObject().put("profile", profile).toString())
            .ifEmpty { rootCommand("apply", profile) }

    /** Techo de frecuencia elegido por el usuario (0..1 del maximo del chip). */
    fun freqCap(): Float = try {
        JSONObject(http("/api/state")).optDouble("freq_cap", 1.0).toFloat()
    } catch (e: Exception) {
        1.0f
    }

    fun setFreqCap(value: Float): String =
        post("/api/freq-cap", JSONObject().put("value", value.toDouble()).toString())
            .ifEmpty { rootCommand("freq-cap", value.toString()) }

    fun setAuto(enabled: Boolean): String =
        post("/api/auto", JSONObject().put("enabled", enabled).toString())
            .ifEmpty { rootCommand("set-auto", if (enabled) "1" else "0") }

    fun setAi(enabled: Boolean): String =
        post("/api/ai/toggle", JSONObject().put("enabled", enabled).toString())
            .ifEmpty { rootCommand("ai-toggle", if (enabled) "1" else "0") }

    /** Preferencias de la IA: objetivo (actuacion/equilibrado/bateria) y
     *  comportamiento de intervencion (calma/equilibrado/decisivo). */
    fun setPrefs(goal: String?, behavior: String?): String {
        val o = JSONObject()
        if (goal != null) o.put("goal", goal)
        if (behavior != null) o.put("behavior", behavior)
        return post("/api/prefs", o.toString())
    }

    fun setGaming(enabled: Boolean): String =
        post("/api/gaming", JSONObject().put("enabled", enabled).toString())
            .ifEmpty { rootCommand("set-gaming", if (enabled) "1" else "0") }

    fun applyApp(pkg: String): String =
        post("/api/app-apply", JSONObject().put("package", pkg).toString())
            .ifEmpty { rootCommand("app-apply", pkg) }

    fun appSetting(pkg: String, key: String, value: String): String =
        post(
            "/api/app-settings",
            JSONObject().put("package", pkg).put("key", key).put("value", value).toString()
        ).ifEmpty { rootCommand("app-settings", pkg, key, value) }

    fun applyProfile(profile: String): String =
        post("/api/profile", JSONObject().put("profile", profile).toString())
            .ifEmpty { rootCommand("apply", profile) }
    fun auditJson(): String = http("/api/audit")
    fun warningsJson(): String = http("/api/warnings")
    fun thermalJson(): String = http("/api/thermal")
    fun runAudit(): String = post("/api/audit/run", "{}")
    fun setKnob(path: String, value: String): String =
        post("/api/knob", JSONObject().put("path", path).put("value", value).toString())
            .ifEmpty { rootCommand("set-knob", path, value) }
    fun selftest(): String = rootCommand("selftest").ifEmpty { http("/api/selftest") }
    fun rescan(): String = http("/api/scan?force=1").ifEmpty { rootCommand("scan-json") }

    fun deviceJson(): String = http("/api/device").ifEmpty { rootCommand("device") }
    fun scanJson(): String = http("/api/scan").ifEmpty { rootCommand("scan-json") }
    fun displayJson(): String = http("/api/display").ifEmpty { rootCommand("display") }
    fun rendererJson(): String = http("/api/renderer").ifEmpty { rootCommand("renderer") }
    fun setRenderer(v: String): String =
        post("/api/renderer", JSONObject().put("value", v).toString())
            .ifEmpty { rootCommand("renderer", v) }
    fun setResolution(v: String): String =
        post("/api/resolution", JSONObject().put("value", v).toString())
            .ifEmpty { rootCommand("resolution", v) }
    fun setRefresh(v: Int): String =
        post("/api/refresh", JSONObject().put("value", v).toString())
            .ifEmpty { rootCommand("refresh", v.toString()) }
    fun repairJson(): String = http("/api/repair").ifEmpty { rootCommand("repair") }
    fun repairApply(fix: String): String =
        post("/api/repair", JSONObject().put("fix", fix).toString())
            .ifEmpty { rootCommand("repair-apply", fix) }
    fun audioJson(): String = http("/api/audio").ifEmpty { rootCommand("audio") }
    fun commandCenterJson(): String =
        http("/api/command_center").ifEmpty { rootCommand("command-center") }
    fun featuresJson(): String = http("/api/features").ifEmpty { rootCommand("features") }
    fun setFeature(k: String, v: Boolean): String =
        post("/api/features", JSONObject().put(k, v).toString())
            .ifEmpty { rootCommand("features-set", k, if (v) "1" else "0") }
    fun quick(action: String, value: String): String =
        post("/api/quick", JSONObject().put("action", action).put("value", value).toString())
            .ifEmpty { rootCommand("quick", action, value) }
    fun bypassStatus(): String = http("/api/bypass").ifEmpty { rootCommand("bypass") }
    fun bypassDiagnose(): String =
        http("/api/bypass-diagnose").ifEmpty { rootCommand("bypass-diagnose") }
    fun bypassSet(enable: Boolean, path: String): String =
        post(
            "/api/bypass-set",
            JSONObject().put("enable", enable).put("path", path).toString()
        ).ifEmpty { rootCommand("bypass-set", if (enable) "1" else "0", path) }
    fun bypassThreshold(v: Int): String =
        post("/api/bypass-threshold", JSONObject().put("value", v).toString())
            .ifEmpty { rootCommand("bypass-threshold", v.toString()) }
    fun historyJson(): String = http("/api/history").ifEmpty { rootCommand("history") }
    fun gamesJson(): String = http("/api/games").ifEmpty { rootCommand("games") }
    fun appIconPath(pkg: String): String =
        "/api/app-icon?package=" + android.net.Uri.encode(pkg)

    /** FPS de la app en primer plano: fuente real del daemon (contadores de
     *  SurfaceFlinger, sin espera) con respaldo gfxinfo si el daemon no está. */
    fun fps(): Int = foregroundPackage()?.let { fpsFor(it) } ?: 0

    /** FPS de un paquete concreto. HTTP primero (instantáneo); gfxinfo solo
     *  como respaldo cuando no hay daemon (con su delta de 1 s). */
    fun fpsFor(pkg: String): Int {
        val http = http("/api/fps?package=" + android.net.Uri.encode(pkg))
        if (http.isNotEmpty()) {
            try {
                return org.json.JSONObject(http).optInt("fps", 0)
            } catch (e: Exception) { }
        }
        val a = gfxFrames(pkg)
        try { Thread.sleep(1000) } catch (e: InterruptedException) { }
        val b = gfxFrames(pkg)
        if (a <= 0 || b < a) return 0
        return (b - a).coerceAtMost(240)
    }

    /** App en primer plano mediante dumpsys (root). */
    fun foregroundPackage(): String? {
        val out = rootCommandRaw("dumpsys", "activity", "activities") ?: return null
        val focus = rootCommandRaw("dumpsys", "window") ?: ""
        val text = out + "\n" + focus
        for (line in text.lines()) {
            if (line.contains("ResumedActivity") || line.contains("mFocusedApp") ||
                line.contains("mCurrentFocus")
            ) {
                val tok = line.split(Regex("\\s+")).firstOrNull { it.contains('/') } ?: continue
                val pkg = tok.split('/').firstOrNull() ?: continue
                if (pkg.isNotEmpty()) return pkg
            }
        }
        return null
    }

    /** GPU en MHz, leyendo devfreq/kgsl/mali con root (best-effort). */
    fun gpuMhz(): Long {
        // Fuente canónica: telemetría del daemon (lee /proc/mali del vendor
        // con su contexto; el shell adb no tiene permiso en varios MTK).
        try {
            val g = org.json.JSONObject(StateCache.telemetry()).optLong("gpu_mhz", 0)
            if (g > 0) return g
        } catch (e: Exception) { }
        val candidates = listOf(
            "/sys/class/devfreq/5d600000.gpubw/cur_freq",
            "/sys/class/devfreq/gpu/cur_freq",
            "/sys/class/kgsl/kgsl-3d0/devfreq/cur_freq",
            "/sys/class/kgsl/kgsl-3d0/gpuclk",
            "/sys/class/misc/mali0/device/cur_freq",
            "/sys/class/misc/mali0/device/dvfs_cur_freq"
        )
        for (p in candidates) {
            val direct = try { java.io.File(p).readText().trim().toLongOrNull() } catch (e: Exception) { null }
            val v = direct ?: rootCommandRaw("cat", p)?.trim()?.toLongOrNull() ?: continue
            if (v <= 0) continue
            return when {
                v > 1_000_000_000L -> v / 1_000_000L
                v > 1_000_000L -> v / 1_000L
                v > 10_000L -> v / 1_000L
                else -> v
            }
        }
        return 0L
    }

    private fun gfxFrames(pkg: String): Int {
        val out = rootCommandRaw("dumpsys", "gfxinfo", pkg) ?: return 0
        val re = Regex("Total frames rendered:\\s*(\\d+)")
        val m = re.find(out) ?: return 0
        return m.groupValues[1].toIntOrNull() ?: 0
    }

    private fun rootCommandRaw(vararg args: String): String? = try {
        val su = su() ?: return null
        val pb = ProcessBuilder(su, "-c", args.joinToString(" "))
        pb.redirectErrorStream(true)
        val proc = pb.start()
        val out = proc.inputStream.bufferedReader().readText()
        proc.waitFor()
        out
    } catch (e: Exception) {
        null
    }
}
