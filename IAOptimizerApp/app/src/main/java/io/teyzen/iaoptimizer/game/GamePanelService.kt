package io.teyzen.iaoptimizer.game

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.provider.Settings
import android.util.TypedValue
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView
import io.teyzen.iaoptimizer.MainActivity
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.GameStore
import io.teyzen.iaoptimizer.core.RustBridge
import io.teyzen.iaoptimizer.core.StateCache
import io.teyzen.iaoptimizer.hud.HudController
import org.json.JSONObject
import kotlin.math.abs

/**
 * Game Space avanzado in-game: centro de control deslizable sobre el juego.
 *
 * - Disparador de borde: una asa fina en el lateral; se arrastra verticalmente
 *   y se desliza hacia dentro (o se toca) para abrir el panel.
 * - Panel: telemetría en vivo (FPS, CPU, GPU, temperatura, RAM, batería,
 *   perfil) + controles en caliente: perfil Eco/Balanceado/Rendimiento/Extremo
 *   (sesión que la IA respeta), no molestar, bloqueo de brillo, HUD y refresco.
 * - Aparece solo con los juegos configurados o detectados; al salir del juego
 *   se recoge solo. La protección térmica del daemon siempre gana.
 *
 * Servicio en primer plano con TYPE_APPLICATION_OVERLAY, mismo patrón que el
 * HUD pero con paleta propia (overlay oscuro gaming, acento verde-cian).
 */
class GamePanelService : Service() {

    companion object {
        const val ACTION_START = "io.teyzen.iaoptimizer.game.START"
        const val ACTION_STOP = "io.teyzen.iaoptimizer.game.STOP"
        private const val CHANNEL = "ia_game_panel"
        private const val NOTIF_ID = 0x0902
        private const val WATCH_MS = 2000L
        private const val ACCENT = "#34D399"
        private const val ACCENT2 = "#22D3EE"
        private const val PANEL_BG = "#F20A0F14"
        private const val CARD_BG = "#1C2530"
        private const val TEXT = "#E8F1EE"
        private const val TEXT_DIM = "#9FB4AC"
    }

    private lateinit var wm: WindowManager
    private val handler = Handler(Looper.getMainLooper())
    private var worker: Thread? = null
    @Volatile private var running = false
    private var tick = 0

    private var trigger: View? = null
    private var panel: View? = null
    private var panelOpen = false
    private var currentPkg: String? = null
    private var triggerY = 320
    private var lastFps = 0
    private var lastGpu = 0L
    private var daemonGames = setOf<String>()
    private var syncingPanel = false
    private var selectedProfile = ""
    private var selectedExtreme = false

    // Vistas de telemetría del panel.
    private var fpsView: TextView? = null
    private var cpuView: TextView? = null
    private var gpuView: TextView? = null
    private var tempView: TextView? = null
    private var ramView: TextView? = null
    private var batView: TextView? = null
    private var profileView: TextView? = null
    private val profilePills = mutableMapOf<String, TextView>()
    private var dndSwitch: Switch? = null
    private var brightSwitch: Switch? = null
    private var hudSwitch: Switch? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        wm = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                hideAll()
                stopSelf()
                return START_NOT_STICKY
            }
            else -> {
                startForegroundCompat()
                startLoop()
            }
        }
        return START_STICKY
    }

    // ------------------------- servicio en primer plano -------------------------

    private fun startForegroundCompat() {
        val notif = buildNotification()
        if (Build.VERSION.SDK_INT >= 34) {
            try {
                startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
                return
            } catch (e: Exception) { }
        }
        startForeground(NOTIF_ID, notif)
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT >= 26) {
            val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (nm.getNotificationChannel(CHANNEL) == null) {
                val ch = NotificationChannel(CHANNEL, "Panel de juego", NotificationManager.IMPORTANCE_LOW)
                ch.description = "Centro de control in-game de IA Motors"
                ch.setShowBadge(false)
                nm.createNotificationChannel(ch)
            }
        }
    }

    private fun buildNotification(): Notification {
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val stop = PendingIntent.getService(
            this, 1, Intent(this, GamePanelService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val b = if (Build.VERSION.SDK_INT >= 26) Notification.Builder(this, CHANNEL)
        else @Suppress("DEPRECATION") Notification.Builder(this)
        b.setContentTitle("Panel de juego activo")
            .setContentText("Desliza el asa del borde dentro de tus juegos")
            .setSmallIcon(R.drawable.ic_games)
            .setOngoing(true)
            .setContentIntent(open)
            .addAction(R.drawable.ic_games, "Apagar", stop)
        return b.build()
    }

    private fun dp(v: Int): Int = TypedValue.applyDimension(
        TypedValue.COMPLEX_UNIT_DIP, v.toFloat(), resources.displayMetrics
    ).toInt()

    private fun overlayType(): Int =
        if (Build.VERSION.SDK_INT >= 26) WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
        else @Suppress("DEPRECATION") WindowManager.LayoutParams.TYPE_PHONE

    // ------------------------- bucle vigilante -------------------------

    private fun startLoop() {
        if (worker != null) return
        running = true
        worker = Thread {
            while (running) {
                try {
                    val stateRaw = RustBridge.http("/api/state").ifEmpty { StateCache.state() }
                    val st = try { JSONObject(stateRaw) } catch (e: Exception) { null }
                    val pkg = st?.optString("app", "") ?: ""
                    if (tick % 15 == 0) daemonGames = fetchDaemonGames()
                    val isGame = pkg.isNotEmpty() && pkg != packageName &&
                        (GameStore.isGame(this, pkg) || daemonGames.contains(pkg))
                    val profile = st?.optString("profile", "") ?: ""

                    if (panelOpen && isGame) updateTelemetry(pkg)

                    handler.post {
                        if (isGame) {
                            if (currentPkg != pkg) {
                                hideAll()
                                currentPkg = pkg
                                showTrigger()
                            }
                            if (panelOpen) paintProfileState(profile)
                        } else if (currentPkg != null) {
                            hideAll()
                        }
                    }
                    if (panelOpen && tick % 5 == 0) syncSession()
                } catch (e: Exception) {
                    // El bucle nunca muere por un dato malo.
                }
                try { Thread.sleep(WATCH_MS) } catch (e: InterruptedException) { break }
                tick++
            }
        }.also { it.isDaemon = true; it.start() }
    }

    private fun fetchDaemonGames(): Set<String> {
        val out = mutableSetOf<String>()
        try {
            val arr = JSONObject(RustBridge.gamesJson()).optJSONArray("games") ?: return out
            for (i in 0 until arr.length()) {
                val o = arr.optJSONObject(i) ?: continue
                if (o.optBoolean("is_game")) out.add(o.optString("package"))
            }
        } catch (e: Exception) { }
        return out
    }

    private fun updateTelemetry(pkg: String) {
        val tel = io.teyzen.iaoptimizer.core.Models.parseTelemetry(
            RustBridge.telemetryJson().ifEmpty { StateCache.telemetry() }
        )
        val fps = RustBridge.fpsFor(pkg)
        lastFps = fps
        val gpu = if (tick % 2 == 0) RustBridge.gpuMhz() else lastGpu
        lastGpu = gpu
        handler.post {
            fpsView?.text = if (fps > 0) "$fps FPS" else "— FPS"
            cpuView?.text = if (tel != null && tel.cpuMhz > 0)
                String.format("%.2f GHz", tel.cpuMhz / 1000.0) else "—"
            gpuView?.text = if (gpu > 0) "$gpu MHz" else "—"
            tempView?.text = tel?.let { String.format("%.1f °C", it.tempC) } ?: "—"
            ramView?.text = tel?.let { String.format("%.0f %%", it.ramPct) } ?: "—"
            batView?.text = tel?.let { "${it.batteryPct} %" + if (it.charging) " ⚡" else "" } ?: "—"
        }
    }

    private fun syncSession() {
        val s = try { JSONObject(RustBridge.http("/api/session")) } catch (e: Exception) { return }
        handler.post {
            syncingPanel = true
            dndSwitch?.isChecked = s.optBoolean("dnd", false)
            brightSwitch?.isChecked = s.optBoolean("brightness_lock", false)
            val p = s.optString("profile", "")
            if (s.optBoolean("active", false) && p.isNotEmpty()) {
                selectedProfile = p
                selectedExtreme = s.optBoolean("extreme", false)
                paintPills()
            }
            syncingPanel = false
        }
    }

    private fun paintProfileState(profile: String) {
        profileView?.text = "Perfil activo: " + when (profile) {
            "eco" -> "Eco"
            "performance" -> "Rendimiento"
            "balanced" -> "Balanceado"
            else -> profile.ifEmpty { "—" }
        }
    }

    // ------------------------- disparador de borde -------------------------

    private fun showTrigger() {
        if (trigger != null || !Settings.canDrawOverlays(this)) return
        val handle = View(this).apply {
            background = GradientDrawable().apply {
                setColor(Color.parseColor("#6634D399"))
                cornerRadii = floatArrayOf(dp(8).toFloat(), dp(8).toFloat(), 0f, 0f, 0f, 0f, dp(8).toFloat(), dp(8).toFloat())
            }
        }
        val params = WindowManager.LayoutParams(
            dp(10), dp(96), overlayType(),
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT
        )
        params.gravity = Gravity.TOP or Gravity.END
        params.x = 0
        params.y = triggerY

        var downX = 0f; var downY = 0f; var startY = 0; var moved = false
        handle.setOnTouchListener { _, e ->
            when (e.action) {
                MotionEvent.ACTION_DOWN -> {
                    downX = e.rawX; downY = e.rawY; startY = params.y; moved = false
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    val dy = (e.rawY - downY).toInt()
                    val dx = e.rawX - downX
                    if (abs(dy) > dp(8) || abs(dx) > dp(8)) moved = true
                    // Arrastre vertical por el borde.
                    params.y = (startY + dy).coerceIn(dp(40), resources.displayMetrics.heightPixels - dp(140))
                    try { wm.updateViewLayout(handle, params) } catch (ex: Exception) { }
                    // Deslizamiento hacia dentro: abre el panel.
                    if (dx < -dp(30)) {
                        triggerY = params.y
                        openPanel()
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    triggerY = params.y
                    if (!moved) openPanel()
                    true
                }
                else -> false
            }
        }
        try {
            wm.addView(handle, params)
            trigger = handle
        } catch (e: Exception) { }
    }

    private fun hideTrigger() {
        trigger?.let { try { wm.removeView(it) } catch (e: Exception) { } }
        trigger = null
    }

    // ------------------------- panel deslizable -------------------------

    private fun openPanel() {
        if (panelOpen || !Settings.canDrawOverlays(this)) return
        val pkg = currentPkg ?: return
        panelOpen = true
        hideTrigger()

        val ctx = this
        val card = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            background = GradientDrawable().apply {
                setColor(Color.parseColor(PANEL_BG))
                cornerRadius = dp(20).toFloat()
                setStroke(dp(1), Color.parseColor("#3334D399"))
            }
            setPadding(dp(14), dp(12), dp(14), dp(12))
        }

        // Encabezado: juego + cierre.
        val header = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        val title = TextView(ctx).apply {
            text = gameLabel(pkg)
            setTextColor(Color.parseColor(ACCENT))
            textSize = 15f
            typeface = android.graphics.Typeface.DEFAULT_BOLD
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        }
        val close = TextView(ctx).apply {
            text = "✕"
            setTextColor(Color.parseColor(TEXT_DIM))
            textSize = 18f
            setPadding(dp(10), dp(2), dp(4), dp(2))
            setOnClickListener { closePanel() }
        }
        header.addView(title)
        header.addView(close)
        card.addView(header)
        val sub = TextView(ctx).apply {
            text = "Centro de control in-game · IA Motors"
            setTextColor(Color.parseColor(TEXT_DIM))
            textSize = 10f
        }
        card.addView(sub)

        val scroll = ScrollView(ctx).apply {
            isVerticalScrollBarEnabled = false
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                (resources.displayMetrics.heightPixels * 0.68).toInt()
            )
        }
        val body = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL }
        scroll.addView(body)
        card.addView(scroll)

        // Telemetría en vivo, con widgets elegibles por juego.
        val teleHeader = LinearLayout(ctx).apply { orientation = LinearLayout.HORIZONTAL }
        teleHeader.addView(sectionLabel("Telemetría en vivo").apply {
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        })
        val chooser = android.widget.HorizontalScrollView(ctx).apply {
            visibility = View.GONE
            setPadding(0, dp(2), 0, dp(6))
        }
        val chooseBtn = TextView(ctx).apply {
            text = "Elegir ▾"
            setTextColor(Color.parseColor(ACCENT2))
            textSize = 10f
            setPadding(dp(8), dp(12), dp(8), dp(6))
            setOnClickListener {
                chooser.visibility = if (chooser.visibility == View.GONE) View.VISIBLE else View.GONE
                text = if (chooser.visibility == View.GONE) "Elegir ▾" else "Ocultar ▴"
            }
        }
        teleHeader.addView(chooseBtn)
        body.addView(teleHeader)
        val tele = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            background = pill(CARD_BG, dp(14))
            setPadding(dp(12), dp(8), dp(12), dp(8))
        }
        fpsView = teleRow(tele, "FPS")
        cpuView = teleRow(tele, "CPU")
        gpuView = teleRow(tele, "GPU")
        tempView = teleRow(tele, "Temperatura")
        ramView = teleRow(tele, "RAM")
        batView = teleRow(tele, "Batería")
        profileView = teleRow(tele, "Perfil")
        body.addView(tele)

        // Selector de widgets: se guarda por juego y se aplica al momento.
        val widgetViews = mapOf(
            "fps" to fpsView, "cpu" to cpuView, "gpu" to gpuView,
            "temp" to tempView, "ram" to ramView, "bat" to batView,
            "profile" to profileView
        )
        val selected = GameStore.widgets(ctx, pkg).split(",")
            .map { it.trim() }.filter { it.isNotEmpty() }.toMutableSet()
        fun applyWidgets() {
            widgetViews.forEach { (k, v) ->
                (v?.parent as? View)?.visibility =
                    if (selected.contains(k)) View.VISIBLE else View.GONE
            }
        }
        fun paintChip(chip: TextView, on: Boolean) {
            chip.background = pill(if (on) ACCENT else CARD_BG, dp(12))
            chip.setTextColor(Color.parseColor(if (on) "#06281C" else TEXT))
        }
        val chipRow = LinearLayout(ctx).apply { orientation = LinearLayout.HORIZONTAL }
        listOf(
            "fps" to "FPS", "cpu" to "CPU", "gpu" to "GPU", "temp" to "Temp",
            "ram" to "RAM", "bat" to "Batería", "profile" to "Perfil"
        ).forEach { (k, label) ->
            val chip = TextView(ctx).apply {
                text = label
                textSize = 11f
                gravity = Gravity.CENTER
                setPadding(dp(10), dp(6), dp(10), dp(6))
                layoutParams = LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
                ).apply { marginEnd = dp(6) }
                setOnClickListener {
                    if (selected.contains(k)) selected.remove(k) else selected.add(k)
                    GameStore.setWidgets(ctx, pkg, selected.joinToString(","))
                    paintChip(this, selected.contains(k))
                    applyWidgets()
                }
            }
            paintChip(chip, selected.contains(k))
            chipRow.addView(chip)
        }
        chooser.addView(chipRow)
        body.addView(chooser)
        applyWidgets()

        // Perfiles en caliente (sesión que la IA respeta).
        body.addView(sectionLabel("Perfil en caliente"))
        val row1 = LinearLayout(ctx).apply { orientation = LinearLayout.HORIZONTAL }
        val row2 = LinearLayout(ctx).apply { orientation = LinearLayout.HORIZONTAL }
        addProfilePill(row1, "Eco", "eco", false)
        addProfilePill(row1, "Balanceado", "balanced", false)
        addProfilePill(row2, "Rendimiento", "performance", false)
        addProfilePill(row2, "Extremo", "performance", true)
        body.addView(row1)
        body.addView(row2)
        body.addView(hint("Tu elección manda mientras juegas; la protección térmica y la batería crítica siempre ganan. Se limpia sola al salir del juego."))

        // Confort.
        body.addView(sectionLabel("Confort"))
        dndSwitch = comfortSwitch(body, "No molestar", "Bloquea notificaciones durante la partida") { on ->
            thread { RustBridge.post("/api/session", JSONObject().put("package", pkg).put("dnd", on).toString()) }
        }
        brightSwitch = comfortSwitch(body, "Bloquear brillo", "Fija el brillo manual y apaga el automático") { on ->
            thread { RustBridge.post("/api/session", JSONObject().put("package", pkg).put("brightness_lock", on).toString()) }
        }
        hudSwitch = comfortSwitch(body, "HUD flotante", "Telemetría compacta arrastrable sobre el juego") { on ->
            if (on) HudController.start(ctx, pkg) else HudController.stop(ctx)
        }

        // Refresco (objetivo de FPS de la pantalla).
        body.addView(sectionLabel("Refresco de pantalla"))
        val hzRow = LinearLayout(ctx).apply { orientation = LinearLayout.HORIZONTAL }
        listOf(0 to "Auto", 60 to "60", 90 to "90", 120 to "120").forEach { (v, label) ->
            val pill = TextView(ctx).apply {
                text = label
                setTextColor(Color.parseColor(TEXT))
                textSize = 12f
                gravity = Gravity.CENTER
                background = pill(CARD_BG, dp(12))
                setPadding(dp(12), dp(8), dp(12), dp(8))
                layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).apply {
                    marginEnd = dp(6)
                }
                setOnClickListener {
                    thread { RustBridge.setRefresh(v) }
                }
            }
            hzRow.addView(pill)
        }
        body.addView(hzRow)
        body.addView(hint("Limita los Hz para ahorrar batería o fija el máximo para fluidez. El FPS real del juego se mide arriba."))

        // Límite de FPS por juego (GameManager del sistema, Android 13+).
        body.addView(sectionLabel("Límite de FPS del juego"))
        val fpsRow = LinearLayout(ctx).apply { orientation = LinearLayout.HORIZONTAL }
        listOf(0 to "Auto", 30 to "30", 45 to "45", 60 to "60", 90 to "90").forEach { (v, label) ->
            val pill = TextView(ctx).apply {
                text = label
                setTextColor(Color.parseColor(TEXT))
                textSize = 12f
                gravity = Gravity.CENTER
                background = pill(CARD_BG, dp(12))
                setPadding(dp(10), dp(8), dp(10), dp(8))
                layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).apply {
                    marginEnd = dp(6)
                }
                setOnClickListener {
                    val pkg = currentPkg ?: return@setOnClickListener
                    thread {
                        val r = RustBridge.post(
                            "/api/session",
                            JSONObject().put("package", pkg).put("fps_target", v).toString()
                        )
                        val applied = try { JSONObject(r).optInt("fps_target", 0) == v } catch (e: Exception) { false }
                        handler.post {
                            android.widget.Toast.makeText(
                                this@GamePanelService,
                                when {
                                    v == 0 -> "Límite de FPS retirado"
                                    applied -> "Límite de FPS aplicado al juego"
                                    else -> "Esta ROM no permite limitar los FPS de este juego"
                                },
                                android.widget.Toast.LENGTH_SHORT
                            ).show()
                        }
                    }
                }
            }
            fpsRow.addView(pill)
        }
        body.addView(fpsRow)
        body.addView(hint("Cap oficial por juego (GameManager). Si tu ROM no lo soporta, no se aplica nada y se dice. Algunos juegos no lo respetan aunque el sistema lo registre. Se restaura al salir del juego."))

        // Respuesta táctil: honestidad sobre el hardware.
        body.addView(sectionLabel("Respuesta táctil"))
        body.addView(hint("Este dispositivo no expone nodos de sensibilidad táctil en el kernel; no se ofrece un control que no existiría."))

        val params = WindowManager.LayoutParams(
            dp(300), WindowManager.LayoutParams.WRAP_CONTENT, overlayType(),
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT
        )
        params.gravity = Gravity.TOP or Gravity.END
        params.x = dp(6)
        params.y = (triggerY - dp(120)).coerceAtLeast(dp(24))

        // Entrada deslizante desde el borde.
        card.translationX = dp(320).toFloat()
        try {
            wm.addView(card, params)
            panel = card
            card.animate().translationX(0f).setDuration(180).start()
        } catch (e: Exception) {
            panelOpen = false
            showTrigger()
        }
        paintPills()
        syncSession()
    }

    private fun closePanel() {
        val p = panel ?: return
        panelOpen = false
        p.animate().translationX(dp(320).toFloat()).setDuration(160).withEndAction {
            try { wm.removeView(p) } catch (e: Exception) { }
            if (panel === p) panel = null
            handler.post { if (currentPkg != null && running) showTrigger() }
        }.start()
    }

    private fun hideAll() {
        panelOpen = false
        panel?.let { try { wm.removeView(it) } catch (e: Exception) { } }
        panel = null
        hideTrigger()
        currentPkg = null
        fpsView = null; cpuView = null; gpuView = null; tempView = null
        ramView = null; batView = null; profileView = null
        dndSwitch = null; brightSwitch = null; hudSwitch = null
        profilePills.clear()
    }

    // ------------------------- piezas de UI -------------------------

    private fun pill(color: String, radius: Int): GradientDrawable =
        GradientDrawable().apply {
            setColor(Color.parseColor(color))
            cornerRadius = radius.toFloat()
        }

    private fun sectionLabel(text: String): TextView =
        TextView(this).apply {
            this.text = text.uppercase()
            setTextColor(Color.parseColor(ACCENT2))
            textSize = 10f
            letterSpacing = 0.08f
            setPadding(0, dp(12), 0, dp(6))
        }

    private fun hint(text: String): TextView =
        TextView(this).apply {
            this.text = text
            setTextColor(Color.parseColor(TEXT_DIM))
            textSize = 10f
            setPadding(0, dp(4), 0, dp(2))
        }

    private fun teleRow(parent: LinearLayout, label: String): TextView {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, dp(4), 0, dp(4))
        }
        val l = TextView(this).apply {
            text = label
            setTextColor(Color.parseColor(TEXT_DIM))
            textSize = 12f
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        }
        val v = TextView(this).apply {
            text = "—"
            setTextColor(Color.parseColor(TEXT))
            textSize = 12f
            typeface = android.graphics.Typeface.MONOSPACE
        }
        row.addView(l)
        row.addView(v)
        parent.addView(row)
        return v
    }

    private fun addProfilePill(row: LinearLayout, label: String, profile: String, extreme: Boolean) {
        val key = if (extreme) "extreme" else profile
        val pill = TextView(this).apply {
            text = label
            textSize = 12f
            gravity = Gravity.CENTER
            setPadding(dp(10), dp(10), dp(10), dp(10))
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).apply {
                marginEnd = dp(6)
                topMargin = dp(4)
            }
            setOnClickListener {
                selectedProfile = profile
                selectedExtreme = extreme
                paintPills()
                val pkg = currentPkg ?: return@setOnClickListener
                thread {
                    RustBridge.post(
                        "/api/session",
                        JSONObject().put("package", pkg).put("profile", profile).put("extreme", extreme).toString()
                    )
                }
            }
        }
        profilePills[key] = pill
        row.addView(pill)
    }

    private fun paintPills() {
        profilePills.forEach { (key, pill) ->
            val active = if (key == "extreme") selectedExtreme
            else !selectedExtreme && key == selectedProfile
            pill.background = pill(if (active) ACCENT else CARD_BG, dp(12))
            pill.setTextColor(Color.parseColor(if (active) "#06281C" else TEXT))
            pill.typeface = if (active) android.graphics.Typeface.DEFAULT_BOLD else android.graphics.Typeface.DEFAULT
        }
    }

    private fun comfortSwitch(parent: LinearLayout, title: String, desc: String, onChange: (Boolean) -> Unit): Switch {
        val sw = Switch(this).apply {
            text = title
            setTextColor(Color.parseColor(TEXT))
            textSize = 13f
            setPadding(0, dp(6), 0, 0)
            setOnCheckedChangeListener { _, on ->
                if (!syncingPanel) onChange(on)
            }
        }
        parent.addView(sw)
        parent.addView(hint(desc))
        return sw
    }

    private fun gameLabel(pkg: String): String = try {
        val pm = packageManager
        pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString()
    } catch (e: Exception) { pkg }

    private fun thread(block: () -> Unit) {
        Thread { try { block() } catch (e: Exception) { } }.also { it.isDaemon = true }.start()
    }

    override fun onDestroy() {
        running = false
        worker?.interrupt()
        worker = null
        handler.post { hideAll() }
        super.onDestroy()
    }
}

/** Arranque/parada del panel in-game (preferencia del usuario en Sistema). */
object GamePanelController {
    fun start(context: Context) {
        val intent = Intent(context, GamePanelService::class.java).setAction(GamePanelService.ACTION_START)
        if (Build.VERSION.SDK_INT >= 26) context.startForegroundService(intent)
        else context.startService(intent)
    }

    fun stop(context: Context) {
        val intent = Intent(context, GamePanelService::class.java).setAction(GamePanelService.ACTION_STOP)
        try { context.startService(intent) } catch (e: Exception) { }
    }
}
