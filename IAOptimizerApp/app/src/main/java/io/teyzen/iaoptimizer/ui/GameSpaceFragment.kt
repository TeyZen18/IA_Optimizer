package io.teyzen.iaoptimizer.ui

import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.Toast
import androidx.fragment.app.Fragment
import androidx.recyclerview.widget.LinearLayoutManager
import io.teyzen.iaoptimizer.MainActivity
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.AppInfo
import io.teyzen.iaoptimizer.core.AppRepository
import io.teyzen.iaoptimizer.core.GameStore
import io.teyzen.iaoptimizer.core.RustBridge
import io.teyzen.iaoptimizer.databinding.FragmentGameSpaceBinding
import io.teyzen.iaoptimizer.hud.HudController
import kotlin.concurrent.thread

/** Game Space: juegos marcados, con perfil propio, HUD y inicio rapido. */
class GameSpaceFragment : Fragment(), io.teyzen.iaoptimizer.VisibleTab {

    private var _binding: FragmentGameSpaceBinding? = null
    private val binding get() = _binding!!

    private val adapter = GameAdapter(
        onLaunch = { launch(it) },
        onProfile = { app, profile -> applyProfile(app, profile) },
        onHud = { app, on -> toggleHud(app, on) }
    )
    private var reloadToken = 0
    private var aiActive = true

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View {
        _binding = FragmentGameSpaceBinding.inflate(inflater, container, false)
        return binding.root
    }

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        binding.carousel.layoutManager =
            LinearLayoutManager(requireContext(), LinearLayoutManager.HORIZONTAL, false)
        binding.carousel.adapter = adapter
        binding.btnManage.setOnClickListener {
            (activity as? MainActivity)?.selectTab(R.id.nav_apps)
        }
        binding.btnRecord.setOnClickListener {
            it.isEnabled = false
            thread {
                val action = if (recording) "stop" else "start"
                val r = RustBridge.post("/api/recorder", org.json.JSONObject().put("action", action).toString())
                val ok = r.contains("\"ok\":true")
                view.post {
                    if (_binding == null) return@post
                    binding.btnRecord.isEnabled = true
                    if (!ok) {
                        val detail = try { org.json.JSONObject(r).optString("detail", "") } catch (e: Exception) { "" }
                        Toast.makeText(requireContext(), detail.ifEmpty { "Sin conexión con el motor" }, Toast.LENGTH_SHORT).show()
                    }
                    refreshRecorder()
                }
            }
        }
        refreshRecorder()
    }

    override fun onResume() {
        super.onResume()
        reload()
    }

    override fun onTabVisible() {
        reload()
    }

    override fun onDestroyView() {
        super.onDestroyView()
        _binding = null
    }

    private fun reload() {
        if (_binding == null) return
        val ctx = context ?: return
        val token = ++reloadToken
        Thread {
            aiActive = try {
                org.json.JSONObject(RustBridge.stateJson()).optBoolean("ai", true)
            } catch (e: Exception) {
                true
            }
            val games = AppRepository.list(ctx, AppRepository.Filter.GAMES, force = true)
            view?.post {
                if (_binding == null || token != reloadToken) return@post
                adapter.submit(games)
                binding.empty.visibility = if (games.isEmpty()) View.VISIBLE else View.GONE
            }
            Thread {
                for (app in games) AppRepository.icon(ctx, app.packageName)
            }.apply { isDaemon = true }.start()
        }.apply { isDaemon = true }.start()
    }

    /** Guarda el perfil del juego y lo aplica al daemon. */
    private fun applyProfile(app: AppInfo, profile: String) {
        if (aiActive) {
            Toast.makeText(requireContext(), "IA Motors está activa: apágala en Inicio para fijar perfiles manuales", Toast.LENGTH_SHORT).show()
            return
        }
        thread {
            RustBridge.appSetting(app.packageName, "profile", profile)
            val r = RustBridge.applyApp(app.packageName)
            view?.post {
                if (_binding != null) {
                    Toast.makeText(
                        requireContext(),
                        "Perfil aplicado: ${Labels_app(profile)}",
                        Toast.LENGTH_SHORT
                    ).show()
                }
            }
        }
    }

    private fun launch(app: AppInfo) {
        val profile = GameStore.profile(requireContext(), app.packageName)
        if (!aiActive) {
            thread {
                RustBridge.appSetting(app.packageName, "profile", profile)
                RustBridge.applyApp(app.packageName)
            }
        }
        if (!AppRepository.launch(requireContext(), app.packageName)) {
            Toast.makeText(requireContext(), "No se pudo abrir la app", Toast.LENGTH_SHORT).show()
        }
    }

    private fun toggleHud(app: AppInfo, on: Boolean) {
        if (on) {
            if (!HudController.canOverlay(requireContext())) {
                HudController.requestOverlay(requireContext())
                Toast.makeText(requireContext(), "Concede el permiso de superposición", Toast.LENGTH_LONG).show()
                return
            }
            HudController.start(requireContext(), app.packageName)
            Toast.makeText(requireContext(), "HUD activado para ${app.label}", Toast.LENGTH_SHORT).show()
        } else {
            HudController.stop(requireContext())
        }
    }

    private fun Labels_app(profile: String): String = when (profile) {
        "eco" -> "Eco"
        "performance" -> "Rendimiento"
        "balanced" -> "Balanceado"
        else -> "Automático"
    }

    // ------------------- Grabadora de sesiones -------------------

    private var recording = false
    private var recSessionsShown = -1

    /** Estado + listado de la grabadora. Las filas solo se reconstruyen si
     *  cambia el número de sesiones: el refresco no mueve el scroll. */
    fun refreshRecorder() {
        if (_binding == null) return
        val ctx = context ?: return
        thread {
            val o = try { org.json.JSONObject(RustBridge.http("/api/recorder")) } catch (e: Exception) { null }
            view?.post {
                if (_binding == null) return@post
                if (o == null) {
                    binding.recStatus.text = "Sin conexión con el motor."
                    return@post
                }
                recording = o.optBoolean("recording", false)
                val elapsed = o.optInt("elapsed_s", 0)
                binding.btnRecord.text = if (recording) "Detener grabación (${elapsed} s)" else "Grabar sesión"
                binding.recStatus.text = if (recording) {
                    "Grabando ${o.optString("package")} · ${elapsed} s · se detiene sola al salir del juego."
                } else {
                    "Graba FPS reales y telemetría del juego en primer plano."
                }
                val sessions = o.optJSONArray("sessions")
                val count = sessions?.length() ?: 0
                if (count != recSessionsShown) {
                    recSessionsShown = count
                    binding.recList.removeAllViews()
                    val fmt = java.text.SimpleDateFormat("dd/MM HH:mm", java.util.Locale.getDefault())
                    if (sessions != null) for (i in 0 until minOf(count, 10)) {
                        val s = sessions.optJSONObject(i) ?: continue
                        val p = (10 * resources.displayMetrics.density).toInt()
                        val card = android.widget.LinearLayout(ctx).apply {
                            orientation = android.widget.LinearLayout.VERTICAL
                            setPadding(p, p, p, p)
                            setBackgroundResource(R.drawable.bg_card)
                            layoutParams = android.widget.LinearLayout.LayoutParams(
                                android.widget.LinearLayout.LayoutParams.MATCH_PARENT,
                                android.widget.LinearLayout.LayoutParams.WRAP_CONTENT
                            ).apply { bottomMargin = p / 2 }
                        }
                        val when_ = fmt.format(java.util.Date(s.optLong("t", 0) * 1000))
                        card.addView(android.widget.TextView(ctx).apply {
                            text = "${s.optString("package")} · $when_"
                            setTextColor(resources.getColor(R.color.on_surface, null))
                            textSize = 12f
                            typeface = android.graphics.Typeface.DEFAULT_BOLD
                        })
                        val low1 = if (s.isNull("low1_fps")) "—" else String.format(java.util.Locale.US, "%.1f", s.optDouble("low1_fps", 0.0))
                        card.addView(android.widget.TextView(ctx).apply {
                            text = "${s.optInt("duration_s")} s · media ${String.format(java.util.Locale.US, "%.1f", s.optDouble("avg_fps", 0.0))} FPS · peor 1 %: $low1 · jank ${s.optInt("jank_s")} s"
                            setTextColor(resources.getColor(R.color.on_surface_variant, null))
                            textSize = 11f
                        })
                        binding.recList.addView(card)
                    }
                }
            }
        }
    }
}
