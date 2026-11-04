package com.sirenstudio.siren_ui

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.ServiceCompat

/**
 * F12: keeps the mesh alive while the app is backgrounded. Without a
 * mediaPlayback foreground service MIUI/others freeze the process within
 * minutes, and the audio (capture + playback + both carriers) dies with it.
 *
 * Started from [MainActivity.onCreate] and stopped in onDestroy; it holds no
 * state of its own — the engines live in the Rust layer.
 */
class SirenPlaybackService : Service() {

    override fun onCreate() {
        super.onCreate()
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_ID,
                    "SirenStudio 组网",
                    NotificationManager.IMPORTANCE_LOW,
                ).apply { setShowBadge(false) },
            )
        }
        val notification: Notification =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                Notification.Builder(this, CHANNEL_ID)
                    .setContentTitle("SirenStudio")
                    .setContentText("组网运行中（音频采集/播放保持活动）")
                    .setSmallIcon(android.R.drawable.stat_sys_headset)
                    .setOngoing(true)
                    .build()
            } else {
                @Suppress("DEPRECATION")
                Notification.Builder(this)
                    .setContentTitle("SirenStudio")
                    .setContentText("组网运行中（音频采集/播放保持活动）")
                    .setSmallIcon(android.R.drawable.stat_sys_headset)
                    .setOngoing(true)
                    .build()
            }
        // mediaPlayback type requires API 29+; below that the plain
        // startForeground call is the only option (and is still better than
        // nothing for the pre-29 devices the app supports).
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ServiceCompat.startForeground(
                this,
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Sticky: if the system kills us anyway, restart with the
        // notification rather than silently dropping the service.
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        private const val CHANNEL_ID = "siren_mesh"
        private const val NOTIFICATION_ID = 4790

        fun start(context: Context) {
            val intent = Intent(context, SirenPlaybackService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, SirenPlaybackService::class.java))
        }
    }
}
