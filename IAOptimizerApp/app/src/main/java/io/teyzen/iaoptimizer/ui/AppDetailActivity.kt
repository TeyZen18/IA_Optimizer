package io.teyzen.iaoptimizer.ui

import android.os.Bundle
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.AppRepository
import io.teyzen.iaoptimizer.core.GameStore
import io.teyzen.iaoptimizer.core.RustBridge
import io.teyzen.iaoptimizer.databinding.ActivityAppDetailBinding

/** Configuracion por app: marcar juego, perfil y limites. */
class AppDetailActivity : AppCompatActivity() {

    companion object {
        const val EXTRA_PACKAGE = "package"
        const val EXTRA_LABEL = "label"
    }

    private lateinit var binding: ActivityAppDetailBinding
    private lateinit var pkg: String
    private var label: String = ""
    private var syncing = false
    private var aiActive = true

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityAppDetailBinding.inflate(layoutInflater)
        setContentView(binding.root)

        pkg = intent.getStringExtra(EXTRA_PACKAGE) ?: ""
        label = intent.getStringExtra(EXTRA_LABEL) ?: pkg
        if (pkg.isEmpty()) { finish(); return }

        binding.name.text = label
        binding.pkg.text = pkg
        AppRepository.icon(this, pkg)?.let { binding.icon.setImageDrawable(it) }

        binding.switchGame.isChecked = GameStore.isGame(this, pkg)
        binding.switchGame.setOnCheckedChangeListener { _, on ->
            GameStore.setGame(this, pkg, on)
            setStatus(if (on) "Marcada como juego" else "Quitada de Game Space")
        }

        val profile = GameStore.profile(this, pkg)
        binding.switchManual.isChecked = profile != "auto"
        checkProfile(profile)
        binding.switchManual.setOnCheckedChangeListener { _, on ->
            val p = if (on) "manual" else "auto"
            GameStore.setProfile(this, pkg, p)
            checkProfile(p)
            apply()
        }
        binding.profileGroup.addOnButtonCheckedListener { _, checkedId, isChecked ->
            if (syncing) return@addOnButtonCheckedListener
            if (!isChecked) return@addOnButtonCheckedListener
            val p = when (checkedId) {
                R.id.btn_eco -> "eco"
                R.id.btn_balanced -> "balanced"
                R.id.btn_performance -> "performance"
                else -> "auto"
            }
            GameStore.setProfile(this, pkg, p)
            binding.switchManual.isChecked = p != "auto"
            apply()
        }
        binding.btnApply.setOnClickListener { apply() }
        binding.btnOpen.setOnClickListener {
            if (!AppRepository.launch(this, pkg)) {
                Toast.makeText(this, R.string.sin_apps, Toast.LENGTH_SHORT).show()
            }
        }

        // Con IA Motors activa, el perfil por app lo decide la IA: el modo
        // manual y los perfiles fijos quedan bloqueados (marcar juego no:
        // alimenta la clasificación de la IA).
        Thread {
            val ai = try {
                org.json.JSONObject(RustBridge.stateJson()).optBoolean("ai", true)
            } catch (e: Exception) {
                true
            }
            runOnUiThread {
                aiActive = ai
                if (ai) {
                    binding.switchManual.isEnabled = false
                    binding.profileGroup.isEnabled = false
                    binding.btnApply.isEnabled = false
                    setStatus("IA Motors decide el perfil de esta app. Apágala en Inicio para modo manual.")
                }
            }
        }.start()
    }

    private fun checkProfile(p: String) {
        val id = when (p) {
            "eco" -> R.id.btn_eco
            "balanced" -> R.id.btn_balanced
            "performance" -> R.id.btn_performance
            else -> R.id.btn_auto
        }
        if (binding.profileGroup.checkedButtonId != id) {
            syncing = true
            binding.profileGroup.check(id)
            syncing = false
        }
    }

    private fun apply() {
        val p = when (binding.profileGroup.checkedButtonId) {
            R.id.btn_eco -> "eco"
            R.id.btn_balanced -> "balanced"
            R.id.btn_performance -> "performance"
            else -> "auto"
        }
        Thread {
            RustBridge.appSetting(pkg, "mode", if (p == "auto") "auto" else "manual")
            val out = RustBridge.applyApp(pkg)
            runOnUiThread { setStatus(out) }
        }.start()
    }

    private fun setStatus(text: String) {
        binding.textStatus.text = text
    }
}
