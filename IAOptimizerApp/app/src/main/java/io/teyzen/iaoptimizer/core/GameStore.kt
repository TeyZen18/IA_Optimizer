package io.teyzen.iaoptimizer.core

import android.content.Context
import org.json.JSONObject

/**
 * Persistencia ligera de los marcadores por app y del perfil elegido.
 * SharedPreferences: suficiente para decenas de apps y sin dependencias.
 */
object GameStore {

    private const val PREFS = "ia_optimizer_app"
    private const val KEY_GAMES = "games"
    private const val KEY_PROFILE = "profile:"
    private const val KEY_SETTING = "setting:"

    private fun prefs(context: Context) =
        context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun readSet(context: Context): MutableSet<String> =
        HashSet(prefs(context).getStringSet(KEY_GAMES, emptySet()) ?: emptySet())

    fun games(context: Context): Set<String> = readSet(context)

    fun isGame(context: Context, pkg: String): Boolean = games(context).contains(pkg)

    fun setGame(context: Context, pkg: String, game: Boolean) {
        val set = readSet(context)
        if (game) set.add(pkg) else set.remove(pkg)
        prefs(context).edit().putStringSet(KEY_GAMES, set).apply()
    }

    fun profile(context: Context, pkg: String): String =
        prefs(context).getString(KEY_PROFILE + pkg, "auto") ?: "auto"

    fun setProfile(context: Context, pkg: String, profile: String) {
        prefs(context).edit().putString(KEY_PROFILE + pkg, profile).apply()
    }

    fun setting(context: Context, pkg: String, key: String, value: String) {
        prefs(context).edit().putString(KEY_SETTING + pkg + ":" + key, value).apply()
    }

    fun setting(context: Context, pkg: String, key: String): String =
        prefs(context).getString(KEY_SETTING + pkg + ":" + key, "") ?: ""

    /** Perfil global del sistema (eco / balanced / performance). */
    fun systemProfile(context: Context): String =
        prefs(context).getString("system_profile", "balanced") ?: "balanced"

    fun setSystemProfile(context: Context, profile: String) {
        prefs(context).edit().putString("system_profile", profile).apply()
    }

    /** Tema de la app: "sistema", "claro" u "oscuro". */
    fun themeMode(context: Context): String =
        prefs(context).getString("theme_mode", "sistema") ?: "sistema"

    fun setThemeMode(context: Context, mode: String) {
        prefs(context).edit().putString("theme_mode", mode).apply()
    }

    /** Panel de juego in-game (Game Space avanzado): activado por el usuario. */
    fun panelEnabled(context: Context): Boolean =
        prefs(context).getBoolean("game_panel_enabled", false)

    fun setPanelEnabled(context: Context, enabled: Boolean) {
        prefs(context).edit().putBoolean("game_panel_enabled", enabled).apply()
    }

    /** Widgets de telemetría elegidos por juego (csv de claves). */
    fun widgets(context: Context, pkg: String): String =
        setting(context, pkg, "widgets").ifEmpty { "fps,cpu,gpu,temp,ram,bat,profile" }

    fun setWidgets(context: Context, pkg: String, csv: String) {
        setting(context, pkg, "widgets", csv)
    }

    /** Parser del JSON reducido que publica el demonio en /api/apps. */
    fun fromDaemonJson(json: String): List<AppInfo> {
        val out = mutableListOf<AppInfo>()
        val arr = try {
            JSONObject(json).getJSONArray("apps")
        } catch (e: Exception) {
            return out
        }
        for (i in 0 until arr.length()) {
            val o = arr.optJSONObject(i) ?: continue
            out.add(
                AppInfo(
                    packageName = o.optString("package"),
                    label = o.optString("name"),
                    isSystem = o.optBoolean("system"),
                    isGame = o.optBoolean("is_game"),
                    sizeMb = o.optLong("size_mb"),
                    versionName = o.optString("installed"),
                    firstInstall = 0L,
                    icon = null
                )
            )
        }
        return out
    }
}
