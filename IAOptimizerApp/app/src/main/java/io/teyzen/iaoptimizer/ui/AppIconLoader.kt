package io.teyzen.iaoptimizer.ui

import android.content.Context
import android.graphics.drawable.Drawable
import io.teyzen.iaoptimizer.core.AppRepository

/** Carga de iconos fuera del hilo de UI (el cache hace el resto). */
object AppIconLoader {
    fun load(context: Context, pkg: String): Drawable? = AppRepository.icon(context, pkg)
}
