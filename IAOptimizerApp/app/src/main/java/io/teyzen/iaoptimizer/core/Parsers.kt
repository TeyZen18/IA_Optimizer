package io.teyzen.iaoptimizer.core

import org.json.JSONArray
import org.json.JSONObject

/** Controles detectados en el kernel (respuesta de /api/scan). */
data class Knob(
    val domain: String,
    val path: String,
    val writable: Boolean,
    val current: String,
    val allowed: List<String>,
    val min: Long?,
    val max: Long?
)

object ScanParser {
    fun parse(json: String): List<Knob> {
        val arr = try { JSONArray(json) } catch (e: Exception) { return emptyList() }
        val out = mutableListOf<Knob>()
        for (i in 0 until arr.length()) {
            val o = arr.optJSONObject(i) ?: continue
            val allowed = mutableListOf<String>()
            val a = o.optJSONArray("allowed")
            if (a != null) for (j in 0 until a.length()) allowed.add(a.optString(j))
            out.add(
                Knob(
                    domain = o.optString("domain"),
                    path = o.optString("path"),
                    writable = o.optBoolean("writable"),
                    current = o.optString("current", "-"),
                    allowed = allowed,
                    min = if (o.isNull("min")) null else o.optLong("min"),
                    max = if (o.isNull("max")) null else o.optLong("max")
                )
            )
        }
        // Red de seguridad: una sola tarjeta por ruta real. El daemon ya
        // elimina los duplicados por symlink; si algun frontend recibe dos
        // rutas equivalentes, aqui no se repiten.
        return out.distinctBy { it.path }
    }
}

/** Entrada del diario de decision. */
data class JournalEntry(
    val t: Long,
    val kind: String,
    val profile: String,
    val reason: String,
    val conf: Float,
    val note: String,
    val path: String,
    val value: String,
    val ok: Boolean,
    val outcome: String,
    val app: String,
    val mode: String,
    val tempD: Float,
    val cpuD: Float
)

data class WeightRow(val mode: String, val kind: String, val tempDelta: Float, val samples: Int)
data class PerAppRow(val app: String, val verified: Int)
data class UnresolvedRow(val t: Long, val kind: String, val path: String, val reason: String)

data class AgentData(
    val activity: String,
    val phase: String,
    val cycle: Int,
    val satisfaction: Float,
    val target: Float,
    val plan: List<String>,
    val verified: Int,
    val applied: Int,
    val failed: Int,
    val honesty: Float,
    val overrides: Int,
    val measured: Int,
    val sample: Int,
    val windowHours: Float,
    val weights: List<WeightRow>,
    val perApp: List<PerAppRow>,
    val app: String,
    val screen: String,
    val profile: String,
    val ramPct: Float,
    val actuacion: Float,
    val bateria: Float,
    val espacio: Float,
    val governor: String,
    val interventions: Int,
    val socMax: Float,
    val batteryC: Float,
    val unresolved: List<UnresolvedRow>,
    val journal: List<JournalEntry>,
    // Preferencias de la IA y presupuesto de calor (nuevos en el daemon).
    val goal: String = "equilibrado",
    val behavior: String = "equilibrado",
    val batteryPct: Int = 0,
    val charging: Boolean = false,
    val features: List<String> = emptyList(),
    val heatMeasured: Boolean = false,
    val heatLastS: Long = 0,
    val heatBestS: Long = 0,
    val heatMedianS: Long = 0,
    val heatSessions: Int = 0,
    val heatInSession: Boolean = false
)

object AgentParser {
    fun parse(json: String): AgentData? = try {
        val o = JSONObject(json)
        val ls = o.optJSONObject("learning_stats") ?: JSONObject()
        val dp = o.optJSONObject("device_profile") ?: JSONObject()
        val ac = o.optJSONObject("app_context") ?: JSONObject()
        val ob = o.optJSONObject("objectives") ?: JSONObject()
        val th = o.optJSONObject("thermal") ?: JSONObject()
        val weights = mutableListOf<WeightRow>()
        val wa = ls.optJSONArray("weights")
        if (wa != null) for (i in 0 until wa.length()) {
            val w = wa.optJSONObject(i) ?: continue
            weights.add(
                WeightRow(
                    w.optString("mode"), w.optString("kind"),
                    w.optDouble("temp_delta", 0.0).toFloat(),
                    w.optInt("samples", 0)
                )
            )
        }
        val perApp = mutableListOf<PerAppRow>()
        val pa = ls.optJSONArray("per_app")
        if (pa != null) for (i in 0 until pa.length()) {
            val p = pa.optJSONObject(i) ?: continue
            perApp.add(PerAppRow(p.optString("app"), p.optInt("verified", 0)))
        }
        val unresolved = mutableListOf<UnresolvedRow>()
        val ua = o.optJSONArray("unresolved_errors")
        if (ua != null) for (i in 0 until ua.length()) {
            val u = ua.optJSONObject(i) ?: continue
            unresolved.add(
                UnresolvedRow(
                    u.optLong("t"), u.optString("kind"),
                    u.optString("path"), u.optString("reason")
                )
            )
        }
        val journal = mutableListOf<JournalEntry>()
        val ja = o.optJSONArray("journal")
        if (ja != null) for (i in 0 until ja.length()) {
            val e = ja.optJSONObject(i) ?: continue
            journal.add(
                JournalEntry(
                    e.optLong("t"), e.optString("kind"), e.optString("profile"),
                    e.optString("reason"), e.optDouble("conf", 0.0).toFloat(),
                    e.optString("note"), e.optString("path"), e.optString("value"),
                    e.optBoolean("ok"), e.optString("outcome"), e.optString("app"),
                    e.optString("mode"), e.optDouble("temp_d", 0.0).toFloat(),
                    e.optDouble("cpu_d", 0.0).toFloat()
                )
            )
        }
        AgentData(
            activity = o.optString("current_activity", "-"),
            phase = o.optString("phase", "-"),
            cycle = o.optInt("cycle", 0),
            satisfaction = o.optDouble("satisfaction", 0.0).toFloat(),
            target = o.optDouble("target", 0.75).toFloat(),
            plan = o.optJSONArray("agent_plan").toStringList(),
            verified = ls.optInt("verified", 0),
            applied = ls.optInt("applied", 0),
            failed = ls.optInt("failed", 0),
            honesty = ls.optDouble("honesty_pct", 0.0).toFloat(),
            overrides = ls.optInt("user_overrides", 0),
            measured = ls.optInt("measured_decisions", 0),
            sample = ls.optInt("sample", 0),
            windowHours = (ls.optInt("window_s", 0) / 3600f),
            weights = weights,
            perApp = perApp,
            app = ac.optString("package", "-"),
            screen = ac.optString("screen", "-"),
            profile = ac.optString("profile", "-"),
            ramPct = ac.optDouble("ram_pct", 0.0).toFloat(),
            actuacion = ob.optDouble("actuacion", 0.0).toFloat(),
            bateria = ob.optDouble("bateria", 0.0).toFloat(),
            espacio = ob.optDouble("espacio_termico", 0.0).toFloat(),
            governor = th.optString("governor", "-"),
            interventions = th.optInt("interventions", 0),
            socMax = th.optDouble("soc_max_c", 0.0).toFloat(),
            batteryC = th.optDouble("battery_c", 0.0).toFloat(),
            unresolved = unresolved,
            journal = journal,
            goal = (o.optJSONObject("prefs") ?: JSONObject()).optString("goal", "equilibrado"),
            behavior = (o.optJSONObject("prefs") ?: JSONObject()).optString("behavior", "equilibrado"),
            batteryPct = ac.optInt("battery_pct", 0),
            charging = ac.optBoolean("charging", false),
            features = dp.optJSONArray("features").toStringList(),
            heatMeasured = (o.optJSONObject("heat_budget") ?: JSONObject()).optBoolean("measured", false),
            heatLastS = (o.optJSONObject("heat_budget") ?: JSONObject()).optLong("last_s", 0),
            heatBestS = (o.optJSONObject("heat_budget") ?: JSONObject()).optLong("best_s", 0),
            heatMedianS = (o.optJSONObject("heat_budget") ?: JSONObject()).optLong("median_s", 0),
            heatSessions = (o.optJSONObject("heat_budget") ?: JSONObject()).optInt("sessions", 0),
            heatInSession = (o.optJSONObject("heat_budget") ?: JSONObject()).optBoolean("in_session", false)
        )
    } catch (e: Exception) {
        null
    }
}

private fun JSONArray?.toStringList(): List<String> {
    if (this == null) return emptyList()
    val out = mutableListOf<String>()
    for (i in 0 until length()) out.add(optString(i))
    return out
}

data class DisplayData(
    val refreshHz: Float,
    val renderer: String,
    val rendererOptions: List<String>,
    val density: String,
    val size: String,
    val overrideSize: String
)

object DisplayParser {
    fun parse(json: String): DisplayData? = try {
        val o = JSONObject(json)
        val rd = o.optJSONObject("renderer") ?: JSONObject()
        val opts = mutableListOf<String>()
        val a = rd.optJSONArray("options")
        if (a != null) for (i in 0 until a.length()) opts.add(a.optJSONObject(i)?.optString("name") ?: "")
        val size = o.optString("size", "")
        val m = Regex("Override size:\\s*([0-9x]+)").find(size)
        DisplayData(
            refreshHz = o.optDouble("refresh_hz", 0.0).toFloat(),
            renderer = rd.optString("current", "-"),
            rendererOptions = opts,
            density = o.optString("density", "-"),
            size = size,
            overrideSize = m?.groupValues?.get(1) ?: ""
        )
    } catch (e: Exception) {
        null
    }
}

data class RepairRow(val name: String, val ok: Boolean, val detail: String, val fix: String?)
data class RepairData(val findings: List<RepairRow>, val blacklist: List<String>)

object RepairParser {
    fun parse(json: String): RepairData? = try {
        val o = JSONObject(json)
        val out = mutableListOf<RepairRow>()
        val a = o.optJSONArray("findings")
        if (a != null) for (i in 0 until a.length()) {
            val f = a.optJSONObject(i) ?: continue
            out.add(
                RepairRow(
                    f.optString("name"), f.optBoolean("ok"),
                    f.optString("detail"),
                    if (f.isNull("fix")) null else f.optString("fix")
                )
            )
        }
        val bl = mutableListOf<String>()
        val ba = o.optJSONArray("blacklist")
        if (ba != null) for (i in 0 until ba.length()) bl.add(ba.optString(i))
        RepairData(out, bl)
    } catch (e: Exception) {
        null
    }
}

data class BypassNode(
    val path: String,
    val desc: String,
    val exists: Boolean,
    val writable: Boolean,
    val state: String,
    val risky: Boolean
)

data class BypassData(
    val supported: Boolean,
    val charger: Boolean,
    val enabled: Boolean,
    val active: Boolean,
    val node: String,
    val threshold: Int,
    val batteryPct: Int,
    val nodes: List<BypassNode>
)

object BypassParser {
    fun parse(json: String): BypassData? = try {
        val o = JSONObject(json)
        val nodes = mutableListOf<BypassNode>()
        val a = o.optJSONArray("nodes")
        if (a != null) for (i in 0 until a.length()) {
            val n = a.optJSONObject(i) ?: continue
            nodes.add(
                BypassNode(
                    n.optString("path"), n.optString("desc"), n.optBoolean("exists"),
                    n.optBoolean("writable"), n.optString("state"), n.optBoolean("risky")
                )
            )
        }
        BypassData(
            o.optBoolean("supported"), o.optBoolean("charger"), o.optBoolean("enabled"),
            o.optBoolean("active"), o.optString("node"), o.optInt("threshold", 80),
            o.optInt("battery_pct", 0), nodes
        )
    } catch (e: Exception) {
        null
    }
}
