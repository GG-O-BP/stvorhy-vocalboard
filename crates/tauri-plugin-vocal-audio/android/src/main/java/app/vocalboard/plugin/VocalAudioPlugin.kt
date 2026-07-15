package app.vocalboard.plugin

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Build
import android.view.WindowManager
import android.webkit.WebView
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class SessionConfigArgs {
    var playback: Boolean = false
}

@InvokeArg
class KeepScreenOnArgs {
    var enabled: Boolean = false
}

@InvokeArg
class ExcludeArgs {
    var paths: List<String> = listOf()
}

/**
 * 오디오 환경 플러그인 (가드레일: RECORD_AUDIO 런타임 권한, 오디오 포커스,
 * FOREGROUND_SERVICE_MICROPHONE, FLAG_KEEP_SCREEN_ON, 헤드폰 라우트 감지).
 *
 * 이벤트: focusChanged / routeChanged. 인터럽션(전화 등)은 Android에서
 * 포커스 손실로 나타나므로 focusChanged로 일원화한다.
 */
@TauriPlugin(
    permissions = [
        Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = "microphone")
    ]
)
class VocalAudioPlugin(private val activity: Activity) : Plugin(activity) {
    private var focusRequest: AudioFocusRequest? = null
    private var deviceCallback: AudioDeviceCallback? = null

    private val audioManager: AudioManager
        get() = activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager

    override fun load(webView: WebView) {
        super.load(webView)
        // 라우트 변경 감시 → routeChanged 이벤트.
        val cb = object : AudioDeviceCallback() {
            override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>?) = emitRoute()
            override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>?) = emitRoute()
        }
        deviceCallback = cb
        audioManager.registerAudioDeviceCallback(cb, null)
    }

    private fun headphoneTypes(): Set<Int> {
        val types = mutableSetOf(
            AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
            AudioDeviceInfo.TYPE_WIRED_HEADSET,
            AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
            AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
            AudioDeviceInfo.TYPE_USB_HEADSET,
        )
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            types.add(AudioDeviceInfo.TYPE_BLE_HEADSET)
        }
        return types
    }

    private fun routeInfo(): JSObject {
        val outputs = audioManager.getDevices(AudioManager.GET_DEVICES_OUTPUTS)
        val hpTypes = headphoneTypes()
        val headphones = outputs.any { it.type in hpTypes }
        val desc = outputs.joinToString("+") { deviceTypeName(it.type) }
        val obj = JSObject()
        obj.put("headphones", headphones)
        obj.put("description", if (desc.isEmpty()) "none" else desc)
        return obj
    }

    private fun deviceTypeName(type: Int): String = when (type) {
        AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "speaker"
        AudioDeviceInfo.TYPE_WIRED_HEADPHONES -> "wired-headphones"
        AudioDeviceInfo.TYPE_WIRED_HEADSET -> "wired-headset"
        AudioDeviceInfo.TYPE_BLUETOOTH_A2DP -> "bt-a2dp"
        AudioDeviceInfo.TYPE_BLUETOOTH_SCO -> "bt-sco"
        AudioDeviceInfo.TYPE_USB_HEADSET -> "usb-headset"
        AudioDeviceInfo.TYPE_BUILTIN_EARPIECE -> "earpiece"
        else -> "type-$type"
    }

    private fun emitRoute() {
        trigger("routeChanged", routeInfo())
    }

    @Command
    fun configureSession(invoke: Invoke) {
        invoke.parseArgs(SessionConfigArgs::class.java)
        val attrs = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()
        val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(attrs)
            .setAcceptsDelayedFocusGain(false)
            .setOnAudioFocusChangeListener { change ->
                val state = when (change) {
                    AudioManager.AUDIOFOCUS_GAIN -> "gain"
                    AudioManager.AUDIOFOCUS_LOSS -> "loss"
                    AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> "lossTransient"
                    AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> "lossTransientCanDuck"
                    else -> "unknown"
                }
                val obj = JSObject()
                obj.put("state", state)
                trigger("focusChanged", obj)
            }
            .build()
        focusRequest = request
        val result = audioManager.requestAudioFocus(request)
        if (result == AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
            invoke.resolve()
        } else {
            invoke.reject("오디오 포커스 획득 실패")
        }
    }

    @Command
    fun releaseSession(invoke: Invoke) {
        focusRequest?.let { audioManager.abandonAudioFocusRequest(it) }
        focusRequest = null
        invoke.resolve()
    }

    @Command
    fun setKeepScreenOn(invoke: Invoke) {
        val args = invoke.parseArgs(KeepScreenOnArgs::class.java)
        activity.runOnUiThread {
            if (args.enabled) {
                activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            } else {
                activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            }
        }
        invoke.resolve()
    }

    @Command
    fun currentRoute(invoke: Invoke) {
        invoke.resolve(routeInfo())
    }

    @Command
    fun startForegroundService(invoke: Invoke) {
        val intent = Intent(activity, CaptureForegroundService::class.java)
        ContextCompat.startForegroundService(activity, intent)
        invoke.resolve()
    }

    @Command
    fun stopForegroundService(invoke: Invoke) {
        activity.stopService(Intent(activity, CaptureForegroundService::class.java))
        invoke.resolve()
    }

    @Command
    fun excludeFromBackup(invoke: Invoke) {
        // Android 백업 제외는 앱 매니페스트의 backup rules(fullBackupContent /
        // dataExtractionRules)로 처리된다 — gen/android 설정 참조. no-op 성공.
        invoke.parseArgs(ExcludeArgs::class.java)
        invoke.resolve()
    }
}
