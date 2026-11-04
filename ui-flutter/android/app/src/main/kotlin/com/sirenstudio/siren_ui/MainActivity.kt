package com.sirenstudio.siren_ui

import android.Manifest
import android.app.Activity
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothSocket
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import androidx.annotation.Keep
import io.flutter.embedding.android.FlutterActivity
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.UUID
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

class MainActivity : FlutterActivity() {
    private var multicastLock: WifiManager.MulticastLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    // ---- Bluetooth RFCOMM bridge (left side of the dual-stack) ----------
    private val btLock = Any()
    private var btGeneration = 0
    @Volatile
    private var btFailures = 0
    private var btThread: Thread? = null

    @Volatile
    private var btRfcomm: BluetoothSocket? = null

    @Volatile
    private var btTcp: Socket? = null

    @Volatile
    private var btServer: ServerSocket? = null

    companion object {
        private const val REQ_RECORD_AUDIO = 7301
        private const val REQ_PROJECTION = 7302
        private const val REQ_BT = 7303
        private const val SPP_UUID = "00001101-0000-1000-8000-00805F9B34FB"

        init {
            // Loads early so JNI_OnLoad registers the JavaVM with the Rust
            // layer before any audio engine starts.
            System.loadLibrary("rust_lib_siren_ui")
        }
    }

    @Keep
    external fun nativeSetContext(appContext: Any, activity: Any)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        nativeSetContext(applicationContext, this)
        // F12: hold a mediaPlayback foreground service for the whole app
        // lifetime so backgrounding does not freeze the mesh.
        try {
            SirenPlaybackService.start(this)
        } catch (error: Throwable) {
            Log.w("siren", "playback service start failed: ${error.message}")
        }
    }

    override fun onDestroy() {
        try {
            SirenPlaybackService.stop(this)
        } catch (_: Throwable) {
        }
        super.onDestroy()
    }

    // ---- Bluetooth: automatic leaf link, no user-facing transport choice --

    private fun btPermissionGranted(): Boolean {
        if (Build.VERSION.SDK_INT < 31) return true
        return checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) ==
            PackageManager.PERMISSION_GRANTED &&
            checkSelfPermission(Manifest.permission.BLUETOOTH_SCAN) ==
            PackageManager.PERMISSION_GRANTED
    }

    private fun requestBtPermission() {
        if (Build.VERSION.SDK_INT >= 31) {
            requestPermissions(
                arrayOf(
                    Manifest.permission.BLUETOOTH_CONNECT,
                    Manifest.permission.BLUETOOTH_SCAN,
                    Manifest.permission.BLUETOOTH_ADVERTISE,
                ),
                REQ_BT,
            )
        }
    }

    private fun btAdvertiseGranted(): Boolean {
        if (Build.VERSION.SDK_INT < 31) return true
        return checkSelfPermission(Manifest.permission.BLUETOOTH_ADVERTISE) ==
            PackageManager.PERMISSION_GRANTED
    }

    /**
     * Hub side: Android has no silent discoverable API (scan mode is
     * system-only and the public path needs a consent dialog). Phones run as
     * leaves in this topology, so report unsupported and let desktops host.
     */
    @Keep
    fun setBtDiscoverable(seconds: Int): Boolean {
        Log.i("siren", "bt: discoverable not available on Android (requested ${seconds}s)")
        return false
    }

    /** Leaf side: classic inquiry; returns "address\tname" lines. */
    @Keep
    fun scanBtDevices(timeoutMs: Int): String {
        if (!btPermissionGranted()) {
            requestBtPermission()
            return ""
        }
        val manager = getSystemService(BluetoothManager::class.java) ?: return ""
        val adapter = manager.adapter ?: return ""
        if (!adapter.isEnabled) return ""
        val results = LinkedHashSet<String>()
        val queue = LinkedBlockingQueue<String>()
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context?, intent: Intent?) {
                if (intent?.action != BluetoothDevice.ACTION_FOUND) return
                @Suppress("DEPRECATION")
                val device = intent.getParcelableExtra<BluetoothDevice>(BluetoothDevice.EXTRA_DEVICE)
                    ?: return
                val name = try {
                    device.name ?: ""
                } catch (_: SecurityException) {
                    ""
                }
                queue.offer("${device.address}\t$name")
            }
        }
        try {
            val filter = IntentFilter(BluetoothDevice.ACTION_FOUND)
            if (Build.VERSION.SDK_INT >= 33) {
                registerReceiver(receiver, filter, Context.RECEIVER_EXPORTED)
            } else {
                @Suppress("UnspecifiedRegisterReceiverFlag")
                registerReceiver(receiver, filter)
            }
            adapter.cancelDiscovery()
            val started = adapter.startDiscovery()
            Log.i("siren", "bt: startDiscovery=$started")
            val deadline = System.currentTimeMillis() + timeoutMs
            while (System.currentTimeMillis() < deadline) {
                val item = queue.poll(200, TimeUnit.MILLISECONDS) ?: continue
                results.add(item)
            }
            adapter.cancelDiscovery()
        } catch (error: Throwable) {
            Log.w("siren", "bt: scan failed: ${error.message}")
        } finally {
            try {
                unregisterReceiver(receiver)
            } catch (_: Throwable) {
            }
        }
        Log.i("siren", "bt: inquiry found ${results.size} device(s)")
        return results.joinToString("\n")
    }

    /** Private storage for the remembered hub (offline join). */
    @Keep
    fun stateDir(): String = filesDir.absolutePath

    /** Local adapter address; masked addresses are treated as unavailable. */
    @Keep
    fun btLocalAddress(): String {
        if (!btPermissionGranted()) {
            requestBtPermission()
            return ""
        }
        return try {
            val adapter = getSystemService(BluetoothManager::class.java)?.adapter
                ?: return ""
            adapter.address ?: ""
        } catch (error: Throwable) {
            Log.w("siren", "bt: local address unavailable: ${error.message}")
            ""
        }
    }

    /**
     * Called from the Rust supervisor for every (re)connect attempt. Dials the
     * elected hub and exposes the RFCOMM socket on 127.0.0.1:bridgePort; the
     * Rust side then connects to that port and owns the framed tunnel.
     */
    @Keep
    fun startBtBridge(mac: String, channel: Int, bridgePort: Int): Boolean {
        if (!btPermissionGranted()) {
            requestBtPermission()
            return false
        }
        synchronized(btLock) {
            btGeneration++
            val generation = btGeneration
            closeBtBridgeLocked()
            return try {
                val adapter = getSystemService(BluetoothManager::class.java)?.adapter
                    ?: error("no Bluetooth adapter")
                if (!adapter.isEnabled) error("Bluetooth is off")
                val device = adapter.getRemoteDevice(mac)
                try {
                    adapter.cancelDiscovery()
                } catch (_: SecurityException) {
                }
                var socket = createInsecureSocket(device, channel)
                Log.i("siren", "bt: dialing $mac channel $channel")
                try {
                    socket.connect()
                } catch (first: Throwable) {
                    // The channel-16 socket sometimes refuses right after a
                    // previous link was torn down; retry once over SPP.
                    Log.w("siren", "bt: channel connect failed (${first.message}), trying SPP")
                    try {
                        socket.close()
                    } catch (_: Throwable) {
                    }
                    socket = device.createInsecureRfcommSocketToServiceRecord(
                        UUID.fromString(SPP_UUID),
                    )
                    socket.connect()
                }
                btFailures = 0
                val server = ServerSocket()
                server.reuseAddress = true
                server.bind(InetSocketAddress(InetAddress.getByName("127.0.0.1"), bridgePort))
                btRfcomm = socket
                btServer = server
                btThread = thread(name = "siren-bt-bridge", start = true) {
                    var tcp: Socket? = null
                    try {
                        tcp = server.accept()
                        btTcp = tcp
                        Log.i("siren", "bt: bridge up on 127.0.0.1:$bridgePort")
                        val up = thread(name = "siren-bt-up", start = true) {
                            pump(socket.inputStream, tcp.outputStream)
                        }
                        val down = thread(name = "siren-bt-down", start = true) {
                            pump(tcp.inputStream, socket.outputStream)
                        }
                        up.join()
                        down.join()
                    } catch (error: Throwable) {
                        Log.w("siren", "bt: bridge ended: ${error.message}")
                    } finally {
                        closeBtBridgeIf(generation)
                    }
                }
                true
            } catch (error: Throwable) {
                Log.e("siren", "bt: dial failed", error)
                closeBtBridgeLocked()
                false
            }
        }
    }

    private fun createInsecureSocket(device: BluetoothDevice, channel: Int): BluetoothSocket {
        return try {
            val method = device.javaClass.getMethod("createInsecureRfcommSocket", Integer.TYPE)
            method.invoke(device, channel) as BluetoothSocket
        } catch (error: Throwable) {
            Log.w("siren", "bt: channel API unavailable, using SPP UUID", error)
            device.createInsecureRfcommSocketToServiceRecord(UUID.fromString(SPP_UUID))
        }
    }

    private fun pump(input: java.io.InputStream, output: java.io.OutputStream) {
        val buffer = ByteArray(8192)
        try {
            while (true) {
                val read = input.read(buffer)
                if (read <= 0) break
                output.write(buffer, 0, read)
                output.flush()
            }
        } catch (error: Throwable) {
            Log.w("siren", "bt: pump ended: ${error.message}")
        }
    }

    private fun closeBtBridgeIf(generation: Int) {
        synchronized(btLock) {
            if (generation == btGeneration) closeBtBridgeLocked()
        }
    }

    private fun closeBtBridgeLocked() {
        try {
            btTcp?.close()
        } catch (_: Exception) {
        }
        try {
            btServer?.close()
        } catch (_: Exception) {
        }
        try {
            btRfcomm?.close()
        } catch (_: Exception) {
        }
        btTcp = null
        btServer = null
        btRfcomm = null
        btThread = null
    }

    // ---- capture permission -------------------------------------------

    /** Called from Rust before any capture starts. */
    @Keep
    fun ensureCapturePermission(): Boolean {
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            return true
        }
        requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), REQ_RECORD_AUDIO)
        return false
    }

    // ---- system audio capture (AudioPlaybackCapture) -------------------

    /** Called from Rust; shows the MediaProjection consent dialog. */
    @Keep
    fun startSystemCapture(sampleRate: Int): Boolean {
        if (!ensureCapturePermission()) {
            // The permission dialog and the projection dialog must not stack.
            return false
        }
        val manager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        startActivityForResult(manager.createScreenCaptureIntent(), REQ_PROJECTION)
        return true
    }

    @Keep
    fun stopSystemCapture() {
        stopService(Intent(this, CaptureService::class.java))
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode == REQ_PROJECTION) {
            if (resultCode == Activity.RESULT_OK && data != null) {
                // Android 14: the projection must be held by a foreground
                // service of type mediaProjection.
                val service = Intent(this, CaptureService::class.java)
                    .putExtra(CaptureService.EXTRA_RESULT_CODE, resultCode)
                    .putExtra(CaptureService.EXTRA_RESULT_DATA, data)
                startForegroundService(service)
            }
            return
        }
        super.onActivityResult(requestCode, resultCode, data)
    }

    // ---- Wi-Fi locks ---------------------------------------------------

    override fun onStart() {
        super.onStart()
        // mDNS discovery needs multicast reception, which Android blocks by
        // default on Wi-Fi. Hold a multicast lock for the process lifetime.
        if (multicastLock == null) {
            val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            multicastLock = try {
                wifi.createMulticastLock("siren_mdns").apply {
                    setReferenceCounted(false)
                    acquire()
                }
            } catch (error: SecurityException) {
                Log.w("siren", "multicast lock refused: $error")
                null
            }
            // A mesh node pushes 5 ms frames: in power-save mode the NIC sleeps
            // between beacons and every second packet arrives late, which the
            // receiver reads as jitter and turns into underruns. The
            // low-latency lock keeps the interface awake while the app lives.
            wifiLock = try {
                wifi.createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "siren_mesh").apply {
                    setReferenceCounted(false)
                    acquire()
                }
            } catch (error: SecurityException) {
                // A ROM that refuses the lock must not take the whole node down.
                Log.w("siren", "wifi lock refused: $error")
                null
            }
        }
    }

    override fun onDestroy() {
        stopSystemCapture()
        synchronized(btLock) {
            btGeneration++
            closeBtBridgeLocked()
        }
        wifiLock?.let {
            if (it.isHeld) it.release()
        }
        wifiLock = null
        multicastLock?.let {
            if (it.isHeld) it.release()
        }
        multicastLock = null
        super.onDestroy()
    }
}