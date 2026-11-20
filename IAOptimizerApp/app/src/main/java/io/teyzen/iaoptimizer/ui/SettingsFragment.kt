package io.teyzen.iaoptimizer.ui

import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.fragment.app.Fragment
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.RustBridge
import io.teyzen.iaoptimizer.databinding.FragmentSettingsBinding
import org.json.JSONArray
import org.json.JSONObject
import kotlin.concurrent.thread

/**
 * Ajustes: solo Registro y Acerca de.
 * Pantalla, HUD y tema se movieron a IA Motors (solapas Pantalla y Sistema).
 */
class SettingsFragment : Fragment(), io.teyzen.iaoptimizer.VisibleTab {

    private var _binding: FragmentSettingsBinding? = null
    private val binding get() = _binding!!
    private val bridge = RustBridge

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View {
        _binding = FragmentSettingsBinding.inflate(inflater, container, false)
        return binding.root
    }

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        binding.setTabs.addOnTabSelectedListener(object : com.google.android.material.tabs.TabLayout.OnTabSelectedListener {
            override fun onTabSelected(tab: com.google.android.material.tabs.TabLayout.Tab) {
                binding.setPanes.displayedChild = tab.position
            }
            override fun onTabUnselected(tab: com.google.android.material.tabs.TabLayout.Tab) {}
            override fun onTabReselected(tab: com.google.android.material.tabs.TabLayout.Tab) {}
        })
        listOf("Registro", "Acerca de").forEach {
            binding.setTabs.addTab(binding.setTabs.newTab().setText(it))
        }
        binding.setTabs.getTabAt(0)?.select()

        binding.root.findViewById<View>(R.id.btnSelftest)?.setOnClickListener {
            thread {
                val r = bridge.selftest()
                activity?.runOnUiThread {
                    if (_binding == null) return@runOnUiThread
                    Toast.makeText(requireContext(), r.ifEmpty { "sin conexión" }, Toast.LENGTH_SHORT).show()
                    refreshRegistro()
                }
            }
        }
        binding.root.findViewById<View>(R.id.btnRescan)?.setOnClickListener {
            thread {
                val r = bridge.rescan()
                activity?.runOnUiThread {
                    if (_binding == null) return@runOnUiThread
                    // /api/scan devuelve un array: contar con JSONObject daba 0.
                    val n = try { JSONArray(r).length() } catch (e: Exception) { 0 }
                    Toast.makeText(requireContext(), "controles: $n", Toast.LENGTH_SHORT).show()
                }
            }
        }
        refreshRegistro()
        paintVersion()
    }

    override fun onTabVisible() {
        refreshRegistro()
    }

    /** Versiones reales: la del modulo (daemon) y la de esta app. */
    private fun paintVersion() {
        var installed = ""
        val app = try {
            val ctx = requireContext()
            val info = ctx.packageManager.getPackageInfo(ctx.packageName, 0)
            installed = java.text.SimpleDateFormat("dd/MM/yyyy", java.util.Locale.getDefault())
                .format(java.util.Date(info.firstInstallTime))
            info.versionName ?: "?"
        } catch (e: Exception) {
            "?"
        }
        Thread {
            val mod = try {
                JSONObject(RustBridge.stateJson()).optString("version", "")
            } catch (e: Exception) {
                ""
            }
            val text = "App v$app" +
                (if (installed.isEmpty()) "" else " · instalada el $installed") +
                (if (mod.isEmpty()) "" else " · Módulo v$mod")
            view?.post {
                if (_binding == null) return@post
                binding.root.findViewById<TextView>(R.id.aboutVersion)?.text = text
            }
        }.start()
    }

    private fun refreshRegistro() {
        thread {
            val agent = bridge.agentJson()
            activity?.runOnUiThread {
                if (_binding == null) return@runOnUiThread
                val box = binding.root.findViewById<LinearLayout>(R.id.registroBox) ?: return@runOnUiThread
                box.removeAllViews()
                try {
                    val o = JSONObject(agent)
                    header(box, "Errores sin resolver")
                    val ua = o.optJSONArray("unresolved_errors")
                    if (ua == null || ua.length() == 0) {
                        row(box, "Estado", "sin errores sin resolver")
                    } else {
                        for (i in 0 until ua.length()) {
                            val u = ua.optJSONObject(i) ?: continue
                            row(box, u.optString("kind"), u.optString("path") + " · " + u.optString("reason"))
                        }
                    }
                    header(box, "Actuación del motor")
                    row(box, "Aplicados", o.optInt("applied_count", 0).toString())
                    row(box, "Fallidos", o.optInt("failed_count", 0).toString())
                } catch (e: Exception) {
                    row(box, "Registro", "sin datos")
                }
            }
        }
    }

    private fun header(parent: LinearLayout, text: String) {
        val tv = TextView(requireContext()).apply {
            this.text = text
            setTextColor(resources.getColor(R.color.primary, null))
            textSize = 12f
            letterSpacing = 0.08f
            setPadding(14, 16, 14, 6)
        }
        parent.addView(tv)
    }

    private fun row(parent: LinearLayout, title: String, value: String) {
        val v = layoutInflater.inflate(R.layout.item_row, parent, false)
        v.findViewById<TextView>(R.id.rowTitle).text = title
        v.findViewById<TextView>(R.id.rowValue).text = value
        parent.addView(v)
    }

    override fun onDestroyView() {
        super.onDestroyView()
        _binding = null
    }
}
