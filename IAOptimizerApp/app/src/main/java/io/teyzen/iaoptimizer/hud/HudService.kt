package io.teyzen.iaoptimizer.hud

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
import android.widget.TextView
import io.teyzen.iaoptimizer.MainActivity
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.Models
import io.teyzen.iaoptimizer.core.RustBridge
import kotlin.math.abs

/**
 * HUD flotante: servicio en primer plano con superposicion (TYPE_APPLICATION_OVERLAY).
 * Dibuja CPU, GPU/termico, FPS y perfil sobre los juegos. Se arrastra con el dedo.
 */
class HudService : Service() {

    companion object {
        const val ACTION_START = "io.teyzen.iaoptimizer.hud.START"
        const val ACTION_STOP = "io.teyzen.iaoptimizer.hud.STOP"
        const val ACTION_TOGGLE = "io.teyzen.iaoptimizer.hud.TOGGLE"
        const val EXTRA_PKG = "io.teyzen.iaoptimizer.hud.PKG"
        private const val CHANNEL = "ia_hud"
        private const val NOTIF_ID = 0x0900
        private const val UPDATE_MS = 1000L
    }

    private lateinit var windowManager: WindowManager
    private var root: LinearLayout? = null
    private var titleView: TextView? = null
    private var cpuView: TextView? = null
    private var gpuView: TextView? = null
    private var tempView: TextView? = null
    private var fpsView: TextView? = null
    private var profileView: TextView? = null

    private val handler = Handler(Looper.getMainLooper())
    private var worker: Thread? = null
    @Volatile
    private var running = false
    private var tick = 0
    private var lastFps = 0
    private var lastGpu = 0L
    private var targetPkg: String? = null

    private var x = 24
    private var y = 120

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        windowManager = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                hide()
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_TOGGLE -> {
                if (root == null) show() else hide()
                return START_STICKY
            }
            else -> {
                targetPkg = intent?.getStringExtra(EXTRA_PKG)
                startForegroundCompat()
                show()
                startLoop()
            }
        }
        return START_STICKY
    }

    private fun startForegroundCompat() {
        val notif = buildNotification()
        if (Build.VERSION.SDK_INT >= 34) {
            try {
                startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
                return
            } catch (e: Exception) {
                // Algunos kernels no aceptan specialUse; se reintenta sin tipo.
            }
        }
        startForeground(NOTIF_ID, notif)
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT >= 26) {
            val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (nm.getNotificationChannel(CHANNEL) == null) {
                val ch = NotificationChannel(
                    CHANNEL,
                    getString(R.string.canal_hud),
                    NotificationManager.IMPORTANCE_LOW
                )
                ch.description = getString(R.string.ajustes_hud_desc)
                ch.setShowBadge(false)
                nm.createNotificationChannel(ch)
            }
        }
    }

    private fun buildNotification(): Notification {
        val open = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val stop = PendingIntent.getService(
            this, 1,
            Intent(this, HudService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val builder = if (Build.VERSION.SDK_INT >= 26) {
            Notification.Builder(this, CHANNEL)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        builder.setContentTitle(getString(R.string.hud_notification_title))
            .setContentText(getString(R.string.hud_notification_text))
            .setSmallIcon(R.drawable.ic_hud)
            .setOngoing(true)
            .setContentIntent(open)
            .addAction(R.drawable.ic_hud, getString(R.string.hud_apagado), stop)
        return builder.build()
    }

    private fun dp(v: Int): Int = TypedValue.applyDimension(
        TypedValue.COMPLEX_UNIT_DIP, v.toFloat(), resources.displayMetrics
    ).toInt()

    private fun show() {
        if (root != null) return
        if (!Settings.canDrawOverlays(this)) return
        val ctx = this
        val pad = dp(10)
        val container = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.parseColor("#CC070B0F"))
            setPadding(pad, pad, pad, pad)
        }
        val lp = LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.WRAP_CONTENT,
            LinearLayout.LayoutParams.WRAP_CONTENT
        )
        titleView = TextView(ctx).apply {
            text = getString(R.string.app_name)
            setTextColor(Color.parseColor("#34D399"))
            textSize = 11f
            setPadding(0, 0, 0, dp(2))
        }
        cpuView = TextView(ctx).apply { setTextColor(Color.WHITE); textSize = 13f }
        gpuView = TextView(ctx).apply { setTextColor(Color.WHITE); textSize = 13f }
        tempView = TextView(ctx).apply { setTextColor(Color.WHITE); textSize = 13f }
        fpsView = TextView(ctx).apply { setTextColor(Color.WHITE); textSize = 13f }
        profileView = TextView(ctx).apply { setTextColor(Color.WHITE); textSize = 11f }
        container.addView(titleView, lp)
        container.addView(cpuView, lp)
        container.addView(gpuView, lp)
        container.addView(tempView, lp)
        container.addView(fpsView, lp)
        container.addView(profileView, lp)

        val type = if (Build.VERSION.SDK_INT >= 26) {
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
        } else {
            @Suppress("DEPRECATION")
            WindowManager.LayoutParams.TYPE_PHONE
        }
        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            type,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
                or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT
        )
        params.gravity = Gravity.TOP or Gravity.START
        params.x = x
        params.y = y

        var startX = 0f
        var startY = 0f
        var touchX = 0
        var touchY = 0
        container.setOnTouchListener { v, e ->
            when (e.action) {
                MotionEvent.ACTION_DOWN -> {
                    startX = e.rawX; startY = e.rawY
                    touchX = params.x; touchY = params.y
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    params.x = touchX + (e.rawX - startX).toInt()
                    params.y = touchY + (e.rawY - startY).toInt()
                    try { windowManager.updateViewLayout(container, params) } catch (ex: Exception) { }
                    true
                }
                else -> false
            }
        }
        try {
            windowManager.addView(container, params)
            root = container
        } catch (e: Exception) {
            root = null
        }
    }

    private fun hide() {
        root?.let {
            try { windowManager.removeView(it) } catch (e: Exception) { }
        }
        root = null
    }

    private fun startLoop() {
        if (worker != null) return
        running = true
        worker = Thread {
            while (running) {
                val telemetry = Models.parseTelemetry(RustBridge.telemetryJson())
                val state = Models.parseState(RustBridge.stateJson())
                val fps = if (tick % 2 == 0) {
                    val pkg = targetPkg
                    if (pkg.isNullOrBlank()) RustBridge.fps() else RustBridge.fpsFor(pkg)
                } else lastFps
                lastFps = fps
                val gpu = if (tick % 2 == 0) RustBridge.gpuMhz() else lastGpu
                lastGpu = gpu
                handler.post {
                    cpuView?.text = getString(R.string.cpu) + "  " +
                        (if (telemetry != null && telemetry.cpuMhz > 0)
                            String.format("%.2f GHz", telemetry.cpuMhz / 1000.0) else "—") +
                        "  " + (state?.cpuLoad?.times(100)?.toInt() ?: 0) + " %"
                    gpuView?.text = getString(R.string.gpu) + "  " +
                        (if (gpu > 0) "$gpu MHz" else "—")
                    tempView?.text = getString(R.string.temperatura) + "  " +
                        (telemetry?.let { String.format("%.1f", it.tempC) } ?: "—") + " °C"
                    fpsView?.text = getString(R.string.fps) + "  " + fps
                    profileView?.text = getString(R.string.perfil_actual) + "  " +
                        (state?.profile ?: "—")
                }
                try { Thread.sleep(UPDATE_MS) } catch (e: InterruptedException) { break }
                tick++
            }
        }.also { it.isDaemon = true; it.start() }
    }

    override fun onDestroy() {
        running = false
        worker?.interrupt()
        worker = null
        hide()
        super.onDestroy()
    }
}

/** Arranque/parada del HUD con comprobacion del permiso de superposicion. */
object HudController {
    fun canOverlay(context: Context): Boolean = Settings.canDrawOverlays(context)

    fun start(context: Context, pkg: String? = null) {
        val intent = Intent(context, HudService::class.java).setAction(HudService.ACTION_START)
        if (!pkg.isNullOrBlank()) intent.putExtra(HudService.EXTRA_PKG, pkg)
        if (Build.VERSION.SDK_INT >= 26) {
            context.startForegroundService(intent)
        } else {
            context.startService(intent)
        }
    }

    fun stop(context: Context) {
        val intent = Intent(context, HudService::class.java).setAction(HudService.ACTION_STOP)
        try { context.startService(intent) } catch (e: Exception) { }
    }

    fun toggle(context: Context) {
        val intent = Intent(context, HudService::class.java).setAction(HudService.ACTION_TOGGLE)
        try { context.startService(intent) } catch (e: Exception) { }
    }

    fun requestOverlay(context: Context) {
        if (Settings.canDrawOverlays(context)) return
        try {
            val intent = Intent(
                Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                android.net.Uri.parse("package:" + context.packageName)
            )
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            context.startActivity(intent)
        } catch (e: Exception) {
            try {
                context.startActivity(Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION).apply {
                    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                })
            } catch (ex: Exception) { }
        }
    }
}
