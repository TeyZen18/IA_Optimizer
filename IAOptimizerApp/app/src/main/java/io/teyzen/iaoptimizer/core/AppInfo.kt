package io.teyzen.iaoptimizer.core

import android.graphics.drawable.Drawable

/**
 * App instalada con los campos que consume la UI.
 * El icono se resuelve aparte (cacheado) para no cargar drawables repetidos.
 */
data class AppInfo(
    val packageName: String,
    val label: String,
    val isSystem: Boolean,
    val isGame: Boolean,
    val sizeMb: Long,
    val versionName: String,
    val firstInstall: Long,
    val icon: Drawable?
)
