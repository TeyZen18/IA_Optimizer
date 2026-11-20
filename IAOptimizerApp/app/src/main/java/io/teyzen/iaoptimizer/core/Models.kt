package io.teyzen.iaoptimizer.core

import org.json.JSONObject

/** Estado y telemetria parseados (JSON del demonio). */
object Models {

    data class State(
        val profile: String,
        val auto: Boolean,
        val ai: Boolean,
        val gaming: Boolean,
        val knobs: Int,
        val suggestionProfile: String,
        val confidence: Float,
        val reason: String,
        val batteryPct: Int,
        val charging: Boolean,
        val cpuLoad: Float,
        val tempC: Float,
        val freqCap: Float
    )

    data class Telemetry(
        val tempC: Float,
        val cpuMhz: Long,
        val cpuMaxMhz: Long,
        val governor: String,
        val ramPct: Float,
        val batteryPct: Int,
        val charging: Boolean,
        val currentMa: Int,
        val powerW: Float,
        val lowPower: Boolean,
        val voltageV: Float,
        val coresOnline: String,
        val memTotalMb: Long,
        val memAvailMb: Long,
        val zramTotalMb: Long,
        val zramUsedMb: Long,
        val storageTotalMb: Long,
        val storageUsedMb: Long,
        val refreshHz: Float
    )

    fun parseState(json: String): State? = try {
        val o = JSONObject(json)
        val m = o.optJSONObject("metrics") ?: JSONObject()
        val sug = o.optJSONObject("suggestion") ?: JSONObject()
        State(
            profile = o.optString("profile", "balanced"),
            auto = o.optBoolean("auto", false),
            ai = o.optBoolean("ai", false),
            gaming = o.optBoolean("gaming", false),
            knobs = o.optInt("knobs", 0),
            suggestionProfile = sug.optString("profile", "-"),
            confidence = sug.optDouble("confidence", 0.0).toFloat(),
            reason = sug.optString("reason", "-"),
            batteryPct = m.optInt("battery_pct", 0),
            charging = m.optBoolean("charging", false),
            cpuLoad = m.optDouble("cpu_load", 0.0).toFloat(),
            tempC = m.optDouble("battery_temp_c", 0.0).toFloat(),
            freqCap = o.optDouble("freq_cap", 1.0).toFloat()
        )
    } catch (e: Exception) {
        null
    }

    fun parseTelemetry(json: String): Telemetry? = try {
        val o = JSONObject(json)
        val bat = o.optJSONObject("bat") ?: JSONObject()
        val mem = o.optJSONObject("mem") ?: JSONObject()
        Telemetry(
            tempC = bat.optDouble("temp_c", 0.0).toFloat(),
            cpuMhz = o.optJSONObject("cpu")?.optLong("cur_mhz", 0) ?: 0L,
            cpuMaxMhz = o.optJSONObject("cpu")?.optLong("max_mhz", 0) ?: 0L,
            governor = readGovernor(),
            // "RAM en uso" es memoria ocupada real, no la presión PSI (que es
            // otra métrica): se deriva de total y disponible.
            ramPct = if (mem.optLong("total_mb", 0) > 0) {
                ((mem.optLong("total_mb", 0) - mem.optLong("avail_mb", 0)).toFloat() /
                    mem.optLong("total_mb", 1).toFloat() * 100f)
            } else {
                mem.optDouble("pressure", 0.0).toFloat()
            },
            batteryPct = bat.optInt("pct", 0),
            charging = bat.optBoolean("charging", false),
            currentMa = bat.optInt("current_ma", 0),
            powerW = bat.optDouble("power_w", 0.0).toFloat(),
            lowPower = o.optString("low_power", "0").trim() == "1",
            voltageV = o.optDouble("voltage_uv", 0.0).toFloat() / 1_000_000f,
            coresOnline = o.optString("cores_online", "-"),
            memTotalMb = mem.optLong("total_mb", 0),
            memAvailMb = mem.optLong("avail_mb", 0),
            zramTotalMb = o.optJSONObject("zram")?.optLong("total_mb", 0) ?: 0L,
            zramUsedMb = o.optJSONObject("zram")?.optLong("used_mb", 0) ?: 0L,
            storageTotalMb = o.optJSONObject("storage")?.optLong("total_mb", 0) ?: 0L,
            storageUsedMb = o.optJSONObject("storage")?.optLong("used_mb", 0) ?: 0L,
            refreshHz = o.optDouble("refresh_hz", 0.0).toFloat()
        )
    } catch (e: Exception) {
        null
    }

    private fun readGovernor(): String = try {
        java.io.File("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
            .readText().trim()
    } catch (e: Exception) {
        "-"
    }
}
