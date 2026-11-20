package io.teyzen.iaoptimizer

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.content.ContextCompat
import androidx.fragment.app.Fragment
import io.teyzen.iaoptimizer.core.GameStore
import io.teyzen.iaoptimizer.core.RustBridge
import io.teyzen.iaoptimizer.databinding.ActivityMainBinding
import io.teyzen.iaoptimizer.ui.AppsFragment
import io.teyzen.iaoptimizer.ui.HomeFragment
import io.teyzen.iaoptimizer.ui.MotorsFragment
import io.teyzen.iaoptimizer.ui.SettingsFragment

/** Pestaña que quiere enterarse de cuándo se muestra y cuándo se oculta. */
interface VisibleTab {
    fun onTabVisible()
    fun onTabHidden() {}
}

/**
 * Actividad principal: barra de navegacion inferior + fragmentos.
 *
 * Los fragmentos se crean una sola vez y se ocultan/muestran (add/hide/show):
 * cambiar de pestaña no re-infla la vista ni repite las consultas al daemon,
 * asi que la transicion es instantanea. Cada pestaña refresca sus datos en
 * segundo plano al mostrarse (VisibleTab), nunca bloqueando la UI.
 */
class MainActivity : AppCompatActivity() {
    private lateinit var binding: ActivityMainBinding
    private val frags = HashMap<Int, Fragment>()
    private var currentId = 0

    private val notifPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        // El tema se aplica antes de inflar: si no, la primera pantalla parpadea.
        when (GameStore.themeMode(this)) {
            "claro" -> AppCompatDelegate.setDefaultNightMode(AppCompatDelegate.MODE_NIGHT_NO)
            "oscuro" -> AppCompatDelegate.setDefaultNightMode(AppCompatDelegate.MODE_NIGHT_YES)
            else -> AppCompatDelegate.setDefaultNightMode(AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM)
        }
        super.onCreate(savedInstanceState)
        binding = ActivityMainBinding.inflate(layoutInflater)
        setContentView(binding.root)

        binding.bottomNav.setOnItemSelectedListener { item ->
            show(item.itemId)
            true
        }
        if (savedInstanceState == null) {
            binding.bottomNav.selectedItemId = R.id.nav_inicio
            // Garantiza el primer fragmento aunque el listener no dispare
            // (si nav_inicio ya era el item seleccionado por defecto).
            show(R.id.nav_inicio)
        } else {
            // Tras recrear (p. ej. cambio de tema) el FragmentManager ya tiene
            // los fragmentos: se reutilizan en vez de crear otros nuevos.
            for (id in tabIds()) {
                supportFragmentManager.findFragmentByTag(tag(id))?.let { frags[id] = it }
            }
            currentId = binding.bottomNav.selectedItemId
            val tx = supportFragmentManager.beginTransaction()
            for ((id, f) in frags) {
                if (id == currentId) tx.show(f) else tx.hide(f)
            }
            tx.commitNowAllowingStateLoss()
            (frags[currentId] as? VisibleTab)?.onTabVisible()
        }
        ensurePermissions()
        // Precarga en segundo plano: el primer dato de cada pantalla llega a
        // la caché antes de que el usuario navegue, así ninguna pestaña abre
        // en blanco ni espera al daemon.
        kotlin.concurrent.thread {
            RustBridge.stateJson()
            RustBridge.agentJson()
            RustBridge.telemetryJson()
            RustBridge.scanJson()
        }
        // El panel in-game se rearma solo si el usuario lo dejó activado.
        if (GameStore.panelEnabled(this) && android.provider.Settings.canDrawOverlays(this)) {
            io.teyzen.iaoptimizer.game.GamePanelController.start(this)
        }
    }

    private fun tabIds() = listOf(
        R.id.nav_inicio, R.id.nav_apps, R.id.nav_motors, R.id.nav_ajustes
    )

    private fun tag(itemId: Int) = "tab_$itemId"

    private fun newFragment(itemId: Int): Fragment = when (itemId) {
        R.id.nav_apps -> AppsFragment()
        R.id.nav_motors -> MotorsFragment()
        R.id.nav_ajustes -> SettingsFragment()
        else -> HomeFragment()
    }

    private fun show(itemId: Int) {
        val previous = frags[currentId]
        if (itemId == currentId && previous != null) {
            (previous as? VisibleTab)?.onTabVisible()
            return
        }
        (previous as? VisibleTab)?.onTabHidden()
        val tx = supportFragmentManager.beginTransaction()
        if (previous != null) tx.hide(previous)
        val f = frags.getOrPut(itemId) { newFragment(itemId) }
        if (f.isAdded) tx.show(f) else tx.add(R.id.nav_host, f, tag(itemId))
        tx.commitNowAllowingStateLoss()
        currentId = itemId
        (f as? VisibleTab)?.onTabVisible()
    }

    /** Cambia de pestana desde un fragmento. */
    fun selectTab(itemId: Int) {
        binding.bottomNav.selectedItemId = itemId
    }

    private fun ensurePermissions() {
        if (Build.VERSION.SDK_INT >= 33) {
            val granted = ContextCompat.checkSelfPermission(
                this, Manifest.permission.POST_NOTIFICATIONS
            ) == PackageManager.PERMISSION_GRANTED
            if (!granted) notifPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
}
