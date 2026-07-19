package app.vocalboard.plugin

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * 백그라운드(화면 꺼짐/앱 전환) 중 마이크 캡처를 유지하는 포그라운드 서비스.
 * API 34+에서는 microphone 타입 명시가 필수 (매니페스트와 일치).
 */
class CaptureForegroundService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val channelId = "vocalboard-capture"
        val manager = getSystemService(NotificationManager::class.java)
        val channel = NotificationChannel(
            channelId,
            "Vocalboard 캡처",
            NotificationManager.IMPORTANCE_LOW,
        )
        channel.description = "피치 모니터링 진행 중"
        manager.createNotificationChannel(channel)
        val notification: Notification = Notification.Builder(this, channelId)
            .setContentTitle("Vocalboard")
            .setContentText("피치 모니터링 진행 중")
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setOngoing(true)
            .build()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        return START_STICKY
    }

    companion object {
        private const val NOTIFICATION_ID = 0x5601
    }
}
