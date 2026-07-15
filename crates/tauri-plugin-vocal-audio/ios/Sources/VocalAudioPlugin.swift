import AVFoundation
import Tauri
import UIKit
import WebKit

class ConfigureSessionArgs: Decodable {
    var playback: Bool?
}

class KeepScreenOnArgs: Decodable {
    var enabled: Bool
}

class ExcludeArgs: Decodable {
    var paths: [String]
}

/**
 * 오디오 환경 플러그인 (가드레일: AVAudioSession .playAndRecord + .measurement
 * 를 플러그인에서 직접 설정, 인터럽션/라우트 변경 이벤트, 화면 유지,
 * 헤드폰 라우트 감지, 백업 제외 플래그).
 *
 * 이벤트: interruption { phase, shouldResume } / routeChanged { headphones, description }.
 * iOS 백그라운드 오디오는 UIBackgroundModes(audio)로 처리 —
 * start/stopForegroundService 는 Android 대응용 no-op.
 */
class VocalAudioPlugin: Plugin {
    override func load(webview: WKWebView) {
        super.load(webview: webview)
        let center = NotificationCenter.default
        center.addObserver(
            self,
            selector: #selector(handleInterruption),
            name: AVAudioSession.interruptionNotification,
            object: nil)
        center.addObserver(
            self,
            selector: #selector(handleRouteChange),
            name: AVAudioSession.routeChangeNotification,
            object: nil)
    }

    @objc private func handleInterruption(notification: Notification) {
        guard let info = notification.userInfo,
            let typeValue = info[AVAudioSessionInterruptionTypeKey] as? UInt,
            let type = AVAudioSession.InterruptionType(rawValue: typeValue)
        else { return }
        var shouldResume = false
        if type == .ended,
            let optionsValue = info[AVAudioSessionInterruptionOptionKey] as? UInt
        {
            shouldResume = AVAudioSession.InterruptionOptions(rawValue: optionsValue)
                .contains(.shouldResume)
        }
        trigger(
            "interruption",
            data: [
                "phase": type == .began ? "began" : "ended",
                "shouldResume": shouldResume,
            ])
    }

    @objc private func handleRouteChange(notification: Notification) {
        trigger("routeChanged", data: routePayload())
    }

    private func headphonePorts() -> [AVAudioSession.Port] {
        var ports: [AVAudioSession.Port] = [
            .headphones, .bluetoothA2DP, .bluetoothHFP, .usbAudio,
        ]
        if #available(iOS 14.0, *) {
            // BLE 오디오 (AirPods 등은 A2DP로 잡히지만 방어적으로 포함)
            ports.append(.bluetoothLE)
        }
        return ports
    }

    private func routePayload() -> JsonObject {
        let outputs = AVAudioSession.sharedInstance().currentRoute.outputs
        let hp = headphonePorts()
        let headphones = outputs.contains { hp.contains($0.portType) }
        let description = outputs.map { $0.portType.rawValue }.joined(separator: "+")
        return [
            "headphones": headphones,
            "description": description.isEmpty ? "none" : description,
        ]
    }

    @objc public override func checkPermissions(_ invoke: Invoke) {
        let state: String
        switch AVAudioSession.sharedInstance().recordPermission {
        case .granted: state = "granted"
        case .denied: state = "denied"
        default: state = "prompt"
        }
        invoke.resolve(["microphone": state])
    }

    @objc public override func requestPermissions(_ invoke: Invoke) {
        AVAudioSession.sharedInstance().requestRecordPermission { granted in
            invoke.resolve(["microphone": granted ? "granted" : "denied"])
        }
    }

    @objc public func configureSession(_ invoke: Invoke) throws {
        _ = try? invoke.parseArgs(ConfigureSessionArgs.self)
        let session = AVAudioSession.sharedInstance()
        // .measurement: OS 신호처리(AGC 등) 최소화 — 피치 분석 입력 왜곡 방지.
        try session.setCategory(
            .playAndRecord,
            mode: .measurement,
            options: [.allowBluetoothA2DP, .defaultToSpeaker])
        try session.setPreferredSampleRate(48_000)
        // 지연 예산(§4)을 위한 낮은 IO 버퍼 (실기기에서 실효 확인 필요).
        try session.setPreferredIOBufferDuration(0.010)
        try session.setActive(true)
        invoke.resolve()
    }

    @objc public func releaseSession(_ invoke: Invoke) throws {
        try AVAudioSession.sharedInstance().setActive(
            false, options: .notifyOthersOnDeactivation)
        invoke.resolve()
    }

    @objc public func setKeepScreenOn(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(KeepScreenOnArgs.self)
        DispatchQueue.main.async {
            UIApplication.shared.isIdleTimerDisabled = args.enabled
        }
        invoke.resolve()
    }

    @objc public func currentRoute(_ invoke: Invoke) {
        invoke.resolve(routePayload())
    }

    @objc public func startForegroundService(_ invoke: Invoke) {
        invoke.resolve()
    }

    @objc public func stopForegroundService(_ invoke: Invoke) {
        invoke.resolve()
    }

    @objc public func excludeFromBackup(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(ExcludeArgs.self)
        for path in args.paths {
            var url = URL(fileURLWithPath: path)
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? url.setResourceValues(values)
        }
        invoke.resolve()
    }
}

@_cdecl("init_plugin_vocal_audio")
func initPlugin() -> Plugin {
    return VocalAudioPlugin()
}
