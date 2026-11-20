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
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import io.teyzen.iaoptimizer.MainActivity
import io.teyzen.iaoptimizer.R
import io.teyzen.iaoptimizer.core.Models
import io.teyzen.iaoptimizer.core.RustBridge
import io.teyzen.iaoptimizer.core.StateCache

/**
 * Isla dinámica: píldora compacta y animada con FPS y temperatura en vivo.
 * Mínima intrusión: solo dos lecturas y un punto que late cuando el motor
 * está activo. Arrastrable a cualquier borde; se detiene con su notificación.
 */
class IslaService : Service() {

    companion object {
        const val ACTION_START = "io.teyzen.iaoptimizer.isla.START"
        const val ACTION_STOP = "io.teyzen.iaoptimizer.isla.STOP"
        private const val CHANNEL = "ia_isla"
        private const val NOTIF_ID = 0x0903
        private const val ACCENT = "#34D399"
        private const val BG = "#E60A0F14"
    }

    private lateinit var wm: WindowManager
    private val handler = Handler(Looper.getMainLooper())
    private var root: LinearLayout? = null
    private var fpsView: TextView? = null
    private var tempView: TextView? = null
    private var dot: android.view.View? = null
    private var worker: Thread? = null
    @Volatile private var running = false
    private var x = 24
    private var y = 200
    private var tick = 0
    private var lastFps = 0

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        wm = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        if (Build.VERSION.SDK_INT >= 26) {
            val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            if (nm.getNotificationChannel(CHANNEL) == null) {
                nm.createNotificationChannel(
                    NotificationChannel(CHANNEL, "Isla dinámica", NotificationManager.IMPORTANCE_LOW)
                )
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                hide()
                stopSelf()
                return START_NOT_STICKY
            }
            else -> {
                startForegroundCompat()
                show()
                startLoop()
            }
        }
        return START_STICKY
    }

    private fun startForegroundCompat() {
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val stop = PendingIntent.getService(
            this, 1, Intent(this, IslaService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val b = if (Build.VERSION.SDK_INT >= 26) Notification.Builder(this, CHANNEL)
        else @Suppress("DEPRECATION") Notification.Builder(this)
        b.setContentTitle("Isla dinámica activa")
            .setContentText("FPS y temperatura en vivo")
            .setSmallIcon(R.drawable.ic_pulse)
            .setOngoing(true)
            .setContentIntent(open)
            .addAction(R.drawable.ic_pulse, "Apagar", stop)
        val notif = b.build()
        if (Build.VERSION.SDK_INT >= 34) {
            try {
                startForeground(NOTIF_ID, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
                return
            } catch (e: Exception) { }
        }
        startForeground(NOTIF_ID, notif)
    }

    private fun dp(v: Int): Int = TypedValue.applyDimension(
        TypedValue.COMPLEX_UNIT_DIP, v.toFloat(), resources.displayMetrics
    ).toInt()

    private fun show() {
        if (root != null || !Settings.canDrawOverlays(this)) return
        val pill = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            background = GradientDrawable().apply {
                setColor(Color.parseColor(BG))
                cornerRadius = dp(999).toFloat()
                setStroke(dp(1), Color.parseColor("#3334D399"))
            }
            setPadding(dp(12), dp(6), dp(12), dp(6))
        }
        dot = android.view.View(this).apply {
            background = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(Color.parseColor(ACCENT))
            }
            layoutParams = LinearLayout.LayoutParams(dp(8), dp(8)).apply { marginEnd = dp(8) }
        }
        fpsView = TextView(this).apply {
            text = "— FPS"
            setTextColor(Color.WHITE)
            textSize = 12f
            typeface = android.graphics.Typeface.MONOSPACE
        }
        val sep = TextView(this).apply {
            text = " · "
            setTextColor(Color.parseColor("#9FB4AC"))
            textSize = 12f
        }
        tempView = TextView(this).apply {
            text = "— °C"
            setTextColor(Color.parseColor(ACCENT))
            textSize = 12f
            typeface = android.graphics.Typeface.MONOSPACE
        }
        pill.addView(dot)
        pill.addView(fpsView)
        pill.addView(sep)
        pill.addView(tempView)

        val type = if (Build.VERSION.SDK_INT >= 26) WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
        else @Suppress("DEPRECATION") WindowManager.LayoutParams.TYPE_PHONE
        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            type,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT
        )
        params.gravity = Gravity.TOP or Gravity.START
        params.x = x
        params.y = y

        var startX = 0f; var startY = 0f; var touchX = 0; var touchY = 0
        pill.setOnTouchListener { _, e ->
            when (e.action) {
                MotionEvent.ACTION_DOWN -> {
                    startX = e.rawX; startY = e.rawY; touchX = params.x; touchY = params.y
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    params.x = touchX + (e.rawX - startX).toInt()
                    params.y = touchY + (e.rawY - startY).toInt()
                    try { wm.updateViewLayout(pill, params) } catch (ex: Exception) { }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    x = params.x; y = params.y
                    true
                }
                else -> false
            }
        }
        try {
            wm.addView(pill, params)
            root = pill
            pulse()
        } catch (e: Exception) { }
    }

    /** Latido suave del punto: la isla está viva y midiendo. */
    private fun pulse() {
        val d = dot ?: return
        d.animate().alpha(0.35f).setDuration(700).withEndAction {
            if (root != null) d.animate().alpha(1f).setDuration(700).withEndAction { pulse() }.start()
        }.start()
    }

    private fun startLoop() {
        if (worker != null) return
        running = true
        worker = Thread {
            while (running) {
                val tel = Models.parseTelemetry(
                    RustBridge.telemetryJson().ifEmpty { StateCache.telemetry() }
                )
                val fps = if (tick % 2 == 0) RustBridge.fps() else lastFps
                lastFps = fps
                handler.post {
                    fpsView?.text = if (fps > 0) "$fps FPS" else "— FPS"
                    tempView?.text = tel?.let { String.format("%.1f °C", it.tempC) } ?: "— °C"
                }
                try { Thread.sleep(2000) } catch (e: InterruptedException) { break }
                tick++
            }
        }.also { it.isDaemon = true; it.start() }
    }

    private fun hide() {
        root?.let { try { wm.removeView(it) } catch (e: Exception) { } }
        root = null
        dot?.animate()?.cancel()
        dot = null
    }

    override fun onDestroy() {
        running = false
        worker?.interrupt()
        worker = null
        handler.post { hide() }
        super.onDestroy()
    }
}

object IslaController {
    fun start(context: Context) {
        val intent = Intent(context, IslaService::class.java).setAction(IslaService.ACTION_START)
        if (Build.VERSION.SDK_INT >= 26) context.startForegroundService(intent)
        else context.startService(intent)
    }

    fun stop(context: Context) {
        val intent = Intent(context, IslaService::class.java).setAction(IslaService.ACTION_STOP)
        try { context.startService(intent) } catch (e: Exception) { }
    }
}
