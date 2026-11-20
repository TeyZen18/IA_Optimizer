package io.teyzen.iaoptimizer.core

import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.drawable.Drawable
import android.os.Build
import android.util.LruCache

/**
 * Listado rapido de apps usando PackageManager nativo (sin shell).
 * - Cachea la lista y los iconos.
 * - Resuelve el nombre real legible y el icono en alta calidad.
 */
object AppRepository {

    private const val CACHE_SIZE = 160

    private var cached: List<AppInfo> = emptyList()
    private var cacheStamp: Long = 0
    private val iconCache = LruCache<String, Drawable.ConstantState>(CACHE_SIZE)

    enum class Filter { ALL, USER, SYSTEM, GAMES }

    @Synchronized
    fun list(context: Context, filter: Filter, force: Boolean = false): List<AppInfo> {
        val now = System.currentTimeMillis()
        if (!force && cached.isNotEmpty() && now - cacheStamp < 30_000) {
            return applyFilter(filter)
        }
        val pm = context.packageManager
        val flags = PackageManager.GET_META_DATA
        val apps = mutableListOf<AppInfo>()
        val packages: List<ApplicationInfo> = try {
            if (Build.VERSION.SDK_INT >= 33) {
                pm.getInstalledApplications(PackageManager.ApplicationInfoFlags.of(flags.toLong()))
            } else {
                @Suppress("DEPRECATION")
                pm.getInstalledApplications(flags)
            }
        } catch (e: Exception) {
            emptyList()
        }
        val games = GameStore.games(context)
        for (ai in packages) {
            val label = try {
                pm.getApplicationLabel(ai).toString()
            } catch (e: Exception) {
                ai.packageName
            }
            val size = try {
                ai.sourceDir?.let { java.io.File(it).length() } ?: 0L
            } catch (e: Exception) {
                0L
            }
            apps.add(
                AppInfo(
                    packageName = ai.packageName,
                    label = label,
                    isSystem = (ai.flags and ApplicationInfo.FLAG_SYSTEM) != 0,
                    isGame = games.contains(ai.packageName),
                    sizeMb = size / (1024 * 1024),
                    versionName = versionName(pm, ai.packageName),
                    firstInstall = firstInstall(pm, ai.packageName),
                    icon = null
                )
            )
        }
        apps.sortWith(compareByDescending<AppInfo> { it.isGame }.thenBy { it.label.lowercase() })
        cached = apps
        cacheStamp = now
        return applyFilter(filter)
    }

    private fun applyFilter(filter: Filter): List<AppInfo> = when (filter) {
        Filter.ALL -> cached
        Filter.USER -> cached.filter { !it.isSystem }
        Filter.SYSTEM -> cached.filter { it.isSystem }
        Filter.GAMES -> cached.filter { it.isGame }
    }

    /** Icono real, cacheado por paquete. */
    fun icon(context: Context, packageName: String): Drawable? {
        iconCache.get(packageName)?.let { return it.newDrawable(context.resources) }
        val pm = context.packageManager
        val d = try {
            if (Build.VERSION.SDK_INT >= 33) {
                pm.getApplicationIcon(packageName)
            } else {
                @Suppress("DEPRECATION")
                pm.getApplicationIcon(packageName)
            }
        } catch (e: Exception) {
            null
        }
        if (d != null) {
            try { iconCache.put(packageName, d.constantState) } catch (e: Exception) { }
        }
        return d
    }

    /** Inicio seguro mediante Intent nativo. */
    fun launch(context: Context, packageName: String): Boolean {
        return try {
            val intent = context.packageManager.getLaunchIntentForPackage(packageName)
            if (intent == null) {
                // Sin actividad launcher: reenviar al usuario con un intent explicito de ajustes.
                false
            } else {
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                context.startActivity(intent)
                true
            }
        } catch (e: Exception) {
            false
        }
    }

    fun exists(context: Context, packageName: String): Boolean = try {
        if (Build.VERSION.SDK_INT >= 33) {
            context.packageManager.getPackageInfo(packageName, PackageManager.PackageInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION")
            context.packageManager.getPackageInfo(packageName, 0)
        }
        true
    } catch (e: Exception) {
        false
    }

    private fun versionName(pm: PackageManager, pkg: String): String = try {
        if (Build.VERSION.SDK_INT >= 33) {
            pm.getPackageInfo(pkg, PackageManager.PackageInfoFlags.of(0)).versionName ?: ""
        } else {
            @Suppress("DEPRECATION")
            pm.getPackageInfo(pkg, 0).versionName ?: ""
        }
    } catch (e: Exception) {
        ""
    }

    private fun firstInstall(pm: PackageManager, pkg: String): Long = try {
        if (Build.VERSION.SDK_INT >= 33) {
            pm.getPackageInfo(pkg, PackageManager.PackageInfoFlags.of(0)).firstInstallTime
        } else {
            @Suppress("DEPRECATION")
            pm.getPackageInfo(pkg, 0).firstInstallTime
        }
    } catch (e: Exception) {
        0L
    }
}
