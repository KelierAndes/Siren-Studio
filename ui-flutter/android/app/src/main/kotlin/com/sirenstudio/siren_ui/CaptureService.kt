package com.sirenstudio.siren_ui

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.IBinder
import android.os.Process
import androidx.annotation.Keep
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Foreground service holding the MediaProjection capture. Android 14 requires
 * a FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION service around getMediaProjection,
 * so the AudioRecord lives here rather than in the activity.
 */
class CaptureService : Service() {
    companion object {
        const val EXTRA_RESULT_CODE = "result_code"
        const val EXTRA_RESULT_DATA = "result_data"
        private const val CHANNEL_ID = "siren_capture"
        private const val NOTIFICATION_ID = 4101
    }

    @Keep
    external fun nativePushCapture(samples: ShortArray)

    private var record: AudioRecord? = null
    private var thread: Thread? = null
    private val running = AtomicBoolean(false)
    private var projection: MediaProjection? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val appLabel = packageManager.getApplicationLabel(applicationInfo).toString()
        val channel = NotificationChannel(
            CHANNEL_ID,
            appLabel,
            NotificationManager.IMPORTANCE_LOW,
        )
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        val notification: Notification = Notification.Builder(this, CHANNEL_ID)
            .setContentTitle(appLabel)
            .setContentText("正在采集系统音频")
            .setSmallIcon(android.R.drawable.ic_media_play)
            .build()
        startForeground(
            NOTIFICATION_ID,
            notification,
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION,
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val resultCode = intent?.getIntExtra(EXTRA_RESULT_CODE, 0) ?: 0
        @Suppress("DEPRECATION")
        val data = if (Build.VERSION.SDK_INT >= 33) {
            intent?.getParcelableExtra(EXTRA_RESULT_DATA, Intent::class.java)
        } else {
            intent?.getParcelableExtra(EXTRA_RESULT_DATA)
        }
        if (data == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        val manager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        try {
            val mediaProjection = manager.getMediaProjection(resultCode, data)
            if (mediaProjection != null) {
                startCapture(mediaProjection)
            }
        } catch (error: SecurityException) {
            android.util.Log.e("siren", "projection denied", error)
        }
        return START_NOT_STICKY
    }

    private fun startCapture(mediaProjection: MediaProjection) {
        projection = mediaProjection
        mediaProjection.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() {
                stopSelf()
            }
        }, null)

        val sampleRate = 48_000
        // Never capture this app's own playback: the mesh monitor output is
        // what we are sending to the peers, feeding it back would loop.
        val config = AudioPlaybackCaptureConfiguration.Builder(mediaProjection)
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .excludeUid(Process.myUid())
            .build()
        val minBuffer = AudioRecord.getMinBufferSize(
            sampleRate,
            AudioFormat.CHANNEL_IN_STEREO,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        if (minBuffer <= 0) {
            stopSelf()
            return
        }
        val audioRecord = AudioRecord.Builder()
            .setAudioFormat(
                AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(sampleRate)
                    .setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
                    .build()
            )
            .setBufferSizeInBytes(minBuffer * 4)
            .setAudioPlaybackCaptureConfig(config)
            .build()
        if (audioRecord.state != AudioRecord.STATE_INITIALIZED) {
            audioRecord.release()
            stopSelf()
            return
        }
        audioRecord.startRecording()
        record = audioRecord
        running.set(true)
        thread = Thread({
            val buffer = ShortArray(480 * 2)
            while (running.get()) {
                val read = audioRecord.read(buffer, 0, buffer.size, AudioRecord.READ_BLOCKING)
                if (read > 0) {
                    nativePushCapture(if (read == buffer.size) buffer else buffer.copyOf(read))
                } else if (read < 0) {
                    break
                }
            }
        }, "siren-syscapture")
        thread?.start()
    }

    override fun onDestroy() {
        running.set(false)
        try {
            record?.stop()
        } catch (_: IllegalStateException) {
        }
        try {
            record?.release()
        } catch (_: Exception) {
        }
        record = null
        projection?.stop()
        projection = null
        super.onDestroy()
    }
}
