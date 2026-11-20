package io.teyzen.iaoptimizer.core

import java.util.concurrent.ConcurrentHashMap

/**
 * Caché en memoria del último dato bueno de cada endpoint del daemon.
 *
 * Objetivo: cero pantallas vacías y cero esperas al cambiar de pestaña.
 * Cada pantalla pinta INMEDIATAMENTE con lo último conocido y refresca en
 * segundo plano; cuando llega el dato nuevo, repinta sin bloquear la UI.
 * El puente (RustBridge) alimenta esta caché en cada respuesta correcta.
 */
object StateCache {
    private data class Entry(val t: Long, val json: String)

    private val map = ConcurrentHashMap<String, Entry>()

    fun put(key: String, json: String) {
        if (json.isEmpty()) return
        val c = json.trimStart().firstOrNull()
        if (c != '{' && c != '[') return // solo JSON válido
        map[key] = Entry(System.currentTimeMillis(), json)
    }

    /** Último JSON conocido ("" si aún no hay ninguno). */
    fun get(key: String): String = map[key]?.json ?: ""

    /** Edad del dato en segundos (Int.MAX_VALUE si no hay dato). */
    fun ageS(key: String): Int {
        val e = map[key] ?: return Int.MAX_VALUE
        return ((System.currentTimeMillis() - e.t) / 1000).toInt()
    }

    fun has(key: String): Boolean = map.containsKey(key)

    // Atajos con la ruta canónica de cada endpoint.
    fun state(): String = get("/api/state")
    fun agent(): String = get("/api/agent")
    fun telemetry(): String = get("/api/telemetry")
    fun scan(): String = get("/api/scan")
    fun history(): String = get("/api/history")
    fun thermal(): String = get("/api/thermal")
    fun warnings(): String = get("/api/warnings")
    fun device(): String = get("/api/device")
    fun display(): String = get("/api/display")
}
